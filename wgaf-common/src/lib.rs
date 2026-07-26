//! D-Bus naming and shared DTOs used across `wgaf-daemon` and `wgaf-cli`
//! (and, for naming purposes only, the GNOME Shell Extension's D-Bus
//! contract in `extension/dbusInterface.js`, which `wgaf-daemon` is a client
//! of).
//!
//! The DTOs here are deliberately plain `serde` types with no `zvariant`
//! coupling: zvariant's `a{sv}` dict (de)serialization wraps every field in
//! a `Variant` marker, which is not interchangeable with plain JSON (a
//! `--json` CLI field would come out as `{"signature": "u", "value": 42}`
//! instead of `42`). `wgaf-daemon` defines its own private `a{sv}`-shaped
//! "wire" structs (see `wgaf-daemon/src/windows/wire.rs`) purely for the two
//! D-Bus hops (extension -> daemon, daemon -> CLI), and converts to/from
//! these DTOs at the edges. These DTOs are what `wgaf-cli` actually
//! serializes for `--json` output.

use serde::{Deserialize, Serialize};

pub mod dict;

// ---------------------------------------------------------------------------
// Daemon's own public D-Bus API (org.wgaf.Daemon1) — Phase 1.
// ---------------------------------------------------------------------------

/// Well-known session bus name the daemon registers on startup.
pub const BUS_NAME: &str = "org.wgaf.Daemon";

/// Object path the daemon's root interface is served at.
pub const OBJECT_PATH: &str = "/org/wgaf/Daemon";

/// Versioned interface name, following the `org.freedesktop.*1` convention
/// so a future `Daemon2` can be introduced without breaking existing clients.
pub const INTERFACE_NAME: &str = "org.wgaf.Daemon1";

// ---------------------------------------------------------------------------
// Daemon's own public window-management D-Bus API (org.wgaf.Windows1) —
// Phase 3. Served on the same bus name/connection as `org.wgaf.Daemon1`
// above, at a sibling object path.
// ---------------------------------------------------------------------------

/// Object path the daemon's window-management interface is served at.
pub const WINDOWS_OBJECT_PATH: &str = "/org/wgaf/Windows";

/// Versioned interface name for the daemon's own window-management API.
pub const WINDOWS_INTERFACE_NAME: &str = "org.wgaf.Windows1";

/// D-Bus error name returned by `org.wgaf.Windows1` methods when the given
/// window id does not exist. Distinct from
/// [`EXTENSION_ERROR_WINDOW_NOT_FOUND`] — this is the daemon's own,
/// stable, public error name; the daemon translates the extension's error
/// into this one rather than leaking the extension's error name to clients.
pub const WINDOWS_ERROR_WINDOW_NOT_FOUND: &str = "org.wgaf.Windows1.Error.WindowNotFound";

/// D-Bus error name returned by `org.wgaf.Windows1` methods when the GNOME
/// Shell Extension bridge is not available (not installed, not enabled, or
/// missing the expected versioned interface).
pub const WINDOWS_ERROR_EXTENSION_UNAVAILABLE: &str =
    "org.wgaf.Windows1.Error.ExtensionUnavailable";

// ---------------------------------------------------------------------------
// GNOME Shell Extension's D-Bus API (client-side naming) — the daemon is a
// `zbus` client of this interface. Canonical definition lives in
// `extension/dbusInterface.js`; keep these in sync with it.
// ---------------------------------------------------------------------------

/// Bus name the GNOME Shell Extension registers on the session bus.
pub const EXTENSION_BUS_NAME: &str = "org.gnome.Shell.Extensions.Wgaf";

/// Object path the extension exports its D-Bus interface at.
pub const EXTENSION_OBJECT_PATH: &str = "/org/gnome/Shell/Extensions/Wgaf";

/// Versioned interface name. The daemon must discover this via
/// `org.freedesktop.DBus.Introspectable.Introspect` on
/// [`EXTENSION_OBJECT_PATH`] rather than assuming it's present — see
/// `wgaf-daemon/src/windows/mod.rs`'s `check_extension_version` and the
/// versioning strategy documented in `extension/dbusInterface.js`.
pub const EXTENSION_INTERFACE_NAME: &str = "org.gnome.Shell.Extensions.Wgaf.V1";

/// D-Bus error name the extension returns when a `FocusWindow`/`MoveWindow`/
/// `ResizeWindow`/`CloseWindow` call is given an id that doesn't correspond
/// to any known window.
pub const EXTENSION_ERROR_WINDOW_NOT_FOUND: &str =
    "org.gnome.Shell.Extensions.Wgaf.Error.WindowNotFound";

// ---------------------------------------------------------------------------
// Shared DTOs
// ---------------------------------------------------------------------------
//
// Field names intentionally match extension/dbusInterface.js's
// `windowRecordToVariantDict`/`workspaceRecordToVariantDict` verbatim
// (snake_case, no renaming) so it's obvious at a glance that they carry the
// same data, even though the wire encoding between the two hops is handled
// by wgaf-daemon's private dict types, not these structs directly.

/// A single window, as reported by the GNOME Shell Extension's
/// `ListWindows`/`WindowCreated` and mirrored by the daemon's own
/// `org.wgaf.Windows1.ListWindows`.
///
/// `id` is Mutter's stable per-window sequence number (see
/// `extension/windows.js`), not an X11 XID — it is only meaningful for the
/// lifetime of the window, not persisted across restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRecord {
    pub id: u32,
    pub title: String,
    pub app_id: String,
    pub workspace: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub focused: bool,
    pub maximized: bool,
}

/// A single workspace, as reported by the GNOME Shell Extension's
/// `GetWorkspaces` and mirrored by the daemon's own
/// `org.wgaf.Windows1.GetWorkspaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub index: i32,
    pub active: bool,
    pub n_windows: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window() -> WindowRecord {
        WindowRecord {
            id: 42,
            title: "Terminal".to_string(),
            app_id: "org.gnome.Terminal".to_string(),
            workspace: 0,
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            focused: true,
            maximized: false,
        }
    }

    fn sample_workspace() -> WorkspaceRecord {
        WorkspaceRecord {
            index: 1,
            active: true,
            n_windows: 3,
        }
    }

    #[test]
    fn window_record_json_round_trips() {
        let record = sample_window();
        let json = serde_json::to_string(&record).expect("serialize");
        let back: WindowRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    #[test]
    fn window_record_json_uses_expected_field_names() {
        // Field names must match extension/dbusInterface.js's
        // windowRecordToVariantDict keys verbatim — this is the contract
        // wgaf-daemon's wire types rely on when decoding the extension's
        // `a{sv}` dicts, so a rename here would silently break interop.
        let value = serde_json::to_value(sample_window()).expect("serialize");
        let obj = value.as_object().expect("object");
        for key in [
            "id",
            "title",
            "app_id",
            "workspace",
            "x",
            "y",
            "width",
            "height",
            "focused",
            "maximized",
        ] {
            assert!(obj.contains_key(key), "missing expected field `{key}`");
        }
    }

    #[test]
    fn workspace_record_json_round_trips() {
        let record = sample_workspace();
        let json = serde_json::to_string(&record).expect("serialize");
        let back: WorkspaceRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    #[test]
    fn workspace_record_json_uses_expected_field_names() {
        let value = serde_json::to_value(sample_workspace()).expect("serialize");
        let obj = value.as_object().expect("object");
        for key in ["index", "active", "n_windows"] {
            assert!(obj.contains_key(key), "missing expected field `{key}`");
        }
    }
}
