//! The three outcomes ADR-0007 defines — error, policy denial, verification
//! failure — and the CLI's own error type that carries one of them from
//! wherever a D-Bus call fails all the way to `main()`, where it decides the
//! exit code and whether the message gets an `error:` prefix.
//!
//! Before this module existed, every command function returned
//! `Box<dyn std::error::Error>`, which erases everything but a message: by
//! the time `main()` saw an `Err`, a policy denial, a verification failure
//! and a genuine failure were indistinguishable, and `main()` treated all
//! three the same way (`error:` on stderr, exit 1) regardless of which one
//! had actually happened.
//!
//! This module also owns what used to be two separate facts about the same
//! set of daemon error-name constants: [`error_name_label`] (a human label, used
//! only when a reply carries no description) and this file's classification
//! into a [`Verdict`]. Keeping those as two `match` statements is exactly the
//! drift risk the ADR warns about — *"these must be decided deliberately and
//! written down, not settled one `match` arm at a time"* — so [`classify`] is
//! the one table both are read from.

use std::fmt;

/// Which of ADR-0007's three outcomes a command's failure is.
///
/// Distinct from `main.rs`'s `Outcome` enum, which is `wgaf status`'s own
/// success/unhealthy self-report and is unrelated to this taxonomy — do not
/// merge them. `wgaf status` reporting an unhealthy subsystem is a
/// *successful* command (it correctly reported what it found), not a
/// `Verdict` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Something failed unexpectedly. Normally stops a script. Exit code 1
    /// (unchanged from before this taxonomy existed), prefixed `error:`.
    Error,
    /// Understood and intentionally refused: permissions, the kill switch,
    /// or a configured limit. The rate limit and `TypeText`'s length cap are
    /// refusals by *configured limit*, which ADR-0007 treats as "disabled by
    /// configuration" — the same category as a policy rule. Must not stop
    /// running automation. Exit code 3.
    Denied,
    /// Permitted and attempted; the world was not, or did not become, what
    /// the caller assumed — a targeted input call's focus precondition
    /// (`INPUT_ERROR_VERIFICATION_FAILED`), or a compositor operation whose
    /// expected state change never appeared
    /// (`WINDOWS_ERROR_OPERATION_NOT_APPLIED`). Must not stop running
    /// automation. Exit code 4.
    Unverified,
}

impl Verdict {
    /// The process exit code for a command that failed with this verdict.
    ///
    /// Deliberately distinct from 0 (success) and 1 (`Error`, unchanged from
    /// before this taxonomy existed), and from 2 — clap's own exit code for
    /// its own argument-parsing failures, confirmed by running
    /// `wgaf --nonsense-flag` (prints `error: unexpected argument...` and
    /// exits 2 without this code ever running). Reusing 2 would blur "you
    /// typed the command wrong" with "the daemon refused it". `Denied` and
    /// `Unverified` get separate codes, not one shared "didn't happen" code,
    /// because they call for different remedies — change policy vs. try
    /// again — per ADR-0007.
    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Error => 1,
            Verdict::Denied => 3,
            Verdict::Unverified => 4,
        }
    }

    /// The `--json` `"outcome"` field's value. Part of the JSON contract —
    /// `plan-first-release.md`'s §16 (W16.2) gives these three strings
    /// verbatim; do not rename them without treating it as a breaking
    /// wire-format change.
    pub fn json_outcome(self) -> &'static str {
        match self {
            Verdict::Error => "error",
            Verdict::Denied => "denied",
            Verdict::Unverified => "unverified",
        }
    }
}

/// A command's failure: a user-readable message (already fit to print, same
/// rule [`describe_dbus_error`] has always followed — never a `Debug` dump)
/// paired with its [`Verdict`].
///
/// Replaces `Box<dyn std::error::Error>` as the error side of every
/// `window`/`input`/`accessibility`/status command function's [`CliResult`],
/// so `main()` can match on the real classification instead of a string that
/// no longer carries it. Deliberately **not** built by downcasting a
/// `Box<dyn Error>` back into a concrete type — every place that used to
/// produce a boxed error now produces this instead, via the `From` impls
/// below.
#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) verdict: Verdict,
    pub(crate) message: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

