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

/// How many kernel events [`type_text`] will emit for `text`.
///
/// Two per character (press + release), doubled for characters needing
/// `KEY_LEFTSHIFT` — the shift press and release are events in their own
/// right. This is why the rate limiter counts events rather than calls: a
/// single `TypeText` at the default character cap is around 16,000 events, and a
/// single `KeyPress` is one. They are not comparable units.
///
/// An unmappable character is charged 2 rather than 0. [`type_text`] stops at
/// the first one it hits, so the true cost is lower — but the charge is made
/// before the text is typed, and guessing where it will stop means resolving
/// every character twice. Overcharging a call that is about to fail is the
/// harmless direction to be wrong in.
pub(crate) fn type_text_event_cost(text: &str) -> u32 {
    text.chars()
        .map(|c| match codes::ascii_to_keycode(c) {
            Some((_, true)) => 4,
            _ => 2,
        })
        .sum()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unshifted_characters_cost_two_events_each() {
        assert_eq!(type_text_event_cost("abc"), 6);
    }

    #[test]
    fn shifted_characters_cost_four() {
        // The shift press and release are events in their own right.
        assert_eq!(type_text_event_cost("A"), 4);
        assert_eq!(type_text_event_cost("!"), 4);
    }

    #[test]
    fn mixed_text_sums_per_character() {
        // 'H' shifted (4) + "i" (2) = 6.
        assert_eq!(type_text_event_cost("Hi"), 6);
    }

    #[test]
    fn empty_text_costs_nothing() {
        assert_eq!(type_text_event_cost(""), 0);
    }

    /// The rate limiter's whole reason for counting events rather than calls:
    /// one maximum-length `TypeText` is worth thousands of `KeyPress` calls.
    #[test]
    fn a_max_length_all_caps_type_text_is_worth_thousands_of_key_presses() {
        let text: String = std::iter::repeat_n('A', 4096).collect();
        assert_eq!(type_text_event_cost(&text), 16_384);
    }

    /// Unmappable characters are charged rather than skipped — [`type_text`]
    /// will reject them, but the charge happens first.
    #[test]
    fn unmappable_characters_are_still_charged() {
        assert_eq!(type_text_event_cost("é"), 2);
    }
}
