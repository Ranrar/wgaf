//! `evdev`/`uinput` event-type, key, button, and axis codes, plus the
//! name/character lookup tables used to translate the daemon's public API
//! (key names as strings, ASCII text, mouse button names) into the raw
//! numeric codes `uinput` expects.
//!
//! The numeric values below come from the kernel's `<linux/input-event-codes.h>`,
//! which is a stable, frozen userspace ABI (values are never renumbered,
//! only appended to) — hand-transcribing them here is standard practice for
//! a `uinput` client that doesn't want to add a C header dependency (`libc`
//! covers the `struct`s in `<linux/input.h>`/`<linux/uinput.h>`, but not
//! these `#define`d codes).

// ---------------------------------------------------------------------------
// Event types (`input_event.type_`)
// ---------------------------------------------------------------------------

pub(crate) const EV_SYN: u16 = 0x00;
pub(crate) const EV_KEY: u16 = 0x01;
pub(crate) const EV_REL: u16 = 0x02;

/// The only `EV_SYN` code we emit: marks the end of a logically-grouped
/// batch of events (e.g. `REL_X` + `REL_Y` for one mouse move), telling
/// readers of the device to process everything since the last `SYN_REPORT`
/// as one atomic update.
pub(crate) const SYN_REPORT: u16 = 0x00;

// ---------------------------------------------------------------------------
// Relative axes (`EV_REL` codes)
// ---------------------------------------------------------------------------

pub(crate) const REL_X: u16 = 0x00;
pub(crate) const REL_Y: u16 = 0x01;
pub(crate) const REL_HWHEEL: u16 = 0x06;
pub(crate) const REL_WHEEL: u16 = 0x08;

// ---------------------------------------------------------------------------
// Mouse buttons (`EV_KEY` codes in the `BTN_MOUSE` range)
// ---------------------------------------------------------------------------

pub(crate) const BTN_LEFT: u16 = 0x110;
pub(crate) const BTN_RIGHT: u16 = 0x111;
pub(crate) const BTN_MIDDLE: u16 = 0x112;

/// Maps a mouse button name (case-insensitive) to its `BTN_*` code.
/// `MouseClick`'s only accepted values are `left`/`right`/`middle`.
pub(crate) fn button_name_to_code(name: &str) -> Option<u16> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Some(BTN_LEFT),
        "right" => Some(BTN_RIGHT),
        "middle" => Some(BTN_MIDDLE),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Keyboard keys (`EV_KEY` codes in the `KEY_*` range, US-QWERTY layout)
// ---------------------------------------------------------------------------

