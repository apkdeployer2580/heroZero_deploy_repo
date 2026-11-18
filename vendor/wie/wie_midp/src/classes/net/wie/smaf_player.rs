use alloc::vec;

use java_class_proto::JavaMethodProto;
use java_runtime::classes::java::io::InputStream;
use jvm::{ClassInstanceRef, Jvm, Result, runtime::JavaLangString};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

// ★ heroZero fork (2026-10-21): now a thin subclass of `net.wie.MediaPlayer`, which took over
// everything this class did — `Audio::load_smaf` at construction, backend `play`/`stop`/`close`,
// and the wie-private `start(Z)V` that `org.kwis.msp.media.Player` (WIPI clips) calls — and adds
// the MIDP 2.0 lifecycle, loop count and `VolumeControl` on top. The class keeps its name and its
// constructor for `Manager.createPlayer(.., "application/vnd.smaf")`.
// class net.wie.SmafPlayer
pub struct SmafPlayer;

impl SmafPlayer {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "net/wie/SmafPlayer",
            parent_class: Some("net/wie/MediaPlayer"),
            interfaces: vec![],
            methods: vec![JavaMethodProto::new("<init>", "(Ljava/io/InputStream;)V", Self::init, Default::default())],
            fields: vec![],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, _context: &mut WieJvmContext, this: ClassInstanceRef<Self>, stream: ClassInstanceRef<InputStream>) -> Result<()> {
        tracing::debug!("net.wie.SmafPlayer::<init>({this:?}, {stream:?})");

        let content_type = JavaLangString::from_rust_string(jvm, "application/vnd.smaf").await?;
        let _: () = jvm
            .invoke_special(
                &this,
                "net/wie/MediaPlayer",
                "<init>",
                "(Ljava/io/InputStream;Ljava/lang/String;)V",
                (stream, content_type),
            )
            .await?;

        Ok(())
    }
}
