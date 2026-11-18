use alloc::{format, vec};

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use java_constants::{ClassAccessFlags, FieldAccessFlags};
use java_runtime::classes::java::lang::String;
use jvm::{ClassInstanceRef, Jvm, Result as JvmResult, runtime::JavaLangString};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::lcdui::Display;

// abstract class javax.microedition.midlet.MIDlet
pub struct MIDlet;

impl MIDlet {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/midlet/MIDlet",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new("<init>", "()V", Self::init, Default::default()),
                JavaMethodProto::new(
                    "getAppProperty",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    Self::get_app_property,
                    Default::default(),
                ),
                JavaMethodProto::new_abstract("startApp", "([Ljava/lang/String;)V", Default::default()),
                JavaMethodProto::new("notifyDestroyed", "()V", Self::notify_destroyed, Default::default()),
                // ★ heroZero fork (2026-10-21): MIDP 2.0, called by `rpg/RPGHeroEx`. See `platform_request`.
                JavaMethodProto::new("platformRequest", "(Ljava/lang/String;)Z", Self::platform_request, Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("currentMIDlet", "Ljavax/microedition/midlet/MIDlet;", FieldAccessFlags::STATIC),
                JavaFieldProto::new("display", "Ljavax/microedition/lcdui/Display;", Default::default()),
            ],
            access_flags: ClassAccessFlags::ABSTRACT,
        }
    }

    async fn init(jvm: &Jvm, _context: &mut WieJvmContext, mut this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("javax.microedition.midlet.MIDlet::<init>({this:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        jvm.put_static_field(
            "javax/microedition/midlet/MIDlet",
            "currentMIDlet",
            "Ljavax/microedition/midlet/MIDlet;",
            this.clone(),
        )
        .await?;

        let display = jvm.new_class("javax/microedition/lcdui/Display", "()V", ()).await?;

        jvm.put_field(&mut this, "display", "Ljavax/microedition/lcdui/Display;", display).await?;

        Ok(())
    }

    async fn get_app_property(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        key: ClassInstanceRef<String>,
    ) -> JvmResult<ClassInstanceRef<String>> {
        tracing::debug!("javax.microedition.midlet.MIDlet::getAppProperty({this:?}, {key:?})");

        let key = JavaLangString::to_rust_string(jvm, &key).await?;
        let system_key = format!("wie.appProperty.{key}");
        let system_key = JavaLangString::from_rust_string(jvm, &system_key).await?;

        jvm.invoke_static("java/lang/System", "getProperty", "(Ljava/lang/String;)Ljava/lang/String;", (system_key,))
            .await
    }

    /// ★ heroZero fork (2026-10-22): the MIDlet saying it has finished — pass it to the platform,
    /// as the WIPI kernel's exit already does (`wie_wipi_c` `MC_knlExit`). It used to be a stub, so
    /// a game's own 「종료」 (영웅서기 Zero: `destroyApp(true)` → `notifyDestroyed()`) left the player
    /// staring at the last frame with nothing running. What "exit" means is the host's call.
    async fn notify_destroyed(_jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::info!("javax.microedition.midlet.MIDlet::notifyDestroyed({this:?})");

        context.system().platform().exit();

        Ok(())
    }

    /// ★ heroZero fork (2026-10-21): MIDP 2.0 `platformRequest` — the MIDlet asking the phone to
    /// open a URL in its browser or dial a `tel:` number. Nothing on this platform can take that
    /// over, so the URL is logged and the request is answered `false` ("the MIDlet need not exit
    /// for it to proceed") without anything happening; there is no browser or dialer to hand it to.
    async fn platform_request(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        url: ClassInstanceRef<String>,
    ) -> JvmResult<bool> {
        tracing::debug!("javax.microedition.midlet.MIDlet::platformRequest({this:?}, {url:?})");

        let url = if url.is_null() {
            "<null>".into()
        } else {
            JavaLangString::to_rust_string(jvm, &url).await?
        };
        tracing::info!("MIDlet.platformRequest({url:?}) ignored: no browser or dialer on this platform");

        Ok(false)
    }

    pub async fn display(jvm: &Jvm, this: &ClassInstanceRef<Self>) -> JvmResult<ClassInstanceRef<Display>> {
        jvm.get_field(this, "display", "Ljavax/microedition/lcdui/Display;").await
    }
}