impl CliError {
    /// Builds an [`Verdict::Error`]-classified `CliError` from anything that
    /// `Display`s sensibly.
    ///
    /// The catch-all for failures that are not one of the daemon's own named
    /// D-Bus errors — a transport failure, a malformed reply, local JSON
    /// serialization or I/O trouble. None of those is ever a denial or a
    /// verification failure by construction: both require the daemon to have
    /// understood and answered the request, which is exactly what did not
    /// happen here. `Error` is therefore always correct in this constructor,
    /// not merely a default.
    fn plain(message: impl fmt::Display) -> Self {
        CliError {
            verdict: Verdict::Error,
            message: message.to_string(),
        }
    }
}

/// The command layer's shared `Result` alias. Every `window`/`input`/
/// `accessibility` subcommand function, plus `ping`/`stop`/`release`/
/// `status` in `commands/mod.rs`, returns this.
pub(crate) type CliResult<T> = Result<T, CliError>;

impl From<zbus::Error> for CliError {
    /// Classifies a `zbus::Error` via [`map_err`], so a bare `?` on any
    /// fallible D-Bus call (`connect()`, a reply's `.deserialize()`, a
    /// signal stream's `.next()`, …) gets the same classification a call
    /// wrapped in `.map_err(map_err)` does.
    fn from(err: zbus::Error) -> Self {
        map_err(err)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        CliError::plain(err)
    }
}

impl From<Box<dyn std::error::Error>> for CliError {
    /// Covers `crate::output::print_json_line`'s boxed return type, the one
    /// remaining place a `?` hands this module something already erased to
    /// `Box<dyn Error>` rather than a concrete `zbus`/`serde_json` error.
    fn from(err: Box<dyn std::error::Error>) -> Self {
        CliError::plain(err)
    }
}

impl From<&str> for CliError {
    /// For the one place a command hand-writes an error message rather than
    /// propagating one — `commands::window::watch`'s "the connection to
    /// wgaf-daemon ended", which is exactly the "something failed
    /// unexpectedly" case `Error` exists for.
    fn from(message: &str) -> Self {
        CliError::plain(message)
    }
}

