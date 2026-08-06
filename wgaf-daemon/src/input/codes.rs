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

/// The mouse buttons the virtual device advertises. Registered alongside
/// [`KEYS`]' codes — see [`registered_codes`].
const BUTTONS: &[u16] = &[BTN_LEFT, BTN_RIGHT, BTN_MIDDLE];

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
pub(crate) const KEY_KPASTERISK: u16 = 55;
pub(crate) const KEY_LEFTALT: u16 = 56;
pub(crate) const KEY_SPACE: u16 = 57;
pub(crate) const KEY_CAPSLOCK: u16 = 58;
pub(crate) const KEY_F1: u16 = 59;
pub(crate) const KEY_F2: u16 = 60;
pub(crate) const KEY_F3: u16 = 61;
pub(crate) const KEY_F4: u16 = 62;
pub(crate) const KEY_F5: u16 = 63;
pub(crate) const KEY_F6: u16 = 64;
pub(crate) const KEY_F7: u16 = 65;
pub(crate) const KEY_F8: u16 = 66;
pub(crate) const KEY_F9: u16 = 67;
pub(crate) const KEY_F10: u16 = 68;
pub(crate) const KEY_NUMLOCK: u16 = 69;
pub(crate) const KEY_SCROLLLOCK: u16 = 70;
pub(crate) const KEY_KP7: u16 = 71;
pub(crate) const KEY_KP8: u16 = 72;
pub(crate) const KEY_KP9: u16 = 73;
pub(crate) const KEY_KPMINUS: u16 = 74;
pub(crate) const KEY_KP4: u16 = 75;
pub(crate) const KEY_KP5: u16 = 76;
pub(crate) const KEY_KP6: u16 = 77;
pub(crate) const KEY_KPPLUS: u16 = 78;
pub(crate) const KEY_KP1: u16 = 79;
pub(crate) const KEY_KP2: u16 = 80;
pub(crate) const KEY_KP3: u16 = 81;
pub(crate) const KEY_KP0: u16 = 82;
pub(crate) const KEY_KPDOT: u16 = 83;

/// The extra key a 105-key (ISO) keyboard has that a 104-key (ANSI) one does
/// not, sitting between left shift and `z`. It carries `<` and `>` on a German
/// or Danish layout, and `\` and `|` on a UK one — so it is not an exotic key,
/// it is where several everyday characters live outside the US.
pub(crate) const KEY_102ND: u16 = 86;
pub(crate) const KEY_F11: u16 = 87;
pub(crate) const KEY_F12: u16 = 88;
pub(crate) const KEY_KPENTER: u16 = 96;
pub(crate) const KEY_RIGHTCTRL: u16 = 97;
pub(crate) const KEY_KPSLASH: u16 = 98;
pub(crate) const KEY_SYSRQ: u16 = 99;

/// AltGr. On every non-US layout this is the third-level modifier that reaches
/// `@ $ { } [ ] | \` and similar — ordinary characters for automating a
/// developer's desktop, not a specialist key.
pub(crate) const KEY_RIGHTALT: u16 = 100;
pub(crate) const KEY_HOME: u16 = 102;
pub(crate) const KEY_UP: u16 = 103;
pub(crate) const KEY_PAGEUP: u16 = 104;
pub(crate) const KEY_LEFT: u16 = 105;
pub(crate) const KEY_RIGHT: u16 = 106;
pub(crate) const KEY_END: u16 = 107;
pub(crate) const KEY_DOWN: u16 = 108;
pub(crate) const KEY_PAGEDOWN: u16 = 109;
pub(crate) const KEY_INSERT: u16 = 110;
pub(crate) const KEY_DELETE: u16 = 111;
pub(crate) const KEY_PAUSE: u16 = 119;
pub(crate) const KEY_LEFTMETA: u16 = 125;
pub(crate) const KEY_RIGHTMETA: u16 = 126;

