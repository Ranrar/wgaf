pub mod accessibility_api;
pub mod input_api;
pub mod windows_api;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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
        let (extension_available, extension_detail) = match self.windows.probe_available().await {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e.to_string()),
        };
        let (uinput_accessible, uinput_detail) = match self.input.probe_device_access().await {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e.to_string()),
        };
        let (accessibility_available, accessibility_detail) =
            match self.accessibility.probe_bus().await {
                Ok(()) => (true, String::new()),
                Err(e) => (false, e.to_string()),
            };

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
