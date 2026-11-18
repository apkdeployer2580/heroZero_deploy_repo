//! Hardware keys (a Bluetooth keyboard, a phone's own keys, `adb shell input keyevent`) → the
//! handset key the game expects. Pure, so it is tested on the host.

use wie_backend::KeyCode;
use winit::keyboard::{KeyCode as Code, NativeKeyCode, PhysicalKey};

/// Android `KeyEvent` codes winit does not name (it passes them through as
/// `NativeKeyCode::Android`). Values from `android.view.KeyEvent`.
const AKEY_SOFT_LEFT: u32 = 1;
const AKEY_SOFT_RIGHT: u32 = 2;
const AKEY_BACK: u32 = 4;
const AKEY_STAR: u32 = 17;
const AKEY_POUND: u32 = 18;
const AKEY_DPAD_CENTER: u32 = 23;

pub fn map(key: PhysicalKey) -> Option<KeyCode> {
    Some(match key {
        PhysicalKey::Code(code) => match code {
            Code::ArrowUp | Code::KeyW => KeyCode::UP,
            Code::ArrowDown | Code::KeyS => KeyCode::DOWN,
            Code::ArrowLeft | Code::KeyA => KeyCode::LEFT,
            Code::ArrowRight | Code::KeyD => KeyCode::RIGHT,
            Code::Enter | Code::NumpadEnter | Code::Space => KeyCode::OK,
            // Q / E: the soft keys on a keyboard, where WASD is the D-pad.
            Code::KeyQ | Code::F1 => KeyCode::LEFT_SOFT_KEY,
            Code::KeyE | Code::F2 | Code::Escape | Code::Backspace => KeyCode::RIGHT_SOFT_KEY,
            Code::Digit0 | Code::Numpad0 => KeyCode::NUM0,
            Code::Digit1 | Code::Numpad1 => KeyCode::NUM1,
            Code::Digit2 | Code::Numpad2 => KeyCode::NUM2,
            Code::Digit3 | Code::Numpad3 => KeyCode::NUM3,
            Code::Digit4 | Code::Numpad4 => KeyCode::NUM4,
            Code::Digit5 | Code::Numpad5 => KeyCode::NUM5,
            Code::Digit6 | Code::Numpad6 => KeyCode::NUM6,
            Code::Digit7 | Code::Numpad7 => KeyCode::NUM7,
            Code::Digit8 | Code::Numpad8 => KeyCode::NUM8,
            Code::Digit9 | Code::Numpad9 => KeyCode::NUM9,
            Code::NumpadMultiply => KeyCode::STAR,
            Code::NumpadSubtract => KeyCode::HASH,
            _ => return None,
        },
        PhysicalKey::Unidentified(NativeKeyCode::Android(code)) => match code {
            AKEY_SOFT_LEFT => KeyCode::LEFT_SOFT_KEY,
            // The phone's back key is the game's back (RSK closes windows and skips dialogue).
            AKEY_SOFT_RIGHT | AKEY_BACK => KeyCode::RIGHT_SOFT_KEY,
            AKEY_STAR => KeyCode::STAR,
            AKEY_POUND => KeyCode::HASH,
            AKEY_DPAD_CENTER => KeyCode::OK,
            // Gamepad buttons reach winit this way (its D-pad arrives as Arrow* above); the
            // sticks come over JNI (`crate::gamepad`).
            code => crate::gamepad::button_key(code)?,
        },
        _ => return None,
    })
}

/// HZ-059: a handset key's walk direction (`hz_java::input::dir`) — the four arrows; two held at once
/// are a diagonal (`KeyProjection::walk_edge`).
pub fn arrow(k: KeyCode) -> Option<i8> {
    use hz_java::input::dir;
    match k {
        KeyCode::UP => Some(dir::UP),
        KeyCode::DOWN => Some(dir::DOWN),
        KeyCode::LEFT => Some(dir::LEFT),
        KeyCode::RIGHT => Some(dir::RIGHT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_and_phone_keys_map_to_handset_keys() {
        assert_eq!(map(PhysicalKey::Code(Code::ArrowUp)), Some(KeyCode::UP));
        assert_eq!(map(PhysicalKey::Code(Code::Enter)), Some(KeyCode::OK));
        assert_eq!(map(PhysicalKey::Code(Code::Digit0)), Some(KeyCode::NUM0));
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(AKEY_POUND))), Some(KeyCode::HASH));
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(AKEY_SOFT_LEFT))), Some(KeyCode::LEFT_SOFT_KEY));
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(AKEY_BACK))), Some(KeyCode::RIGHT_SOFT_KEY));
        assert_eq!(map(PhysicalKey::Code(Code::KeyZ)), None);
        // Gamepad buttons, through winit's Android pass-through.
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(crate::gamepad::KEYCODE_BUTTON_A))), Some(KeyCode::OK));
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(crate::gamepad::KEYCODE_BUTTON_START))), Some(KeyCode::LEFT_SOFT_KEY));
        assert_eq!(map(PhysicalKey::Unidentified(NativeKeyCode::Android(200))), None);
    }
}