/// The context-menu key, right of the right-hand Meta key.
///
/// **This is `KEY_COMPOSE` (127), not `KEY_MENU` (139)**, despite the key being
/// labelled "Menu" and despite `KEY_MENU` existing — the kernel's PC keyboard
/// map has always sent 127 for it, and 139 belongs to a different key that
/// standard keyboards do not have. Getting this backwards produces a key press
/// the compositor accepts and no application reacts to, so the name
/// `"menu"` is deliberately bound to this code below.
pub(crate) const KEY_COMPOSE: u16 = 127;

/// Every key this daemon can address, paired with the names
/// [`key_name_to_code`] accepts for it.
///
/// **One table, deliberately.** The registered-capability list and the
/// name lookup were two hand-maintained tables until 2026-07-31, and they
/// disagreed: `key_name_to_code` was the shorter of the two in some places and
/// the longer in others, so a key could be *nameable but never advertised* (the
/// press is accepted, the compositor drops it, nothing reports an error) or
/// *advertised but unnameable*. Deriving both from this table makes that class
/// of drift impossible rather than merely tested for. Add a key here and it is
/// registered and pressable; there is nowhere else to update.
///
/// **Scope: the physical keys of a standard 105-key PC keyboard, and nothing
/// else.** Media, browser, and power keys are deliberately absent — they are a
/// different automation story with a different security argument, and
/// registering keys we have no use for would misrepresent what this device is
/// (`libinput` and Wayland compositors inspect these bits to decide how to
/// treat it).
///
/// Ordered by evdev code so the table can be read straight down against
/// `<linux/input-event-codes.h>`. Names are uppercase and carry no `KEY_`
/// prefix, because [`key_name_to_code`] normalizes its input to that form
/// before matching — a lowercase or prefixed entry here would simply never
/// match, which `key_table_names_are_normalized` guards against.
pub(crate) const KEYS: &[(u16, &[&str])] = &[
    (KEY_ESC, &["ESC", "ESCAPE"]),
    (KEY_1, &["1"]),
    (KEY_2, &["2"]),
    (KEY_3, &["3"]),
    (KEY_4, &["4"]),
    (KEY_5, &["5"]),
    (KEY_6, &["6"]),
    (KEY_7, &["7"]),
    (KEY_8, &["8"]),
    (KEY_9, &["9"]),
    (KEY_0, &["0"]),
    (KEY_MINUS, &["MINUS", "DASH"]),
    (KEY_EQUAL, &["EQUAL", "EQUALS"]),
    (KEY_BACKSPACE, &["BACKSPACE"]),
    (KEY_TAB, &["TAB"]),
    (KEY_Q, &["Q"]),
    (KEY_W, &["W"]),
    (KEY_E, &["E"]),
    (KEY_R, &["R"]),
    (KEY_T, &["T"]),
    (KEY_Y, &["Y"]),
    (KEY_U, &["U"]),
    (KEY_I, &["I"]),
    (KEY_O, &["O"]),
    (KEY_P, &["P"]),
    (KEY_LEFTBRACE, &["LEFTBRACE"]),
    (KEY_RIGHTBRACE, &["RIGHTBRACE"]),
    (KEY_ENTER, &["ENTER", "RETURN"]),
    (KEY_LEFTCTRL, &["LEFTCTRL", "CTRL", "CONTROL"]),
    (KEY_A, &["A"]),
    (KEY_S, &["S"]),
    (KEY_D, &["D"]),
    (KEY_F, &["F"]),
    (KEY_G, &["G"]),
    (KEY_H, &["H"]),
    (KEY_J, &["J"]),
    (KEY_K, &["K"]),
    (KEY_L, &["L"]),
    (KEY_SEMICOLON, &["SEMICOLON"]),
    (KEY_APOSTROPHE, &["APOSTROPHE", "QUOTE"]),
    (KEY_GRAVE, &["GRAVE", "BACKTICK"]),
    (KEY_LEFTSHIFT, &["LEFTSHIFT", "SHIFT"]),
    (KEY_BACKSLASH, &["BACKSLASH"]),
    (KEY_Z, &["Z"]),
    (KEY_X, &["X"]),
    (KEY_C, &["C"]),
    (KEY_V, &["V"]),
    (KEY_B, &["B"]),
    (KEY_N, &["N"]),
    (KEY_M, &["M"]),
    (KEY_COMMA, &["COMMA"]),
    (KEY_DOT, &["DOT", "PERIOD"]),
    (KEY_SLASH, &["SLASH"]),
    (KEY_RIGHTSHIFT, &["RIGHTSHIFT"]),
    (KEY_KPASTERISK, &["KPASTERISK", "KPMULTIPLY"]),
    (KEY_LEFTALT, &["LEFTALT", "ALT"]),
    (KEY_SPACE, &["SPACE"]),
    (KEY_CAPSLOCK, &["CAPSLOCK", "CAPS"]),
    (KEY_F1, &["F1"]),
    (KEY_F2, &["F2"]),
    (KEY_F3, &["F3"]),
    (KEY_F4, &["F4"]),
    (KEY_F5, &["F5"]),
    (KEY_F6, &["F6"]),
    (KEY_F7, &["F7"]),
    (KEY_F8, &["F8"]),
    (KEY_F9, &["F9"]),
    (KEY_F10, &["F10"]),
    (KEY_NUMLOCK, &["NUMLOCK"]),
    (KEY_SCROLLLOCK, &["SCROLLLOCK"]),
    (KEY_KP7, &["KP7"]),
    (KEY_KP8, &["KP8"]),
    (KEY_KP9, &["KP9"]),
    (KEY_KPMINUS, &["KPMINUS"]),
    (KEY_KP4, &["KP4"]),
    (KEY_KP5, &["KP5"]),
    (KEY_KP6, &["KP6"]),
    (KEY_KPPLUS, &["KPPLUS"]),
    (KEY_KP1, &["KP1"]),
    (KEY_KP2, &["KP2"]),
    (KEY_KP3, &["KP3"]),
    (KEY_KP0, &["KP0"]),
    (KEY_KPDOT, &["KPDOT"]),
    (KEY_102ND, &["102ND", "INTLBACKSLASH"]),
    (KEY_F11, &["F11"]),
    (KEY_F12, &["F12"]),
    (KEY_KPENTER, &["KPENTER"]),
    (KEY_RIGHTCTRL, &["RIGHTCTRL"]),
    (KEY_KPSLASH, &["KPSLASH", "KPDIVIDE"]),
    (KEY_SYSRQ, &["SYSRQ", "PRINTSCREEN", "PRTSC"]),
    (KEY_RIGHTALT, &["RIGHTALT", "ALTGR"]),
    (KEY_HOME, &["HOME"]),
    (KEY_UP, &["UP"]),
    (KEY_PAGEUP, &["PAGEUP", "PGUP"]),
    (KEY_LEFT, &["LEFT"]),
    (KEY_RIGHT, &["RIGHT"]),
    (KEY_END, &["END"]),
    (KEY_DOWN, &["DOWN"]),
    (KEY_PAGEDOWN, &["PAGEDOWN", "PGDN"]),
    (KEY_INSERT, &["INSERT", "INS"]),
    (KEY_DELETE, &["DELETE", "DEL"]),
    (KEY_PAUSE, &["PAUSE"]),
    (KEY_LEFTMETA, &["LEFTMETA", "META", "SUPER", "WIN"]),
    (KEY_RIGHTMETA, &["RIGHTMETA"]),
    // `"MENU"` maps here on purpose — see [`KEY_COMPOSE`].
    (KEY_COMPOSE, &["COMPOSE", "MENU", "CONTEXTMENU"]),
];

