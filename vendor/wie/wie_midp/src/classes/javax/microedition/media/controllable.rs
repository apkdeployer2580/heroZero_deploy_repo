use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): MMAPI, the parent interface of `Player`. The game calls
// `getControl("VolumeControl")` through it (`invokeinterface Controllable.getControl`), so it
// must exist and `Player` must extend it. Implemented by `net.wie.MediaPlayer`.
// interface javax.microedition.media.Controllable
pub struct Controllable;

impl Controllable {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/media/Controllable",
            parent_class: None,
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new_abstract("getControl", "(Ljava/lang/String;)Ljavax/microedition/media/Control;", Default::default()),
                JavaMethodProto::new_abstract("getControls", "()[Ljavax/microedition/media/Control;", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
