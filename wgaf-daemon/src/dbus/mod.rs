pub mod accessibility_api;
pub mod input_api;
pub mod windows_api;

use std::fmt::Display;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wgaf_common::DaemonStatus;
use wgaf_common::dict::DaemonStatusDict;
use zbus::interface;

use crate::accessibility::AccessibilityBackend;
use crate::input::InputBackend;
use crate::permissions::PermissionGate;
use crate::windows::WindowManager;

/// The daemon's own `org.wgaf.Daemon1` interface: liveness (`Ping`),
/// version, and the cross-cutting `Status` self-report.
///
/// Unlike `WindowsApi`/`InputApi`/`AccessibilityApi`, which each own one
/// subsystem, this one holds a handle to *all* of them — `Status` is
/// deliberately cross-cutting, and this interface is the only sensible home
/// for a question that spans every subsystem at once. Every handle is an
/// `Arc` shared with the interface that actually owns the subsystem, the
/// same way `PermissionGate` was already shared across all three.
pub struct Daemon {
    bus_name: String,
    config_path: Option<PathBuf>,
    permissions_path: Option<PathBuf>,
    started: Instant,
    windows: Arc<WindowManager>,
    input: Arc<InputBackend>,
    accessibility: Arc<AccessibilityBackend>,
    permissions: Arc<PermissionGate>,
}

impl Daemon {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bus_name: impl Into<String>,
        config_path: Option<PathBuf>,
        permissions_path: Option<PathBuf>,
        windows: Arc<WindowManager>,
        input: Arc<InputBackend>,
        accessibility: Arc<AccessibilityBackend>,
        permissions: Arc<PermissionGate>,
    ) -> Self {
        Self {
            bus_name: bus_name.into(),
            config_path,
            permissions_path,
            started: Instant::now(),
            windows,
            input,
            accessibility,
            permissions,
        }
    }
}

/// Renders an optional path for the wire. Empty string means "no location at
/// all", which `a{sv}` expresses more simply than a nullable type would.
fn path_string(path: Option<&PathBuf>) -> String {
    path.map(|p| p.display().to_string()).unwrap_or_default()
}

/// Whether a resolved location actually holds a file. Reported alongside the
/// path so "here is where it goes" is never mistaken for "here is what is
/// being applied".
fn path_present(path: Option<&PathBuf>) -> bool {
    path.map(|p| p.exists()).unwrap_or(false)
}

/// How long any one subsystem probe may take before [`Daemon::status`] gives up
/// on it.
///
/// Every probe is a local D-Bus round trip or a file open, so a healthy one
/// answers in milliseconds — this is roughly a hundredfold headroom, not a
/// tuned value. It exists because a probe can hang *indefinitely*: the
/// accessibility probe was observed stalling mid-handshake against an a11y bus
/// that was answering other clients perfectly at the same moment, and `Status`
/// never returned.
///
/// Erring long is the right direction. A probe wrongly reported unavailable is
/// a misleading line in one report; a probe allowed to hang takes the entire
/// report with it, at the exact moment the user is trying to find out what is
/// wrong.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Runs one subsystem probe, turning both failure *and* hanging into a
/// reportable answer.
///
/// The distinction matters to whoever reads the output: "unavailable because it
/// said so" and "unavailable because it never replied" are different faults
/// with different fixes, so the timeout says which one happened.
/// The timeout is a parameter rather than reaching for [`PROBE_TIMEOUT`]
/// directly so that the tests can prove the hang path in milliseconds instead
/// of spending three real seconds each demonstrating a property about time.
async fn probe<E: Display>(
    subsystem: &str,
    timeout: Duration,
    probe: impl Future<Output = Result<(), E>>,
) -> (bool, String) {
    match tokio::time::timeout(timeout, probe).await {
        Ok(Ok(())) => (true, String::new()),
        Ok(Err(error)) => (false, error.to_string()),
        Err(_elapsed) => (
            false,
            format!(
                "the {subsystem} probe did not answer within {}s and was abandoned. \
                 Everything else in this report is unaffected.",
                timeout.as_secs_f32()
            ),
        ),
    }
}

