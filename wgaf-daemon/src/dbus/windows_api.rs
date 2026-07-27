//! The daemon's own public window-management D-Bus API
//! (`org.wgaf.Windows1`). Thin delegation to [`crate::windows::WindowManager`]
//! — this module's only job is D-Bus marshaling (converting to/from the
//! `a{sv}` wire types in `wgaf_common::dict`) and translating
//! [`crate::windows::WindowsError`] into a stable, named D-Bus error
//! (`WindowsApiError`) rather than leaking the extension's own error names
//! or a raw `zbus::Error` to clients.

use std::sync::Arc;

use zbus::DBusError;
use zbus::interface;
use zbus::message::Header;

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};

use crate::permissions::{Capability, PermissionError, PermissionGate};
use crate::windows::{WindowManager, WindowsError};

/// D-Bus error names for `org.wgaf.Windows1`, matching
/// `wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND`/
/// `WINDOWS_ERROR_EXTENSION_UNAVAILABLE`/`WINDOWS_ERROR_PERMISSION_DENIED`
/// (asserted in this module's tests).
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.wgaf.Windows1.Error")]
enum WindowsApiError {
    /// Catch-all for D-Bus-level failures not otherwise translated below.
    #[zbus(error)]
    ZBus(zbus::Error),
    WindowNotFound(String),
    ExtensionUnavailable(String),
    /// The call was refused by `permissions.toml`'s policy (or the
    /// caller declined an interactive `Prompt`) — see `crate::permissions`.
    PermissionDenied(String),
}

impl From<WindowsError> for WindowsApiError {
    fn from(err: WindowsError) -> Self {
        match err {
            WindowsError::WindowNotFound(id) => {
                Self::WindowNotFound(format!("window {id} not found"))
            }
            WindowsError::ExtensionUnavailable { .. } => {
                Self::ExtensionUnavailable(err.to_string())
            }
            WindowsError::DBus(e) => Self::ZBus(e),
        }
    }
}

impl From<PermissionError> for WindowsApiError {
    fn from(err: PermissionError) -> Self {
        match err {
            PermissionError::Denied { .. } | PermissionError::DeniedByPrompt { .. } => {
                Self::PermissionDenied(err.to_string())
            }
            PermissionError::DBus(e) => Self::ZBus(e),
        }
    }
}

pub struct WindowsApi {
    manager: WindowManager,
    permissions: Arc<PermissionGate>,
}

impl WindowsApi {
    pub fn new(manager: WindowManager, permissions: Arc<PermissionGate>) -> Self {
        Self {
            manager,
            permissions,
        }
    }
}

// Interface name must match `wgaf_common::WINDOWS_INTERFACE_NAME` (zbus
// requires a string literal here, so it can't reference the constant
// directly) — see the existing convention in `dbus/mod.rs`.
#[interface(name = "org.wgaf.Windows1")]
impl WindowsApi {
    async fn list_windows(&self) -> Result<Vec<WindowRecordDict>, WindowsApiError> {
        let windows = self.manager.list_windows().await?;
        Ok(windows.into_iter().map(WindowRecordDict::from).collect())
    }

    async fn focus_window(
        &self,
        id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::FocusWindow, connection, &header)
            .await?;
        Ok(self.manager.focus_window(id).await?)
    }

    async fn move_window(
        &self,
        id: u32,
        x: i32,
        y: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::MoveWindow, connection, &header)
            .await?;
        Ok(self.manager.move_window(id, x, y).await?)
    }

    async fn resize_window(
        &self,
        id: u32,
        width: i32,
        height: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::ResizeWindow, connection, &header)
            .await?;
        Ok(self.manager.resize_window(id, width, height).await?)
    }

    async fn close_window(
        &self,
        id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::CloseWindow, connection, &header)
            .await?;
        Ok(self.manager.close_window(id).await?)
    }

    async fn get_workspaces(&self) -> Result<Vec<WorkspaceRecordDict>, WindowsApiError> {
        let workspaces = self.manager.get_workspaces().await?;
        Ok(workspaces
            .into_iter()
            .map(WorkspaceRecordDict::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_prefix_matches_wgaf_common_constants() {
        let not_found = WindowsApiError::WindowNotFound("window 1 not found".to_string());
        let unavailable = WindowsApiError::ExtensionUnavailable("unavailable".to_string());
        let denied = WindowsApiError::PermissionDenied("denied".to_string());
        assert_eq!(
            not_found.name().as_str(),
            wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND
        );
        assert_eq!(
            unavailable.name().as_str(),
            wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE
        );
        assert_eq!(
            denied.name().as_str(),
            wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED
        );
    }
}
