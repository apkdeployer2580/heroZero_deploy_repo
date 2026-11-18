use alloc::vec;

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use java_constants::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};
use jvm::{Jvm, Result as JvmResult, runtime::JavaLangString};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

// ★ heroZero fork (2026-10-21): GCF interface, declared so the game's classes resolve. See `io.rs`.
//
// Only what the game's `bd` (QQ telemetry POST) touches: the three request/response methods it
// calls, and the constants it was compiled against — javac inlined those (`200`, `"POST"`), so
// the fields exist for completeness rather than because anything reads them at run time.
// interface javax.microedition.io.HttpConnection
pub struct HttpConnection;

impl HttpConnection {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/HttpConnection",
            parent_class: None,
            interfaces: vec!["javax/microedition/io/ContentConnection"],
            methods: vec![
                JavaMethodProto::new("<clinit>", "()V", Self::cl_init, MethodAccessFlags::STATIC),
                JavaMethodProto::new_abstract("getResponseCode", "()I", Default::default()),
                JavaMethodProto::new_abstract("setRequestMethod", "(Ljava/lang/String;)V", Default::default()),
                JavaMethodProto::new_abstract("setRequestProperty", "(Ljava/lang/String;Ljava/lang/String;)V", Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("GET", "Ljava/lang/String;", FieldAccessFlags::STATIC),
                JavaFieldProto::new("HEAD", "Ljava/lang/String;", FieldAccessFlags::STATIC),
                JavaFieldProto::new("POST", "Ljava/lang/String;", FieldAccessFlags::STATIC),
                JavaFieldProto::new("HTTP_OK", "I", FieldAccessFlags::STATIC),
            ],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }

    async fn cl_init(jvm: &Jvm, _: &mut WieJvmContext) -> JvmResult<()> {
        tracing::debug!("javax.microedition.io.HttpConnection::<clinit>()");

        for method in ["GET", "HEAD", "POST"] {
            let value = JavaLangString::from_rust_string(jvm, method).await?;
            jvm.put_static_field("javax/microedition/io/HttpConnection", method, "Ljava/lang/String;", value)
                .await?;
        }
        jvm.put_static_field("javax/microedition/io/HttpConnection", "HTTP_OK", "I", 200).await?;

        Ok(())
    }
}