pub(crate) const KEY_ESC: u16 = 1;
pub(crate) const KEY_1: u16 = 2;
pub(crate) const KEY_2: u16 = 3;
pub(crate) const KEY_3: u16 = 4;
pub(crate) const KEY_4: u16 = 5;
pub(crate) const KEY_5: u16 = 6;
pub(crate) const KEY_6: u16 = 7;
pub(crate) const KEY_7: u16 = 8;
pub(crate) const KEY_8: u16 = 9;
pub(crate) const KEY_9: u16 = 10;
pub(crate) const KEY_0: u16 = 11;
pub(crate) const KEY_MINUS: u16 = 12;
pub(crate) const KEY_EQUAL: u16 = 13;
pub(crate) const KEY_BACKSPACE: u16 = 14;
pub(crate) const KEY_TAB: u16 = 15;
pub(crate) const KEY_Q: u16 = 16;
pub(crate) const KEY_W: u16 = 17;
pub(crate) const KEY_E: u16 = 18;
pub(crate) const KEY_R: u16 = 19;
pub(crate) const KEY_T: u16 = 20;
pub(crate) const KEY_Y: u16 = 21;
pub(crate) const KEY_U: u16 = 22;
pub(crate) const KEY_I: u16 = 23;
pub(crate) const KEY_O: u16 = 24;
pub(crate) const KEY_P: u16 = 25;
pub(crate) const KEY_LEFTBRACE: u16 = 26;
pub(crate) const KEY_RIGHTBRACE: u16 = 27;
pub(crate) const KEY_ENTER: u16 = 28;
pub(crate) const KEY_LEFTCTRL: u16 = 29;
pub(crate) const KEY_A: u16 = 30;
pub(crate) const KEY_S: u16 = 31;
pub(crate) const KEY_D: u16 = 32;
pub(crate) const KEY_F: u16 = 33;
pub(crate) const KEY_G: u16 = 34;
pub(crate) const KEY_H: u16 = 35;
pub(crate) const KEY_J: u16 = 36;
pub(crate) const KEY_K: u16 = 37;
pub(crate) const KEY_L: u16 = 38;
pub(crate) const KEY_SEMICOLON: u16 = 39;
pub(crate) const KEY_APOSTROPHE: u16 = 40;
pub(crate) const KEY_GRAVE: u16 = 41;
pub(crate) const KEY_LEFTSHIFT: u16 = 42;
pub(crate) const KEY_BACKSLASH: u16 = 43;
pub(crate) const KEY_Z: u16 = 44;
pub(crate) const KEY_X: u16 = 45;
pub(crate) const KEY_C: u16 = 46;
pub(crate) const KEY_V: u16 = 47;
pub(crate) const KEY_B: u16 = 48;
pub(crate) const KEY_N: u16 = 49;
pub(crate) const KEY_M: u16 = 50;
pub(crate) const KEY_COMMA: u16 = 51;
pub(crate) const KEY_DOT: u16 = 52;
pub(crate) const KEY_SLASH: u16 = 53;
pub(crate) const KEY_RIGHTSHIFT: u16 = 54;
pub(crate) const KEY_LEFTALT: u16 = 56;
pub(crate) const KEY_SPACE: u16 = 57;
pub(crate) const KEY_CAPSLOCK: u16 = 58;

/// Every `KEY_*`/`BTN_*` code this daemon's virtual device registers via
/// `UI_SET_KEYBIT` at creation time (see `device.rs`). Deliberately a fixed,
/// enumerable list rather than "all 0..KEY_MAX" — registering only the keys
/// we can actually address keeps the virtual device's advertised
/// capabilities honest (`libinput`/Wayland compositors inspect these bits to
/// decide how to treat the device).
pub(crate) const ALL_KEYS: &[u16] = &[
    KEY_ESC,
    KEY_1,
    KEY_2,
    KEY_3,
    KEY_4,
    KEY_5,
    KEY_6,
    KEY_7,
    KEY_8,
    KEY_9,
    KEY_0,
    KEY_MINUS,
    KEY_EQUAL,
    KEY_BACKSPACE,
    KEY_TAB,
    KEY_Q,
    KEY_W,
    KEY_E,
    KEY_R,
    KEY_T,
    KEY_Y,
    KEY_U,
    KEY_I,
    KEY_O,
    KEY_P,
    KEY_LEFTBRACE,
    KEY_RIGHTBRACE,
    KEY_ENTER,
    KEY_LEFTCTRL,
    KEY_A,
    KEY_S,
    KEY_D,
    KEY_F,
    KEY_G,
    KEY_H,
    KEY_J,
    KEY_K,
    KEY_L,
    KEY_SEMICOLON,
    KEY_APOSTROPHE,
    KEY_GRAVE,
    KEY_LEFTSHIFT,
    KEY_BACKSLASH,
    KEY_Z,
    KEY_X,
    KEY_C,
    KEY_V,
    KEY_B,
    KEY_N,
    KEY_M,
    KEY_COMMA,
    KEY_DOT,
    KEY_SLASH,
    KEY_RIGHTSHIFT,
    KEY_LEFTALT,
    KEY_SPACE,
    KEY_CAPSLOCK,
    BTN_LEFT,
    BTN_RIGHT,
    BTN_MIDDLE,
];

