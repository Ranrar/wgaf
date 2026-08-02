pub mod accessibility;
pub mod input;
pub mod window;

use zbus::Connection;

pub async fn ping(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await
        // `ping` is usually the first command a user runs, and the most
        // likely thing to go wrong for them is "the daemon isn't running" —
        // so it deserves the same readable rendering as every other command
        // rather than the propagated raw `zbus::Error` it used before.
        .map_err(map_err)?;
    let response: String = reply.body().deserialize()?;
    // FIXED: `--json` is a global flag advertised on every subcommand's
    // `--help` (including this one), but this command silently ignored it
    // and always printed plain text — found during the Phase 7 --help
    // consistency pass. Every other command family already honors it.
    crate::output::print_ok_response(json, &response);
    Ok(())
}

/// `wgaf stop` — the kill switch.
///
/// Ungated by design: no `permissions.toml` policy can refuse it, so this
/// command fails only if the daemon cannot be reached at all.
pub async fn stop(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    daemon_call(bus_name, "Stop").await?;
    crate::output::print_ok(
        json,
        "input stopped — no keystrokes, clicks or scrolls will be synthesized. \
         Run `wgaf release` to allow them again.",
    );
    Ok(())
}

/// `wgaf release` — lifts the kill switch.
///
/// The message says what the command does *not* do, because that is the part
/// people expect: releasing a brake is not resuming a journey, and the command
/// that was interrupted has to be run again.
pub async fn release(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    daemon_call(bus_name, "Release").await?;
    crate::output::print_ok(
        json,
        "input released — new commands are allowed again. Whatever was stopped \
         is not restarted; run it again if you still want it.",
    );
    Ok(())
}

/// Calls an argument-less, reply-less `org.wgaf.Daemon1` method.
async fn daemon_call(bus_name: &str, method: &str) -> Result<(), Box<dyn std::error::Error>> {
    connect()
        .await?
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            method,
            &(),
        )
        .await
        .map_err(map_err)?;
    Ok(())
}

/// `wgaf status` — the daemon's self-report, rendered for a human (or as
/// JSON).
///
/// Returns `Ok(false)` when some subsystem is unavailable, so `main` can exit
/// non-zero without treating it as a CLI error: an unhealthy subsystem is a
/// successful *report*, not a failed command, and conflating the two would
/// make `wgaf status` print its own error text instead of the daemon's
/// actionable guidance.
pub async fn status(bus_name: &str, json: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Status",
            &(),
        )
        .await
        .map_err(map_err)?;
    let dict: wgaf_common::dict::DaemonStatusDict = reply.body().deserialize()?;
    let status: wgaf_common::DaemonStatus = dict.into();

    let healthy =
        status.extension_available && status.uinput_accessible && status.accessibility_available;

    if json {
        crate::output::print_json(&status)?;
    } else {
        crate::output::print_status(&status);
    }
    Ok(healthy)
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
    match name {
        wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND => Some("window not found"),
        wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE => {
            Some("GNOME Shell Extension bridge unavailable")
        }
        wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE => Some("input device unavailable"),
        wgaf_common::INPUT_ERROR_UNKNOWN_KEY => Some("unknown key"),
        wgaf_common::INPUT_ERROR_INVALID_BUTTON => Some("invalid mouse button"),
        wgaf_common::INPUT_ERROR_TEXT_TOO_LONG => Some("text too long"),
        wgaf_common::INPUT_ERROR_RATE_LIMITED => Some("input rate limit exceeded"),
        wgaf_common::INPUT_ERROR_STOPPED => Some("input stopped"),
        wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE => {
            Some("AT-SPI accessibility bus unavailable")
        }
        wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND => Some("accessible application not found"),
        wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND => Some("accessible element not found"),
        wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF => Some("invalid element reference"),
        wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED => Some("action not supported"),
        wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED
        | wgaf_common::INPUT_ERROR_PERMISSION_DENIED
        | wgaf_common::ACCESSIBILITY_ERROR_PERMISSION_DENIED => Some("permission denied"),
        _ => None,
    }
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

/// Opens a session-bus connection. Shared by every command family — see
/// [`map_err`] for the other half of this pair.
pub(crate) async fn connect() -> zbus::Result<Connection> {
    Connection::session().await
}

/// Turns a [`zbus::Error`] into a boxed error carrying a user-readable
/// message, for the `?` paths in `window`/`input`/`accessibility`.
///
/// Previously duplicated verbatim in all three command modules, whose only
/// difference was a doc comment naming that interface's error set:
/// `org.wgaf.Windows1`'s window-not-found/extension-unavailable,
/// `org.wgaf.Input1`'s device-unavailable/unknown-key/invalid-button, and
/// `org.wgaf.Accessibility1`'s bus-unavailable/app-not-found/
/// element-not-found/invalid-element-ref/action-not-supported. All three sets
/// are handled by [`describe_dbus_error`], so one copy suffices.
pub(crate) fn map_err(err: zbus::Error) -> Box<dyn std::error::Error> {
    describe_dbus_error(&err).into()
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
    const COMMON_SOURCE: &str = include_str!("../../../wgaf-common/src/lib.rs");

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
            checked, 16,
            "expected 16 daemon error-name constants, found {checked}. If an error was \
             genuinely added or removed, update this number; if not, the scan has stopped \
             matching how `wgaf-common` declares them and is silently passing."
        );
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
