use alloc::vec;

use java_class_proto::JavaMethodProto;
use java_constants::MethodAccessFlags;
use java_runtime::classes::java::{io::InputStream, lang::String};
use jvm::{ClassInstanceRef, Jvm, Result, runtime::JavaLangString};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::media::Player;

/// ★ heroZero fork (2026-10-21): the types `net.wie.MediaPlayer` takes besides SMAF. 영웅서기 Zero
/// asks for `audio/midi` (BGM) and `audio/x-wav` (effects); upstream threw MediaException for
/// both, so every one of the game's players was null. What sounds is decided by the content,
/// not by this list — see `MediaPlayer`.
const MEDIA_PLAYER_TYPES: [&str; 6] = ["audio/midi", "audio/mid", "audio/x-midi", "audio/sp-midi", "audio/x-wav", "audio/wav"];

// class javax.microedition.media.Manager
pub struct Manager;

impl Manager {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/media/Manager",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![JavaMethodProto::new(
                "createPlayer",
                "(Ljava/io/InputStream;Ljava/lang/String;)Ljavax/microedition/media/Player;",
                Self::create_player,
                MethodAccessFlags::STATIC,
            )],
            fields: vec![],
            access_flags: Default::default(),
        }
    }

    async fn create_player(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        stream: ClassInstanceRef<InputStream>,
        r#type: ClassInstanceRef<String>,
    ) -> Result<ClassInstanceRef<Player>> {
        tracing::debug!("javax.microedition.media.Manager::createPlayer({stream:?}, {type:?})");

        // ★ heroZero fork (2026-10-21): MIDP — a null stream is an IllegalArgumentException, and a
        // null type means "work it out from the data", which `MediaPlayer` does by sniffing.
        if stream.is_null() {
            return Err(jvm.exception("java/lang/IllegalArgumentException", "stream is null").await);
        }
        let type_string = if r#type.is_null() {
            None
        } else {
            Some(JavaLangString::to_rust_string(jvm, &r#type).await?)
        };

        if type_string.as_deref() == Some("application/vnd.smaf") {
            Ok(jvm.new_class("net/wie/SmafPlayer", "(Ljava/io/InputStream;)V", (stream,)).await?.into())
        } else if type_string
            .as_deref()
            .is_none_or(|x| MEDIA_PLAYER_TYPES.iter().any(|known| known.eq_ignore_ascii_case(x)))
        {
            Ok(jvm
                .new_class("net/wie/MediaPlayer", "(Ljava/io/InputStream;Ljava/lang/String;)V", (stream, r#type))
                .await?
                .into())
        } else {
            Err(jvm.exception("javax/microedition/media/MediaException", "Unsupported media type").await)
        }
    }
}
