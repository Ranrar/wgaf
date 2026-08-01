//! Deciding *which* layout of the session's keymap `wgaf type` types against.
//!
//! A keymap carries every layout the session is configured with, not just the
//! one in use. This module turns a configuration value into a single layout
//! index, once at startup, and that choice is then held for the daemon's
//! lifetime.
//!
//! # Why the choice is configuration and not a lookup
//!
//! The active layout index arrives in `wl_keyboard.modifiers`, which Mutter
//! only sends to a client holding keyboard focus. The daemon is headless and
//! will never have a surface — measured, not assumed — so it can read the
//! keymap but not which of its layouts is live. Giving the daemon a surface to
//! find out is not available either: it would take keyboard focus, which is the
//! hazard the project already documents.
//!
//! So `auto` means **layout index 0**, and nothing pretends to track a
//! mid-session switch. Changing layout while the daemon runs means restarting
//! it, and the documentation says so rather than implying otherwise.
//!
//! # Two namespaces, both accepted
//!
//! A layout has several names and users have met different ones:
//!
//! | String | Where a user sees it |
//! |---|---|
//! | `dk` | `localectl status`, GNOME settings, `setxkbmap` |
//! | `Danish` | what the keymap itself reports |
//!
//! `xkb_keymap_layout_get_name` returns the **description** (`"Danish"`), never
//! the code, so a configured `dk` can never match it directly. Both forms are
//! accepted: a description matches the session keymap's own names outright, and
//! a code is resolved by compiling one throwaway keymap from it and reading the
//! name back. All 595 layout+variant descriptions are unique, so neither form is
//! ambiguous.
//!
//! **A layout is not a language, and the two must not be conflated.** `eng`
//! covers ten layouts, and `us` and `us(dvorak)` are both English with nearly
//! every key in a different place. No language code can identify a layout, which
//! is why none is accepted.

use xkbcommon::xkb;

/// Why a configured layout could not be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LayoutError {
    /// The value is not a layout xkb knows about — a typo, or a language code
    /// where a layout code was wanted.
    #[error(
        "unknown keyboard layout `{spec}`. Expected a layout code such as `dk`, \
         a code with a variant such as `dk(nodeadkeys)`, or a full name such as \
         `Danish` — run `localectl list-x11-keymap-layouts` to see the codes. \
         Note this is a keyboard *layout*, not a language: `en` is not a layout, \
         because English has ten of them."
    )]
    Unknown { spec: String },

    /// The layout exists but the session is not configured with it, so the
    /// compositor would not interpret our keystrokes that way.
    #[error(
        "keyboard layout `{spec}` is not one this session is configured with. \
         Available: {available}. Either add it to your desktop's input sources, \
         or set `input_keyboard_layout` to one of those."
    )]
    NotInSession { spec: String, available: String },

    /// A keymap with no layouts at all. Should not happen; better named than
    /// silently indexed.
    #[error("the session keymap contains no layouts")]
    Empty,
}

/// The configuration value meaning "resolve it from the session".
pub(crate) const AUTO: &str = "auto";

/// A context whose compile errors do not reach the user's terminal.
///
/// `libxkbcommon` writes several `XKB-338` lines to stderr when a layout name
/// does not compile. A typo in `config.toml` should produce our own error
/// message, not a wall of C library diagnostics, so probe compilation runs
/// quietly and the caller reports the failure itself.
fn quiet_context() -> xkb::Context {
    let mut ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    ctx.set_log_level(xkb::LogLevel::Critical);
    ctx.set_log_verbosity(0);
    ctx
}

/// Every layout in `keymap`, by the name it reports for itself.
pub(crate) fn available_layouts(keymap: &xkb::Keymap) -> Vec<String> {
    (0..keymap.num_layouts())
        .map(|i| keymap.layout_get_name(i).to_string())
        .collect()
}

/// Splits `dk(nodeadkeys)` into `("dk", "nodeadkeys")`, or `dk` into
/// `("dk", "")`.
fn split_variant(spec: &str) -> (&str, &str) {
    match spec.split_once('(') {
        // Trim before stripping the bracket, or a trailing `) ` leaves the
        // space attached to the variant and nothing resolves.
        Some((layout, rest)) => (layout.trim(), rest.trim().trim_end_matches(')').trim()),
        None => (spec.trim(), ""),
    }
}

/// The description a layout code compiles to, e.g. `dk` → `"Danish"`.
///
/// Compiles a throwaway keymap purely to read its name back. That is the whole
/// bridge between the code a user writes and the description the session keymap
/// reports — and it is why no registry file needs parsing and no second library
/// is needed.
fn description_for_code(spec: &str) -> Option<String> {
    let (layout, variant) = split_variant(spec);
    if layout.is_empty() {
        return None;
    }

    let ctx = quiet_context();
    let probe = xkb::Keymap::new_from_names(
        &ctx,
        "",
        "",
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )?;

    (probe.num_layouts() > 0).then(|| probe.layout_get_name(0).to_string())
}

