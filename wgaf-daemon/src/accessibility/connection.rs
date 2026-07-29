//! Lazy connection to the AT-SPI accessibility bus (`org.a11y.Bus`) — a
//! distinct D-Bus bus from the session bus, with its address discovered via
//! `org.a11y.Bus.GetAddress` on the session bus. That indirection is not
//! hand-rewritten here: `atspi::AccessibilityConnection::new()` (from the
//! `atspi-connection` crate, re-exported as `atspi::connection`/
//! `atspi::AccessibilityConnection`) already performs it.
//!
//! **The failure path is ours, though.** `atspi` renders any underlying
//! `zbus::Error` with `Debug` and stores the result as a `String`
//! (`AtspiError::Zbus(format!("{e:?}"))`, `atspi-common`'s `From<zbus::Error>`),
//! so its error text arrives here already containing things like
//! `InputOutput(Os { code: 2, kind: NotFound, ... })`. There is no way to
//! un-format that back into something presentable, and `wgaf status` prints
//! this text verbatim. So the atspi error is kept for the debug log only, and
//! the user-facing reason is produced by [`diagnose`], which re-walks the same
//! two steps atspi took and reports *which* one failed. That distinction is the
//! point: a session with no accessibility bus at all and a session whose bus
//! has exited behind a stale address are different problems with different
//! remedies, and a single guessed hint served neither.

use std::path::Path;

use super::AccessibilityError;

/// Session-bus coordinates of the service that publishes the accessibility
/// bus's address. Only used on the failure path — the happy path never names
/// them, since `atspi` does that lookup itself.
const A11Y_BUS_SERVICE: &str = "org.a11y.Bus";
const A11Y_BUS_PATH: &str = "/org/a11y/bus";

/// What `org.freedesktop.DBus` answers with when a well-known name is neither
/// owned nor activatable.
const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";

/// Connects to the AT-SPI bus, translating a failure into
/// [`AccessibilityError::BusUnavailable`] with a clear, actionable message —
/// never a raw `AtspiError` debug dump. Called only from
/// `AccessibilityBackend::connection`'s `OnceCell::get_or_try_init`, so this
/// only actually runs on first use, and again on every subsequent call if it
/// failed (nothing here is cached on failure — see `mod.rs`).
pub(crate) async fn connect() -> Result<atspi::AccessibilityConnection, AccessibilityError> {
    match atspi::AccessibilityConnection::new().await {
        Ok(connection) => Ok(connection),
        Err(err) => {
            // Developers get atspi's own text, Debug dump and all; users get
            // the diagnosis below instead. See the module docs for why these
            // cannot be the same string.
            tracing::debug!(error = %err, "AT-SPI connection failed, diagnosing the cause");
            Err(AccessibilityError::BusUnavailable {
                reason: diagnose().await,
            })
        }
    }
}

/// Works out *why* the accessibility bus could not be reached, by repeating
/// the lookup `atspi::AccessibilityConnection::new()` performs and observing
/// where it breaks down. Runs only after a connection attempt has already
/// failed, so the extra round trip costs nothing on the happy path.
///
/// Always returns a message rather than an error: this is called when
/// something has *already* gone wrong, and a diagnosis that can itself fail to
/// produce an answer would just move the problem.
async fn diagnose() -> String {
    let session = match zbus::Connection::session().await {
        Ok(session) => session,
        Err(err) => return explain_no_session_bus(&err),
    };

    match a11y_bus_address(&session).await {
        Ok(address) => {
            let socket = unix_socket_path(&address);
            explain_address(&address, socket.map(|path| Path::new(path).exists()))
        }
        Err(err) => explain_lookup_failure(&err),
    }
}

/// Asks `org.a11y.Bus.GetAddress` for the accessibility bus address. Uses an
/// untyped [`zbus::Proxy`] rather than `atspi`'s own `BusProxy` so the
/// `zbus::Error` arrives intact and can be rendered with `Display`.
async fn a11y_bus_address(session: &zbus::Connection) -> Result<String, zbus::Error> {
    let proxy =
        zbus::Proxy::new(session, A11Y_BUS_SERVICE, A11Y_BUS_PATH, A11Y_BUS_SERVICE).await?;
    proxy.call("GetAddress", &()).await
}

fn explain_no_session_bus(err: &zbus::Error) -> String {
    format!(
        "the session bus itself is unreachable ({err}), so the accessibility bus address could \
         not be looked up — this usually means the daemon is running outside a desktop session"
    )
}

/// Splits a failed `GetAddress` into the two cases worth telling apart: the
/// service is not there at all, versus it is there and refused.
fn explain_lookup_failure(err: &zbus::Error) -> String {
    if let zbus::Error::MethodError(name, _, _) = err
        && name.as_str() == SERVICE_UNKNOWN
    {
        return format!(
            "no `{A11Y_BUS_SERVICE}` service is available on the session bus and it could not be \
             started — accessibility is not enabled for this session (the service is provided by \
             the `at-spi2-core` package)"
        );
    }

    format!("asking `{A11Y_BUS_SERVICE}` for the accessibility bus address failed: {err}")
}

