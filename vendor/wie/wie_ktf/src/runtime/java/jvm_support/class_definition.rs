use alloc::{boxed::Box, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};
use core::{
    fmt::{self, Debug, Formatter},
    mem::size_of,
    ops::{Deref, DerefMut},
};

use java_class_proto::JavaClassProto;
use java_constants::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};
use jvm::{ClassDefinition, ClassInstance, Field, JavaType, JavaValue, Jvm, Method, Result as JvmResult};
use wipi_types::ktf::java::{JavaClass as RawJavaClass, JavaClassDescriptor as RawJavaClassDescriptor};

use wie_core_arm::{Allocator, ArmCore};
use wie_util::{
    WieError, read_generic, read_null_terminated_string_bytes, read_null_terminated_table, write_generic, write_null_terminated_string_bytes,
    write_null_terminated_table,
};

use crate::runtime::java::JavaSvcFunctions;

use super::{
    KtfJvmWord, Result, class_instance::JavaClassInstance, field::JavaField, method::JavaMethod, name::JavaFullName, oom, value::JavaValueExt,
    vtable::JavaVtable,
};

/// One member of a class, with everything a by-name lookup needs already resolved.
///
/// The point is that none of these come from guest memory once the table is built: the
/// old lookup re-read the 16-byte definition record AND the name record for every
/// candidate it compared.
struct MemberEntry {
    name: Arc<JavaFullName>,
    ptr_raw: u32,
    access_flags: u32,
    /// The record's `offset_or_value`: an instance field's offset into the field block, a
    /// static's value slot. Read once here so a by-name instance read needs no guest read
    /// for the definition at all.
    offset_or_value: u32,
}

struct ClassMembers {
    /// `ptr_parent_class`, 0 at the root - so the by-name walk up the chain is a memo
    /// lookup per level instead of two guest reads.
    parent: u32,
    /// This class's OWN methods. The guest's table carries the whole hierarchy and
    /// `ptr_class` is what separates them - the same filter the old `methods()` applied.
    methods: Vec<MemberEntry>,
    /// Fields, unfiltered: the guest's field table IS the flattened hierarchy and the
    /// JVM relies on that (a subclass instance must resolve a parent's field).
    fields: Vec<MemberEntry>,
}

/// CLASS-MEMBER TABLE MEMO (2026-09-09) - the second half of the hero2 frame cost.
///
/// With the name memo in `name.rs` in place, the remaining hot work was the SCAN
/// itself: `method()` / `field()` walked the class's NULL-terminated table on every
/// call - `read_null_terminated_table` (one guest read per entry plus a `Vec`), then per
/// candidate a 16-byte guest read for the definition record and a `JavaFullName` clone
/// (two `String` allocations) just to compare two `&str`s. Measured on hero2 (probe,
/// 1200 frames): 6.34 M compared entries, about 5,300 per frame, and none of it depends
/// on anything that ever changes.
///
/// Caching it turns a by-name member lookup into a walk over host-side `&str`s: no
/// guest reads, no allocations.
///
/// SAFETY, the same argument as `NAME_CACHE` and established by the same audit: no
/// `Allocator::free` site in the tree frees a `JavaClass`, a descriptor, or a
/// method/field definition, so a cached table stays that class's table. Nothing appends
/// to a class's tables after `JavaClassDefinition::new` writes them either - the guest's
/// own classes have theirs compiled into the image. The one hazard is a recycled
/// address, and `new` drops the entry for the address it is about to write.
static MEMBERS: spin::Mutex<BTreeMap<u32, Arc<ClassMembers>>> = spin::Mutex::new(BTreeMap::new());

