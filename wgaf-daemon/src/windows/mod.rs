//! Daemon-side window management: a `zbus` client of the GNOME Shell
//! Extension's window bridge (`org.gnome.Shell.Extensions.Wgaf.V1`), with
//! extension-availability discovery (the Phase 2 leftover TODO) and
//! translation of the extension's D-Bus errors into daemon-level errors.
//! Exposed to the CLI via the daemon's own `org.wgaf.Windows1` interface,
//! see `crate::dbus::windows_api`.

mod proxy;

use thiserror::Error;
use tokio::sync::OnceCell;
use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};

use proxy::ShellExtensionProxy;

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
        })
    }

    /// Confirms the extension is present on the bus and exposes the
    /// expected versioned interface, via
    /// `org.freedesktop.DBus.Introspectable.Introspect` — per the Phase 2
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
        if xml.contains(&needle) {
            Ok(())
        } else {
            Err(unavailable(format!(
                "extension is running but does not expose interface `{}` — it may need to be \
                 upgraded",
                self.extension_interface_name
            )))
        }
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
