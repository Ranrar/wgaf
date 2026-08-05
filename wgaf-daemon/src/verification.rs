//! Action verification: how much pre-condition checking a gated action
//! performs before it runs.
//!
//! This module currently holds only [`VerificationLevel`], the
//! `verification_level` setting on [`crate::config::Config`]. Nothing reads
//! it yet — the checks it will gate (a focus pre-condition on targeted
//! input, and later an AT-SPI post-check) are a later phase of the same
//! feature.
//!
//! It lives in its own module rather than as a bare `String` on `Config`,
//! or folded into `crate::input` or `crate::permissions`, because the
//! checks it will gate span both: a focus correction that is
//! permission-adjacent (gated by `Capability::FocusWindow`), and the input
//! path itself. Neither module is the sole owner, so this one is.

use serde::Deserialize;

/// How much pre-condition checking a gated action performs before running.
///
/// Read from `verification_level` in `config.toml` — see
/// [`crate::config::Config::verification_level`] for the full semantics of
/// each level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationLevel {
    /// No pre-condition checking. Reproduces the daemon's behaviour from
    /// before this setting existed, exactly.
    None,
    /// Refuses to synthesize input against a *named* target that is not
    /// focused, attempting to correct focus first rather than merely
    /// refusing. The default.
    Basic,
    /// Reserved for AT-SPI post-checks that don't exist yet. A config file
    /// naming `strict` is rejected at load time rather than silently
    /// treated as `basic` — see [`crate::config::Config::load_required`].
    Strict,
}

/// Default [`VerificationLevel`], applied when `verification_level` is
/// absent from `config.toml`.
///
/// `basic` was chosen over `none` after real escapes of synthesized input
/// into the wrong window — see plan-first-release.md's "Action
/// verification" entry. `none` would change nothing for anyone who doesn't
/// already read the changelog, which defeats the point.
pub(crate) const DEFAULT_VERIFICATION_LEVEL: VerificationLevel = VerificationLevel::Basic;

#[cfg(test)]
mod tests {
    use super::*;

    /// `VerificationLevel` is only ever deserialized as a field of another
    /// TOML table (`Config`), never as a bare top-level value, so this
    /// wraps it the same way to exercise the real code path.
    #[derive(Debug, Deserialize)]
    struct Wrapper {
        level: VerificationLevel,
    }

    #[test]
    fn deserializes_from_lowercase_toml_strings() {
        assert_eq!(
            toml::from_str::<Wrapper>("level = \"none\"").unwrap().level,
            VerificationLevel::None
        );
        assert_eq!(
            toml::from_str::<Wrapper>("level = \"basic\"")
                .unwrap()
                .level,
            VerificationLevel::Basic
        );
        assert_eq!(
            toml::from_str::<Wrapper>("level = \"strict\"")
                .unwrap()
                .level,
            VerificationLevel::Strict
        );
    }
}
