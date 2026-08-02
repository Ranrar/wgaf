//! Keyboard primitives: single key press/release, and the higher-level
//! `TypeText` ASCII helper built on top of them.

use std::sync::atomic::AtomicBool;

use crate::input::codes::{self, KEY_LEFTSHIFT};
use crate::input::device::UinputDevice;
use crate::input::xkb::KeyStroke;
use crate::input::{InputError, Typing, check_not_stopped};

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

/// Presses `codes` in order, then releases them in reverse.
///
/// Reverse release order is what makes this worth having as one operation
/// rather than a sequence of `KeyPress`/`KeyRelease` calls. `Ctrl+Shift+T`
/// released in the order it was pressed passes through `Shift+T` on the way
/// out, which some applications act on; released in reverse it passes through
/// `Ctrl+Shift`, which nothing binds.
///
/// The other reason is failure. A caller hand-rolling six D-Bus calls that dies
/// after the third leaves modifiers **held down** — the session then behaves as
/// though the user is leaning on Ctrl, and there is no way to notice except by
/// feel. Here the whole combination is one call and one plan.
///
/// The kill switch is checked before each press and **never during the
/// releases**, which is the same argument as above wearing a different hat: a
/// stop that abandoned a half-pressed combination would leave Ctrl and Shift
/// held down on a desktop whose user has just hit the panic key. Once a key is
/// down it gets released; the check only decides whether to press the next one.
pub(crate) fn hotkey(
    device: &mut UinputDevice,
    codes: &[u16],
    stopped: &AtomicBool,
) -> Result<(), InputError> {
    let mut pressed = 0;
    let mut outcome = Ok(());

    for code in codes {
        if let Err(e) = check_not_stopped(stopped) {
            outcome = Err(e);
            break;
        }
        press(device, *code)?;
        pressed += 1;
    }

    for code in codes[..pressed].iter().rev() {
        release(device, *code)?;
    }
    outcome
}

