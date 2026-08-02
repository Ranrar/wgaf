//! Daemon-side window management: a `zbus` client of the GNOME Shell
//! Extension's window bridge (`org.gnome.Shell.Extensions.Wgaf.V1`), with
//! extension-availability discovery and translation of the extension's
//! D-Bus errors into daemon-level errors.
//! Exposed to the CLI via the daemon's own `org.wgaf.Windows1` interface,
//! see `crate::dbus::windows_api`.

pub mod display_config;
mod proxy;

use thiserror::Error;
use tokio::sync::{OnceCell, RwLock};
use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};

use display_config::{DisplayConfig, DisplayConfigError, MonitorLayout};
use proxy::ShellExtensionProxy;

/// Every method this daemon calls on the extension's interface.
///
/// Checked against the extension's introspection XML at availability time, per
/// [ADR-0002](../../../.vscode/Documentation/adr/adr-0002-extension-interface-versioning.md):
/// because additive changes stay within `V1` rather than bumping to `V2`, the
/// interface *name* cannot tell a current extension from an outdated one, and
/// only the member list can.
///
/// **This list must equal the proxy's method set.** `proxy.rs`'s drift test
/// asserts exactly that — a list that silently rots is worse than no list,
/// because it converts a clear "your extension needs updating" back into an
/// unexplained `UnknownMethod`.
const REQUIRED_EXTENSION_METHODS: &[&str] = &[
    "ListWindows",
    "FocusWindow",
    "MoveWindow",
    "ResizeWindow",
    "CloseWindow",
    "GetWorkspaces",
    "WarpPointer",
    "GetPointer",
];

