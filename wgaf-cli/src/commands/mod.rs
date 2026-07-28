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
        assert!(!rendered.contains("MethodError"), "must not leak Debug output");
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
