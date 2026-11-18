use alloc::vec;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): GCF interface, declared so the game's classes resolve. See `io.rs`.
// interface javax.microedition.io.StreamConnection
pub struct StreamConnection;

impl StreamConnection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/StreamConnection",
            parent_class: None,
            interfaces: vec!["javax/microedition/io/InputConnection", "javax/microedition/io/OutputConnection"],
            methods: vec![],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
