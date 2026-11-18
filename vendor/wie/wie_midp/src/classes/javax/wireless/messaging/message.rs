use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): WMA interface, declared so the game's classes resolve. See `messaging.rs`.
// interface javax.wireless.messaging.Message
pub struct Message;

impl Message {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/wireless/messaging/Message",
            parent_class: None,
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new_abstract("getAddress", "()Ljava/lang/String;", Default::default()),
                JavaMethodProto::new_abstract("setAddress", "(Ljava/lang/String;)V", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
