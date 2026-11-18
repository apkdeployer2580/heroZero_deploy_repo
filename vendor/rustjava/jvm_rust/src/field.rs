use alloc::{
    string::{String, ToString},
    sync::Arc,
};
use core::cmp::Ordering;

use classfile::FieldInfo;
use java_class_proto::JavaFieldProto;
use java_constants::FieldAccessFlags;
use jvm::Field;

// ★ heroZero fork (2026-10-24): the declaring class is part of a field's identity. An instance
// keys its storage by `FieldImpl`, and a subclass may declare a field with its superclass's name
// and descriptor - 영웅서기 Zero has 33 such pairs (two `private boolean e` in the window `o` and
// its inventory `ds`; `ba.d:B` and the equipment level `bx.d:B`). Without the class they were ONE
// slot, so the superclass's writes landed in the subclass's field (`o` clearing its first-paint
// flag cleared the inventory's "on the bag tabs", and the bag tabs could never be reached).
// `class` is LAST so the derived order only compares it between two such fields.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FieldInner {
    name: String,
    descriptor: String,
    access_flags: FieldAccessFlags,
    class: String,
}

// ★ heroZero fork (2026-10-27): compared by content as before, but the same field - one `Arc`,
// handed out by its class and by every lookup since - answers at the pointer. An object's storage
// is a map keyed by these, read on every field access.
#[derive(Clone, Debug)]
pub struct FieldImpl {
    inner: Arc<FieldInner>,
}

impl PartialEq for FieldImpl {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || self.inner == other.inner
    }
}

impl Eq for FieldImpl {}

impl Ord for FieldImpl {
    fn cmp(&self, other: &Self) -> Ordering {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            Ordering::Equal
        } else {
            self.inner.cmp(&other.inner)
        }
    }
}

impl PartialOrd for FieldImpl {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FieldImpl {
    pub fn new(class: &str, name: &str, descriptor: &str, access_flags: FieldAccessFlags) -> Self {
        Self {
            inner: Arc::new(FieldInner {
                name: name.to_string(),
                descriptor: descriptor.to_string(),
                access_flags,
                class: class.to_string(),
            }),
        }
    }

    pub fn from_field_proto(class: &str, proto: JavaFieldProto) -> Self {
        Self::new(class, &proto.name, &proto.descriptor, proto.access_flags)
    }

    /// ★ heroZero fork (2026-10-27): the name and descriptor without the copy [`Field::name`] and
    /// [`Field::descriptor`] make — a class's field lookup compares every candidate, on every
    /// `getfield`, and allocated two strings a candidate doing it.
    pub fn name_str(&self) -> &str {
        &self.inner.name
    }

    pub fn descriptor_str(&self) -> &str {
        &self.inner.descriptor
    }

    pub fn from_field_info(class: &str, field_info: FieldInfo) -> Self {
        Self {
            inner: Arc::new(FieldInner {
                name: field_info.name.to_string(),
                descriptor: field_info.descriptor.to_string(),
                access_flags: field_info.access_flags,
                class: class.to_string(),
            }),
        }
    }
}

impl Field for FieldImpl {
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn descriptor(&self) -> String {
        self.inner.descriptor.clone()
    }

    fn access_flags(&self) -> FieldAccessFlags {
        self.inner.access_flags
    }
}