/// Turns a successfully-retrieved address into a reason, given whether its
/// socket exists (`None` when the address names no filesystem socket — see
/// [`unix_socket_path`]).
///
/// Kept pure and separate from the I/O so the three outcomes can be tested
/// without an accessibility bus in any particular state.
fn explain_address(address: &str, socket_exists: Option<bool>) -> String {
    match (unix_socket_path(address), socket_exists) {
        // The case that motivated this whole function: `GetAddress` keeps
        // answering with the address of a bus that has since exited, so the
        // path it names is simply not there. Nothing is misconfigured and
        // nothing needs enabling — the address is stale.
        (Some(path), Some(false)) => format!(
            "`{A11Y_BUS_SERVICE}` reports the accessibility bus at `{path}`, but no socket \
             exists there — the bus it refers to has exited and the address is stale. Logging \
             out and back in restores it"
        ),
        (Some(path), _) => format!(
            "the accessibility bus socket at `{path}` exists but could not be connected to — \
             run the daemon with `--log-level debug` for the underlying D-Bus error"
        ),
        (None, _) => format!(
            "the accessibility bus at `{address}` could not be connected to — run the daemon \
             with `--log-level debug` for the underlying D-Bus error"
        ),
    }
}

/// Extracts the socket path from a D-Bus address such as
/// `unix:path=/run/user/1000/at-spi/bus,guid=…`, or `None` if the address does
/// not name one — an abstract socket or a non-`unix:` transport is perfectly
/// valid, just not something whose existence can be checked on disk.
fn unix_socket_path(address: &str) -> Option<&str> {
    let params = address.strip_prefix("unix:")?;
    params
        .split(',')
        .find_map(|param| param.strip_prefix("path="))
        .filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_socket_path_reads_the_path_parameter() {
        assert_eq!(
            unix_socket_path("unix:path=/run/user/1000/at-spi/bus"),
            Some("/run/user/1000/at-spi/bus")
        );
    }

    /// The literal shape `org.a11y.Bus.GetAddress` answers with on GNOME —
    /// captured from a live session, since the trailing `guid` is exactly what
    /// a naive `strip_prefix`-only parser would swallow into the path.
    #[test]
    fn unix_socket_path_ignores_other_parameters() {
        assert_eq!(
            unix_socket_path(
                "unix:path=/run/user/1000/at-spi/bus,guid=d7ccf62d756d4cb2210d1d346a6a605a"
            ),
            Some("/run/user/1000/at-spi/bus")
        );
        assert_eq!(
            unix_socket_path("unix:guid=abc123,path=/run/user/1000/at-spi/bus"),
            Some("/run/user/1000/at-spi/bus")
        );
    }

    #[test]
    fn unix_socket_path_is_none_without_a_filesystem_socket() {
        assert_eq!(unix_socket_path("unix:abstract=/tmp/dbus-Xyz"), None);
        assert_eq!(unix_socket_path("tcp:host=localhost,port=1234"), None);
        assert_eq!(unix_socket_path("unix:path="), None);
    }

    #[test]
    fn a_missing_socket_is_reported_as_stale_not_as_disabled() {
        let reason = explain_address("unix:path=/run/user/1000/at-spi/bus", Some(false));

        assert!(reason.contains("/run/user/1000/at-spi/bus"), "{reason}");
        assert!(reason.contains("stale"), "{reason}");
        // The wrong answer this issue existed to remove: the bus having exited
        // says nothing about whether accessibility is enabled, and in the case
        // that produced it, accessibility was enabled.
        assert!(!reason.contains("not enabled"), "{reason}");
    }

    #[test]
    fn an_existing_socket_points_at_the_debug_log_instead_of_guessing() {
        let reason = explain_address("unix:path=/run/user/1000/at-spi/bus", Some(true));

        assert!(reason.contains("/run/user/1000/at-spi/bus"), "{reason}");
        assert!(reason.contains("--log-level debug"), "{reason}");
        assert!(!reason.contains("stale"), "{reason}");
    }

    #[test]
    fn an_address_without_a_socket_path_still_names_the_address() {
        let reason = explain_address("unix:abstract=/tmp/dbus-Xyz", None);

        assert!(reason.contains("unix:abstract=/tmp/dbus-Xyz"), "{reason}");
        assert!(reason.contains("--log-level debug"), "{reason}");
    }

    /// The whole point of the change: whatever branch is taken, the reason
    /// must read as prose, never as Rust type syntax.
    #[test]
    fn no_explanation_contains_a_debug_dump() {
        let io = zbus::Error::InputOutput(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No such file or directory",
        )));

        let reasons = [
            explain_no_session_bus(&io),
            explain_lookup_failure(&io),
            explain_address("unix:path=/run/user/1000/at-spi/bus", Some(false)),
            explain_address("unix:path=/run/user/1000/at-spi/bus", Some(true)),
            explain_address("unix:abstract=/tmp/dbus-Xyz", None),
        ];

        for reason in reasons {
            assert!(!reason.contains("Os {"), "{reason}");
            assert!(!reason.contains("InputOutput("), "{reason}");
            assert!(!reason.contains("kind:"), "{reason}");
        }
    }
}