/// CLASS-NAME MEMO (2026-09-18) - the third of these, and the one the COLLECTOR needed.
///
/// `name()` reads the class record, then its descriptor record, then walks the name out
/// of guest memory ONE BYTE AT A TIME (`read_null_terminated_string_bytes`) and builds a
/// `String`. That is fine at human rates; the garbage collector calls it for every live
/// object on every collection, which on this port is every painted frame. Measured with
/// the host phase profiler (roadmap #9): with the per-collection class cache already in
/// (`jvm/src/garbage_collector.rs`), `gc` was still 37 % of the emulator thread's busy
/// time, and this is what was left inside it - about 4,000 names a collection, ~16
/// collections a second, each a couple of dozen guest reads.
///
/// SAFETY: exactly the argument `MEMBERS` and `NAME_CACHE` are built on, and the same
/// audit backs it - nothing in the tree frees a `JavaClass` or its descriptor, so a
/// cached name stays that class's name. The one hazard is a RECYCLED address, and
/// `forget_members` (which `JavaClassDefinition::new` already calls for the address it is
/// about to write) drops this entry too.
static CLASS_NAMES: spin::Mutex<BTreeMap<u32, Arc<str>>> = spin::Mutex::new(BTreeMap::new());

/// What the collector's mark walk needs to know about a class - see
/// [`JavaClassInstance::gc_visit_references`](super::class_instance::JavaClassInstance).
pub struct ClassGcInfo {
    /// `[`-prefixed: the field block is `length` + elements, not fields.
    pub is_array: bool,
    /// `[L...` / `[[...`: the elements are references and the walk recurses into them.
    pub object_array: bool,
    /// Field-block offsets of every reference-typed instance field, the whole hierarchy,
    /// deduplicated and sorted. A primitive field can never point at anything, so the walk
    /// never reads one.
    pub ref_offsets: Vec<u32>,
    /// Guest addresses of this class's OWN reference-typed statics - the value slot inside
    /// each field record (`JavaField::static_address`).
    pub static_ref_addresses: Vec<u32>,
}

/// CLASS GC-INFO MEMO (2026-09-19, roadmap #3) - the collector's half of the member memo.
///
/// The JVM's mark walk used to rebuild a per-class "which fields are references" list once
/// per COLLECTION (`jvm/src/garbage_collector.rs`, 2026-09-18), keyed by class name - which
/// still cost the class-name `String`, a boxed `ClassDefinition` and a boxed value per
/// visited object. This memo is keyed by the class pointer and lives for the class, so the
/// walk is: read the instance record, look this up, read one word per reference field.
///
/// SAFETY: the same argument as `MEMBERS` - class metadata is never freed - and the same
/// hazard, closed the same way: `forget_members` drops it for a recycled address.
static GC_INFO: spin::Mutex<BTreeMap<u32, Arc<ClassGcInfo>>> = spin::Mutex::new(BTreeMap::new());

#[derive(Clone)]
pub struct JavaClassDefinition {
    pub ptr_raw: u32,
    core: ArmCore,
}

impl JavaClassDefinition {
    pub fn from_raw(ptr_raw: u32, core: &ArmCore) -> Self {
        Self { ptr_raw, core: core.clone() }
    }