/// Errors surfaced by the daemon's window-management layer.
#[derive(Debug, Error)]
pub enum WindowsError {
    /// The GNOME Shell Extension is not installed, not enabled, or its
    /// `org.gnome.Shell.Extensions.Wgaf.V1` interface is missing/outdated —
    /// discovered via `Introspectable.Introspect`, never inferred from a
    /// raw method-call failure (a stale/timed-out call would be
    /// indistinguishable from many other faults).
    #[error(
        "GNOME Shell Extension bridge unavailable: {reason} (bus name `{bus_name}`, object \
         path `{object_path}`, expected interface `{interface}`) — is the wgaf GNOME Shell \
         Extension installed and enabled?"
    )]
    ExtensionUnavailable {
        reason: String,
        bus_name: String,
        object_path: String,
        interface: String,
    },

    /// The extension reported that no window with this id exists.
    #[error("window {0} not found")]
    WindowNotFound(u32),

    /// The requested pointer position is not on any monitor.
    ///
    /// Raised **before** asking the compositor to move the pointer, not after.
    /// Mutter silently clamps an off-screen coordinate — no error, no signal —
    /// so a caller would otherwise be told the move succeeded while the pointer
    /// sat somewhere they never asked for, and a subsequent click would land
    /// there. Measured on GNOME 50.1, 2026-08-02.
    #[error("({x}, {y}) is not on any monitor — the current layout is: {layout}")]
    OutOfBounds { x: i32, y: i32, layout: String },

    /// The monitor layout could not be read, so no coordinate can be validated.
    ///
    /// Distinct from [`Self::ExtensionUnavailable`] on purpose: a session can
    /// have a perfectly working wgaf extension and still fail here, and telling
    /// the user to check their extension would send them the wrong way.
    #[error("{0}")]
    DisplayConfig(#[from] DisplayConfigError),

    /// Any other D-Bus-level failure talking to the extension.
    #[error("D-Bus error talking to the GNOME Shell Extension: {0}")]
    DBus(#[from] zbus::Error),
}

impl WindowsError {
    fn extension_unavailable(
        reason: impl Into<String>,
        bus_name: &str,
        object_path: &str,
        interface: &str,
    ) -> Self {
        Self::ExtensionUnavailable {
            reason: reason.into(),
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
            interface: interface.to_string(),
        }
    }
}

/// Wraps the GNOME Shell Extension's D-Bus interface: version/availability
/// discovery, window-management delegation, and error translation. One
/// instance is created at daemon startup and served for the lifetime of the
/// `org.wgaf.Windows1` interface.
pub struct WindowManager {
    connection: zbus::Connection,
    proxy: ShellExtensionProxy<'static>,
    extension_bus_name: String,
    extension_object_path: String,
    extension_interface_name: String,
    /// Caches a *successful* version check only. Left unset on failure so a
    /// later call retries — this lets the daemon recover automatically if
    /// the user enables the extension after the daemon has started, rather
    /// than requiring a daemon restart.
    verified: OnceCell<()>,
    /// Mutter's display configuration, for bounds-checking pointer
    /// coordinates. Built lazily for the same reason `verified` is: a daemon
    /// started before its session is ready must recover without a restart.
    display_config: OnceCell<DisplayConfig>,
    /// Bus name to read the monitor layout from. Configurable so tests can
    /// substitute a stub — the real Mutter already owns the default on any
    /// live session, so nothing else could take it.
    display_config_bus_name: String,
    /// The monitor layout, cached because the pointer path wants it on every
    /// call and a D-Bus round trip per warp would be waste.
    ///
    /// **Refresh policy: cache, then re-query before rejecting.** The daemon
    /// runs under a systemd user unit for weeks, and layouts change inside that
    /// window — a projector is plugged in, a panel is rotated, a laptop is
    /// undocked. A stale layout can fail in two directions, and they are not
    /// equally bad: wrongly *accepting* a coordinate costs a clamp Mutter would
    /// have done anyway, while wrongly *rejecting* one breaks automation that
    /// is entirely correct, which is precisely the failure `OutOfBounds` exists
    /// to prevent, arriving by the back door. So a miss re-reads and only then
    /// rejects, and the happy path stays a cache hit.
    ///
    /// Subscribing to `MonitorsChanged` would also work and was considered; it
    /// costs a background subscription for an outcome this achieves with none.
    monitor_layout: RwLock<Option<MonitorLayout>>,
}

impl WindowManager {
    /// Connects to the extension at an explicit bus name/object
    /// path/interface. `main.rs` always passes `wgaf_common`'s
    /// `EXTENSION_OBJECT_PATH`/`EXTENSION_INTERFACE_NAME` constants and a
    /// configurable `extension_bus_name` (defaulting to
    /// `wgaf_common::EXTENSION_BUS_NAME`); tests use this same entry point
    /// to point at a stub service on a private, unique bus name instead —
    /// see `wgaf-daemon/tests/windows_stub.rs`.
    pub async fn connect_to(
        connection: zbus::Connection,
        extension_bus_name: &str,
        extension_object_path: &str,
        extension_interface_name: &str,
        display_config_bus_name: &str,
    ) -> zbus::Result<Self> {
        let proxy = ShellExtensionProxy::builder(&connection)
            .destination(extension_bus_name.to_string())?
            .path(extension_object_path.to_string())?
            .build()
            .await?;

        Ok(Self {
            connection,
            proxy,
            extension_bus_name: extension_bus_name.to_string(),
            extension_object_path: extension_object_path.to_string(),
            extension_interface_name: extension_interface_name.to_string(),
            verified: OnceCell::new(),
            display_config: OnceCell::new(),
            display_config_bus_name: display_config_bus_name.to_string(),
            monitor_layout: RwLock::new(None),
        })
    }

    /// Confirms the extension is present on the bus and exposes the
    /// expected versioned interface, via
    /// `org.freedesktop.DBus.Introspectable.Introspect` — per the
    /// versioning strategy documented in `extension/dbusInterface.js`.
    /// Cached only on success (see `verified`'s doc comment).
    async fn ensure_extension_available(&self) -> Result<(), WindowsError> {
        if self.verified.initialized() {
            return Ok(());
        }
        self.verified
            .get_or_try_init(|| self.check_extension_version())
            .await?;
        Ok(())
    }

    async fn check_extension_version(&self) -> Result<(), WindowsError> {
        let unavailable = |reason: String| {
            WindowsError::extension_unavailable(
                reason,
                &self.extension_bus_name,
                &self.extension_object_path,
                &self.extension_interface_name,
            )
        };

        // First distinguish "nobody owns this bus name" (extension not
        // installed/enabled) from "bus name exists but wrong/missing
        // interface" (extension present, outdated) — these are different
        // problems and deserve different diagnostics.
        let dbus_proxy = zbus::fdo::DBusProxy::new(&self.connection).await?;
        let has_owner = dbus_proxy
            .name_has_owner(self.extension_bus_name.as_str().try_into().map_err(|_| {
                unavailable(format!(
                    "`{}` is not a valid D-Bus bus name",
                    self.extension_bus_name
                ))
            })?)
            .await
            .map_err(zbus::Error::from)?;
        if !has_owner {
            return Err(unavailable(
                "no owner for the extension's D-Bus name — the wgaf GNOME Shell Extension is \
                 not installed or not enabled"
                    .to_string(),
            ));
        }

        let introspectable = zbus::fdo::IntrospectableProxy::builder(&self.connection)
            .destination(self.extension_bus_name.as_str())?
            .path(self.extension_object_path.as_str())?
            .build()
            .await?;

        let xml = introspectable
            .introspect()
            .await
            .map_err(|e| unavailable(format!("introspection failed: {e}")))?;

        // A targeted substring check rather than full XML parsing: the
        // interface name is a strict, namespaced literal
        // (`org.gnome.Shell.Extensions.Wgaf.V1`), so a false-positive
        // substring match is not a realistic concern, and this avoids
        // pulling in an XML parsing dependency for one check.
        let needle = format!("interface name=\"{}\"", self.extension_interface_name);
        if !xml.contains(&needle) {
            return Err(unavailable(format!(
                "extension is running but does not expose interface `{}` — it may need to be \
                 upgraded",
                self.extension_interface_name
            )));
        }

        // Per ADR-0002, the interface name alone is not enough. Methods are
        // added within `V1` rather than bumping to `V2`, so an older extension
        // exposes the right interface name and is still missing methods a newer
        // daemon calls. Without this, that surfaces as `UnknownMethod` from a
        // single command while everything else works — a confusing failure that
        // does not point at the real fix, which is updating the extension.
        if let Some(missing) = REQUIRED_EXTENSION_METHODS
            .iter()
            .find(|method| !xml.contains(&format!("method name=\"{method}\"")))
        {
            return Err(unavailable(format!(
                "extension is running but its `{}` interface has no `{missing}` method — the \
                 installed wgaf GNOME Shell Extension is older than this daemon and needs \
                 updating",
                self.extension_interface_name
            )));
        }

        Ok(())
    }

    /// Freshly checks whether the extension bridge is reachable, for
    /// `org.wgaf.Daemon1.Status`.
    ///
    /// Deliberately bypasses [`Self::ensure_extension_available`]'s
    /// `verified` cache. That cache stores success permanently — correct for
    /// method calls, where re-introspecting on every request would be waste,
    /// but wrong for a status query: once the extension had been used
    /// successfully, status would keep reporting "available" forever, even
    /// after the user disabled the extension. Reporting stale health is worse
    /// than not reporting it.
    ///
    /// Populating the cache on a *successful* fresh check would be harmless,
    /// but this does not bother: the check is two D-Bus round trips, and
    /// keeping the probe free of side effects entirely is easier to keep true
    /// than a rule about which side effects are acceptable.
    pub async fn probe_available(&self) -> Result<(), WindowsError> {
        self.check_extension_version().await
    }

    /// The bus name this manager expects the extension to own.
    pub fn extension_bus_name(&self) -> &str {
        &self.extension_bus_name
    }

    pub async fn list_windows(&self) -> Result<Vec<WindowRecord>, WindowsError> {
        self.ensure_extension_available().await?;
        let dicts = self.proxy.list_windows().await?;
        Ok(dicts.into_iter().map(WindowRecordDict::into).collect())
    }

    pub async fn focus_window(&self, id: u32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .focus_window(id)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    pub async fn move_window(&self, id: u32, x: i32, y: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .move_window(id, x, y)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    pub async fn resize_window(
        &self,
        id: u32,
        width: i32,
        height: i32,
    ) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .resize_window(id, width, height)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    pub async fn close_window(&self, id: u32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .close_window(id)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    pub async fn get_workspaces(&self) -> Result<Vec<WorkspaceRecord>, WindowsError> {
        self.ensure_extension_available().await?;
        let dicts = self.proxy.get_workspaces().await?;
        Ok(dicts.into_iter().map(WorkspaceRecordDict::into).collect())
    }

    /// Mutter's display-configuration client, built on first use.
    ///
    /// Deliberately **not** behind `ensure_extension_available`: a session with
    /// no wgaf extension still has a monitor layout, and conflating the two
    /// would make the daemon unable to report one without the other.
    async fn display_config(&self) -> Result<&DisplayConfig, WindowsError> {
        self.display_config
            .get_or_try_init(|| {
                DisplayConfig::connect(self.connection.clone(), &self.display_config_bus_name)
            })
            .await
            .map_err(WindowsError::from)
    }

    /// The current monitor layout, reading it from Mutter if it is not cached.
    pub async fn monitor_layout(&self) -> Result<MonitorLayout, WindowsError> {
        if let Some(layout) = self.monitor_layout.read().await.clone() {
            return Ok(layout);
        }
        self.refresh_monitor_layout().await
    }

    /// Re-reads the layout from Mutter and replaces the cache.
    async fn refresh_monitor_layout(&self) -> Result<MonitorLayout, WindowsError> {
        let layout = self.display_config().await?.read().await?;
        *self.monitor_layout.write().await = Some(layout.clone());
        Ok(layout)
    }

    /// The pointer's current position in global logical pixels.
    pub async fn get_pointer(&self) -> Result<(i32, i32), WindowsError> {
        self.ensure_extension_available().await?;
        Ok(self.proxy.get_pointer().await?)
    }

    /// Moves the pointer to an absolute position, returning where it landed.
    ///
    /// The coordinate is validated against the monitor layout **first**, and an
    /// off-screen one is refused rather than passed on. This is not
    /// belt-and-braces: Mutter clamps silently, so handing it a bad coordinate
    /// would produce a successful-looking move to somewhere the caller never
    /// asked for — and the caller's next action is typically a click.
    ///
    /// Clamping ourselves was considered and rejected for the same reason: an
    /// automation tool that quietly aims somewhere else is worse than one that
    /// refuses, because the refusal is visible and the misfire is not.
    pub async fn warp_pointer(&self, x: i32, y: i32) -> Result<(i32, i32), WindowsError> {
        self.ensure_extension_available().await?;

        if !self.monitor_layout().await?.contains(x, y) {
            // The cache may simply be old — a monitor plugged in since the last
            // read makes a valid coordinate look invalid. Re-read before
            // refusing, so a stale cache costs a round trip rather than a wrong
            // rejection. Only if the fresh layout also excludes the point is it
            // genuinely off-screen.
            let fresh = self.refresh_monitor_layout().await?;
            if !fresh.contains(x, y) {
                return Err(WindowsError::OutOfBounds {
                    x,
                    y,
                    layout: fresh.describe(),
                });
            }
        }

        Ok(self.proxy.warp_pointer(x, y).await?)
    }
}

/// Maps a D-Bus method-error reply carrying the extension's
/// `WindowNotFound` error name (`wgaf_common::EXTENSION_ERROR_WINDOW_NOT_FOUND`)
/// to `WindowsError::WindowNotFound`, using the `id` already known from the
/// call site rather than parsing the error's free-text description.
fn translate_window_error(err: zbus::Error, id: u32) -> WindowsError {
    if let zbus::Error::MethodError(name, _, _) = &err
        && name.as_str() == wgaf_common::EXTENSION_ERROR_WINDOW_NOT_FOUND
    {
        return WindowsError::WindowNotFound(id);
    }
    WindowsError::from(err)
}
