use alloc::vec;

use java_class_proto::JavaMethodProto;

use java_constants::ClassAccessFlags;
use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): GCF interface, declared so the game's classes resolve. See `io.rs`.
// interface javax.microedition.io.OutputConnection
pub struct OutputConnection;

impl OutputConnection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/OutputConnection",
            parent_class: None,
            interfaces: vec!["javax/microedition/io/Connection"],
            methods: vec![
                JavaMethodProto::new_abstract("openOutputStream", "()Ljava/io/OutputStream;", Default::default()),
                JavaMethodProto::new_abstract("openDataOutputStream", "()Ljava/io/DataOutputStream;", Default::default()),
            ],
            fields: vec![],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }
}
