use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): GCF interface, declared so the game's classes resolve. See `io.rs`.
// interface javax.microedition.io.ContentConnection
pub struct ContentConnection;

impl ContentConnection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/ContentConnection",
            parent_class: None,
            interfaces: vec!["javax/microedition/io/StreamConnection"],
            methods: vec![
                JavaMethodProto::new_abstract("getType", "()Ljava/lang/String;", Default::default()),
                JavaMethodProto::new_abstract("getEncoding", "()Ljava/lang/String;", Default::default()),
                JavaMethodProto::new_abstract("getLength", "()J", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
