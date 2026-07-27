//! Keyboard primitives: single key press/release, and the higher-level
//! `TypeText` ASCII helper built on top of them.

use crate::input::InputError;
use crate::input::codes::{self, KEY_LEFTSHIFT};
use crate::input::device::UinputDevice;

/// Presses (value `1`) the given evdev key code.
pub(crate) fn press(device: &mut UinputDevice, code: u16) -> Result<(), InputError> {
    device.key_event(code, 1)
}

/// Releases (value `0`) the given evdev key code.
pub(crate) fn release(device: &mut UinputDevice, code: u16) -> Result<(), InputError> {
    device.key_event(code, 0)
}

/// Resolves a key name via [`codes::key_name_to_code`], or
/// [`InputError::UnknownKey`] naming the input verbatim.
pub(crate) fn resolve_key(name: &str) -> Result<u16, InputError> {
    codes::key_name_to_code(name).ok_or_else(|| InputError::UnknownKey(name.to_string()))
}

/// Types `text` one character at a time: for each character, looks it up in
/// the ASCII/US-QWERTY table ([`codes::ascii_to_keycode`]), holding
/// `KEY_LEFTSHIFT` around the key press/release when the character needs
/// it. Scoped to 7-bit ASCII (plus `\n`/`\t`) — see the module-level docs on
/// `codes::ascii_to_keycode` for why Unicode/other-layout input isn't
/// attempted.
///
/// Stops at the first unmappable character and returns
/// [`InputError::UnknownKey`] naming it — a partially-typed string is a
/// clearer failure mode than silently dropping characters.
pub(crate) fn type_text(device: &mut UinputDevice, text: &str) -> Result<(), InputError> {
    for c in text.chars() {
        let (code, needs_shift) =
            codes::ascii_to_keycode(c).ok_or_else(|| InputError::UnknownKey(c.to_string()))?;

        if needs_shift {
            press(device, KEY_LEFTSHIFT)?;
        }
        press(device, code)?;
        release(device, code)?;
        if needs_shift {
            release(device, KEY_LEFTSHIFT)?;
        }
    }
    Ok(())
}
