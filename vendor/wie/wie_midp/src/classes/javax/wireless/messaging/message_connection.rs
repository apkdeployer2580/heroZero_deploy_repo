use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): WMA interface, declared so the game's classes resolve. See `messaging.rs`.
// interface javax.wireless.messaging.MessageConnection
pub struct MessageConnection;

impl MessageConnection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/wireless/messaging/MessageConnection",
            parent_class: None,
            interfaces: vec!["javax/microedition/io/Connection"],
            methods: vec![
                JavaMethodProto::new_abstract("newMessage", "(Ljava/lang/String;)Ljavax/wireless/messaging/Message;", Default::default()),
                JavaMethodProto::new_abstract(
                    "newMessage",
                    "(Ljava/lang/String;Ljava/lang/String;)Ljavax/wireless/messaging/Message;",
                    Default::default(),
                ),
                JavaMethodProto::new_abstract("send", "(Ljavax/wireless/messaging/Message;)V", Default::default()),
                JavaMethodProto::new_abstract("receive", "()Ljavax/wireless/messaging/Message;", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