/// One error name's human label (used only when a reply carries no
/// description — see [`render_method_error`]) and its ADR-0007
/// classification.
///
/// **The one canonical table**, and the merge this module's doc comment
/// describes. Every arm's classification is ADR-0007's own decision, not
/// invented here — including the two genuinely arguable ones:
/// `TextTooLong`/`RateLimited` as denials by *configured limit* ("disabled
/// by configuration", the same category as a policy rule), and
/// `ActionNotSupported` as an error, because "nothing was attempted and the
/// world is exactly as expected; the toolkit simply cannot do it" — the
/// third outcome does not rescue it either.
fn classify(name: &str) -> Option<(&'static str, Verdict)> {
    use Verdict::{Denied, Error, Unverified};
    Some(match name {
        wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND => ("window not found", Error),
        wgaf_common::WINDOWS_ERROR_WORKSPACE_NOT_FOUND => ("workspace not found", Error),
        // The second `Unverified`, and ADR-0007 names this case in as many
        // words: "an action returned successfully and the expected state
        // change never appeared". Nothing malfunctioned — the extension issued
        // the operation, re-read the state, and reported honestly that the
        // desktop had not moved. Calling it an error would tell a user to check
        // their setup, and calling it a denial would send them to
        // `permissions.toml`; neither has anything to do with it, and unlike
        // both it may simply need trying again.
        wgaf_common::WINDOWS_ERROR_OPERATION_NOT_APPLIED => {
            ("the operation was not applied", Unverified)
        }
        wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE => {
            ("GNOME Shell Extension bridge unavailable", Error)
        }
        wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE => ("input device unavailable", Error),
        wgaf_common::INPUT_ERROR_UNKNOWN_KEY => ("unknown key", Error),
        // A capability limit of the active keyboard layout, not a policy
        // choice or an unexpected failure — same bucket as `UnknownKey`/
        // `ActionNotSupported`.
        wgaf_common::INPUT_ERROR_CHARACTER_NOT_TYPEABLE => ("character not typeable", Error),
        wgaf_common::INPUT_ERROR_KEYBOARD_LAYOUT_UNAVAILABLE => {
            ("keyboard layout unavailable", Error)
        }
        wgaf_common::INPUT_ERROR_INVALID_BUTTON => ("invalid mouse button", Error),
        // Configured-limit refusals: "disabled by configuration", the same
        // category ADR-0007 puts policy rules in.
        wgaf_common::INPUT_ERROR_TEXT_TOO_LONG => ("text too long", Denied),
        wgaf_common::INPUT_ERROR_RATE_LIMITED => ("input rate limit exceeded", Denied),
        // The kill switch: the cleanest denial there is — nothing failed,
        // wgaf refused on purpose.
        wgaf_common::INPUT_ERROR_STOPPED => ("input stopped", Denied),
        wgaf_common::INPUT_ERROR_OUT_OF_BOUNDS => ("off screen", Error),
        // Two error names, one fault, deliberately not merged: they are raised
        // by different interfaces (`GetMonitors` and the pointer path), so a
        // client that only ever calls one still only has to know one name.
        wgaf_common::INPUT_ERROR_MONITOR_LAYOUT_UNAVAILABLE
        | wgaf_common::WINDOWS_ERROR_MONITOR_LAYOUT_UNAVAILABLE => {
            ("monitor layout unavailable", Error)
        }
        wgaf_common::INPUT_ERROR_WINDOW_NOT_FOUND => ("window not found", Error),
        // The one D-Bus error this taxonomy's third outcome exists for.
        wgaf_common::INPUT_ERROR_VERIFICATION_FAILED => ("focus could not be verified", Unverified),
        wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE => {
            ("AT-SPI accessibility bus unavailable", Error)
        }
        wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND => {
            ("accessible application not found", Error)
        }
        wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND => {
            ("accessible element not found", Error)
        }
        wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF => {
            ("invalid element reference", Error)
        }
        // The hard case: a toolkit that doesn't implement the AT-SPI
        // interface an action needs is neither a wgaf failure nor a policy
        // choice, and nothing was attempted for the third outcome to cover
        // either — an error by elimination, same bucket as an unknown key.
        wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED => ("action not supported", Error),
        wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED
        | wgaf_common::INPUT_ERROR_PERMISSION_DENIED
        | wgaf_common::ACCESSIBILITY_ERROR_PERMISSION_DENIED => ("permission denied", Denied),
        _ => return None,
    })
}

/// Short fallback label for each of the daemon's named D-Bus errors, used
/// **only** when a reply carries no description at all.
///
/// The daemon's own `#[error(...)]` text is the single source of truth for
/// what went wrong: every one of these errors already opens by naming itself
/// (`"unknown key \`x\`"`, `"GNOME Shell Extension bridge unavailable: …"`),
/// and it is the side that knows the specifics — which key, which bus name,
/// which udev rule to add. This function therefore does **not** build the
/// user-facing message; it only recognizes the error name, so an unrecognized
/// one can be told apart from one of ours.
///
/// Restating the label in front of the description is what produced the
/// stuttering `"unknown key: unknown key \`notakey\`"` output that this
/// replaced.
fn error_name_label(name: &str) -> Option<&'static str> {
    classify(name).map(|(label, _)| label)
}

/// The [`Verdict`] for a named D-Bus error, defaulting to [`Verdict::Error`]
/// for anything [`classify`] does not recognize — any generic/transport-level
/// `zbus::Error` that is not one of the daemon's own named errors is, per
/// ADR-0007, an error like any other, unchanged from today.
fn verdict_for_error_name(name: &str) -> Verdict {
    classify(name)
        .map(|(_, verdict)| verdict)
        .unwrap_or(Verdict::Error)
}

/// Renders any [`zbus::Error`] as a message fit to show a user.
///
/// Three cases, none of which ever reaches for `Debug`:
///
/// 1. **One of the daemon's named errors** — print its description verbatim.
///    The daemon wrote it to be read (see [`error_name_label`]).
/// 2. **Any other `MethodError`** — print its description too. A D-Bus error
///    reply from someone else's service is still a human-readable sentence;
///    what is *not* readable is `MethodError(OwnedErrorName("…"), Some("…"),
///    Msg { type: Error, serial: 61, sender: UniqueName(":1.5042"), … })`,
///    which is what a user saw for something as ordinary as a mistyped
///    element reference before this existed.
/// 3. **A transport-level failure** (no bus, disconnected, …) — `Display`,
///    which `zbus` already implements sensibly.
pub(crate) fn describe_dbus_error(err: &zbus::Error) -> String {
    let zbus::Error::MethodError(name, description, _) = err else {
        return err.to_string();
    };
    render_method_error(name.as_str(), description.as_deref())
}

