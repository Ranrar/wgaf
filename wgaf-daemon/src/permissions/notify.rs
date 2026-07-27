//! Real "Prompt" policy implementation: shows an actionable desktop
//! notification via the freedesktop.org Notifications spec
//! (`org.freedesktop.Notifications`, the same interface GNOME Shell's own
//! notification banners implement — no GNOME-specific API used here), then
//! waits for the user's choice via that spec's `ActionInvoked`/
//! `NotificationClosed` signals.
//!
//! This is a *client* of the desktop's own always-running notification
//! service, reached over the same session-bus [`zbus::Connection`] the
//! daemon already serves `org.wgaf.*` on — not a new daemon-owned D-Bus
//! surface, and not GNOME Shell Extension involvement (unlike
//! `windows::WindowManager`, there is no `wgaf`-specific bridge here; every
//! desktop implementing the freedesktop Notifications spec works
//! identically).
//!
//! **Fail closed.** A timeout, an explicit "Deny" click, or the notification
//! being dismissed/closed without a button press (e.g. the user clicking
//! the notification body itself, or it expiring) all resolve to `Ok(false)`
//! — only an explicit "Allow" click resolves to `Ok(true)`. Silence is
//! treated as refusal, never as permission.
//!
//! **Fail closed must mean "the user didn't answer", not "we weren't
//! listening yet".** Signal subscription therefore happens *before* the
//! notification is posted — see the ordering note in [`prompt_user`]. Get
//! that order wrong and a fast "Allow" click is silently converted into a
//! 60-second stall followed by a denial, which looks identical to the user
//! ignoring the prompt.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::zvariant::Value;

use super::PermissionError;
use super::policy::Capability;

/// How long a `Prompt` notification waits for the user before resolving to
/// a (fail-safe) denial. Not currently configurable: long enough that a
/// user who glances away doesn't lose the prompt, short enough that an
/// automation script blocked on this call doesn't hang indefinitely.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// Well-known bus name of the session's notification service, per the
/// freedesktop.org Notifications spec. Overridable only via
/// [`prompt_user_at`], for the same reason `Config::extension_bus_name`
/// exists on the Windows1 side: it lets a test point this at a stub service
/// on a private, unique bus name instead of the real one, which is otherwise
/// owned by GNOME Shell and impossible to fake on a live session.
const NOTIFICATIONS_BUS_NAME: &str = "org.freedesktop.Notifications";