    pub async fn new<C, Context>(
        core: &mut ArmCore,
        jvm: &Jvm,
        proto: JavaClassProto<C>,
        context: Context,
        java_functions: JavaSvcFunctions,
    ) -> Result<Self>
    where
        C: ?Sized + 'static + Send,
        Context: Deref<Target = C> + DerefMut + Clone + 'static + Sync + Send,
    {
        let parent_class = if let Some(x) = proto.parent_class {
            let class = jvm.resolve_class(x).await.unwrap().definition;
            let class = class.as_any().downcast_ref::<JavaClassDefinition>().unwrap().clone();

            Some(class)
        } else {
            None
        };

        let field_offset_base: u32 = if let Some(x) = &parent_class { x.field_size()? as _ } else { 0 };

        let ptr_raw = Allocator::alloc(core, size_of::<RawJavaClass>() as u32)?;
        // A recycled allocation must not serve the previous class's memoised member
        // tables. See the `MEMBERS` note.
        Self::forget_members(ptr_raw);

        let mut methods = Vec::new();
        for method in proto.methods.into_iter() {
            let method = JavaMethod::new(core, jvm, ptr_raw, method, context.clone(), java_functions.clone())?;

            methods.push(method.ptr_raw);
        }
        let ptr_methods = Allocator::alloc(core, ((methods.len() + 1) * size_of::<u32>()) as _)?;
        write_null_terminated_table(core, ptr_methods, &methods)?;

        let mut fields = Vec::new();
        let mut field_offset = field_offset_base;
        for field in proto.fields.into_iter() {
            let offset_or_value = if field.access_flags.contains(FieldAccessFlags::STATIC) {
                0
            } else {
                let field_size = if field.descriptor == "J" || field.descriptor == "D" { 8 } else { 4 };

                field_offset += field_size;

                field_offset - field_size
            };

            let field = JavaField::new(core, ptr_raw, field, offset_or_value)?;

            fields.push(field.ptr_raw);
        }
        let ptr_fields = Allocator::alloc(core, ((fields.len() + 1) * size_of::<u32>()) as _)?;
        write_null_terminated_table(core, ptr_fields, &fields)?;

        let ptr_name = Allocator::alloc(core, (proto.name.len() + 1) as u32)?;
        write_null_terminated_string_bytes(core, ptr_name, proto.name.as_bytes())?;

        let ptr_descriptor = Allocator::alloc(core, size_of::<RawJavaClassDescriptor>() as u32)?;
        write_generic(
            core,
            ptr_descriptor,
            RawJavaClassDescriptor {
                ptr_name,
                unk1: 0,
                ptr_parent_class: parent_class.map(|x| x.ptr_raw).unwrap_or(0),
                ptr_methods,
                ptr_interfaces: 0,
                ptr_fields_or_element_type: ptr_fields,
                method_count: methods.len() as u16,
                fields_size: field_offset as u16,
                access_flag: 0x21, // ACC_PUBLIC | ACC_SUPER
                unk6: 0,
                unk7: 0,
                unk8: 0,
            },
        )?;

        write_generic(
            core,
            ptr_raw,
            RawJavaClass {
                ptr_next: ptr_raw + 4,
                unk1: 0,
                ptr_descriptor,
                ptr_vtable: 0,
                vtable_count: 0,
                unk_flag: 8,
            },
        )?;

        let result = Self::from_raw(ptr_raw, core);
        let vtable = JavaVtable::new(core, &result)?; // this will also fill each method's vtable index

        // fill vtable info
        // TODO we can calculate it before allocating the class
        write_generic(core, ptr_raw + 12, vtable.ptr_raw)?;
        write_generic(core, ptr_raw + 16, vtable.len()? as u16)?;

        tracing::trace!("Wrote definition {} at {ptr_raw:#x}", proto.name);

        Ok(result)
    }

    pub fn read_class_hierarchy(&self) -> Result<Vec<JavaClassDefinition>> {
        let mut result = vec![];

        let mut current_class = self.ptr_raw;
        loop {
            result.push(JavaClassDefinition::from_raw(current_class, &self.core));

            let raw: RawJavaClass = read_generic(&self.core, current_class)?;
            let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor)?;

            if descriptor.ptr_parent_class != 0 {
                current_class = descriptor.ptr_parent_class;
            } else {
                break;
            }
        }

