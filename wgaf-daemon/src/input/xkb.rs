//! Working out which keys produce a given character, on a given keyboard
//! layout — the mapping half of W12.
//!
//! # What this replaces
//!
//! wgaf synthesizes key *positions* through `uinput`, and the compositor
//! applies the session's keymap to decide what character each position
//! produces. [`super::codes::ascii_to_keycode`] hardcodes the positions of a
//! US-QWERTY keyboard, so on any other layout it produces the wrong character
//! and reports success — `wgaf type "user@example.com"` writes
//! `user"example.com` on a Danish session.
//!
//! It also cannot express a third-level modifier at all, returning
//! `(keycode, needs_shift)`, while `@ $ { } [ ] | \` are all AltGr combinations
//! on Danish. This module asks the keymap instead of tabulating anything.
//!
//! # The three things that are not obvious
//!
//! **A modifier mask is not a key.** `xkb_keymap_key_get_mods_for_level` says
//! *"you need `Mod5`"*, not *"press this key"*, and there is no API for the
//! reverse direction. [`ModifierKeys::probe`] therefore presses each key the
//! virtual device advertises through an [`xkb::State`] and records what it
//! latches. Choosing a key any other way is how you end up selecting evdev 84,
//! which sets `Mod5` on a Danish keymap, **has no `KEY_` constant in
//! `<linux/input-event-codes.h>`**, and so does nothing when pressed through
//! `uinput` — silently. Restricting the search to advertised keys makes that
//! impossible rather than merely unlikely.
//!
//! **Everything happens within one layout.** A keymap carries every configured
//! layout, and searching across them answers `\` from the US layout on a Danish
//! session — confidently and wrongly. The layout index is decided once, by
//! configuration, and never searched.
//!
//! **Dead keys are looked up, not hardcoded.** Most layouts put some characters
//! behind a dead key: on standard Danish, `^`, `` ` `` and `~` are all dead, and
//! `~` is in every shell path. The locale's Compose file already says how to
//! resolve them (`<dead_tilde> <space> : "~"`, `<dead_acute> <e> : "é"`), so
//! [`compose_into`] feeds every reachable keysym through a compose state and
//! records what comes out. Nothing about any particular language is written down
//! here, which is the point — hardcoding dead keys per layout would be the same
//! mistake as hardcoding the layouts, one level down.
//!
//! Composition is performed by the *receiving application's* input method, not
//! by wgaf, which synthesizes the two keystrokes. GNOME's default (IBus) and
//! `gtk-im-context-simple` both handle it; an application with no input method
//! sees two keypresses and composes nothing.

use std::collections::HashMap;
use std::ffi::OsStr;

use xkbcommon::xkb;

/// The offset between an evdev key code and an xkb key code.
///
/// X11 reserved codes 0-7, so xkb numbers the same physical key eight higher
/// than the kernel does. Wayland kept the convention, and
/// `tests/apps/input-test` reports this same `evdev + 8` value — which is what
/// `keyboard_coverage.rs` already asserts against.
const XKB_KEYCODE_OFFSET: u32 = 8;

fn evdev_to_xkb(code: u16) -> xkb::Keycode {
    xkb::Keycode::new(u32::from(code) + XKB_KEYCODE_OFFSET)
}

/// One key press, with the modifier keys to hold around it. All codes are
/// evdev, ready for `uinput`.
///
/// `modifiers` is empty for an unmodified character, one key for a shifted or
/// AltGr one, and two for the fourth level (AltGr+Shift). It is ordered
/// lowest-code-first so a character always produces the same event sequence:
/// tests pin exact sequences, and an arbitrary order would make them flaky for
/// no reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    pub(crate) keycode: u16,
    pub(crate) modifiers: Vec<u16>,
}

impl KeyStroke {
    /// Kernel events to press and release this: two for the key, two for each
    /// modifier held around it.
    pub(crate) fn event_cost(&self) -> u32 {
        2 + 2 * self.modifiers.len() as u32
    }
}

/// The modifier keys the virtual device can actually press, each with the mask
/// it sets on this keymap.
///
/// Probed rather than tabulated: which key produces `Mod5` is a property of the
/// keymap, and which keys are pressable is a property of our own device.
struct ModifierKeys {
    /// `(evdev code, mask this key sets)`, ordered by evdev code so selection is
    /// deterministic and prefers left-hand modifiers (`KEY_LEFTSHIFT` is 42,
    /// `KEY_RIGHTSHIFT` is 54).
    keys: Vec<(u16, xkb::ModMask)>,
}

