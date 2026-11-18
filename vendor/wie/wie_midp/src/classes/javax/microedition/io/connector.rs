use alloc::{format, vec};

use java_class_proto::JavaMethodProto;
use java_constants::MethodAccessFlags;
use java_runtime::classes::java::{
    io::{DataInputStream, DataOutputStream, InputStream, OutputStream},
    lang::String,
};
use jvm::{ClassInstanceRef, JavaError, Jvm, Result as JvmResult, runtime::JavaLangString};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::io::Connection;

// ★ heroZero fork (2026-10-21): the GCF factory, OFFLINE BY DESIGN — see `io.rs`.
//
// Every entry point refuses with `ConnectionNotFoundException` (an `IOException`) carrying the
// URL, after one info-level line naming it. That is what the game's own error paths expect from
// a phone with no coverage: `bd` catches `Exception` around `open` and drops the telemetry POST,
// and `ep`'s `sms://` billing send fails the same way. Nothing is ever opened, so no object
// implementing `Connection` is ever handed out.
// class javax.microedition.io.Connector
pub struct Connector;

impl Connector {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/io/Connector",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new(
                    "open",
                    "(Ljava/lang/String;)Ljavax/microedition/io/Connection;",
                    Self::open,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "open",
                    "(Ljava/lang/String;I)Ljavax/microedition/io/Connection;",
                    Self::open_with_mode,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "open",
                    "(Ljava/lang/String;IZ)Ljavax/microedition/io/Connection;",
                    Self::open_with_mode_and_timeouts,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "openInputStream",
                    "(Ljava/lang/String;)Ljava/io/InputStream;",
                    Self::open_input_stream,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "openDataInputStream",
                    "(Ljava/lang/String;)Ljava/io/DataInputStream;",
                    Self::open_data_input_stream,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "openOutputStream",
                    "(Ljava/lang/String;)Ljava/io/OutputStream;",
                    Self::open_output_stream,
                    MethodAccessFlags::STATIC,
                ),
                JavaMethodProto::new(
                    "openDataOutputStream",
                    "(Ljava/lang/String;)Ljava/io/DataOutputStream;",
                    Self::open_data_output_stream,
                    MethodAccessFlags::STATIC,
                ),
            ],
            fields: vec![],
            access_flags: Default::default(),
        }
    }

    async fn open(jvm: &Jvm, _: &mut WieJvmContext, name: ClassInstanceRef<String>) -> JvmResult<ClassInstanceRef<Connection>> {
        tracing::debug!("javax.microedition.io.Connector::open({name:?})");

        Err(Self::refuse(jvm, "open", &name).await)
    }

    async fn open_with_mode(jvm: &Jvm, _: &mut WieJvmContext, name: ClassInstanceRef<String>, mode: i32) -> JvmResult<ClassInstanceRef<Connection>> {
        tracing::debug!("javax.microedition.io.Connector::open({name:?}, {mode})");

        Err(Self::refuse(jvm, "open", &name).await)
    }

    async fn open_with_mode_and_timeouts(
        jvm: &Jvm,
        _: &mut WieJvmContext,
        name: ClassInstanceRef<String>,
        mode: i32,
        timeouts: bool,
    ) -> JvmResult<ClassInstanceRef<Connection>> {
        tracing::debug!("javax.microedition.io.Connector::open({name:?}, {mode}, {timeouts})");

        Err(Self::refuse(jvm, "open", &name).await)
    }

    async fn open_input_stream(jvm: &Jvm, _: &mut WieJvmContext, name: ClassInstanceRef<String>) -> JvmResult<ClassInstanceRef<InputStream>> {
        tracing::debug!("javax.microedition.io.Connector::openInputStream({name:?})");

        Err(Self::refuse(jvm, "openInputStream", &name).await)
    }

    async fn open_data_input_stream(
        jvm: &Jvm,
        _: &mut WieJvmContext,
        name: ClassInstanceRef<String>,
    ) -> JvmResult<ClassInstanceRef<DataInputStream>> {
        tracing::debug!("javax.microedition.io.Connector::openDataInputStream({name:?})");

        Err(Self::refuse(jvm, "openDataInputStream", &name).await)
    }

    async fn open_output_stream(jvm: &Jvm, _: &mut WieJvmContext, name: ClassInstanceRef<String>) -> JvmResult<ClassInstanceRef<OutputStream>> {
        tracing::debug!("javax.microedition.io.Connector::openOutputStream({name:?})");

        Err(Self::refuse(jvm, "openOutputStream", &name).await)
    }

    async fn open_data_output_stream(
        jvm: &Jvm,
        _: &mut WieJvmContext,
        name: ClassInstanceRef<String>,
    ) -> JvmResult<ClassInstanceRef<DataOutputStream>> {
        tracing::debug!("javax.microedition.io.Connector::openDataOutputStream({name:?})");

        Err(Self::refuse(jvm, "openDataOutputStream", &name).await)
    }

    /// The one refusal every entry point shares: log the URL once, at info, and build the
    /// `ConnectionNotFoundException` that carries it.
    async fn refuse(jvm: &Jvm, method: &str, name: &ClassInstanceRef<String>) -> JavaError {
        let url = if name.is_null() {
            "<null>".into()
        } else {
            match JavaLangString::to_rust_string(jvm, name).await {
                Ok(url) => url,
                Err(err) => return err,
            }
        };

        tracing::info!("Connector.{method}({url:?}) refused: this build is offline and never opens a connection");

        jvm.exception(
            "javax/microedition/io/ConnectionNotFoundException",
            &format!("offline build, no connection to {url}"),
        )
        .await
    }
}