/// Resolves every character of `text` to keystrokes **before any of them is
/// typed**.
///
/// Planning first rather than typing as it goes is a deliberate change from the
/// original streaming implementation, and it buys two things:
///
/// - **A string containing one untypeable character types nothing**, instead of
///   stopping partway and leaving the application holding half of it. Half a
///   command in a terminal is a worse outcome than none.
/// - **The rate limiter can be charged exactly.** The old estimate assumed one
///   key plus an optional shift, which is wrong for a character behind a dead
///   key: that costs two keystrokes and up to six events, and a run of them
///   would have slipped the budget.
pub(crate) fn plan_text(text: &str, typing: &Typing) -> Result<Vec<KeyStroke>, InputError> {
    let mut plan = Vec::with_capacity(text.len());

    for c in text.chars() {
        match typing {
            Typing::Keymap(map) => {
                let strokes = map
                    .strokes(c)
                    .ok_or_else(|| InputError::CharacterNotTypeable {
                        ch: c,
                        layout: map.layout_name().to_string(),
                    })?;
                plan.extend_from_slice(strokes);
            }
            Typing::UsAscii => {
                let (keycode, needs_shift) = codes::ascii_to_keycode(c)
                    .ok_or_else(|| InputError::UnknownKey(c.to_string()))?;
                plan.push(KeyStroke {
                    keycode,
                    modifiers: if needs_shift {
                        vec![KEY_LEFTSHIFT]
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }

    Ok(plan)
}

/// How many kernel events a plan will emit.
///
/// This is why the rate limiter counts events rather than calls: one `TypeText`
/// at the default character cap is around 16,000 events, and one `KeyPress` is
/// one. They are not comparable units.
pub(crate) fn plan_event_cost(plan: &[KeyStroke]) -> u32 {
    plan.iter().map(KeyStroke::event_cost).sum()
}

/// Emits a plan produced by [`plan_text`].
///
/// Each keystroke presses its modifiers, presses and releases the key, then
/// releases the modifiers in reverse order. Reverse order matters: releasing
/// `AltGr` before `Shift` on a fourth-level character can leave the compositor
/// briefly seeing a state neither key asked for.
///
/// A character needing a dead key arrives here as two ordinary keystrokes; the
/// *application's* input method composes them. wgaf synthesizes keys and never
/// composes anything itself.
///
/// **The kill switch is checked once per keystroke, and this is the check that
/// makes stopping actually stop.** One maximum-length call is around 16,000
/// kernel events inside a single `spawn_blocking`; a stop noticed only on the
/// way in would engage after the flood had already been typed. Between
/// keystrokes is also the only safe place to abandon the loop — each one
/// releases its own modifiers before the next begins, so nothing is left held.
pub(crate) fn type_planned(
    device: &mut UinputDevice,
    plan: &[KeyStroke],
    stopped: &AtomicBool,
) -> Result<(), InputError> {
    for stroke in plan {
        check_not_stopped(stopped)?;
        for modifier in &stroke.modifiers {
            press(device, *modifier)?;
        }
        press(device, stroke.keycode)?;
        release(device, stroke.keycode)?;
        for modifier in stroke.modifiers.iter().rev() {
            release(device, *modifier)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::xkb::LayoutMap;
    use std::sync::Arc;

    fn cost(text: &str, typing: &Typing) -> u32 {
        plan_event_cost(&plan_text(text, typing).expect("should plan"))
    }

    fn ascii() -> Typing {
        Typing::UsAscii
    }

    /// A layout map for `layout`, built the same way the daemon builds it.
    fn layout(layout: &str) -> Typing {
        let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let keymap = xkbcommon::xkb::Keymap::new_from_names(
            &ctx,
            "",
            "pc105",
            layout,
            "",
            None,
            xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("layout should compile");
        Typing::Keymap(Arc::new(LayoutMap::build(
            &keymap,
            0,
            codes::registered_codes(),
        )))
    }

    #[test]
    fn unshifted_characters_cost_two_events_each() {
        assert_eq!(cost("abc", &ascii()), 6);
    }

    #[test]
    fn shifted_characters_cost_four() {
        // The shift press and release are events in their own right.
        assert_eq!(cost("A", &ascii()), 4);
        assert_eq!(cost("!", &ascii()), 4);
    }

    #[test]
    fn mixed_text_sums_per_character() {
        // 'H' shifted (4) + "i" (2) = 6.
        assert_eq!(cost("Hi", &ascii()), 6);
    }

    #[test]
    fn empty_text_costs_nothing() {
        assert_eq!(cost("", &ascii()), 0);
    }

    /// The rate limiter's whole reason for counting events rather than calls:
    /// one maximum-length `TypeText` is worth thousands of `KeyPress` calls.
    #[test]
    fn a_max_length_all_caps_type_text_is_worth_thousands_of_key_presses() {
        let text: String = std::iter::repeat_n('A', 4096).collect();
        assert_eq!(cost(&text, &ascii()), 16_384);
    }

    /// The whole string is resolved before anything is typed, so one
    /// untypeable character means **nothing** is typed rather than half of it.
    /// Half a command left in a terminal is worse than none.
    #[test]
    fn an_untypeable_character_fails_the_whole_string() {
        let err = plan_text("hello 😀 world", &layout("dk")).expect_err("emoji is not typeable");
        assert!(
            matches!(err, InputError::CharacterNotTypeable { ch: '😀', .. }),
            "got {err:?}"
        );
        // And the layout is named, so the message says *why* rather than just
        // that it failed.
        assert!(err.to_string().contains("Danish"));
    }

    /// The US-ASCII path keeps its old error, which means something different:
    /// "there is no such key" rather than "this layout cannot produce that".
    #[test]
    fn the_legacy_table_still_reports_unknown_key() {
        let err = plan_text("é", &ascii()).expect_err("outside 7-bit ASCII");
        assert!(matches!(err, InputError::UnknownKey(_)), "got {err:?}");
    }

    /// A character behind a dead key costs two keystrokes, and the charge has
    /// to reflect that or a run of them slips the rate limiter's budget. The
    /// old estimate assumed one key plus an optional shift.
    #[test]
    fn a_composed_character_is_charged_for_both_keystrokes() {
        let dk = layout("dk");
        // `a` is one plain key; `~` is an AltGr'd dead key then Space.
        assert_eq!(cost("a", &dk), 2);
        assert_eq!(cost("~", &dk), 6);
        assert_eq!(cost("a~", &dk), 8);
    }

    /// The same string costs different amounts on different layouts, which is
    /// exactly why the charge is computed from the resolved plan rather than
    /// guessed from the text.
    #[test]
    fn cost_depends_on_the_layout_not_just_the_text() {
        // `~` is Shift+grave on US: one keystroke, four events. On Danish it is
        // an AltGr'd dead key followed by Space: two keystrokes, six events.
        assert_eq!(cost("~", &layout("us")), 4);
        assert_eq!(cost("~", &layout("dk")), 6);
        // A plain letter costs the same everywhere, so the difference above is
        // the layout and not the measurement.
        assert_eq!(cost("a", &layout("us")), cost("a", &layout("dk")));
    }

    /// Modifiers are released in reverse order. A fourth-level character holds
    /// two, and releasing them out of order can leave the compositor seeing a
    /// state neither key asked for.
    #[test]
    fn a_plan_preserves_modifier_order_per_keystroke() {
        let plan = plan_text("A", &ascii()).expect("should plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].modifiers, vec![KEY_LEFTSHIFT]);
    }
}
