use alloc::{boxed::Box, format, string::String, vec::Vec};
use core::{
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    iter,
    mem::size_of,
};
use java_constants::FieldAccessFlags;

use jvm::{ClassDefinition, ClassInstance, Field, JavaType, JavaValue, Result as JvmResult};
use wipi_types::ktf::java::JavaClassInstance as RawJavaClassInstance;

use wie_core_arm::{Allocator, ArmCore};
use wie_util::{ByteWrite, WieError, read_generic, write_generic};

use crate::runtime::java::jvm_support::KtfJvmSupport;

use super::{KtfJvmWord, Result, class_definition::JavaClassDefinition, field::JavaField, liveness, name::JavaFullName, value::JavaValueExt};

#[derive(Clone)]
pub struct JavaClassInstance {
    pub ptr_raw: u32,
    core: ArmCore,
}

impl JavaClassInstance {
    pub fn from_raw(ptr_raw: u32, core: &ArmCore) -> Self {
        Self { ptr_raw, core: core.clone() }
    }

    pub fn new(core: &mut ArmCore, class: &JavaClassDefinition) -> Result<Self> {
        let field_size = class.field_size()?;

        let instance = Self::instantiate(core, class, field_size)?;

        tracing::trace!("Instantiated {} at {:#x}", class.name()?, instance.ptr_raw);

        Ok(instance)
    }

    pub fn destroy(mut self, field_size: KtfJvmWord) -> Result<()> {
        let raw = self.read_raw()?;

        // A second destroy of the same instance would push `ptr_fields` - by now a free-chain
        // slot INDEX, not an address - back into the allocator, and `ptr_raw` onto its chain
        // twice. Both are guarded downstream (the bucket allocator leaks a non-slot address and
        // refuses a double free), but the guard fires as an `error!` far from the cause. Refuse
        // here instead, where we can still name the instance.
        if !liveness::fields_pointer_is_live(raw.ptr_fields) {
            liveness::report("destroy", self.ptr_raw, Some((raw.ptr_fields, raw.ptr_class)), || {
                format!("field_size={field_size:#x}")
            });
            return Ok(());
        }

        Allocator::free(&mut self.core, raw.ptr_fields, (field_size + 4) as _)?;
        Allocator::free(&mut self.core, self.ptr_raw, size_of::<RawJavaClassInstance>() as _)?;

        Ok(())
    }

    /// The `ptr_class` of this instance if the record still describes a LIVE object, else
    /// `None` — the test in `liveness` (a freed record's `ptr_fields` is a free-chain word,
    /// never a heap address), and nothing more: deliberately NOT the class name, because
    /// on a handle that was never an instance `ptr_class` is a random word and reading a
    /// name through it would `unwrap` non-UTF-8 and poison the class-name memo.
    ///
    /// For a HANDLE the host holds across ticks (`Emulator::call_java_method`): a heap
    /// address can have been freed, or freed and handed out again as something else,
    /// since the host read it, and the JVM's own accessors `unwrap()` on a bad record —
    /// inside a spawned task that is a worker-killing panic. So the caller asks this
    /// first and compares the pointer against the class it resolved BY NAME.
    pub fn live_class_ptr(&self) -> Option<u32> {
        let raw = self.read_raw().ok()?;
        liveness::fields_pointer_is_live(raw.ptr_fields).then_some(raw.ptr_class)
    }

    pub fn class(&self) -> Result<JavaClassDefinition> {
        let raw = self.read_raw()?;

        Ok(JavaClassDefinition::from_raw(raw.ptr_class, &self.core))
    }

    pub(super) fn field_address(&self, offset: u32) -> Result<u32> {
        let raw = self.live_raw("field_address", || format!("offset={offset:#x}"))?;

        Ok(raw.ptr_fields + offset + 4)
    }

    /// Reads the instance record and refuses it if the collector already freed it.
    ///
    /// This is the single choke point for the use-after-free described in `liveness`. It costs
    /// nothing on the live path: the record had to be read anyway, and the test is two
    /// comparisons on a word already in a register.
    fn live_raw(&self, what: &str, detail: impl FnOnce() -> String) -> Result<RawJavaClassInstance> {
        let raw = match self.read_raw() {
            Ok(x) => x,
            Err(e) => {
                // Not the use-after-free fingerprint - `ptr_raw` itself is unreadable, which
                // means the pointer never was an instance. Say so rather than blaming the GC.
                liveness::report(what, self.ptr_raw, None, detail);
                return Err(e);
            }
        };

        if liveness::fields_pointer_is_live(raw.ptr_fields) {
            return Ok(raw);
        }

        liveness::report(what, self.ptr_raw, Some((raw.ptr_fields, raw.ptr_class)), detail);

        Err(WieError::InvalidMemoryAccess(raw.ptr_fields))
    }

