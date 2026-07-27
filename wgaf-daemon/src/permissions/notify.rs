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
    let proxy = NotificationsProxy::new(connection).await?;

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

    let mut action_invoked = proxy.receive_action_invoked().await?;
    let mut notification_closed = proxy.receive_notification_closed().await?;

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