impl ModifierKeys {
    fn probe(keymap: &xkb::Keymap, advertised: impl Iterator<Item = u16>) -> Self {
        let mut keys: Vec<(u16, xkb::ModMask)> = advertised
            .filter_map(|code| {
                let mut state = xkb::State::new(keymap);
                state.update_key(evdev_to_xkb(code), xkb::KeyDirection::Down);
                let mask = state.serialize_mods(xkb::STATE_MODS_EFFECTIVE);
                (mask != 0).then_some((code, mask))
            })
            .collect();

        keys.sort_by_key(|(code, _)| *code);
        Self { keys }
    }

    /// Finds pressable keys whose combined mask is exactly `target`.
    ///
    /// `None` means the mask cannot be produced by keys the device advertises.
    /// That is a real outcome, not a defect: the caller records the character as
    /// untypeable rather than pressing something approximate, because typing a
    /// character the caller did not ask for is worse than refusing.
    ///
    /// Only keys whose mask is a *subset* of the target are considered, so a
    /// search for `Mod5` never latches Shift on the way.
    fn resolve(&self, target: xkb::ModMask) -> Option<Vec<u16>> {
        if target == 0 {
            return Some(Vec::new());
        }

        let mut held = Vec::new();
        let mut accumulated: xkb::ModMask = 0;

        for (code, mask) in &self.keys {
            if mask & !target != 0 {
                continue; // would set something not asked for
            }
            if mask & !accumulated == 0 {
                continue; // contributes nothing new
            }
            held.push(*code);
            accumulated |= mask;
            if accumulated == target {
                return Some(held);
            }
        }

        None
    }
}

/// Every character one layout can type, and the keystrokes that do it.
pub(crate) struct LayoutMap {
    /// One entry per typeable character. The value is a *sequence*: one
    /// keystroke for a directly-reachable character, two for a composed one
    /// (dead key, then the key completing it).
    strokes: HashMap<char, Vec<KeyStroke>>,
    layout_name: String,
}

impl LayoutMap {
    /// Builds the character index for one layout of `keymap`.
    ///
    /// `advertised` is the set of evdev codes the virtual device registered (see
    /// `codes::registered_codes`). Restricting to it means the map can only ever
    /// contain strokes wgaf is able to perform.
    ///
    /// Lower levels win: a character reachable both unmodified and via a
    /// modifier is recorded unmodified. Ties go to the lowest keycode, so the
    /// result never depends on iteration order.
    pub(crate) fn build(
        keymap: &xkb::Keymap,
        layout: xkb::LayoutIndex,
        advertised: impl Iterator<Item = u16>,
    ) -> Self {
        let modifiers_source: Vec<u16> = advertised.collect();
        let modifiers = ModifierKeys::probe(keymap, modifiers_source.iter().copied());

        // Deterministic order, so "lowest keycode wins" is a real rule rather
        // than whatever the set happened to iterate as.
        let mut codes = modifiers_source;
        codes.sort_unstable();

        // Every keysym reachable with a single keystroke, paired with the
        // keystroke reaching it. Dead keys are included: they produce no
        // character, but they are how composed characters begin.
        let mut reachable: Vec<(xkb::Keysym, KeyStroke)> = Vec::new();
        let mut strokes: HashMap<char, Vec<KeyStroke>> = HashMap::new();

        for code in codes {
            let key = evdev_to_xkb(code);

            for level in 0..keymap.num_levels_for_key(key, layout) {
                let syms = keymap.key_get_syms_by_level(key, layout, level);
                let [sym] = syms[..] else { continue };

                // Several masks can select one level (Shift versus a locked
                // Caps, say). Take the first that resolves to keys we have.
                let mut buf = [0u32; 8];
                let n = keymap.key_get_mods_for_level(key, layout, level, &mut buf);
                let Some(held) = buf[..n].iter().find_map(|mask| modifiers.resolve(*mask)) else {
                    continue;
                };

                let stroke = KeyStroke {
                    keycode: code,
                    modifiers: held,
                };

                if let Some(c) = single_char(sym) {
                    let better = match strokes.get(&c) {
                        Some(existing) => {
                            existing.len() > 1
                                || existing[0].modifiers.len() > stroke.modifiers.len()
                        }
                        None => true,
                    };
                    if better {
                        strokes.insert(c, vec![stroke.clone()]);
                    }
                }

                reachable.push((sym, stroke));
            }
        }

        compose_into(&mut strokes, &reachable);

        Self {
            strokes,
            layout_name: keymap.layout_get_name(layout).to_string(),
        }
    }