/// Resolves a configured layout to an index into `keymap`.
///
/// `spec` is [`AUTO`], a layout description (`"Danish"`), or a layout code with
/// an optional variant (`"dk"`, `"dk(nodeadkeys)"`).
pub(crate) fn resolve(keymap: &xkb::Keymap, spec: &str) -> Result<xkb::LayoutIndex, LayoutError> {
    let count = keymap.num_layouts();
    if count == 0 {
        return Err(LayoutError::Empty);
    }

    let spec = spec.trim();
    if spec.eq_ignore_ascii_case(AUTO) {
        // Index 0, per the module docs: the live group is not observable to a
        // surfaceless client, so this is a decision rather than a detection.
        return Ok(0);
    }

    let names = available_layouts(keymap);

    // A description matches the session keymap outright.
    if let Some(i) = names.iter().position(|n| n == spec) {
        return Ok(i as xkb::LayoutIndex);
    }

    // Otherwise treat it as a code and resolve through a probe keymap.
    let Some(description) = description_for_code(spec) else {
        return Err(LayoutError::Unknown {
            spec: spec.to_string(),
        });
    };

    names
        .iter()
        .position(|n| *n == description)
        .map(|i| i as xkb::LayoutIndex)
        .ok_or_else(|| LayoutError::NotInSession {
            spec: spec.to_string(),
            available: names.join(", "),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn auto_resolves_to_layout_zero() {
        let km = keymap("dk,us", "");
        assert_eq!(resolve(&km, AUTO).unwrap(), 0);
        assert_eq!(km.layout_get_name(0), "Danish");
    }

    #[test]
    fn auto_is_case_insensitive_and_tolerates_whitespace() {
        let km = keymap("dk", "");
        assert_eq!(resolve(&km, "AUTO").unwrap(), 0);
        assert_eq!(resolve(&km, "  auto  ").unwrap(), 0);
    }

    /// The two-namespace problem, as an assertion: config holds `dk`, the
    /// keymap reports `"Danish"`, and both must reach the same index.
    #[test]
    fn a_layout_code_and_its_description_resolve_identically() {
        let km = keymap("dk,us", "");
        assert_eq!(resolve(&km, "dk").unwrap(), 0);
        assert_eq!(resolve(&km, "Danish").unwrap(), 0);
        assert_eq!(resolve(&km, "us").unwrap(), 1);
        assert_eq!(resolve(&km, "English (US)").unwrap(), 1);
    }

    #[test]
    fn a_variant_resolves_by_code_and_by_description() {
        let km = keymap("dk(nodeadkeys)", "");
        assert_eq!(km.layout_get_name(0), "Danish (no dead keys)");
        assert_eq!(resolve(&km, "dk(nodeadkeys)").unwrap(), 0);
        assert_eq!(resolve(&km, "Danish (no dead keys)").unwrap(), 0);
    }

    /// Selecting the second layout must work, or a multi-layout user silently
    /// gets the first one.
    #[test]
    fn a_later_layout_is_selectable() {
        let km = keymap("us,dk,de", "");
        assert_eq!(resolve(&km, "de").unwrap(), 2);
        assert_eq!(resolve(&km, "German").unwrap(), 2);
    }

    #[test]
    fn an_unknown_layout_is_named_in_the_error() {
        let km = keymap("dk", "");
        let err = resolve(&km, "zz").expect_err("`zz` is not a layout");
        assert!(matches!(err, LayoutError::Unknown { .. }));
        assert!(err.to_string().contains("zz"));
        // The message has to teach the namespace, since a language code is the
        // most likely wrong guess.
        assert!(err.to_string().contains("localectl"));
    }

    /// A language code must be rejected rather than guessed at. `en` covers ten
    /// layouts; picking one would be the wrong-character-reported-as-success
    /// failure this whole item exists to end.
    #[test]
    fn a_language_code_is_rejected_not_guessed() {
        let km = keymap("us", "");
        let err = resolve(&km, "en").expect_err("`en` is a language, not a layout");
        assert!(matches!(err, LayoutError::Unknown { .. }));
        assert!(err.to_string().contains("not a language"));
    }

    /// A real layout the session is not configured with is a different error
    /// from a nonexistent one, and lists what is actually available.
    #[test]
    fn a_layout_absent_from_the_session_lists_what_is_present() {
        let km = keymap("dk", "");
        let err = resolve(&km, "fr").expect_err("`fr` is not in this session");
        match &err {
            LayoutError::NotInSession { available, .. } => {
                assert!(available.contains("Danish"), "got {available:?}");
            }
            other => panic!("expected NotInSession, got {other:?}"),
        }
        assert!(err.to_string().contains("Danish"));
    }

    #[test]
    fn available_layouts_reports_descriptions_in_order() {
        let km = keymap("dk,us", "");
        assert_eq!(available_layouts(&km), vec!["Danish", "English (US)"]);
    }

    #[test]
    fn variant_splitting_handles_both_forms() {
        assert_eq!(split_variant("dk"), ("dk", ""));
        assert_eq!(split_variant("dk(nodeadkeys)"), ("dk", "nodeadkeys"));
        assert_eq!(split_variant(" dk ( nodeadkeys ) "), ("dk", "nodeadkeys"));
    }

    /// `us` and `us(dvorak)` are both English and both resolve, to different
    /// things. This is the case that rules out ever accepting a language code.
    #[test]
    fn dvorak_is_selectable_separately_from_qwerty() {
        let km = keymap("us,us(dvorak)", "");
        assert_eq!(resolve(&km, "us").unwrap(), 0);
        assert_eq!(resolve(&km, "us(dvorak)").unwrap(), 1);
        assert_eq!(resolve(&km, "English (Dvorak)").unwrap(), 1);
    }
}
