use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): MMAPI. The game's sound manager `cc` sets each player's level with
// `((VolumeControl) player.getControl("VolumeControl")).setLevel(v)`. Implemented, per player, by
// `net.wie.PlayerVolumeControl`.
// interface javax.microedition.media.control.VolumeControl
pub struct VolumeControl;

impl VolumeControl {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/media/control/VolumeControl",
            parent_class: None,
            interfaces: vec!["javax/microedition/media/Control"],
            methods: vec![
                JavaMethodProto::new_abstract("setMute", "(Z)V", Default::default()),
                JavaMethodProto::new_abstract("isMuted", "()Z", Default::default()),
                JavaMethodProto::new_abstract("setLevel", "(I)I", Default::default()),
                JavaMethodProto::new_abstract("getLevel", "()I", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
