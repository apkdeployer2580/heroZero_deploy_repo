use alloc::{format, vec};

use java_class_proto::{JavaFieldProto, JavaMethodProto};
use jvm::{ClassInstanceRef, Jvm, Result as JvmResult};

use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use crate::classes::javax::microedition::lcdui::{Command, CommandListener, Display, Graphics};

// class javax.microedition.lcdui.Displayable
pub struct Displayable;

impl Displayable {
    pub fn as_proto() -> WieJavaClassProto {
        WieJavaClassProto {
            name: "javax/microedition/lcdui/Displayable",
            parent_class: Some("java/lang/Object"),
            interfaces: vec![],
            methods: vec![
                JavaMethodProto::new("<init>", "()V", Self::init, Default::default()),
                JavaMethodProto::new(
                    "addCommand",
                    "(Ljavax/microedition/lcdui/Command;)V",
                    Self::add_command,
                    Default::default(),
                ),
                JavaMethodProto::new(
                    "setCommandListener",
                    "(Ljavax/microedition/lcdui/CommandListener;)V",
                    Self::set_command_listener,
                    Default::default(),
                ),
                JavaMethodProto::new("getWidth", "()I", Self::get_width, Default::default()),
                JavaMethodProto::new("getHeight", "()I", Self::get_height, Default::default()),
                // ★ heroZero fork (2026-10-21): MIDP 2.0, called by the game's `f`. See `is_shown`.
                JavaMethodProto::new("isShown", "()Z", Self::is_shown, Default::default()),
                // wie private methods...
                JavaMethodProto::new(
                    "setDisplay",
                    "(Ljavax/microedition/lcdui/Display;)V",
                    Self::set_display,
                    Default::default(),
                ),
                JavaMethodProto::new("handleKeyEvent", "(II)V", Self::handle_key_event, Default::default()),
                JavaMethodProto::new(
                    "handlePaintEvent",
                    "(Ljavax/microedition/lcdui/Graphics;)V",
                    Self::handle_paint_event,
                    Default::default(),
                ),
                JavaMethodProto::new("handleNotifyEvent", "(III)V", Self::handle_notify_event, Default::default()),
            ],
            fields: vec![
                JavaFieldProto::new("currentDisplay", "Ljavax/microedition/lcdui/Display;", Default::default()),
                JavaFieldProto::new("isInFullScreenMode", "Z", Default::default()),
            ],
            access_flags: Default::default(),
        }
    }

    async fn init(jvm: &Jvm, _context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<()> {
        tracing::debug!("javax.microedition.lcdui.Displayable::<init>({this:?})");

        let _: () = jvm.invoke_special(&this, "java/lang/Object", "<init>", "()V", ()).await?;

        Ok(())
    }

    async fn add_command(
        _jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        command: ClassInstanceRef<Command>,
    ) -> JvmResult<()> {
        tracing::warn!("stub javax.microedition.lcdui.Displayable::addCommand({this:?}, {command:?})");

        Ok(())
    }

    async fn set_command_listener(
        _jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        listener: ClassInstanceRef<CommandListener>,
    ) -> JvmResult<()> {
        tracing::warn!("stub javax.microedition.lcdui.Displayable::setCommandListener({this:?}, {listener:?})");

        Ok(())
    }

    async fn set_display(
        jvm: &Jvm,
        _context: &mut WieJvmContext,
        mut this: ClassInstanceRef<Self>,
        display: ClassInstanceRef<Display>,
    ) -> JvmResult<()> {
        // tracing hates variable named `display`..
        let log = format!("javax.microedition.lcdui.Displayable::setDisplay({this:?}, {display:?})");
        tracing::debug!("{log}");

        jvm.put_field(&mut this, "currentDisplay", "Ljavax/microedition/lcdui/Display;", display)
            .await?;

        Ok(())
    }

    async fn get_width(jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<i32> {
        tracing::debug!("javax.microedition.lcdui.Displayable::getWidth({this:?})");

        let display: ClassInstanceRef<Display> = jvm.get_field(&this, "currentDisplay", "Ljavax/microedition/lcdui/Display;").await?;
        let width = if display.is_null() {
            context.system().platform().screen().width() as i32
        } else {
            jvm.invoke_virtual(&display, "getWidth", "()I", ()).await?
        };

        Ok(width)
    }

    async fn get_height(jvm: &Jvm, context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<i32> {
        tracing::debug!("javax.microedition.lcdui.Displayable::getHeight({this:?})");

        let display: ClassInstanceRef<Display> = jvm.get_field(&this, "currentDisplay", "Ljavax/microedition/lcdui/Display;").await?;
        let height = if display.is_null() {
            context.system().platform().screen().height() as i32
        } else {
            jvm.invoke_virtual(&display, "getHeight", "()I", ()).await?
        };

        Ok(height)
    }

    /// ★ heroZero fork (2026-10-21): MIDP 2.0 `isShown`.
    ///
    /// Shown = this is its Display's current Displayable. `Display::setCurrent` hands the new
    /// current one its Display through `setDisplay` and clears the previous one's, so
    /// `currentDisplay` is non-null only on the current Displayable; the identity check against
    /// `Display.currentDisplayable` is there so a stale back-pointer can never read as shown.
    ///
    /// Not modelled, because this emulator has neither: the MIDlet being paused or backgrounded,
    /// and a system screen obscuring the Displayable. A current Displayable always reads as shown.
    async fn is_shown(jvm: &Jvm, _context: &mut WieJvmContext, this: ClassInstanceRef<Self>) -> JvmResult<bool> {
        tracing::debug!("javax.microedition.lcdui.Displayable::isShown({this:?})");

        let display: ClassInstanceRef<Display> = jvm.get_field(&this, "currentDisplay", "Ljavax/microedition/lcdui/Display;").await?;
        if display.is_null() {
            return Ok(false);
        }

        let current: ClassInstanceRef<Displayable> = jvm
            .get_field(&display, "currentDisplayable", "Ljavax/microedition/lcdui/Displayable;")
            .await?;

        Ok(!current.is_null() && current.instance == this.instance)
    }

    async fn handle_key_event(_jvm: &Jvm, _context: &mut WieJvmContext, this: ClassInstanceRef<Self>, event_type: i32, code: i32) -> JvmResult<()> {
        tracing::debug!("javax.microedition.lcdui.Displayable::handleKeyEvent({this:?}, {event_type}, {code})");

        Ok(())
    }

    async fn handle_paint_event(
        _jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        graphics: ClassInstanceRef<Graphics>,
    ) -> JvmResult<()> {
        tracing::debug!("javax.microedition.lcdui.Displayable::handlePaintEvent({this:?}, {graphics:?})");

        Ok(())
    }

    async fn handle_notify_event(
        _jvm: &Jvm,
        _context: &mut WieJvmContext,
        this: ClassInstanceRef<Self>,
        r#type: i32,
        param1: i32,
        param2: i32,
    ) -> JvmResult<()> {
        tracing::debug!(
            "javax.microedition.lcdui.Displayable::handleNotifyEvent({this:?}, {}, {param1}, {param2})",
            r#type,
        );

        Ok(())
    }
}