        Ok(result)
    }

    pub fn ptr_vtable(&self) -> Result<u32> {
        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw)?;

        Ok(raw.ptr_vtable)
    }

    pub fn field_size(&self) -> Result<usize> {
        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw)?;
        let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor)?;

        Ok(descriptor.fields_size as _)
    }

    /// Build (or fetch) this class's member tables. See [`MEMBERS`].
    fn members(&self) -> Result<Arc<ClassMembers>> {
        if let Some(hit) = MEMBERS.lock().get(&self.ptr_raw) {
            return Ok(hit.clone());
        }

        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw)?;
        let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor)?;

        let mut methods = Vec::new();
        if descriptor.ptr_methods != 0 {
            for ptr in read_null_terminated_table(&self.core, descriptor.ptr_methods)? {
                let method = JavaMethod::from_raw(ptr, &self.core);
                // Own methods only - the guest's table carries the hierarchy.
                if method.ptr_class() != self.ptr_raw {
                    continue;
                }
                methods.push(MemberEntry {
                    name: method.name_shared()?,
                    ptr_raw: ptr,
                    access_flags: method.access_flags().bits() as _,
                    offset_or_value: 0,
                });
            }
        }

        let mut fields = Vec::new();
        // An array class reuses `ptr_fields_or_element_type` for its element type, so it
        // must not be walked as a table. Same guard the old `fields()` had.
        if descriptor.ptr_fields_or_element_type != 0 && !self.is_array()? {
            for ptr in read_null_terminated_table(&self.core, descriptor.ptr_fields_or_element_type)? {
                let field = JavaField::from_raw(ptr, &self.core);
                let raw = field.raw()?;
                fields.push(MemberEntry {
                    name: field.name_of(&raw)?,
                    ptr_raw: ptr,
                    access_flags: raw.access_flags as _,
                    offset_or_value: raw.offset_or_value,
                });
            }
        }

        let built = Arc::new(ClassMembers {
            parent: descriptor.ptr_parent_class,
            methods,
            fields,
        });
        MEMBERS.lock().insert(self.ptr_raw, built.clone());

        Ok(built)
    }

    /// Drop the memoised tables for `ptr_class`, so a recycled allocation cannot serve
    /// the previous class's members.
    pub fn forget_members(ptr_class: u32) {
        MEMBERS.lock().remove(&ptr_class);
        CLASS_NAMES.lock().remove(&ptr_class);
        GC_INFO.lock().remove(&ptr_class);
    }

    /// Build (or fetch) the collector's view of this class. See [`GC_INFO`].
    pub fn gc_info(&self) -> Result<Arc<ClassGcInfo>> {
        if let Some(hit) = GC_INFO.lock().get(&self.ptr_raw) {
            return Ok(hit.clone());
        }

        let name = self.name_shared()?;
        let is_array = name.starts_with('[');
        let object_array = name.starts_with("[L") || name.starts_with("[[");

        let mut ref_offsets = Vec::new();
        let mut static_ref_addresses = Vec::new();
        if !is_array {
            for f in self.members()?.fields.iter() {
                let is_static = FieldAccessFlags::from_bits_truncate(f.access_flags as _).contains(FieldAccessFlags::STATIC);
                if is_static && matches!(f.name.r#type(), JavaType::Class(_) | JavaType::Array(_)) {
                    static_ref_addresses.push(JavaField::from_raw(f.ptr_raw, &self.core).static_address()?);
                }
            }

            // The guest's own classes carry the flattened hierarchy in their table and
            // host-registered ones list only their own fields; walking the chain covers both,
            // and the dedup below absorbs the overlap.
            let mut class = self.clone();
            loop {
                let members = class.members()?;
                for f in members.fields.iter() {
                    let is_static = FieldAccessFlags::from_bits_truncate(f.access_flags as _).contains(FieldAccessFlags::STATIC);
                    if !is_static && matches!(f.name.r#type(), JavaType::Class(_) | JavaType::Array(_)) {
                        ref_offsets.push(f.offset_or_value);
                    }
                }
                if members.parent == 0 {
                    break;
                }
                class = JavaClassDefinition::from_raw(members.parent, &self.core);
            }
            ref_offsets.sort_unstable();
            ref_offsets.dedup();
        }

        let built = Arc::new(ClassGcInfo {
            is_array,
            object_array,
            ref_offsets,
            static_ref_addresses,
        });
        GC_INFO.lock().insert(self.ptr_raw, built.clone());

        Ok(built)
    }

    pub fn methods(&self) -> Result<Vec<JavaMethod>> {
        Ok(self.members()?.methods.iter().map(|m| JavaMethod::from_raw(m.ptr_raw, &self.core)).collect())
    }

    pub fn fields(&self) -> Result<Vec<JavaField>> {
        Ok(self.members()?.fields.iter().map(|f| JavaField::from_raw(f.ptr_raw, &self.core)).collect())
    }

    pub fn name(&self) -> Result<String> {
        Ok(self.name_shared()?.as_ref().into())
    }

    /// The memoised name without the `String` copy `name()` makes. Prefer this wherever
    /// the caller only LOOKS at the name — `is_array` below, the collector's per-class
    /// cache key — because on hero2 that is every reference read and every marked object.
    pub fn name_shared(&self) -> Result<Arc<str>> {
        if let Some(hit) = CLASS_NAMES.lock().get(&self.ptr_raw) {
            return Ok(hit.clone());
        }

        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw)?;
        let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor)?;

        let bytes = read_null_terminated_string_bytes(&self.core, descriptor.ptr_name)?;
        let name: Arc<str> = String::from_utf8(bytes).unwrap().into();

        CLASS_NAMES.lock().insert(self.ptr_raw, name.clone());

        Ok(name)
    }

    /// `[`-prefixed name — an array class. One memo lookup, no allocation; this used to be
    /// a full `name()` copy on every object reference that crossed the boundary.
    pub fn is_array(&self) -> Result<bool> {
        Ok(self.name_shared()?.starts_with('['))
    }

    pub fn parent_class(&self) -> Result<Option<JavaClassDefinition>> {
        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw)?;
        let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor)?;

        if descriptor.ptr_parent_class != 0 {
            Ok(Some(JavaClassDefinition::from_raw(descriptor.ptr_parent_class, &self.core)))
        } else {
            Ok(None)
        }
    }

    pub fn method(&self, name: &str, descriptor: &str, is_static: bool) -> Result<Option<JavaMethod>> {
        let members = self.members()?;

        for (i, m) in members.methods.iter().enumerate() {
            if m.name.name == name
                && m.name.descriptor == descriptor
                && MethodAccessFlags::from_bits_truncate(m.access_flags as _).contains(MethodAccessFlags::STATIC) == is_static
            {
                wie_util::stats::add_jvm_member(i as u64 + 1);
                return Ok(Some(JavaMethod::from_raw(m.ptr_raw, &self.core)));
            }
        }
        wie_util::stats::add_jvm_member(members.methods.len() as u64);

        Ok(None)
    }

    pub fn field(&self, name: &str, descriptor: &str, is_static: bool) -> Result<Option<JavaField>> {
        let members = self.members()?;

        for (i, f) in members.fields.iter().enumerate() {
            if f.name.name == name
                && f.name.descriptor == descriptor
                && FieldAccessFlags::from_bits_truncate(f.access_flags as _).contains(FieldAccessFlags::STATIC) == is_static
            {
                wie_util::stats::add_jvm_member(i as u64 + 1);
                return Ok(Some(JavaField::from_raw(f.ptr_raw, &self.core)));
            }
        }
        wie_util::stats::add_jvm_member(members.fields.len() as u64);

        Ok(None)
    }

    /// A non-static field by name, resolved through this class and its parents from the
    /// memo alone: `(offset into the field block, the field's name record)`. The guest's
    /// own classes carry the flattened hierarchy in their table, so they resolve at the
    /// first level; host-registered classes list only their own fields and walk up.
    pub fn find_instance_field(&self, name: &str, descriptor: &str) -> Result<Option<(u32, Arc<JavaFullName>)>> {
        let mut class = self.clone();
        loop {
            let members = class.members()?;
            for (i, f) in members.fields.iter().enumerate() {
                if f.name.name == name
                    && f.name.descriptor == descriptor
                    && !FieldAccessFlags::from_bits_truncate(f.access_flags as _).contains(FieldAccessFlags::STATIC)
                {
                    wie_util::stats::add_jvm_member(i as u64 + 1);
                    return Ok(Some((f.offset_or_value, f.name.clone())));
                }
            }
            wie_util::stats::add_jvm_member(members.fields.len() as u64);

            if members.parent == 0 {
                return Ok(None);
            }
            class = JavaClassDefinition::from_raw(members.parent, &self.core);
        }
    }

    pub fn read_static_field(&self, field: &JavaField) -> Result<KtfJvmWord> {
        let address = field.static_address()?;
        let result: KtfJvmWord = read_generic(&self.core, address)?;

        Ok(result as _)
    }

    pub fn write_static_field(&self, field: &JavaField, value: KtfJvmWord) -> Result<()> {
        let address = field.static_address()?;

        write_generic(&mut self.core.clone(), address, value)
    }
}

