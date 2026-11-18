//! The game's vibration (`Display.vibrate`) → the phone's vibrator, OFF the emulator thread (C1,
//! hero2's `vibration.rs`, 2026-08-03 — its player: *"크리티컬이 뜨면 살짝 느려진다"*).
//!
//! The game buzzes from its own thread `az`: `au.a(B)V` calls `Display.vibrate(80)` twice, 80 ms
//! apart (javap). wie runs every guest thread on the emulator worker, so the call used to land there
//! as it was: a JNI attach and detach, a class lookup through the app's loader, `getSystemService`
//! and a synchronous binder call to system_server — inside a frame. Now that thread does one
//! `try_send` on a bounded channel (it cannot block: a full queue drops the buzz, which nobody
//! notices; a stalled game everybody does), and this thread, attached once for the life of the
//! process with the `Vibrator` held as a global ref, makes the call where nothing waits on it.
//! Requests that pile up while a call is in flight are one buzz, the longest.
//!
//! Failures are logged, never fatal: without a vibrator (or the JNI side) the thread ends and later
//! requests are dropped.

#[cfg(target_os = "android")]
use std::sync::{
    OnceLock,
    mpsc::{Receiver, SyncSender, sync_channel},
};

/// Queue depth: haptics are a "now" signal, a backlog is worse than a drop.
#[cfg(target_os = "android")]
const QUEUE: usize = 4;

/// Ceiling on one buzz, so a bogus guest value cannot vibrate for long.
pub const MAX_MS: u64 = 5_000;

#[cfg(target_os = "android")]
static TX: OnceLock<Option<SyncSender<u64>>> = OnceLock::new();

/// Ask for a buzz. Never blocks and never touches the JVM — the emulator thread calls it.
#[cfg(target_os = "android")]
pub fn vibrate(ms: u64) {
    if ms == 0 {
        return;
    }
    let Some(tx) = TX.get_or_init(spawn) else { return };
    let _ = tx.try_send(ms.min(MAX_MS));
}

#[cfg(not(target_os = "android"))]
pub fn vibrate(_ms: u64) {}

#[cfg(target_os = "android")]
fn spawn() -> Option<SyncSender<u64>> {
    let (tx, rx) = sync_channel::<u64>(QUEUE);
    match std::thread::Builder::new().name("hz-vibe".into()).spawn(move || worker(rx)) {
        Ok(_) => Some(tx),
        Err(e) => {
            log::warn!("vibration: could not start the vibrator thread: {e}");
            None
        }
    }
}

#[cfg(target_os = "android")]
fn worker(rx: Receiver<u64>) {
    use jni::objects::{JObject, JValue};

    let ctx = ndk_context::android_context();
    let vm = match unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
        Ok(vm) => vm,
        Err(e) => {
            log::warn!("vibration: JavaVM::from_raw failed: {e:?}");
            return;
        }
    };
    // Once, as a daemon, never detached: attach and detach per buzz was much of the cost.
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(e) => {
            log::warn!("vibration: attach failed: {e:?}");
            return;
        }
    };
    // The Vibrator once, through the context native code is handed (a framework class resolves
    // from this thread; an app class would not — `crate::java` goes through the app's loader).
    let vibrator = {
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let resolved = (|| -> Result<Option<jni::objects::GlobalRef>, jni::errors::Error> {
            let name = env.new_string("vibrator")?;
            let v = env.call_method(&context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()?;
            if v.is_null() || !env.call_method(&v, "hasVibrator", "()Z", &[])?.z()? {
                return Ok(None);
            }
            Ok(Some(env.new_global_ref(&v)?))
        })();
        match resolved {
            Ok(Some(v)) => v,
            Ok(None) => {
                log::info!("vibration: no vibrator on this device");
                return;
            }
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
                log::warn!("vibration: cannot reach the Vibrator: {e:?}");
                return;
            }
        }
    };
    log::info!("vibration: ready (its own thread, attached once)");
    while let Ok(first) = rx.recv() {
        let ms = coalesce(first, std::iter::from_fn(|| rx.try_recv().ok()));
        // VibrationEffect (API 26, this app's floor), as MainActivity.vibrate did.
        let r = (|| -> Result<(), jni::errors::Error> {
            let effect = env
                .call_static_method("android/os/VibrationEffect", "createOneShot", "(JI)Landroid/os/VibrationEffect;", &[JValue::Long(ms as i64), JValue::Int(-1)])?
                .l()?;
            env.call_method(&vibrator, "vibrate", "(Landroid/os/VibrationEffect;)V", &[JValue::Object(&effect)])?;
            env.delete_local_ref(effect)?;
            Ok(())
        })();
        if r.is_err() {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            log::warn!("vibration: vibrate({ms} ms) failed");
        }
    }
}

/// What piled up behind `first` while the last call was in flight is one buzz: the longest.
pub fn coalesce(first: u64, more: impl Iterator<Item = u64>) -> u64 {
    more.fold(first, u64::max).min(MAX_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_one_buzz_the_longest_and_never_past_the_ceiling() {
        assert_eq!(coalesce(80, [80, 200, 40].into_iter()), 200);
        assert_eq!(coalesce(80, std::iter::empty()), 80);
        assert_eq!(coalesce(9_999, std::iter::empty()), MAX_MS);
        vibrate(80); // the host build is a no-op
    }
}