/// Maps a key name (case-insensitive, optional `KEY_` prefix, matching
/// `<linux/input-event-codes.h>` naming) to its evdev code. Used by
/// `KeyPress`/`KeyRelease` — these are low-level primitives operating on one
/// physical key at a time, with no ASCII/shift awareness (that's
/// `TypeText`'s job, via [`ascii_to_keycode`]); holding a modifier for a
/// capital letter or symbol is the caller's responsibility, e.g.
/// `key press leftshift`, `key press a`, `key release a`, `key release
/// leftshift`.
///
/// Single-character names `"0"`-`"9"` and `"a"`-`"z"` are accepted directly
/// alongside the full `KEY_*` names, since typing `wgaf key press a` is far
/// more natural than `wgaf key press KEY_A`.
pub(crate) fn key_name_to_code(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    let bare = upper.strip_prefix("KEY_").unwrap_or(&upper);

    // Single letters/digits.
    if let Some(c) = single_char(bare) {
        if c.is_ascii_uppercase() {
            return Some(KEY_A + (c as u16 - b'A' as u16));
        }
        if c.is_ascii_digit() {
            return Some(if c == '0' {
                KEY_0
            } else {
                KEY_1 + (c as u16 - b'1' as u16)
            });
        }
    }

    Some(match bare {
        "ESC" | "ESCAPE" => KEY_ESC,
        "MINUS" | "DASH" => KEY_MINUS,
        "EQUAL" | "EQUALS" => KEY_EQUAL,
        "BACKSPACE" => KEY_BACKSPACE,
        "TAB" => KEY_TAB,
        "LEFTBRACE" => KEY_LEFTBRACE,
        "RIGHTBRACE" => KEY_RIGHTBRACE,
        "ENTER" | "RETURN" => KEY_ENTER,
        "LEFTCTRL" | "CTRL" | "CONTROL" => KEY_LEFTCTRL,
        "SEMICOLON" => KEY_SEMICOLON,
        "APOSTROPHE" | "QUOTE" => KEY_APOSTROPHE,
        "GRAVE" | "BACKTICK" => KEY_GRAVE,
        "LEFTSHIFT" | "SHIFT" => KEY_LEFTSHIFT,
        "BACKSLASH" => KEY_BACKSLASH,
        "RIGHTSHIFT" => KEY_RIGHTSHIFT,
        "LEFTALT" | "ALT" => KEY_LEFTALT,
        "SPACE" => KEY_SPACE,
        "CAPSLOCK" | "CAPS" => KEY_CAPSLOCK,
        "COMMA" => KEY_COMMA,
        "DOT" | "PERIOD" => KEY_DOT,
        "SLASH" => KEY_SLASH,
        _ => return None,
    })
}

fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    if chars.next().is_none() {
        Some(c)
    } else {
        None
    }
}