/// The rendering decision for a D-Bus method-error reply, split out from
/// [`describe_dbus_error`] so it can be unit-tested: `zbus::Error::MethodError`
/// carries a whole `zbus::Message`, which cannot reasonably be constructed in
/// a test, while the choice this function makes is pure.
fn render_method_error(name: &str, description: Option<&str>) -> String {
    match (description, error_name_label(name)) {
        // The overwhelmingly common case: our daemon, with its own message.
        (Some(d), _) => d.to_string(),
        // Ours, but the reply carried no description — fall back to the label
        // so the user still gets a sentence rather than a bare error name.
        (None, Some(label)) => label.to_string(),
        // Someone else's error, no description: the name is all there is, and
        // it is at least a readable dotted string.
        (None, None) => name.to_string(),
    }
}

/// Turns a [`zbus::Error`] into this crate's own error type, carrying both a
/// user-readable message ([`describe_dbus_error`]) and its ADR-0007
/// classification ([`verdict_for_error_name`]) — the pair `main()` needs to
/// decide both what to print and which exit code to use.
///
/// Previously duplicated verbatim in all three command modules, whose only
/// difference was a doc comment naming that interface's error set:
/// `org.wgaf.Windows1`'s window-not-found/extension-unavailable,
/// `org.wgaf.Input1`'s device-unavailable/unknown-key/invalid-button, and
/// `org.wgaf.Accessibility1`'s bus-unavailable/app-not-found/
/// element-not-found/invalid-element-ref/action-not-supported. All three sets
/// are handled by [`describe_dbus_error`], so one copy suffices.
pub(crate) fn map_err(err: zbus::Error) -> CliError {
    let verdict = match &err {
        zbus::Error::MethodError(name, _, _) => verdict_for_error_name(name.as_str()),
        // A transport-level failure, not one of the daemon's own named
        // errors: nothing was understood and refused, something just did not
        // work.
        _ => Verdict::Error,
    };
    CliError {
        verdict,
        message: describe_dbus_error(&err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the double-prefixed output this replaced: the CLI
    /// used to prepend its own label to a daemon message that already opened
    /// with the same words, producing `"unknown key: unknown key \`notakey\`"`.
    /// The daemon's text is the single source of truth and must pass through
    /// untouched.
    #[test]
    fn named_error_with_a_description_is_passed_through_verbatim() {
        let rendered = render_method_error(
            wgaf_common::INPUT_ERROR_UNKNOWN_KEY,
            Some("unknown key `notakey`"),
        );
        assert_eq!(rendered, "unknown key `notakey`");
        assert!(
            !rendered.starts_with("unknown key: unknown key"),
            "the label must not be restated in front of the description"
        );
    }

    /// Every one of the daemon's named errors is self-describing, so none of
    /// them should ever come back with its label stuttered in front. Guards
    /// the whole table at once rather than one arm.
    #[test]
    fn no_named_error_restates_its_own_label() {
        for name in [
            wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND,
            wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE,
            wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE,
            wgaf_common::INPUT_ERROR_UNKNOWN_KEY,
            wgaf_common::INPUT_ERROR_INVALID_BUTTON,
            wgaf_common::INPUT_ERROR_TEXT_TOO_LONG,
            wgaf_common::INPUT_ERROR_RATE_LIMITED,
            wgaf_common::INPUT_ERROR_STOPPED,
            wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE,
            wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND,
            wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND,
            wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF,
            wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED,
            wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED,
        ] {
            let label = error_name_label(name).expect("every arm above is a recognized name");
            let daemon_text = format!("{label}: the specifics");
            assert_eq!(render_method_error(name, Some(&daemon_text)), daemon_text);
        }
    }

    /// `wgaf-common`'s source, scanned by the drift test below.
    ///
    /// Read as text rather than imported because Rust offers no way to
    /// enumerate a module's constants — the same constraint that made the
    /// extension-interface drift test parse `dbusInterface.js`.
    const COMMON_SOURCE: &str = include_str!("../../wgaf-common/src/lib.rs");

    /// Every named D-Bus error the daemon can send **must** be recognized by
    /// [`error_name_label`].
    ///
    /// This is the test that was missing. The hand-maintained list in
    /// `no_named_error_restates_its_own_label` only covers names someone
    /// remembered to add to it, so `INPUT_ERROR_RATE_LIMITED` was introduced,
    /// exported, drift-asserted daemon-side, and still absent from the CLI's
    /// table — with nothing failing. A description-less reply would have
    /// printed the raw dotted error name at a user.
    ///
    /// Scanning `wgaf-common`'s source means a new `*_ERROR_*` constant fails
    /// this until the CLI decides what to call it, rather than being noticed
    /// whenever someone next reads the table.
    #[test]
    fn every_daemon_error_constant_is_recognized_by_the_cli() {
        let mut checked = 0;

        // Split on the declaration keyword and read each one up to its `;`,
        // rather than scanning line by line: six of these constants wrap onto
        // a continuation line, and a line-based scan skipped them silently.
        for decl in COMMON_SOURCE.split("pub const ").skip(1) {
            let Some((decl, _)) = decl.split_once(';') else {
                continue;
            };
            // No trailing space in the separator: a wrapped declaration puts a
            // newline after the `=`, not a space.
            let Some((const_name, value_part)) = decl.split_once(": &str =") else {
                continue;
            };
            if !const_name.contains("_ERROR_") {
                continue;
            }
            // The extension's own error names travel daemon-inward only:
            // `translate_window_error` converts them into `org.wgaf.Windows1`
            // errors, so the CLI never sees one and must not claim to know it.
            if const_name.starts_with("EXTENSION_") {
                continue;
            }

            let value = value_part.trim().trim_matches('"').to_string();
            assert!(
                error_name_label(&value).is_some(),
                "`{const_name}` (\"{value}\") is a named D-Bus error the daemon can send, \
                 but `error_name_label` does not recognize it — add an arm, or the CLI will \
                 print the raw error name for a description-less reply"
            );
            checked += 1;
        }

        // Guards the scan itself: a refactor that moves or reformats these
        // constants would otherwise turn this test into a silent no-op.
        assert_eq!(
            checked, 25,
            "expected 25 daemon error-name constants, found {checked}. If an error was \
             genuinely added or removed, update this number; if not, the scan has stopped \
             matching how `wgaf-common` declares them and is silently passing."
        );
    }

    /// The other half of `every_daemon_error_constant_is_recognized_by_the_cli`:
    /// every one of the same 25 constants also maps to the `Verdict` W16.1
    /// (`plan-first-release.md` §16 / ADR-0007) documents for it. Hand-listed
    /// rather than scanned, since the *value* — not merely the presence — is
    /// what a scan cannot check; keep the count in sync with that test's.
    #[test]
    fn every_constant_maps_to_its_documented_verdict() {
        use Verdict::{Denied, Error, Unverified};
        let cases: &[(&str, Verdict)] = &[
            (wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND, Error),
            (wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE, Error),
            (wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED, Denied),
            (wgaf_common::WINDOWS_ERROR_MONITOR_LAYOUT_UNAVAILABLE, Error),
            (wgaf_common::WINDOWS_ERROR_WORKSPACE_NOT_FOUND, Error),
            (wgaf_common::WINDOWS_ERROR_OPERATION_NOT_APPLIED, Unverified),
            (wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE, Error),
            (wgaf_common::INPUT_ERROR_UNKNOWN_KEY, Error),
            (wgaf_common::INPUT_ERROR_CHARACTER_NOT_TYPEABLE, Error),
            (wgaf_common::INPUT_ERROR_KEYBOARD_LAYOUT_UNAVAILABLE, Error),
            (wgaf_common::INPUT_ERROR_INVALID_BUTTON, Error),
            (wgaf_common::INPUT_ERROR_TEXT_TOO_LONG, Denied),
            (wgaf_common::INPUT_ERROR_RATE_LIMITED, Denied),
            (wgaf_common::INPUT_ERROR_STOPPED, Denied),
            (wgaf_common::INPUT_ERROR_PERMISSION_DENIED, Denied),
            (wgaf_common::INPUT_ERROR_OUT_OF_BOUNDS, Error),
            (wgaf_common::INPUT_ERROR_MONITOR_LAYOUT_UNAVAILABLE, Error),
            (wgaf_common::INPUT_ERROR_WINDOW_NOT_FOUND, Error),
            (wgaf_common::INPUT_ERROR_VERIFICATION_FAILED, Unverified),
            (wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE, Error),
            (wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND, Error),
            (wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND, Error),
            (wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF, Error),
            (wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED, Error),
            (wgaf_common::ACCESSIBILITY_ERROR_PERMISSION_DENIED, Denied),
        ];
        assert_eq!(
            cases.len(),
            25,
            "keep this list's length in sync with \
             every_daemon_error_constant_is_recognized_by_the_cli's count"
        );
        for (name, expected) in cases {
            assert_eq!(
                verdict_for_error_name(name),
                *expected,
                "wrong verdict for {name}"
            );
        }
    }

    /// A name `classify` does not recognize — a transport-level
    /// `zbus::Error`, or someone else's D-Bus service — must default to
    /// `Error`, never a denial or a verification failure. Neither of those
    /// can be inferred from an unrecognized name.
    #[test]
    fn unrecognized_error_name_defaults_to_error_verdict() {
        assert_eq!(
            verdict_for_error_name("org.freedesktop.DBus.Error.UnknownMethod"),
            Verdict::Error
        );
    }

    #[test]
    fn verdict_exit_codes_are_distinct_from_success_error_and_clap() {
        // clap's own parse-failure exit code is 2 — verified by running
        // `wgaf --nonsense-flag`, which exits 2 without this code ever
        // running (see plan-first-release.md §16).
        assert_eq!(Verdict::Error.exit_code(), 1);
        assert_eq!(Verdict::Denied.exit_code(), 3);
        assert_eq!(Verdict::Unverified.exit_code(), 4);
    }

    #[test]
    fn verdict_json_outcome_strings_match_the_plan_verbatim() {
        assert_eq!(Verdict::Error.json_outcome(), "error");
        assert_eq!(Verdict::Denied.json_outcome(), "denied");
        assert_eq!(Verdict::Unverified.json_outcome(), "unverified");
    }

    /// `CliError`'s `Display` is the bare message, with no verdict-derived
    /// framing — `main()` decides the `error:` prefix (or its absence), not
    /// this type.
    #[test]
    fn cli_error_display_is_the_bare_message() {
        let err = CliError {
            verdict: Verdict::Denied,
            message: "denied by policy".to_string(),
        };
        assert_eq!(err.to_string(), "denied by policy");
    }

    /// Regression test for the raw-`Debug`-dump issue: an error name the CLI
    /// does not recognize must still render as its human-readable description,
    /// never as `MethodError(OwnedErrorName(...), ..., Msg { serial: 61, ... })`.
    #[test]
    fn unrecognized_error_name_still_renders_its_description() {
        let rendered = render_method_error(
            "org.freedesktop.zbus.Error",
            Some("D-Bus error talking to the accessibility bus: Invalid bus name"),
        );
        assert_eq!(
            rendered,
            "D-Bus error talking to the accessibility bus: Invalid bus name"
        );
        assert!(
            !rendered.contains("MethodError"),
            "must not leak Debug output"
        );
    }

    /// A description-less reply is legal on D-Bus. Ours fall back to the
    /// label; anyone else's to the error name — either way a printable string,
    /// never an empty message.
    #[test]
    fn description_less_replies_fall_back_to_a_label_or_the_name() {
        assert_eq!(
            render_method_error(wgaf_common::INPUT_ERROR_INVALID_BUTTON, None),
            "invalid mouse button"
        );
        assert_eq!(
            render_method_error("com.example.Whatever", None),
            "com.example.Whatever"
        );
    }

    /// The new `InvalidElementRef` name must be recognized, so a malformed
    /// element reference is reported as a caller mistake rather than falling
    /// into the unrecognized-name path.
    #[test]
    fn invalid_element_ref_is_a_recognized_error_name() {
        assert_eq!(
            error_name_label(wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF),
            Some("invalid element reference")
        );
    }
}