    /// The field address, or `None` once the collector has freed this instance - in which case
    /// it has already been reported, naming the class, the field and its descriptor. Those three
    /// are what turn "somebody is using a corpse" into "THIS reference was missed by the root
    /// set", so they are worth the reads: they only happen on the failing path.
    fn field_address_or_report(&self, offset: u32, name: &str, descriptor: &str, what: &str) -> Option<u32> {
        let instance = self.live_raw(what, || self.field_detail(name, descriptor)).ok()?;

        Some(instance.ptr_fields + offset + 4)
    }

    /// Only ever rendered on a failing access, so the two extra guest reads behind `class()` are
    /// free in every run that has nothing to report.
    fn field_detail(&self, name: &str, descriptor: &str) -> String {
        let class = self.class().and_then(|x| x.name()).unwrap_or_else(|_| String::from("<unreadable>"));

        format!("class={class}, field={name}:{descriptor}")
    }

    /// What a field of a freed instance reads as. Zero for every primitive, `null` for every
    /// reference - so the collector's mark walk stops at the corpse instead of recursing into
    /// whatever the free chain left in its field block.
    fn dead_value(&self, field_type: &JavaType) -> JavaValue {
        if matches!(field_type, JavaType::Long | JavaType::Double) {
            JavaValue::from_raw64(0, 0, field_type)
        } else {
            JavaValue::from_raw(0, field_type, &self.core)
        }
    }

    pub(super) fn instantiate(core: &mut ArmCore, class: &JavaClassDefinition, field_size: usize) -> Result<Self> {
        let ptr_raw = Allocator::alloc(core, size_of::<RawJavaClassInstance>() as _)?;
        let ptr_fields = Allocator::alloc(core, (field_size + 4) as _)?;

        let zero = iter::repeat_n(0, (field_size + 4) as _).collect::<Vec<_>>();
        core.write_bytes(ptr_fields, &zero)?;

        let vtable_index = KtfJvmSupport::get_vtable_index(core, class)?;

        write_generic(
            core,
            ptr_raw,
            RawJavaClassInstance {
                ptr_fields,
                ptr_class: class.ptr_raw,
            },
        )?;
        write_generic(core, ptr_fields, (vtable_index * 4) << 5)?;

        tracing::trace!("Instantiate {}, vtable_index {vtable_index:#x} at {ptr_raw:#x}", class.name()?);

        Ok(Self::from_raw(ptr_raw, core))
    }

    fn read_raw(&self) -> Result<RawJavaClassInstance> {
        let instance: RawJavaClassInstance = read_generic(&self.core, self.ptr_raw as _)?;

        Ok(instance)
    }
}