// Interface name must match `wgaf_common::INTERFACE_NAME` (zbus requires a
// string literal here, so it can't reference the constant directly).
#[interface(name = "org.wgaf.Daemon1")]
impl Daemon {
    async fn ping(&self) -> String {
        "pong".to_string()
    }

    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// The kill switch: stop synthesizing input, now, and refuse everything
    /// further until [`Self::release`].
    ///
    /// **On `Daemon1` rather than `Input1`, and ungated.** Stopping is not an
    /// input operation — it controls the daemon — and no policy may take it
    /// away: `permissions.toml` restricts what wgaf may do *to the desktop*,
    /// whereas this is the user's brake on wgaf itself. A policy file that
    /// could deny someone their own emergency stop would be an unsafe design,
    /// and gating only `Release` would strand them on the other side of it.
    ///
    /// Callable by anything on the session bus, which is the point: the GNOME
    /// Shell Extension's keyboard shortcut calls exactly this method, and so
    /// does `wgaf stop`.
    async fn stop(&self) {
        self.input.stop().await;
    }

    /// Releases the kill switch. Ungated for the same reason [`Self::stop`] is.
    ///
    /// Deliberately a separate method rather than a toggle. The emergency key
    /// only ever stops; coming back is this, called once, deliberately, after
    /// the runaway script is dead.
    ///
    /// **Release, not resume:** it lifts the brake and nothing more. Whatever
    /// was interrupted stays interrupted — the daemon never held a queue to
    /// carry on with, and the caller it refused has long since given up.
    async fn release(&self) {
        self.input.release();
    }

    /// Whether wgaf currently holds a virtual input device — that is, whether
    /// it can type at this moment.
    ///
    /// **Exists for the GNOME Shell Extension's emergency key.** The extension
    /// registers that shortcut while this is `true` and gives it back when it
    /// goes `false`, so `Escape` belongs to the user's applications except
    /// during the seconds a script can actually drive the desktop. Registering
    /// it for the whole session instead took the key away from every
    /// application on the machine, including while the daemon was idle or not
    /// running at all.
    ///
    /// A property rather than a method so that it carries change
    /// notification: the extension must learn the device went away, not
    /// discover it by asking.
    ///
    /// Never creates a device as a side effect of being read — see
    /// [`crate::input::InputBackend::device_created`].
    #[zbus(property)]
    async fn input_device_active(&self) -> bool {
        self.input.device_created()
    }

