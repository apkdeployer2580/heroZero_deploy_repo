//! Calls into `net.wie.emulator.MainActivity`'s static methods over JNI.
//!
//! ⛔ The class is looked up BY NAME at call time. Moving MainActivity to another package compiles
//! cleanly and then fails here, silently, the first time orientation or a dialog is used — keep
//! the Gradle `namespace` at `net.wie.emulator` whatever the applicationId is.

use jni::objects::{JClass, JObject, JValue};

const ACTIVITY: &str = "net.wie.emulator.MainActivity";

/// An argument of a [`call_static`]: a primitive as it is, or a string made on the call's own
/// attached thread (a local reference lives only as long as that attachment).
enum Arg<'a> {
    Value(JValue<'a, 'a>),
    Str(&'a str),
}

/// `MainActivity.<method>(<args>)` on the app's class loader (the native thread's default loader
/// cannot see app classes). Failures are logged, never raised: none of these calls is worth a
/// crash.
fn call_static(method: &str, sig: &str, args: &[Arg]) {
    let result: Result<(), jni::errors::Error> = (|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let r: Result<(), jni::errors::Error> = (|| {
            let loader = env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string(ACTIVITY)?;
            let cls = env.call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?.l()?;
            let strings = args
                .iter()
                .map(|a| match a {
                    Arg::Str(s) => env.new_string(s).map(JObject::from),
                    Arg::Value(_) => Ok(JObject::null()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let values: Vec<JValue> = args
                .iter()
                .zip(&strings)
                .map(|(a, s)| match a {
                    Arg::Str(_) => JValue::Object(s),
                    Arg::Value(v) => *v,
                })
                .collect();
            env.call_static_method(JClass::from(cls), method, sig, &values)?;
            Ok(())
        })();
        if r.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        r
    })();
    if let Err(e) = result {
        log::error!("JNI MainActivity.{method}{sig} failed: {e:?}");
    }
}

/// `ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE` / `SENSOR_PORTRAIT`: either way up.
pub fn request_orientation(landscape: bool) {
    call_static("requestOrientation", "(I)V", &[Arg::Value(JValue::Int(if landscape { 6 } else { 7 }))]);
}

/// Settings › 진단 정보 (B3, hero2's `show_diagnostics`): `MainActivity.showDiagnostics` puts the
/// app version, Android and the device above `info` and offers 복사 · 닫기 · 세이브 관리.
pub fn show_diagnostics(info: &str) {
    call_static("showDiagnostics", "(Ljava/lang/String;)V", &[Arg::Str(info)]);
}

/// What the boot's 세이브 옮기기 import did (B4): `MainActivity.showSaveMessage` — the first line is
/// the dialog's title.
pub fn show_save_message(text: &str) {
    call_static("showSaveMessage", "(Ljava/lang/String;)V", &[Arg::Str(text)]);
}

/// The game said 「종료」: close the activity and end the process. Saves are already on disk —
/// every record write is synchronous and fsynced (`database::write_atomic`).
pub fn quit_app() {
    call_static("quitApp", "()V", &[]);
}