#[async_trait::async_trait]
impl ClassDefinition for JavaClassDefinition {
    fn name(&self) -> String {
        self.name().unwrap()
    }

    fn super_class_name(&self) -> Option<String> {
        self.parent_class().unwrap().map(|x| x.name().unwrap())
    }

    fn access_flags(&self) -> ClassAccessFlags {
        let raw: RawJavaClass = read_generic(&self.core, self.ptr_raw).unwrap();
        let descriptor: RawJavaClassDescriptor = read_generic(&self.core, raw.ptr_descriptor).unwrap();

        ClassAccessFlags::from_bits_truncate(descriptor.access_flag)
    }

    async fn instantiate(&self, jvm: &Jvm) -> JvmResult<Box<dyn ClassInstance>> {
        match JavaClassInstance::new(&mut self.core.clone(), self) {
            Ok(instance) => Ok(Box::new(instance)),
            Err(e) => {
                // Same trap as `instantiate_array`: `Jvm::exception` instantiates the exception itself, so reporting an allocation failure this
                // way re-enters the allocator that just failed. `oom::java_error` hands back the pre-allocated instance instead.
                let allocation_failure = matches!(e, WieError::AllocationFailure);

                Err(oom::java_error(jvm, &self.core, allocation_failure, move || format!("Failed to instantiate class: {e}")).await)
            }
        }
    }