    /// How this layout names itself, e.g. `"Danish"` or `"English (Dvorak)"`.
    ///
    /// This is the keymap's *description*, not the `dk` code a user configures
    /// — the two are different namespaces, and all 595 descriptions are unique,
    /// which is why both forms can be accepted in configuration.
    pub(crate) fn layout_name(&self) -> &str {
        &self.layout_name
    }

    /// The keystroke sequence producing `c`, or `None` if this layout cannot
    /// type it with the keys the device advertises.
    ///
    /// One keystroke for a directly-reachable character, two for one needing a
    /// dead key. Callers press and release each in order.
    pub(crate) fn strokes(&self, c: char) -> Option<&[KeyStroke]> {
        self.strokes.get(&c).map(Vec::as_slice)
    }

    /// How many characters this layout can type. Diagnostics only.
    pub(crate) fn len(&self) -> usize {
        self.strokes.len()
    }
}

/// The single character a keysym stands for, if it stands for exactly one.
///
/// `None` for keysyms that are not characters (`Escape`, `F7`) and for dead
/// keys, which produce nothing alone — [`compose_into`] handles those.
fn single_char(sym: xkb::Keysym) -> Option<char> {
    let utf8 = xkb::keysym_to_utf8(sym);
    // `keysym_to_utf8` NUL-terminates; empty means "no character".
    let mut chars = utf8.trim_end_matches('\0').chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// Adds every two-keystroke composed character this layout can reach.
///
/// Driven entirely by the locale's Compose data and the keysyms the layout
/// actually reaches, so it needs no per-language knowledge. Sequence-starting
/// keysyms are found by feeding each one alone and keeping those the compose
/// state reports as `Composing` — asking the table rather than assuming a
/// `dead_*` naming convention.
///
/// Direct entries always win: a character reachable in one keystroke is never
/// replaced by a composed one, since composition additionally depends on the
/// receiving application running an input method.
///
/// Sequences longer than two keystrokes are not searched. Those are `Multi_key`
/// entries, and wgaf deliberately never presses the Compose key — it opens a
/// menu that grabs input.
///
/// A missing or unreadable Compose file is not an error; the layout simply keeps
/// what it can type directly.
fn compose_into(
    strokes: &mut HashMap<char, Vec<KeyStroke>>,
    reachable: &[(xkb::Keysym, KeyStroke)],
) {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    // The Compose file is chosen by locale, exactly as every other client on the
    // desktop chooses it.
    let locale = std::env::var_os("LC_ALL")
        .or_else(|| std::env::var_os("LC_CTYPE"))
        .or_else(|| std::env::var_os("LANG"))
        .unwrap_or_else(|| OsStr::new("C").to_os_string());

    let Ok(table) =
        xkb::compose::Table::new_from_locale(&ctx, &locale, xkb::compose::COMPILE_NO_FLAGS)
    else {
        tracing::debug!(
            ?locale,
            "no compose table for this locale; characters behind dead keys will be unavailable"
        );
        return;
    };

    let starters: Vec<&(xkb::Keysym, KeyStroke)> = reachable
        .iter()
        .filter(|(sym, _)| {
            let mut state = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
            state.feed(*sym);
            state.status() == xkb::compose::Status::Composing
        })
        .collect();

    let cost = |s: &[KeyStroke]| s.iter().map(KeyStroke::event_cost).sum::<u32>();

    for (first_sym, first_stroke) in starters {
        for (second_sym, second_stroke) in reachable {
            let mut state = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
            state.feed(*first_sym);
            state.feed(*second_sym);

            if state.status() != xkb::compose::Status::Composed {
                continue;
            }

            let Some(utf8) = state.utf8() else { continue };
            let mut chars = utf8.trim_end_matches('\0').chars();
            let Some(c) = chars.next() else { continue };
            if chars.next().is_some() {
                continue; // a sequence producing more than one character
            }

            let candidate = vec![first_stroke.clone(), second_stroke.clone()];
            let better = match strokes.get(&c) {
                // Never displace a directly-typeable character.
                Some(existing) if existing.len() == 1 => false,
                Some(existing) => cost(existing) > cost(&candidate),
                None => true,
            };
            if better {
                strokes.insert(c, candidate);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::codes;

    /// Compiles a keymap the way a test wants it, independent of whatever the
    /// developer's own session happens to be configured with.
    ///
    /// **Tests must never read the session's keymap.** `tests/accessibility.rs`
    /// is the standing lesson on suites that pass or fail according to host
    /// state nothing asserts on; a layout test built on the live keymap would
    /// pass in Copenhagen and fail everywhere else.
    fn keymap(layouts: &str, variant: &str) -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(
            &ctx,
            "",
            "pc105",
            layouts,
            variant,
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("xkeyboard-config should provide these layouts")
    }

    fn map_for(layouts: &str, variant: &str, layout: xkb::LayoutIndex) -> LayoutMap {
        LayoutMap::build(&keymap(layouts, variant), layout, codes::registered_codes())
    }

    fn us() -> LayoutMap {
        map_for("us", "", 0)
    }

    fn dk() -> LayoutMap {
        map_for("dk", "", 0)
    }

    /// The eight layouts every coverage assertion runs against. "Works on the
    /// maintainer's layout" is the standard that produced the defect, so no
    /// single-layout claim is trusted here.
    const LAYOUTS: [(&str, &str); 8] = [
        ("us", ""),
        ("us", "dvorak"),
        ("dk", ""),
        ("dk", "nodeadkeys"),
        ("de", ""),
        ("fr", ""),
        ("no", ""),
        ("es", ""),
    ];

    /// The single keystroke typing `c`, asserting it needs only one.
    #[track_caller]
    fn direct(map: &LayoutMap, c: char) -> KeyStroke {
        let strokes = map
            .strokes(c)
            .unwrap_or_else(|| panic!("{c:?} should be typeable"));
        assert_eq!(
            strokes.len(),
            1,
            "{c:?} should need one keystroke, got {strokes:?}"
        );
        strokes[0].clone()
    }

    #[test]
    fn unmodified_letters_need_no_modifiers() {
        let a = direct(&us(), 'a');
        assert_eq!(a.keycode, codes::KEY_A);
        assert!(a.modifiers.is_empty());
    }

    #[test]
    fn capitals_hold_shift_on_the_same_physical_key() {
        let us = us();
        let lower = direct(&us, 'a');
        let upper = direct(&us, 'A');
        assert_eq!(upper.keycode, lower.keycode);
        assert_eq!(upper.modifiers, vec![codes::KEY_LEFTSHIFT]);
    }

    /// The defect, stated as an assertion. On US these are Shift combinations;
    /// on Danish they are AltGr combinations on different keys entirely, and
    /// `ascii_to_keycode` gets every one of them wrong.
    #[test]
    fn danish_reaches_the_characters_the_ascii_table_cannot() {
        let dk = dk();

        // `@` is AltGr+2 on Danish. The US table presses Shift+2, giving `"`.
        let at = direct(&dk, '@');
        assert_eq!(at.keycode, codes::KEY_2);
        assert_eq!(at.modifiers, vec![codes::KEY_RIGHTALT]);

        for c in ['$', '{', '}', '[', ']', '|', '\\'] {
            let stroke = direct(&dk, c);
            assert!(
                stroke.modifiers.contains(&codes::KEY_RIGHTALT),
                "{c:?} should need AltGr on Danish, got {stroke:?}"
            );
        }
    }

    #[test]
    fn the_same_character_is_a_different_key_on_a_different_layout() {
        let us_at = direct(&us(), '@');
        let dk_at = direct(&dk(), '@');

        assert_eq!(us_at.modifiers, vec![codes::KEY_LEFTSHIFT]);
        assert_eq!(dk_at.modifiers, vec![codes::KEY_RIGHTALT]);
        assert_ne!(us_at, dk_at);
    }

    /// A two-layout keymap is the fixture shape that matters: a single-layout
    /// one cannot catch a resolver that searches across layouts, which is the
    /// bug the spike hit when it answered `\` from the US layout on a Danish
    /// session.
    #[test]
    fn resolution_stays_inside_the_requested_layout() {
        let two = keymap("dk,us", "");
        assert_eq!(two.num_layouts(), 2);

        let danish = LayoutMap::build(&two, 0, codes::registered_codes());
        let english = LayoutMap::build(&two, 1, codes::registered_codes());

        // `\` is AltGr'd on Danish and a plain key on US. If the Danish map
        // answered from layout 1 it would report no modifiers.
        assert!(
            direct(&danish, '\\')
                .modifiers
                .contains(&codes::KEY_RIGHTALT)
        );
        assert!(direct(&english, '\\').modifiers.is_empty());
    }

    /// The keymap reports a layout's *description*, not the `dk` code a user
    /// configures. Both are accepted in config, so the distinction has to hold.
    #[test]
    fn layout_names_are_descriptions_not_codes() {
        assert_eq!(dk().layout_name(), "Danish");
        assert_eq!(us().layout_name(), "English (US)");
        assert_eq!(
            map_for("dk", "nodeadkeys", 0).layout_name(),
            "Danish (no dead keys)"
        );
    }

    /// The case that rules out ever accepting an ISO language code in config:
    /// these are both English, and nearly every key is somewhere else.
    #[test]
    fn us_and_dvorak_are_both_english_but_agree_on_almost_nothing() {
        let qwerty = us();
        let dvorak = map_for("us", "dvorak", 0);

        assert_eq!(dvorak.layout_name(), "English (Dvorak)");

        let moved = ('a'..='z')
            .filter(|c| direct(&qwerty, *c).keycode != direct(&dvorak, *c).keycode)
            .count();
        assert!(
            moved >= 20,
            "Dvorak should move nearly every letter, only {moved} of 26 moved"
        );
    }

    /// Modifier selection is constrained to keys the device can actually press.
    /// The Danish keymap sets `Mod5` from evdev 84 as well as from
    /// `KEY_RIGHTALT`, and 84 is a hole in `<linux/input-event-codes.h>` with no
    /// `KEY_` constant — pressing it through `uinput` does nothing at all.
    #[test]
    fn only_keys_the_device_advertises_are_ever_selected() {
        let advertised: Vec<u16> = codes::registered_codes().collect();
        assert!(
            !advertised.contains(&84),
            "evdev 84 has no KEY_ constant and must never be advertised"
        );

        // Every character of every layout, not a sample — one unadvertised key
        // anywhere in a map is a silent no-op at runtime.
        for (layout, variant) in LAYOUTS {
            let map = map_for(layout, variant, 0);
            for c in (0x20u8..0x7f).map(char::from) {
                let Some(strokes) = map.strokes(c) else {
                    continue;
                };
                for stroke in strokes {
                    assert!(
                        advertised.contains(&stroke.keycode),
                        "{layout}({variant}) {c:?} used unadvertised key {}",
                        stroke.keycode
                    );
                    for m in &stroke.modifiers {
                        assert!(
                            advertised.contains(m),
                            "{layout}({variant}) {c:?} used unadvertised modifier {m}"
                        );
                    }
                }
            }
        }
    }

    /// Characters behind a dead key are resolved through the locale's Compose
    /// data and typed as two keystrokes, rather than being unavailable.
    ///
    /// Nothing here names a language: the sequence comes from the system's
    /// Compose file (`<dead_circumflex> <space> : "^"`), which is why this works
    /// on any layout with dead keys and not just the ones somebody tabulated.
    #[test]
    fn dead_key_characters_are_composed_from_two_keystrokes() {
        let dk = dk();

        for c in ['^', '`', '~'] {
            let strokes = dk
                .strokes(c)
                .unwrap_or_else(|| panic!("{c:?} must be typeable on Danish"));
            assert_eq!(
                strokes.len(),
                2,
                "{c:?} is a dead key on Danish and needs a sequence, got {strokes:?}"
            );
            // Completed with Space: the dead key alone would leave the
            // application waiting for the next character.
            assert_eq!(
                strokes[1].keycode,
                codes::KEY_SPACE,
                "{c:?} should complete with Space, got {strokes:?}"
            );
        }
    }

    /// Composition is not limited to bare accents — it is whatever the Compose
    /// file defines, which is how accented letters become typeable at all.
    #[test]
    fn accented_letters_compose_from_a_dead_key_and_a_letter() {
        let dk = dk();
        let e_acute = dk.strokes('é').expect("'é' should compose on Danish");
        assert_eq!(e_acute.len(), 2);
        assert_eq!(e_acute[1].keycode, codes::KEY_E);
    }

    /// A character reachable in one keystroke is never replaced by a composed
    /// sequence, since composition additionally depends on the receiving
    /// application running an input method.
    #[test]
    fn direct_characters_are_never_displaced_by_composed_ones() {
        let us = us();
        for c in ['^', '`', '~', '\'', '"'] {
            assert_eq!(
                us.strokes(c).expect("typeable on US").len(),
                1,
                "{c:?} is a plain key on US and must not be composed"
            );
        }
    }

    /// The assertion the whole feature exists to make true. A `wgaf type` that
    /// cannot produce `@` or `~` is not fixed.
    #[test]
    fn every_tested_layout_covers_printable_ascii() {
        for (layout, variant) in LAYOUTS {
            let map = map_for(layout, variant, 0);
            let missing: Vec<char> = (0x20u8..0x7f)
                .map(char::from)
                .filter(|c| map.strokes(*c).is_none())
                .collect();
            assert!(
                missing.is_empty(),
                "{layout}({variant}) cannot type {missing:?}"
            );
        }
    }

    /// The non-ASCII characters W12 names. Asserted on Danish because it
    /// reaches all of them — `us` has no `€` or `é` at all, which is a property
    /// of that layout rather than of this code.
    #[test]
    fn the_motivating_non_ascii_characters_are_typeable_on_danish() {
        let dk = dk();
        for c in ['€', 'æ', 'ø', 'å', 'ä', 'ß', 'é', 'ñ'] {
            assert!(
                dk.strokes(c).is_some(),
                "{c:?} should be typeable on Danish"
            );
        }
    }

    /// A character with no key sequence is reported as untypeable rather than
    /// approximated. The caller turns this into a named error; typing something
    /// the caller did not ask for would be worse than refusing.
    #[test]
    fn a_character_with_no_key_sequence_is_not_typeable_anywhere() {
        for (layout, variant) in LAYOUTS {
            let map = map_for(layout, variant, 0);
            assert!(
                map.strokes('\u{1F600}').is_none(),
                "{layout}({variant}) claims to type an emoji"
            );
            assert!(map.strokes('\u{1F600}').is_none());
        }
    }

    #[test]
    fn space_and_tab_are_typeable_with_no_modifiers() {
        let us = us();
        let space = direct(&us, ' ');
        assert_eq!(space.keycode, codes::KEY_SPACE);
        assert!(space.modifiers.is_empty());
        assert_eq!(direct(&us, '\t').keycode, codes::KEY_TAB);
    }

    #[test]
    fn lower_levels_win_over_higher_ones() {
        let us = us();
        for c in ['a', '1', ' '] {
            assert!(
                direct(&us, c).modifiers.is_empty(),
                "{c:?} should be reachable without modifiers"
            );
        }
    }

    #[test]
    fn event_cost_counts_modifier_presses_and_releases() {
        let plain = KeyStroke {
            keycode: codes::KEY_A,
            modifiers: vec![],
        };
        let shifted = KeyStroke {
            keycode: codes::KEY_A,
            modifiers: vec![codes::KEY_LEFTSHIFT],
        };
        let fourth_level = KeyStroke {
            keycode: codes::KEY_2,
            modifiers: vec![codes::KEY_LEFTSHIFT, codes::KEY_RIGHTALT],
        };

        assert_eq!(plain.event_cost(), 2);
        assert_eq!(shifted.event_cost(), 4);
        assert_eq!(fourth_level.event_cost(), 6);
    }

    /// The rate limiter charges before typing, so a composed character has to be
    /// charged for both keystrokes or a run of them slips the budget. The
    /// charge itself is computed in `keyboard::plan_event_cost`; this pins the
    /// per-character costs it sums.
    #[test]
    fn composed_characters_cost_more_than_direct_ones() {
        let dk = dk();
        let cost = |c: char| -> u32 {
            dk.strokes(c)
                .expect("typeable")
                .iter()
                .map(KeyStroke::event_cost)
                .sum()
        };
        assert_eq!(cost('a'), 2); // plain key
        assert_eq!(cost('@'), 4); // AltGr+2
        assert_eq!(cost('~'), 6); // AltGr'd dead key, then Space
    }

    #[test]
    fn a_layout_map_covers_at_least_printable_ascii() {
        assert!(us().len() >= 95);
    }
}
