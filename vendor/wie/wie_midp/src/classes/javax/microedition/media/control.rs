// ★ heroZero fork (2026-10-21): this file is both the `javax.microedition.media.Control` interface
// (MMAPI's empty marker for everything `Controllable.getControl` returns) and the parent of the
// `javax.microedition.media.control` package module, which holds `VolumeControl`.
use alloc::vec;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

mod volume_control;

pub use self::volume_control::VolumeControl;

// interface javax.microedition.media.Control
pub struct Control;

impl Control {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/media/Control",
            parent_class: None,
            interfaces: vec![],
            methods: vec![],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