    fn interface_names(&self) -> Vec<String> {
        Vec::new() // TODO we don't store interface now
    }

    async fn prepare(&self, _: &Jvm) -> JvmResult<()> {
        Ok(())
    }

    fn method(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Method>> {
        self.method(name, descriptor, is_static).unwrap().map(|x| Box::new(x) as _)
    }

    fn field(&self, name: &str, descriptor: &str, is_static: bool) -> Option<Box<dyn Field>> {
        self.field(name, descriptor, is_static).unwrap().map(|x| Box::new(x) as _)
    }

    fn fields(&self) -> Vec<Box<dyn Field>> {
        self.fields().unwrap().into_iter().map(|x| Box::new(x) as _).collect()
    }

    fn gc_visit_static_references(&self, visit: &mut dyn FnMut(&dyn ClassInstance)) -> bool {
        let Ok(info) = self.gc_info() else { return false };
        for &address in info.static_ref_addresses.iter() {
            match read_generic::<u32, _>(&self.core, address) {
                Ok(0) => {}
                Ok(child) => visit(&JavaClassInstance::from_raw(child, &self.core)),
                // The generic walk logs and skips an unreadable static; the object it names stays
                // in `all_objects` and is swept, so the line is the only trace of it.
                Err(_) => tracing::error!("Could not read static slot {address:#x} of {:?} while marking; skipping it", self.name()),
            }
        }

        true
    }

    fn get_static_field(&self, field: &dyn Field) -> JvmResult<JavaValue> {
        let field = field.as_any().downcast_ref::<JavaField>().unwrap();
        let value = self.read_static_field(field).unwrap();

        let name = field.name_shared().unwrap();
        Ok(JavaValue::from_raw(value, name.r#type(), &self.core))
    }

    fn put_static_field(&self, field: &dyn Field, value: JavaValue) -> JvmResult<()> {
        let field = field.as_any().downcast_ref::<JavaField>().unwrap();
        let value = value.as_raw();

        self.write_static_field(field, value as _).unwrap();

        Ok(())
    }
}

impl Debug for JavaClassDefinition {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethod").field("ptr_raw", &self.ptr_raw).finish()
    }
}
