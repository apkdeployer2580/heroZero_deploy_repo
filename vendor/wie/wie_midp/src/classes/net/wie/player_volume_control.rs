use alloc::vec;

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use jvm::{ClassInstanceRef, Jvm, Result as JvmResult};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::net::wie::MediaPlayer;

// ★ heroZero fork (2026-10-21): the `VolumeControl` `net.wie.MediaPlayer.getControl` hands out —
// one per player, bound to it. A view only: the level and mute flag live on the player, which
// pushes them to its backend handle (see `MediaPlayer::set_level`). On a player the backend does
// not render they are kept and reported, nothing more.
// class net.wie.PlayerVolumeControl
pub struct PlayerVolumeControl;

impl PlayerVolumeControl {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "net/wie/PlayerVolumeControl",
            parent_class: Some("java/lang/Object"),
            interfaces: vec!["javax/microedition/media/control/VolumeControl"],
            methods: vec![
                JavaMethodProto::new("<init>", "(Lnet/wie/MediaPlayer;)V", Self::init, Default::default()),
                JavaMethodProto::new("setLevel", "(I)I", Self::set_level, Default::default()),
                JavaMethodProto::new("getLevel", "()I", Self::get_level, Default::default()),
                JavaMethodProto::new("setMute", "(Z)V", Self::set_mute, Default::default()),
                JavaMethodProto::new("isMuted", "()Z", Self::is_muted, Default::default()),
            ],
            fields: vec![JavaFieldProto::new("player", "Lnet/wie/MediaPlayer;", Default::default())],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, _: &mut WieJvmContext, mut this: ClassInstanceRef<Self>, player: ClassInstanceRef<MediaPlayer>) -> JvmResult<()> {
        tracing::debug!("net.wie.PlayerVolumeControl::<init>({this:?}, {player:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        jvm.put_field(&mut this, "player", "Lnet/wie/MediaPlayer;", player).await?;

        Ok(())
    }

    async fn set_level(jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>, level: i32) -> JvmResult<i32> {
        tracing::debug!("net.wie.PlayerVolumeControl::setLevel({this:?}, {level})");

        let mut player = Self::player(jvm, &this).await?;

        MediaPlayer::set_level(jvm, context, &mut player, level).await
    }

    async fn get_level(jvm: &Jvm, _: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<i32> {
        tracing::debug!("net.wie.PlayerVolumeControl::getLevel({this:?})");

        let player = Self::player(jvm, &this).await?;

        MediaPlayer::level(jvm, &player).await
    }

    async fn set_mute(jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>, muted: bool) -> JvmResult<()> {
        tracing::debug!("net.wie.PlayerVolumeControl::setMute({this:?}, {muted})");

        let mut player = Self::player(jvm, &this).await?;

        MediaPlayer::set_mute(jvm, context, &mut player, muted).await
    }

    async fn is_muted(jvm: &Jvm, _: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<bool> {
        tracing::debug!("net.wie.PlayerVolumeControl::isMuted({this:?})");

        let player = Self::player(jvm, &this).await?;

        MediaPlayer::is_muted(jvm, &player).await
    }

    async fn player(jvm: &Jvm, this: &ClassInstanceRef<Self>) -> JvmResult<ClassInstanceRef<MediaPlayer>> {
        jvm.get_field(this, "player", "Lnet/wie/MediaPlayer;").await
    }
}