    /// Reports which subsystems are usable right now and what policy is being
    /// enforced. See [`DaemonStatus`] for the two rules this method exists to
    /// uphold — it must change nothing, and it must expose nothing secret.
    ///
    /// **Ungated on purpose.** There is no `Capability` variant for it, so
    /// `permissions.toml` cannot switch it off: a transparency mechanism a
    /// policy could disable would defeat itself, and this exposes nothing a
    /// caller could not already learn by attempting the operations it
    /// describes. It reads state and probes availability, mutating nothing —
    /// putting it in the same read-only class as `ListWindows`/`ListApps`.
    async fn status(&self) -> DaemonStatusDict {
        // Each probe is deliberately the non-caching variant: they must
        // report what is true now, not what was true the first time the
        // subsystem was used. See each `probe_*` method's doc comment.
        //
        // Concurrently, and each under its own timeout. Both halves of that
        // matter. **The timeout is the fix for a real fault** — one subsystem
        // that never answers used to withhold every other section of this
        // report, including the input, permission and configuration ones that
        // had nothing to do with it, and `wgaf status` is precisely what
        // someone runs when something is already wrong. Running them together
        // then bounds the whole method at one timeout rather than three, and
        // is safe because these are independent read-only probes that mutate
        // nothing and do not depend on each other's results.
        let (
            (extension_available, extension_detail),
            (uinput_accessible, uinput_detail),
            (accessibility_available, accessibility_detail),
        ) = tokio::join!(
            probe(
                "GNOME Shell extension",
                PROBE_TIMEOUT,
                self.windows.probe_available()
            ),
            probe("uinput", PROBE_TIMEOUT, self.input.probe_device_access()),
            probe(
                "accessibility",
                PROBE_TIMEOUT,
                self.accessibility.probe_bus()
            ),
        );

        let status = DaemonStatus {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            daemon_bus_name: self.bus_name.clone(),
            daemon_pid: std::process::id(),
            daemon_uptime_seconds: self.started.elapsed().as_secs(),
            config_path: path_string(self.config_path.as_ref()),
            config_present: path_present(self.config_path.as_ref()),

            extension_available,
            extension_bus_name: self.windows.extension_bus_name().to_string(),
            extension_detail,

            uinput_accessible,
            uinput_detail,
            input_device_name: self.input.device_name().to_string(),
            input_device_created: self.input.device_created(),
            input_keyboard_layout_configured: self.input.layout_spec().to_string(),
            // Read without resolving: asking for status must never open a
            // Wayland connection, exactly as it must never create a uinput
            // device. Empty means "not resolved yet", not "no layout".
            input_keyboard_layout_resolved: self.input.resolved_layout_name().unwrap_or_default(),
            input_stopped: self.input.is_stopped(),

            accessibility_available,
            accessibility_detail,
            accessibility_connected: self.accessibility.is_connected(),

            permissions_path: path_string(self.permissions_path.as_ref()),
            permissions_present: path_present(self.permissions_path.as_ref()),
            permissions_restricted: self
                .permissions
                .restrictions()
                .into_iter()
                .map(|(capability, value)| format!("{capability}={value:?}"))
                .collect(),
            permissions_prompt_decisions: self
                .permissions
                .prompt_decisions()
                .into_iter()
                .map(|(capability, allowed)| {
                    let outcome = if allowed { "allowed" } else { "denied" };
                    format!("{capability}={outcome}")
                })
                .collect(),
        };

        status.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe timeout short enough to keep these tests instant.
    const INSTANTLY: Duration = Duration::from_millis(10);

    /// The fault this exists for: a probe that never answers must be reported,
    /// not awaited.
    #[tokio::test]
    async fn a_probe_that_never_answers_is_reported_rather_than_awaited() {
        let (available, detail) = probe(
            "test",
            INSTANTLY,
            std::future::pending::<Result<(), String>>(),
        )
        .await;

        assert!(!available, "a probe that never answered is not available");
        assert!(
            detail.contains("did not answer"),
            "the detail must say the probe hung rather than that it failed — \
             they are different faults with different fixes. Got: {detail}"
        );
        assert!(
            detail.contains("Everything else in this report is unaffected"),
            "the detail must tell the reader the rest of the report is still \
             good, because the whole point is that one subsystem no longer \
             withholds the others. Got: {detail}"
        );
    }

    /// A probe that answers with a failure keeps reporting *its own* reason.
    /// The timeout must not flatten every unavailable subsystem into "hung".
    #[tokio::test]
    async fn a_probe_that_fails_reports_its_own_reason() {
        let (available, detail) = probe(
            "test",
            INSTANTLY,
            std::future::ready(Err::<(), _>("no such device".to_string())),
        )
        .await;

        assert!(!available);
        assert_eq!(detail, "no such device");
    }

    #[tokio::test]
    async fn a_probe_that_succeeds_reports_no_detail() {
        let (available, detail) =
            probe("test", INSTANTLY, std::future::ready(Ok::<(), String>(()))).await;

        assert!(available);
        assert!(
            detail.is_empty(),
            "a working subsystem has nothing to explain"
        );
    }
}