#[async_trait::async_trait]
impl ClassInstance for JavaClassInstance {
    fn destroy(self: Box<Self>) {
        // Every step here can fail on an instance the collector has already freed, and this
        // signature has no error channel - so a failure used to be a worker-killing panic on the
        // teardown path. Leaking the two blocks is strictly better: the allocator stays
        // consistent and the app keeps running.
        let field_size = match self.class().and_then(|x| x.field_size()) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("Could not size {:#x} to destroy it, leaking it: {e:?}", self.ptr_raw);
                return;
            }
        };

        if let Err(e) = (*self).destroy(field_size as _) {
            tracing::error!("Could not destroy a java instance, leaking it: {e:?}");
        }
    }

    fn class_definition(&self) -> Box<dyn ClassDefinition> {
        Box::new(self.class().unwrap())
    }

    fn equals(&self, other: &dyn ClassInstance) -> JvmResult<bool> {
        let other = other.as_any().downcast_ref::<JavaClassInstance>();
        if other.is_none() {
            return Ok(false);
        }

        Ok(self.ptr_raw == other.unwrap().ptr_raw)
    }

    // ★ ONE guest read of the field record, ONE descriptor fetch, ONE type parse
    // (2026-09-09). This used to call `field.descriptor()` TWICE and `access_flags()`
    // and `offset()` once each - four separate 16-byte reads out of guest memory (each
    // a core-mutex round trip), two `String` clones out of the name memo, and the
    // descriptor parsed twice - for every single instance field read. hero2 does about
    // 1,200 by-name member resolutions per frame, so this is on the hot path.
    fn get_field(&self, field: &dyn Field) -> JvmResult<JavaValue> {
        let field = field.as_any().downcast_ref::<JavaField>().unwrap();
        let raw = field.raw().unwrap();
        // ⛔ `assert!`, not `debug_assert!`. A STATIC field's `offset_or_value` is the VALUE,
        // not an offset, so letting one through here computes a garbage address and the read
        // below fails with a confusing `unwrap` panic instead of naming the real mistake.
        // (This guard was briefly weakened to `debug_assert!` on 2026-09-09 while collapsing
        // the redundant reads; that was wrong and is why it is spelled out now.)
        assert!(!FieldAccessFlags::from_bits_truncate(raw.access_flags as _).contains(FieldAccessFlags::STATIC));

        let name = field.name_of(&raw).unwrap();

        Ok(self.read_field_at(raw.offset_or_value, &name))
    }

    /// ★ The by-name read without the two boxed trait objects the default costs (2026-09-19):
    /// one guest read for the instance record, a memo walk for the field, one guest read for
    /// the value. `Ok(None)` = no such field, and the JVM turns that into `NoSuchFieldError`
    /// exactly as before.
    fn get_field_by_name(&self, _: &jvm::Jvm, name: &str, descriptor: &str) -> JvmResult<Option<JavaValue>> {
        let Some((offset, full_name)) = self.class().unwrap().find_instance_field(name, descriptor).unwrap() else {
            return Ok(None);
        };

        Ok(Some(self.read_field_at(offset, &full_name)))
    }

    fn put_field_by_name(&mut self, _: &jvm::Jvm, name: &str, descriptor: &str, value: JavaValue) -> JvmResult<bool> {
        let Some((offset, full_name)) = self.class().unwrap().find_instance_field(name, descriptor).unwrap() else {
            return Ok(false);
        };

        self.write_field_at(offset, &full_name, value);

        Ok(true)
    }

    fn gc_identity(&self) -> u64 {
        self.ptr_raw as u64
    }

    /// The instance record's own guest address — what a conservative scan of the guest's
    /// registers and stacks finds. See `ArmCore::scan_conservative_roots`.
    ///
    /// Same value as [`Self::gc_identity`] here, and deliberately a separate method: the
    /// identity is allowed to collide (the default one hashes), an address never may.
    fn gc_raw_address(&self) -> Option<u64> {
        Some(self.ptr_raw as u64)
    }

    fn gc_visit_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        self.gc_visit_references_raw(visit);

        true
    }

    fn put_field(&mut self, field: &dyn Field, value: JavaValue) -> JvmResult<()> {
        let field = field.as_any().downcast_ref::<JavaField>().unwrap();
        let raw = field.raw().unwrap();
        // ⛔ `assert!`, not `debug_assert!`. A STATIC field's `offset_or_value` is the VALUE,
        // not an offset, so letting one through here computes a garbage address and the read
        // below fails with a confusing `unwrap` panic instead of naming the real mistake.
        // (This guard was briefly weakened to `debug_assert!` on 2026-09-09 while collapsing
        // the redundant reads; that was wrong and is why it is spelled out now.)
        assert!(!FieldAccessFlags::from_bits_truncate(raw.access_flags as _).contains(FieldAccessFlags::STATIC));

        let name = field.name_of(&raw).unwrap();
        self.write_field_at(raw.offset_or_value, &name, value);

        Ok(())
    }
}