/// Maps one ASCII character to `(keycode, needs_shift)` for `TypeText`,
/// following a US-QWERTY layout — deliberately the same scope limitation
/// `ydotool` documents for its own `type` command: no Unicode, no
/// layout-awareness (XKB), just a flat 7-bit ASCII table. Returns `None` for
/// anything outside printable ASCII plus `\n`/`\t`, which `TypeText`
/// surfaces as [`crate::input::InputError::UnknownKey`] naming the
/// offending character rather than silently skipping it.
pub(crate) fn ascii_to_keycode(c: char) -> Option<(u16, bool)> {
    if !c.is_ascii() {
        return None;
    }
    let b = c as u8;
    Some(match b {
        b'\n' => (KEY_ENTER, false),
        b'\t' => (KEY_TAB, false),
        b' ' => (KEY_SPACE, false),
        b'0'..=b'9' => {
            let code = if b == b'0' {
                KEY_0
            } else {
                KEY_1 + (b - b'1') as u16
            };
            (code, false)
        }
        b'a'..=b'z' => (KEY_A + (b - b'a') as u16, false),
        b'A'..=b'Z' => (KEY_A + (b - b'A') as u16, true),
        b'!' => (KEY_1, true),
        b'"' => (KEY_APOSTROPHE, true),
        b'#' => (KEY_3, true),
        b'$' => (KEY_4, true),
        b'%' => (KEY_5, true),
        b'&' => (KEY_7, true),
        b'\'' => (KEY_APOSTROPHE, false),
        b'(' => (KEY_9, true),
        b')' => (KEY_0, true),
        b'*' => (KEY_8, true),
        b'+' => (KEY_EQUAL, true),
        b',' => (KEY_COMMA, false),
        b'-' => (KEY_MINUS, false),
        b'.' => (KEY_DOT, false),
        b'/' => (KEY_SLASH, false),
        b':' => (KEY_SEMICOLON, true),
        b';' => (KEY_SEMICOLON, false),
        b'<' => (KEY_COMMA, true),
        b'=' => (KEY_EQUAL, false),
        b'>' => (KEY_DOT, true),
        b'?' => (KEY_SLASH, true),
        b'@' => (KEY_2, true),
        b'[' => (KEY_LEFTBRACE, false),
        b'\\' => (KEY_BACKSLASH, false),
        b']' => (KEY_RIGHTBRACE, false),
        b'^' => (KEY_6, true),
        b'_' => (KEY_MINUS, true),
        b'`' => (KEY_GRAVE, false),
        b'{' => (KEY_LEFTBRACE, true),
        b'|' => (KEY_BACKSLASH, true),
        b'}' => (KEY_RIGHTBRACE, true),
        b'~' => (KEY_GRAVE, true),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_codes_match_known_kernel_constants() {
        // Cross-check against the well-known literal values documented in
        // <linux/uinput.h> (unchanged for decades) — see also
        // `device.rs`'s own `ioc`-formula self-check test. Kept here too
        // since these codes are what `ALL_KEYS`/button lookups feed into.
        assert_eq!(BTN_LEFT, 0x110);
        assert_eq!(BTN_RIGHT, 0x111);
        assert_eq!(BTN_MIDDLE, 0x112);
    }

    #[test]
    fn key_name_lookup_is_case_insensitive_and_accepts_key_prefix() {
        assert_eq!(key_name_to_code("a"), Some(KEY_A));
        assert_eq!(key_name_to_code("A"), Some(KEY_A));
        assert_eq!(key_name_to_code("KEY_A"), Some(KEY_A));
        assert_eq!(key_name_to_code("key_a"), Some(KEY_A));
        assert_eq!(key_name_to_code("enter"), Some(KEY_ENTER));
        assert_eq!(key_name_to_code("Return"), Some(KEY_ENTER));
        assert_eq!(key_name_to_code("leftshift"), Some(KEY_LEFTSHIFT));
        assert_eq!(key_name_to_code("shift"), Some(KEY_LEFTSHIFT));
        assert_eq!(key_name_to_code("9"), Some(KEY_9));
        assert_eq!(key_name_to_code("0"), Some(KEY_0));
    }

    #[test]
    fn key_name_lookup_rejects_unknown_names() {
        assert_eq!(key_name_to_code("not_a_real_key"), None);
        assert_eq!(key_name_to_code(""), None);
    }

    #[test]
    fn button_name_lookup_is_case_insensitive() {
        assert_eq!(button_name_to_code("left"), Some(BTN_LEFT));
        assert_eq!(button_name_to_code("Right"), Some(BTN_RIGHT));
        assert_eq!(button_name_to_code("MIDDLE"), Some(BTN_MIDDLE));
        assert_eq!(button_name_to_code("nope"), None);
    }

    #[test]
    fn ascii_table_covers_all_printable_ascii_and_whitespace_helpers() {
        for b in 0x20u8..=0x7e {
            assert!(
                ascii_to_keycode(b as char).is_some(),
                "printable ASCII 0x{b:02x} (`{}`) should map to a key",
                b as char
            );
        }
        assert!(ascii_to_keycode('\n').is_some());
        assert!(ascii_to_keycode('\t').is_some());
    }

    #[test]
    fn ascii_table_rejects_non_ascii_and_other_control_chars() {
        assert_eq!(ascii_to_keycode('é'), None);
        assert_eq!(ascii_to_keycode('日'), None);
        assert_eq!(ascii_to_keycode('\r'), None);
        assert_eq!(ascii_to_keycode('\x01'), None);
    }

    #[test]
    fn shifted_symbols_are_flagged_correctly() {
        assert_eq!(ascii_to_keycode('a'), Some((KEY_A, false)));
        assert_eq!(ascii_to_keycode('A'), Some((KEY_A, true)));
        assert_eq!(ascii_to_keycode('1'), Some((KEY_1, false)));
        assert_eq!(ascii_to_keycode('!'), Some((KEY_1, true)));
        assert_eq!(ascii_to_keycode('0'), Some((KEY_0, false)));
        assert_eq!(ascii_to_keycode(')'), Some((KEY_0, true)));
    }
}
