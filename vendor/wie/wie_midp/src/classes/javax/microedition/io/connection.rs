use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): GCF interface, declared so the game's classes resolve. See `io.rs`.
// interface javax.microedition.io.Connection
pub struct Connection;

impl Connection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/Connection",
            parent_class: None,
            interfaces: vec![],
            methods: vec![JavaMethodProto::new_abstract("close", "()V", Default::default())],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