impl JavaClassInstance {
    /// ★ THE MARK WALK, on guest memory directly (2026-09-19, roadmap #3).
    ///
    /// Per object: one guest read for the instance record, one memo lookup for the class
    /// ([`ClassGcInfo`](super::class_definition::ClassGcInfo)), one guest read per reference
    /// field (or per element of an object array) - and no host allocation at all. The
    /// children are handed over as stack temporaries; the collector only takes their
    /// identity (`ptr_raw`) and recurses. This replaced a walk that boxed a class
    /// definition, a `String` name and a value per object: ~20,000 host allocations per
    /// collection, every painted frame.
    ///
    /// A corpse (the free-chain fingerprint in `ptr_fields`) is reported once and NOT walked
    /// - recursing into whatever the free chain left in its field block is how the 2026-09-09
    /// freeze started. The generic walk had the same stop, one field read later.
    pub(super) fn gc_visit_references_raw(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) {
        let Ok(raw) = self.read_raw() else {
            liveness::report("gc mark", self.ptr_raw, None, String::new);
            return;
        };
        if !liveness::fields_pointer_is_live(raw.ptr_fields) {
            liveness::report("gc mark", self.ptr_raw, Some((raw.ptr_fields, raw.ptr_class)), String::new);
            return;
        }

        let Ok(info) = JavaClassDefinition::from_raw(raw.ptr_class, &self.core).gc_info() else {
            liveness::report_refusal("gc mark (class)", self.ptr_raw, String::new);
            return;
        };

        // Field block layout: word 0 is the vtable index, fields (or `length` + elements
        // for an array) start at +4 - the `+ 4` in `field_address`.
        let base = raw.ptr_fields + 4;
        if info.is_array {
            if !info.object_array {
                return;
            }
            let Ok(length) = read_generic::<u32, _>(&self.core, base) else {
                liveness::report_access("gc mark (array length)", self.ptr_raw, base, String::new);
                return;
            };
            for i in 0..length {
                let address = base + 4 + i * 4;
                match read_generic::<u32, _>(&self.core, address) {
                    Ok(0) => {}
                    Ok(child) => visit(&JavaClassInstance::from_raw(child, &self.core)),
                    Err(_) => {
                        liveness::report_access("gc mark (array element)", self.ptr_raw, address, String::new);
                        return;
                    }
                }
            }
            return;
        }

        for &offset in info.ref_offsets.iter() {
            let address = base + offset;
            match read_generic::<u32, _>(&self.core, address) {
                Ok(0) => {}
                Ok(child) => visit(&JavaClassInstance::from_raw(child, &self.core)),
                // The generic walk skipped an unreadable field and went on; so does this.
                Err(_) => liveness::report_access("gc mark (field)", self.ptr_raw, address, String::new),
            }
        }
    }

    /// The instance-field read both `get_field` and `get_field_by_name` end in: the field
    /// block address (with the liveness check), then ONE guest read on the scalar path and
    /// two for Long/Double.
    fn read_field_at(&self, offset: u32, name: &JavaFullName) -> JavaValue {
        let field_type = name.r#type();

        // ⛔ This used to be `self.field_address(...).unwrap()`, and the read below is the
        // statement the 2026-09-09 device freeze panicked on. `JvmResult`'s only error is a java
        // exception, which a sync method cannot build, so a dead instance reads as zero/null and
        // says so in the log once. See `liveness` for why a dead instance gets this far at all.
        let address = match self.field_address_or_report(offset, &name.name, &name.descriptor, "get_field") {
            Some(x) => x,
            None => return self.dead_value(field_type),
        };

        // ⚠ Still exactly ONE guest read on the scalar path, and two only for Long/Double - the
        // 2026-09-09 collapse of the redundant reads is what this hot path costs its 7 ms/frame on.
        if matches!(field_type, JavaType::Long | JavaType::Double) {
            let lo: Result<KtfJvmWord> = read_generic(&self.core, address);
            let hi: Result<KtfJvmWord> = read_generic(&self.core, address + 4);

            let (Ok(value), Ok(value_high)) = (lo, hi) else {
                liveness::report_access("get_field", self.ptr_raw, address, || self.field_detail(&name.name, &name.descriptor));
                return self.dead_value(field_type);
            };

            JavaValue::from_raw64(value, value_high, field_type)
        } else {
            let value: Result<KtfJvmWord> = read_generic(&self.core, address);

            let Ok(value) = value else {
                liveness::report_access("get_field", self.ptr_raw, address, || self.field_detail(&name.name, &name.descriptor));
                return self.dead_value(field_type);
            };

            JavaValue::from_raw(value, field_type, &self.core)
        }
    }

    /// The instance-field write both `put_field` and `put_field_by_name` end in.
    fn write_field_at(&mut self, offset: u32, name: &JavaFullName, value: JavaValue) {
        let field_type = name.r#type();

        // Dropping the write is the only option left once the instance is gone - writing through
        // a free-chain index would corrupt the allocator's own bookkeeping. Same reasoning as
        // `read_field_at` above; `--gc 0` used to die on this statement rather than that one.
        let Some(address) = self.field_address_or_report(offset, &name.name, &name.descriptor, "put_field") else {
            return;
        };

        let written = if matches!(field_type, JavaType::Long | JavaType::Double) {
            let (value, value_high) = value.as_raw64();

            write_generic(&mut self.core, address, value).and_then(|_| write_generic(&mut self.core, address + 4, value_high))
        } else {
            write_generic(&mut self.core, address, value.as_raw())
        };

        if written.is_err() {
            liveness::report_access("put_field", self.ptr_raw, address, || self.field_detail(&name.name, &name.descriptor));
        }
    }
}

impl Debug for JavaClassInstance {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.ptr_raw)
    }
}

impl Hash for JavaClassInstance {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ptr_raw.hash(state)
    }
}
