//! `zbus` client proxy for the GNOME Shell Extension's window-management
//! D-Bus interface. This must match `extension/dbusInterface.js`'s
//! `DBUS_INTERFACE_XML` exactly — method names/arities/types are checked at
//! runtime by `zbus`, not the compiler, so keep the two in sync by hand.
//!
//! The `interface`/`default_service`/`default_path` attributes below must
//! be string literals (macro limitation), so they can't reference
//! `wgaf_common::EXTENSION_*` directly; a unit test in this module asserts
//! they stay in sync instead. `WindowManager::connect_to` (see `mod.rs`)
//! additionally allows overriding the destination/path at runtime, which
//! `wgaf_common::EXTENSION_BUS_NAME`/`EXTENSION_OBJECT_PATH` are passed
//! into in production — the attributes below only supply the fallback
//! defaults used by `ShellExtensionProxy::new`.

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};

#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions.Wgaf.V1",
    default_service = "org.gnome.Shell.Extensions.Wgaf",
    default_path = "/org/gnome/Shell/Extensions/Wgaf"
)]
pub(crate) trait ShellExtension {
    /// Enumerate all windows known to Mutter.
    fn list_windows(&self) -> zbus::Result<Vec<WindowRecordDict>>;

    /// Focus (activate) the window with the given id.
    fn focus_window(&self, id: u32) -> zbus::Result<()>;

    /// Move the window with the given id so its top-left corner is at
    /// `(x, y)`.
    fn move_window(&self, id: u32, x: i32, y: i32) -> zbus::Result<()>;

    /// Resize the window with the given id to `(width, height)`.
    fn resize_window(&self, id: u32, width: i32, height: i32) -> zbus::Result<()>;

    /// Close (request deletion of) the window with the given id.
    fn close_window(&self, id: u32) -> zbus::Result<()>;

    /// Enumerate all workspaces.
    fn get_workspaces(&self) -> zbus::Result<Vec<WorkspaceRecordDict>>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn proxy_attributes_match_wgaf_common_constants() {
        assert_eq!(
            wgaf_common::EXTENSION_INTERFACE_NAME,
            "org.gnome.Shell.Extensions.Wgaf.V1"
        );
        assert_eq!(
            wgaf_common::EXTENSION_BUS_NAME,
            "org.gnome.Shell.Extensions.Wgaf"
        );
        assert_eq!(
            wgaf_common::EXTENSION_OBJECT_PATH,
            "/org/gnome/Shell/Extensions/Wgaf"
        );
    }
}