/// Client proxy for the freedesktop.org Notifications spec
/// (`org.freedesktop.Notifications` at `/org/freedesktop/Notifications`,
/// registered by the session's notification daemon — e.g. `gnome-shell`
/// itself on a stock GNOME session). Only the subset this module needs
/// (`Notify`, `ActionInvoked`, `NotificationClosed`) is declared; see
/// https://specifications.freedesktop.org/notification-spec/latest/ for the
/// full interface.
#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    /// Returns the new notification's id (used to match the
    /// `ActionInvoked`/`NotificationClosed` signals below back to this
    /// specific prompt).
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Shows an Allow/Deny notification for `capability` and waits (up to
/// [`PROMPT_TIMEOUT`]) for the user's choice — see module docs for the
/// fail-closed semantics.
pub(crate) async fn prompt_user(
    connection: &zbus::Connection,
    capability: Capability,
) -> Result<bool, PermissionError> {
    prompt_user_at(connection, NOTIFICATIONS_BUS_NAME, capability).await
}

/// [`prompt_user`] with the notification service's bus name made explicit.
/// Production always passes [`NOTIFICATIONS_BUS_NAME`]; the tests below pass
/// a private, unique name owned by a stub service, mirroring how
/// `Config::extension_bus_name` lets `tests/windows_stub.rs` stand in for the
/// GNOME Shell Extension.
async fn prompt_user_at(
    connection: &zbus::Connection,
    destination: &str,
    capability: Capability,
) -> Result<bool, PermissionError> {
    // Owned `String` (not a borrowed `&str`) so the proxy's lifetime isn't
    // tied to `destination` — same convention as
    // `windows::proxy::ShellExtensionProxy`'s `.destination(….to_string())`.
    let proxy = NotificationsProxy::builder(connection)
        .destination(destination.to_string())?
        .build()
        .await?;

    // ORDER MATTERS: subscribe to both signals BEFORE calling `Notify`.
    //
    // FIXED: this function used to call `Notify` first and subscribe
    // afterwards, which left a window between the notification appearing on
    // screen and the match rules being installed on the bus. A user who
    // clicked "Allow" inside that window had the click dropped entirely, and
    // the call then blocked the full `PROMPT_TIMEOUT` before failing closed —
    // so an instant, deliberate "yes" was indistinguishable from walking
    // away. The window is short, but it is widest exactly when the user is
    // already watching for the prompt and reacts immediately, which is the
    // common interactive case rather than a rare one.
    //
    // `receive_*` installs the match rule and begins buffering matching
    // signals as soon as it returns, so with this ordering nothing emitted
    // after this point can be missed, however fast the reply comes back.
    //
    // Subscribing before we know `notification_id` is intentional and safe:
    // these streams also carry other applications' notification signals, and
    // the `args.id == notification_id` checks in the loop below discard
    // everything that isn't ours.
    let mut action_invoked = proxy.receive_action_invoked().await?;
    let mut notification_closed = proxy.receive_notification_closed().await?;

    let summary = format!("wgaf automation: allow \"{}\"?", capability.as_str());
    let body = "A script or CLI command is requesting permission to perform this action. Your \
                choice is remembered for the rest of this wgaf-daemon session.";
    let notification_id = proxy
        .notify(
            "wgaf",
            0,
            "dialog-question-symbolic",
            &summary,
            body,
            // Action list is (key, label) pairs flattened: "allow"/"Allow"
            // is the button labeled "Allow", carrying the action key
            // "allow" back in `ActionInvoked`.
            &["allow", "Allow", "deny", "Deny"],
            HashMap::new(),
            0, // never expire on its own; PROMPT_TIMEOUT below bounds the wait instead.
        )
        .await?;

    let wait_for_choice = async {
        loop {
            tokio::select! {
                Some(signal) = action_invoked.next() => {
                    if let Ok(args) = signal.args()
                        && args.id == notification_id
                    {
                        return args.action_key == "allow";
                    }
                }
                Some(signal) = notification_closed.next() => {
                    if let Ok(args) = signal.args()
                        && args.id == notification_id
                    {
                        // Closed without an explicit action (dismissed,
                        // expired, or the notification server itself closed
                        // it) — fail closed, not open.
                        return false;
                    }
                }
                else => return false,
            }
        }
    };

    Ok(tokio::time::timeout(PROMPT_TIMEOUT, wait_for_choice)
        .await
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::object_server::SignalEmitter;

    // These tests need a session bus (the whole `wgaf-daemon` integration
    // suite already does — see `tests/ping.rs`). They deliberately do NOT
    // skip when one is absent: a silently-skipped regression test is worse
    // than a failing one.

    /// Bounds every test well below [`PROMPT_TIMEOUT`], so a regression fails
    /// in seconds rather than stalling the suite for a minute per test.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// The id the stub assigns to every notification it accepts.
    const STUB_NOTIFICATION_ID: u32 = 1;

    /// How the stub should answer a `Notify` call.
    #[derive(Clone, Copy)]
    enum StubResponse {
        Allow,
        Deny,
        /// Closed without any button press (dismissed/expired) — reason 2 is
        /// the spec's "dismissed by the user".
        Dismiss,
    }

    /// A stand-in for the desktop's notification service that answers
    /// **before** replying to `Notify`.
    ///
    /// That ordering is the whole point, and is what makes these tests
    /// deterministic rather than timing-dependent. On a real bus a signal is
    /// only delivered to connections that already have a matching match rule
    /// installed. So if `prompt_user_at` posts the notification *before*
    /// subscribing (the bug fixed on 2026-07-27), the bus has nowhere to
    /// route this signal and drops it outright — guaranteed, not
    /// occasionally — and the call then blocks until `PROMPT_TIMEOUT` and
    /// fails closed. Subscribing first means the match rule exists when the
    /// stub emits, so the signal is buffered and picked up as soon as the
    /// `Notify` reply lands.
    ///
    /// Net effect: reorder those two statements in `prompt_user_at` and all
    /// three tests below fail within `TEST_TIMEOUT`.
    struct NotificationsStub {
        response: StubResponse,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl NotificationsStub {
        #[allow(clippy::too_many_arguments)]
        async fn notify(
            &self,
            _app_name: &str,
            _replaces_id: u32,
            _app_icon: &str,
            _summary: &str,
            _body: &str,
            _actions: Vec<String>,
            _hints: HashMap<String, zbus::zvariant::OwnedValue>,
            _expire_timeout: i32,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> u32 {
            let emitted = match self.response {
                StubResponse::Allow => {
                    Self::action_invoked(&emitter, STUB_NOTIFICATION_ID, "allow").await
                }
                StubResponse::Deny => {
                    Self::action_invoked(&emitter, STUB_NOTIFICATION_ID, "deny").await
                }
                StubResponse::Dismiss => {
                    Self::notification_closed(&emitter, STUB_NOTIFICATION_ID, 2).await
                }
            };
            // Report rather than swallow: the assertion below would fail
            // anyway, but silently is a miserable way to debug it.
            if let Err(e) = emitted {
                eprintln!("notification stub failed to emit its response signal: {e}");
            }
            STUB_NOTIFICATION_ID
        }

        #[zbus(signal)]
        pub async fn action_invoked(
            emitter: &SignalEmitter<'_>,
            id: u32,
            action_key: &str,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        pub async fn notification_closed(
            emitter: &SignalEmitter<'_>,
            id: u32,
            reason: u32,
        ) -> zbus::Result<()>;
    }

    /// Serves the stub on a private, unique bus name and leaks the connection
    /// for the rest of the test process — the same pattern (and the same
    /// rationale for leaking) as `tests/windows_stub.rs`'s
    /// `start_stub_extension`.
    async fn start_stub(bus_name: &str, response: StubResponse) {
        let connection = zbus::connection::Builder::session()
            .expect("session bus builder")
            .name(bus_name)
            .expect("valid bus name")
            .serve_at("/org/freedesktop/Notifications", NotificationsStub {
                response,
            })
            .expect("serve stub notification service")
            .build()
            .await
            .expect("stub notification service registers on the session bus");
        std::mem::forget(connection);
    }

    /// Runs one prompt against a freshly-started stub. `tag` keeps each
    /// test's bus name distinct, since these run concurrently by default and
    /// a bus name is a session-global resource.
    ///
    /// `tag` and the pid are concatenated into a *single* name element
    /// (`…NotificationsAllow1234`, not `…Notifications.Allow.1234`) because a
    /// D-Bus name element may not begin with a digit — the same convention
    /// `tests/windows_stub.rs` already uses for its unique names.
    async fn prompt_against_stub(tag: &str, response: StubResponse) -> bool {
        let bus_name = format!("org.wgaf.Test.Notifications{tag}{}", std::process::id());
        start_stub(&bus_name, response).await;

        let connection = zbus::Connection::session()
            .await
            .expect("connect to session bus");

        tokio::time::timeout(
            TEST_TIMEOUT,
            prompt_user_at(&connection, &bus_name, Capability::TypeText),
        )
        .await
        .expect(
            "prompt_user_at blocked waiting for a response the stub had already sent — the \
             ActionInvoked/NotificationClosed subscriptions must happen BEFORE Notify is called",
        )
        .expect("the prompt flow should complete without a D-Bus error")
    }

    #[tokio::test]
    async fn allow_click_arriving_before_the_notify_reply_is_not_missed() {
        assert!(
            prompt_against_stub("Allow", StubResponse::Allow).await,
            "an explicit Allow must resolve to permission granted"
        );
    }

    #[tokio::test]
    async fn deny_click_resolves_to_refusal() {
        assert!(
            !prompt_against_stub("Deny", StubResponse::Deny).await,
            "an explicit Deny must resolve to permission refused"
        );
    }

    #[tokio::test]
    async fn dismissal_without_a_button_press_resolves_to_refusal() {
        assert!(
            !prompt_against_stub("Dismiss", StubResponse::Dismiss).await,
            "a notification closed without a button press must fail closed, not open"
        );
    }
}
