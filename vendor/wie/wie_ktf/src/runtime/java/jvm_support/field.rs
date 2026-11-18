use alloc::string::String;
use core::{
    fmt::{self, Debug, Formatter},
    mem::size_of,
};

use java_class_proto::JavaFieldProto;
use java_constants::FieldAccessFlags;
use jvm::Field;
use wipi_types::ktf::java::JavaFieldDefinition as RawJavaField;

use wie_core_arm::{Allocator, ArmCore};
use wie_util::{ByteWrite, read_generic, write_generic};

use super::{Result, name::JavaFullName};

pub struct JavaField {
    pub ptr_raw: u32,
    core: ArmCore,
}

impl JavaField {
    pub fn from_raw(ptr_raw: u32, core: &ArmCore) -> Self {
        Self { ptr_raw, core: core.clone() }
    }

    pub fn new(core: &mut ArmCore, ptr_class: u32, proto: JavaFieldProto, offset_or_value: u32) -> Result<Self> {
        let full_name = JavaFullName::new(0, proto.name, proto.descriptor);
        let full_name_bytes = full_name.as_bytes();
        let ptr_name = Allocator::alloc(core, full_name_bytes.len() as u32)?;
        // A recycled allocation must not serve the previous occupant's memoised name.
        // See the `NAME_CACHE` note in `name.rs`.
        JavaFullName::forget(ptr_name);
        core.write_bytes(ptr_name, &full_name_bytes)?;

        let ptr_raw = Allocator::alloc(core, size_of::<RawJavaField>() as u32)?;

        write_generic(
            core,
            ptr_raw,
            RawJavaField {
                access_flags: proto.access_flags.bits() as _,
                ptr_class,
                ptr_name,
                offset_or_value,
            },
        )?;

        tracing::trace!("Wrote field {} at {ptr_raw:#x}", full_name.name);

        Ok(Self::from_raw(ptr_raw, core))
    }

    /// The 16-byte definition record. Exposed so a caller that needs more than one of
    /// `name` / `offset` / `access_flags` can pay for ONE guest read instead of three -
    /// see `JavaClassInstance::get_field`, which used to make five.
    pub fn raw(&self) -> Result<RawJavaField> {
        read_generic(&self.core, self.ptr_raw)
    }

    /// The memoised name, given a record the caller has already read.
    pub fn name_of(&self, raw: &RawJavaField) -> Result<alloc::sync::Arc<JavaFullName>> {
        JavaFullName::from_ptr_shared(&self.core, raw.ptr_name)
    }

    pub fn name(&self) -> Result<JavaFullName> {
        Ok((*self.name_shared()?).clone())
    }

    /// The memoised name. Prefer this where the caller only reads it - `name()` has to
    /// clone two `String`s back out. See the `NAME_CACHE` note in `name.rs`.
    pub fn name_shared(&self) -> Result<alloc::sync::Arc<JavaFullName>> {
        let raw: RawJavaField = read_generic(&self.core, self.ptr_raw)?;

        JavaFullName::from_ptr_shared(&self.core, raw.ptr_name)
    }

    pub fn offset(&self) -> Result<u32> {
        let raw: RawJavaField = read_generic(&self.core, self.ptr_raw)?;

        Ok(raw.offset_or_value)
    }

    pub fn static_address(&self) -> Result<u32> {
        let address = self.ptr_raw + 12; // offsetof offset_or_value

        Ok(address)
    }
}

impl Field for JavaField {
    fn name(&self) -> String {
        let name = self.name().unwrap();

        name.name.clone()
    }

    fn descriptor(&self) -> String {
        let name = self.name().unwrap();

        name.descriptor.clone()
    }

    fn access_flags(&self) -> FieldAccessFlags {
        let raw: RawJavaField = read_generic(&self.core, self.ptr_raw).unwrap();

        FieldAccessFlags::from_bits_truncate(raw.access_flags as _)
    }
}

impl Debug for JavaField {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethod").field("ptr_raw", &self.ptr_raw).finish()
    }
}
