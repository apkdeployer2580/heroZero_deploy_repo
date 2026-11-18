use alloc::{collections::BTreeMap, format, string::String, sync::Arc, vec::Vec};
use core::fmt::Display;

use jvm::JavaType;
use wie_core_arm::ArmCore;
use wie_util::{read_generic, read_null_terminated_string_bytes};

use super::Result;

/// ★★ NAME-RECORD MEMO (2026-09-09) — the single biggest host cost in a hero2 frame.
///
/// ── WHAT IT REPLACES ────────────────────────────────────────────────────────
/// A name record is `tag + descriptor + '+' + name + NUL` in GUEST memory, and
/// [`JavaFullName::from_ptr`] used to re-read and re-parse it on every single call.
/// Neither half is cheap:
///   * `read_null_terminated_string_bytes` walks the string **one byte at a time**
///     through `ByteRead`, and for `ArmCore` each of those bytes is a mutex lock plus
///     an engine memory read;
///   * the parse then allocates three `String`s.
/// And it is called constantly, because `jvm::Field::name()`/`descriptor()` are
/// implemented in terms of it: one KTF instance field read costs two of these, and
/// resolving a field by name first SCANS the class's field table calling it once per
/// entry. Measured on hero2 (probe, 1200 frames, `[jvm]` gauge):
/// **12.5 M name-record parses = 188 MB of byte-at-a-time guest reads, ~157 KB/frame.**
///
/// ── WHY MEMOISING IS SOUND, NOT JUST FASTER ─────────────────────────────────
/// A cache keyed on a GUEST ADDRESS is only safe if that address cannot come to mean
/// something else. For name records it cannot, and this was established by auditing
/// **every** `Allocator::free` call site in the tree (2026-09-09). Exactly five free
/// live objects: thread stacks (`thread.rs`), the temporary class-name C string in
/// `KtfClassLoader::findClass`, class INSTANCES (`class_instance.rs`), method-call
/// argument containers (`method.rs`), and WIPI-C allocations (`wipi_c/context.rs`).
/// **Nothing frees a name record, a `JavaClass`, a descriptor, a method/field
/// definition or a vtable** — the class metadata is allocated once per class and
/// leaks by design. So a cached entry's address stays that same name record forever.
///
/// The one remaining hazard is the reverse direction: a *freed* allocation (say that
/// temp class-name string) being handed back out as a NEW name record. That is why
/// [`JavaFullName::forget`] exists and why `JavaField::new` / `JavaMethod::new` call it
/// on the address they are about to write — a stale entry at a recycled address is
/// dropped at the moment the new record is created, before anyone can read it.
static NAME_CACHE: spin::Mutex<BTreeMap<u32, Arc<JavaFullName>>> = spin::Mutex::new(BTreeMap::new());

#[derive(Clone)]
pub struct JavaFullName {
    pub tag: u8,
    pub name: String,
    pub descriptor: String,
    /// ★ The descriptor PARSED, once (2026-09-19, roadmap #3). `JavaType::parse` allocates
    /// (a `String` per class name, a `Vec` per method) and it was being called on every
    /// field read, every field write and every method return — per crossing, for a
    /// descriptor that never changes. Memoised with the name it belongs to.
    r#type: JavaType,
    /// `name + descriptor`, built once — what the JVM's stack frame wants for a stack trace.
    /// It used to be `format!` of two fresh `String` clones on every method call.
    signature: Arc<str>,
}

impl JavaFullName {
    pub fn new(tag: u8, name: String, descriptor: String) -> Self {
        let r#type = JavaType::parse(&descriptor);
        let signature = format!("{name}{descriptor}").into();

        Self {
            tag,
            name,
            descriptor,
            r#type,
            signature,
        }
    }

    /// The parsed descriptor: a field's type, or `JavaType::Method` for a method.
    pub fn r#type(&self) -> &JavaType {
        &self.r#type
    }

    /// `(parameter types, return type)`. Panics on a field name — same contract as
    /// `JavaType::as_method`.
    pub fn method_type(&self) -> (&[JavaType], &JavaType) {
        self.r#type.as_method()
    }

    pub fn signature(&self) -> &Arc<str> {
        &self.signature
    }

    pub fn from_ptr(core: &ArmCore, ptr: u32) -> Result<Self> {
        Ok((*Self::from_ptr_shared(core, ptr)?).clone())
    }

    /// The memoised read. Prefer this over [`from_ptr`](Self::from_ptr) wherever the
    /// caller only needs to LOOK at the name: cloning it back out allocates the two
    /// `String`s again, which is the last cost this cache cannot remove for callers
    /// that insist on an owned value.
    pub fn from_ptr_shared(core: &ArmCore, ptr: u32) -> Result<Arc<Self>> {
        if let Some(hit) = NAME_CACHE.lock().get(&ptr) {
            return Ok(hit.clone());
        }

        let tag = read_generic(core, ptr)?;

        let value = read_null_terminated_string_bytes(core, ptr + 1)?;
        // ★ Counted here because THIS is the expensive part: the read above is one
        // locked `ByteRead` round trip per byte, and the parse below allocates three
        // `String`s. See `wie_util::stats::JVM_NAME_READS`. With the cache in front it
        // should now count DISTINCT name records, not calls — if it keeps tracking the
        // frame count, the cache is being invalidated or bypassed.
        wie_util::stats::add_jvm_name_read(value.len() as u64 + 1);
        let value = String::from_utf8(value).unwrap();
        let mut values = value.split('+');

        let descriptor = values.next().unwrap().into();
        let name = values.next().unwrap().into();

        let parsed = Arc::new(JavaFullName::new(tag, name, descriptor));
        NAME_CACHE.lock().insert(ptr, parsed.clone());

        Ok(parsed)
    }

    /// Drop any memo for `ptr`. Called when a NEW name record is written at an address,
    /// so that a recycled allocation can never serve the previous occupant's name.
    /// See the `NAME_CACHE` note for why this is the only hazard that needed closing.
    pub fn forget(ptr: u32) {
        NAME_CACHE.lock().remove(&ptr);
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        bytes.push(self.tag);
        bytes.extend_from_slice(self.descriptor.as_bytes());
        bytes.push(b'+');
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.push(0);

        bytes
    }
}

impl Display for JavaFullName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.name.fmt(f)?;
        self.descriptor.fmt(f)?;
        write!(f, "@{}", self.tag)?;

        Ok(())
    }
}

impl PartialEq for JavaFullName {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor == other.descriptor && self.name == other.name
    }
}
