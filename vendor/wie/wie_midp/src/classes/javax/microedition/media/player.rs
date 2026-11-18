use alloc::vec;

use java_class_proto::{JavaFieldProto, JavaMethodProto};

use java_constants::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};
use jvm::{Jvm, Result as JvmResult};
use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

// ★ heroZero fork (2026-10-21): the MIDP 2.0 lifecycle. Upstream declared only start/stop/close;
// 영웅서기 Zero drives every player through `realize(); prefetch(); setLoopCount(n); start()` and
// polls `getState()`, and reaches `getControl` through the `Controllable` parent. All of it is
// implemented by `net.wie.MediaPlayer` (which `net.wie.SmafPlayer` extends).
//
// The state values are MIDP's. javac inlines them into the game, so these Rust constants are
// what `MediaPlayer` actually compares against; the Java fields exist for completeness.
pub const CLOSED: i32 = 0;
pub const UNREALIZED: i32 = 100;
pub const REALIZED: i32 = 200;
pub const PREFETCHED: i32 = 300;
pub const STARTED: i32 = 400;
/// `getDuration` / `getMediaTime` when the time cannot be determined, in microseconds.
pub const TIME_UNKNOWN: i64 = -1;

// interface javax.microedition.media.Player
pub struct Player;

impl Player {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/media/Player",
            parent_class: None,
            interfaces: vec!["javax/microedition/media/Controllable"],
            methods: vec![
                JavaMethodProto::new("<clinit>", "()V", Self::cl_init, MethodAccessFlags::STATIC),
                JavaMethodProto::new_abstract("realize", "()V", Default::default()),
                JavaMethodProto::new_abstract("prefetch", "()V", Default::default()),
                JavaMethodProto::new_abstract("start", "()V", Default::default()),
                JavaMethodProto::new_abstract("stop", "()V", Default::default()),
                JavaMethodProto::new_abstract("deallocate", "()V", Default::default()),
                JavaMethodProto::new_abstract("close", "()V", Default::default()),
                JavaMethodProto::new_abstract("getState", "()I", Default::default()),
                JavaMethodProto::new_abstract("setLoopCount", "(I)V", Default::default()),
                JavaMethodProto::new_abstract("getDuration", "()J", Default::default()),
                JavaMethodProto::new_abstract("getContentType", "()Ljava/lang/String;", Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("CLOSED", "I", FieldAccessFlags::STATIC),
                JavaFieldProto::new("UNREALIZED", "I", FieldAccessFlags::STATIC),
                JavaFieldProto::new("REALIZED", "I", FieldAccessFlags::STATIC),
                JavaFieldProto::new("PREFETCHED", "I", FieldAccessFlags::STATIC),
                JavaFieldProto::new("STARTED", "I", FieldAccessFlags::STATIC),
                JavaFieldProto::new("TIME_UNKNOWN", "J", FieldAccessFlags::STATIC),
            ],
            access_flags: ClassAccessFlags::INTERFACE,
        }
    }

    async fn cl_init(jvm: &Jvm, _: &mut WieJvmContext) -> JvmResult<()> {
        tracing::debug!("javax.microedition.media.Player::<clinit>()");

        for (name, value) in [
            ("CLOSED", CLOSED),
            ("UNREALIZED", UNREALIZED),
            ("REALIZED", REALIZED),
            ("PREFETCHED", PREFETCHED),
            ("STARTED", STARTED),
        ] {
            jvm.put_static_field("javax/microedition/media/Player", name, "I", value).await?;
        }
        jvm.put_static_field("javax/microedition/media/Player", "TIME_UNKNOWN", "J", TIME_UNKNOWN)
            .await?;

        Ok(())
    }
}