/// Every `KEY_*`/`BTN_*` code this daemon's virtual device registers via
/// `UI_SET_KEYBIT` at creation time (see `device.rs`): [`KEYS`]' codes plus the
/// mouse buttons. Derived rather than listed, so the device advertises exactly
/// what the daemon can address — no more, and crucially no less.
pub(crate) fn registered_codes() -> impl Iterator<Item = u16> {
    KEYS.iter()
        .map(|(code, _)| *code)
        .chain(BUTTONS.iter().copied())
}

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

    // A linear scan of ~110 entries, once per `KeyPress`/`KeyRelease` D-Bus
    // call. Not worth a map: the call has already crossed a bus, and a table
    // that can be read in source order is worth more here than the nanoseconds.
    KEYS.iter()
        .find(|(_, names)| names.contains(&bare))
        .map(|(code, _)| *code)
}

/// evdev keycodes for `a` through `z`, in alphabetical order.
///
/// **This table cannot be replaced by arithmetic on [`KEY_A`], and once was.**
/// evdev numbers the letter keys by their position on the physical keyboard,
/// row by row — `KEY_Q` is 16 because `q` is the top-left letter — so the codes
/// run `q w e r t y u i o p`, then `a s d f g h j k l`, then `z x c v b n m`,
/// and are alphabetical nowhere. `KEY_A + (c - b'a')` therefore produced the
/// right code only for `a` itself: it typed `f` for `d`, `z` for `o`, and for
/// `y` and `z` it left the letter range entirely and pressed `KEY_RIGHTSHIFT`
/// and `KEY_KPASTERISK`.
///
/// The mistake survived because nothing checked the far side. `tests/input.rs`
/// reads the synthesized events back from the kernel and compares them against
/// this same table, so it agreed with the bug; it took `tests/apps/input-test`
/// reporting the characters an application actually received to expose it.
const LETTER_KEYS: [u16; 26] = [
    KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I, KEY_J, KEY_K, KEY_L, KEY_M,
    KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R, KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X, KEY_Y, KEY_Z,
];

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
        // Indexed lookup, never arithmetic on `KEY_A` — see [`LETTER_KEYS`].
        b'a'..=b'z' => (LETTER_KEYS[(b - b'a') as usize], false),
        b'A'..=b'Z' => (LETTER_KEYS[(b - b'A') as usize], true),
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
        // since these codes are what [`KEYS`]/button lookups feed into.
        assert_eq!(BTN_LEFT, 0x110);
        assert_eq!(BTN_RIGHT, 0x111);
        assert_eq!(BTN_MIDDLE, 0x112);
    }

    /// The keys added by the 2026-07-31 audit, pinned against the literal
    /// values in `<linux/input-event-codes.h>` rather than against this
    /// module's own constants — the same anchoring the letter-regression test
    /// uses, and for the same reason: an assertion against our own constants
    /// would agree with any typo we made in them.
    ///
    /// Spot-checks rather than all ~110, chosen for the codes a transcription
    /// error is most likely to land on: the boundaries of each run, and the
    /// keys whose numbering is not where you would guess (`KEY_102ND` sits
    /// above the keypad, `KEY_RIGHTALT` above `KEY_SYSRQ`, and the navigation
    /// block is not adjacent to the arrow keys' F-key neighbours).
    #[test]
    fn audited_keys_match_the_kernel_header() {
        for (name, expected) in [
            ("kpasterisk", 55),
            ("f1", 59),
            ("f10", 68),
            ("numlock", 69),
            ("kp7", 71),
            ("kp0", 82),
            ("kpdot", 83),
            ("102nd", 86),
            ("f11", 87),
            ("f12", 88),
            ("kpenter", 96),
            ("rightctrl", 97),
            ("sysrq", 99),
            ("rightalt", 100),
            ("home", 102),
            ("up", 103),
            ("left", 105),
            ("right", 106),
            ("down", 108),
            ("delete", 111),
            ("pause", 119),
            ("leftmeta", 125),
            ("compose", 127),
        ] {
            assert_eq!(
                key_name_to_code(name),
                Some(expected),
                "`{name}` must map to evdev code {expected}"
            );
        }
    }

    /// **The check that would have caught the gap this table was built to
    /// close.** Before 2026-07-31 the registered-capability list and the name
    /// lookup were separate, and a key present in one but not the other failed
    /// silently: the press was accepted, the compositor dropped it, and nothing
    /// reported an error. Deriving both from [`KEYS`] makes that unrepresentable
    /// — this asserts the derivation actually holds.
    #[test]
    fn every_nameable_key_is_registered_on_the_device() {
        let registered: Vec<u16> = registered_codes().collect();
        for (code, names) in KEYS {
            assert!(
                registered.contains(code),
                "`{}` (code {code}) can be named but is never advertised to the kernel",
                names[0]
            );
        }
    }

    /// The same invariant from the other direction, across the *other* pair of
    /// tables: `TypeText` resolves characters through [`ascii_to_keycode`],
    /// which is independent of [`KEYS`]. A character mapping to an unregistered
    /// key would type nothing, with success reported.
    #[test]
    fn every_typeable_character_maps_to_a_registered_key() {
        let registered: Vec<u16> = registered_codes().collect();
        for b in 0x20u8..=0x7e {
            let (code, _) = ascii_to_keycode(b as char).expect("printable ASCII must map");
            assert!(
                registered.contains(&code),
                "`{}` types code {code}, which the device never advertises",
                b as char
            );
        }
        for c in ['\n', '\t'] {
            let (code, _) = ascii_to_keycode(c).expect("newline and tab must map");
            assert!(
                registered.contains(&code),
                "{c:?} types an unadvertised key"
            );
        }
    }

    /// A hand-written 110-row table's real failure mode is a typo, not a
    /// design error. Two rows sharing a code means one key is unreachable by
    /// its own name.
    #[test]
    fn no_two_key_table_rows_share_a_code() {
        let mut codes: Vec<u16> = KEYS.iter().map(|(code, _)| *code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two rows of KEYS share a keycode");
    }

    /// A duplicated *name* is worse than a duplicated code: the lookup returns
    /// the first match, so the second key silently becomes unnameable while
    /// both rows look correct.
    #[test]
    fn no_two_key_table_rows_share_a_name() {
        let mut names: Vec<&str> = KEYS.iter().flat_map(|(_, names)| *names).copied().collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two keys accept the same name");
    }

    /// [`key_name_to_code`] uppercases its input and strips a `KEY_` prefix
    /// before matching, so a table entry written lowercase or prefixed would
    /// never match anything. Nothing about the entry would look wrong.
    #[test]
    fn key_table_names_are_normalized() {
        for (code, names) in KEYS {
            assert!(
                !names.is_empty(),
                "code {code} has no name and is unreachable"
            );
            for name in *names {
                assert_eq!(
                    *name,
                    name.to_ascii_uppercase(),
                    "`{name}` must be uppercase to be matchable"
                );
                assert!(
                    !name.starts_with("KEY_"),
                    "`{name}` must not carry the KEY_ prefix — it is stripped before matching"
                );
                assert_eq!(
                    key_name_to_code(name),
                    Some(*code),
                    "`{name}` does not resolve to its own row"
                );
            }
        }
    }

    /// The keys the audit existed for, stated as capabilities rather than as
    /// codes: these are the ones whose absence made `wgaf key press` unable to
    /// drive a dialog or reach a non-US layout's symbols.
    #[test]
    fn the_keys_the_audit_was_filed_for_are_reachable() {
        for name in [
            "rightalt",  // AltGr — @ $ { } [ ] | on most non-US layouts
            "altgr",     // and by the name a user of one would reach for
            "102nd",     // `<>` / `\|`, the 105th key
            "rightctrl", // the other missing right-hand modifier
            "up",        // arrowing through a menu
            "down",
            "left",
            "right",
            // Not "dismissing a dialog", which is what this used to say: while
            // wgaf holds an input device the compositor takes Escape for the
            // emergency stop, so a synthesized one reaches no application at
            // all. `docs/cli-reference.md` says so, and points at `wgaf a11y`
            // for dismissing a dialog. Kept here because the key still
            // resolves and is still worth covering.
            "escape",
            "tab", // moving through a dialog
            "f1",  // function keys
            "f12",
            "home",
            "end",
            "pageup",
            "pagedown",
            "insert",
            "delete",
            "super", // GNOME's overview and every Shell shortcut
            "kp0",   // the keypad
            "kpenter",
        ] {
            assert!(
                key_name_to_code(name).is_some(),
                "`wgaf key press {name}` must resolve — this key is why the audit was filed"
            );
        }
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

    /// The regression that `KEY_A + (c - b'a')` was. Pinned against the literal
    /// values in `<linux/input-event-codes.h>` rather than against this
    /// module's own constants, so that the assertion is anchored to the kernel
    /// and not to whatever the code currently believes.
    ///
    /// Letters from every row of the keyboard, because the old arithmetic was
    /// correct only for `a` and drifted further the further from it you went.
    #[test]
    fn letters_map_to_their_physical_evdev_positions_not_alphabetical_offsets() {
        for (c, code) in [
            ('a', 30),
            ('b', 48),
            ('d', 32),
            ('e', 18),
            ('h', 35),
            ('l', 38),
            ('m', 50),
            ('o', 24),
            ('q', 16),
            ('r', 19),
            ('w', 17),
            ('z', 44),
        ] {
            assert_eq!(
                ascii_to_keycode(c),
                Some((code, false)),
                "`{c}` must map to evdev code {code}"
            );
            assert_eq!(
                ascii_to_keycode(c.to_ascii_uppercase()),
                Some((code, true)),
                "`{}` must map to evdev code {code} with shift",
                c.to_ascii_uppercase()
            );
        }
    }

    /// `wgaf key press d` had the same defect as `wgaf type d`, from a second
    /// copy of the same arithmetic in a different function.
    #[test]
    fn named_single_letter_keys_map_to_the_same_codes_as_typing_them() {
        for c in 'a'..='z' {
            let typed = ascii_to_keycode(c).map(|(code, _)| code);
            assert_eq!(
                key_name_to_code(&c.to_string()),
                typed,
                "`wgaf key press {c}` and `wgaf type {c}` must press the same key"
            );
        }
    }

    /// The old arithmetic ran off the end of the letters for `y` and `z`,
    /// pressing a modifier and a keypad key. Nothing may map into those.
    #[test]
    fn no_letter_maps_onto_a_modifier_or_keypad_key() {
        for c in 'a'..='z' {
            let (code, _) = ascii_to_keycode(c).expect("every letter must map");
            assert!(
                !matches!(
                    code,
                    KEY_LEFTSHIFT | KEY_RIGHTSHIFT | KEY_LEFTCTRL | KEY_LEFTALT
                ),
                "`{c}` maps onto modifier code {code}"
            );
            // 55 is `KEY_KPASTERISK`, the first keypad code above the letters.
            assert!(
                code < 55,
                "`{c}` maps to {code}, outside the main key block"
            );
        }
    }

    /// Every letter must map somewhere different; the old arithmetic happened
    /// to be injective, so this is not what caught it — it is what stops a
    /// hand-written table from acquiring a typo.
    #[test]
    fn every_letter_maps_to_a_distinct_key() {
        let mut codes: Vec<u16> = ('a'..='z')
            .map(|c| ascii_to_keycode(c).expect("every letter must map").0)
            .collect();
        codes.sort_unstable();
        let distinct = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), distinct, "two letters share a keycode");
    }
}
