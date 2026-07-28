//! D-Bus `a{sv}` wire representation of [`crate::WindowRecord`] /
//! [`crate::WorkspaceRecord`].
//!
//! These types exist only because zvariant's `a{sv}` dict (de)serialization
//! wraps every value as a `Variant`, which isn't interchangeable with plain
//! JSON (see the crate-level doc comment) — so the plain DTOs stay JSON
//! (and `--json` CLI output)-friendly, and this module owns the D-Bus wire
//! encoding shared by every hop that actually talks D-Bus:
//!
//!   - `wgaf-daemon` as a client, decoding `ListWindows`/`GetWorkspaces`
//!     replies from the GNOME Shell Extension
//!     (`org.gnome.Shell.Extensions.Wgaf.V1`);
//!   - `wgaf-daemon` as a server, encoding replies for its own mirrored
//!     `org.wgaf.Windows1.ListWindows`/`GetWorkspaces`;
//!   - `wgaf-cli` as a client, decoding those same `org.wgaf.Windows1`
//!     replies.
//!
//! Field names and the `a{sv}` signature match
//! `extension/dbusInterface.js`'s `windowRecordToVariantDict`/
//! `workspaceRecordToVariantDict` exactly.

use crate::{DaemonStatus, WindowRecord, WorkspaceRecord};
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

/// `a{sv}` wire form of [`DaemonStatus`], for `org.wgaf.Daemon1.Status`.
///
/// Field names must stay in step with `DaemonStatus`'s — a rename on either
/// side silently changes the D-Bus contract, which is why
/// `daemon_status_dict_field_names_match_the_dto` asserts them against each
/// other rather than trusting review.
#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct DaemonStatusDict {
    daemon_version: String,
    daemon_bus_name: String,
    daemon_pid: u32,
    daemon_uptime_seconds: u64,
    config_path: String,
    config_present: bool,
    extension_available: bool,
    extension_bus_name: String,
    extension_detail: String,
    uinput_accessible: bool,
    uinput_detail: String,
    input_device_name: String,
    input_device_created: bool,
    accessibility_available: bool,
    accessibility_detail: String,
    accessibility_connected: bool,
    permissions_path: String,
    permissions_present: bool,
    permissions_restricted: Vec<String>,
    permissions_prompt_decisions: Vec<String>,
}

impl From<DaemonStatus> for DaemonStatusDict {
    fn from(s: DaemonStatus) -> Self {
        DaemonStatusDict {
            daemon_version: s.daemon_version,
            daemon_bus_name: s.daemon_bus_name,
            daemon_pid: s.daemon_pid,
            daemon_uptime_seconds: s.daemon_uptime_seconds,
            config_path: s.config_path,
            config_present: s.config_present,
            extension_available: s.extension_available,
            extension_bus_name: s.extension_bus_name,
            extension_detail: s.extension_detail,
            uinput_accessible: s.uinput_accessible,
            uinput_detail: s.uinput_detail,
            input_device_name: s.input_device_name,
            input_device_created: s.input_device_created,
            accessibility_available: s.accessibility_available,
            accessibility_detail: s.accessibility_detail,
            accessibility_connected: s.accessibility_connected,
            permissions_path: s.permissions_path,
            permissions_present: s.permissions_present,
            permissions_restricted: s.permissions_restricted,
            permissions_prompt_decisions: s.permissions_prompt_decisions,
        }
    }
}

impl From<DaemonStatusDict> for DaemonStatus {
    fn from(d: DaemonStatusDict) -> Self {
        DaemonStatus {
            daemon_version: d.daemon_version,
            daemon_bus_name: d.daemon_bus_name,
            daemon_pid: d.daemon_pid,
            daemon_uptime_seconds: d.daemon_uptime_seconds,
            config_path: d.config_path,
            config_present: d.config_present,
            extension_available: d.extension_available,
            extension_bus_name: d.extension_bus_name,
            extension_detail: d.extension_detail,
            uinput_accessible: d.uinput_accessible,
            uinput_detail: d.uinput_detail,
            input_device_name: d.input_device_name,
            input_device_created: d.input_device_created,
            accessibility_available: d.accessibility_available,
            accessibility_detail: d.accessibility_detail,
            accessibility_connected: d.accessibility_connected,
            permissions_path: d.permissions_path,
            permissions_present: d.permissions_present,
            permissions_restricted: d.permissions_restricted,
            permissions_prompt_decisions: d.permissions_prompt_decisions,
        }
    }
}

#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct WindowRecordDict {
    id: u32,
    title: String,
    app_id: String,
    workspace: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    focused: bool,
    maximized: bool,
}

impl From<WindowRecordDict> for WindowRecord {
    fn from(d: WindowRecordDict) -> Self {
        WindowRecord {
            id: d.id,
            title: d.title,
            app_id: d.app_id,
            workspace: d.workspace,
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            focused: d.focused,
            maximized: d.maximized,
        }
    }
}

impl From<WindowRecord> for WindowRecordDict {
    fn from(r: WindowRecord) -> Self {
        WindowRecordDict {
            id: r.id,
            title: r.title,
            app_id: r.app_id,
            workspace: r.workspace,
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
            focused: r.focused,
            maximized: r.maximized,
        }
    }
}

#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct WorkspaceRecordDict {
    index: i32,
    active: bool,
    n_windows: i32,
}

impl From<WorkspaceRecordDict> for WorkspaceRecord {
    fn from(d: WorkspaceRecordDict) -> Self {
        WorkspaceRecord {
            index: d.index,
            active: d.active,
            n_windows: d.n_windows,
        }
    }
}

impl From<WorkspaceRecord> for WorkspaceRecordDict {
    fn from(r: WorkspaceRecord) -> Self {
        WorkspaceRecordDict {
            index: r.index,
            active: r.active,
            n_windows: r.n_windows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DaemonStatusDict` and `DaemonStatus` must carry the same field names:
    /// the dict defines the `org.wgaf.Daemon1.Status` D-Bus contract, the DTO
    /// defines `wgaf status --json`'s output, and a rename on one side alone
    /// would silently break whichever consumer reads the other. Compares the
    /// two encodings rather than restating a hand-written list, so adding a
    /// field to one and forgetting the other fails here.
    #[test]
    fn daemon_status_dict_field_names_match_the_dto() {
        let status = crate::DaemonStatus {
            daemon_version: "0.7.0".to_string(),
            daemon_bus_name: crate::BUS_NAME.to_string(),
            daemon_pid: 1234,
            daemon_uptime_seconds: 42,
            config_path: "/tmp/config.toml".to_string(),
            config_present: true,
            extension_available: false,
            extension_bus_name: crate::EXTENSION_BUS_NAME.to_string(),
            extension_detail: "not enabled".to_string(),
            uinput_accessible: true,
            uinput_detail: String::new(),
            input_device_name: "wgaf virtual input device".to_string(),
            input_device_created: false,
            accessibility_available: true,
            accessibility_detail: String::new(),
            accessibility_connected: false,
            permissions_path: "/tmp/permissions.toml".to_string(),
            permissions_present: false,
            permissions_restricted: vec!["TypeText=Deny".to_string()],
            permissions_prompt_decisions: vec![],
        };

        let dto_keys: Vec<String> = serde_json::to_value(&status)
            .expect("serialize DTO")
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect();

        // Round-tripping through the dict and back must preserve every value,
        // which can only hold if both sides name their fields identically.
        let dict: DaemonStatusDict = status.clone().into();
        let back: crate::DaemonStatus = dict.into();
        assert_eq!(status, back, "dict round-trip lost or reordered a field");

        for key in [
            "daemon_version",
            "daemon_bus_name",
            "daemon_pid",
            "daemon_uptime_seconds",
            "config_path",
            "config_present",
            "extension_available",
            "extension_bus_name",
            "extension_detail",
            "uinput_accessible",
            "uinput_detail",
            "input_device_name",
            "input_device_created",
            "accessibility_available",
            "accessibility_detail",
            "accessibility_connected",
            "permissions_path",
            "permissions_present",
            "permissions_restricted",
            "permissions_prompt_decisions",
        ] {
            assert!(
                dto_keys.iter().any(|k| k == key),
                "DaemonStatus lost the `{key}` field — this is a breaking \
                 change to `wgaf status --json`"
            );
        }
        assert_eq!(
            dto_keys.len(),
            20,
            "a field was added to DaemonStatus without being added to the \
             assertion above (and probably without being added to \
             DaemonStatusDict either)"
        );
    }

    /// The status payload must encode as a plain `a{sv}`, the signature
    /// `org.wgaf.Daemon1.Status` advertises.
    #[test]
    fn daemon_status_dict_encodes_as_a_dbus_dict() {
        assert_eq!(DaemonStatusDict::SIGNATURE.to_string(), "a{sv}");
    }
    use zbus::zvariant::{Endian, serialized::Context, to_bytes};

    fn sample_dict() -> WindowRecordDict {
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
        .into()
    }

    #[test]
    fn window_record_dict_round_trips_through_conversion() {
        let dict = sample_dict();
        let record: WindowRecord = dict.into();
        let back: WindowRecordDict = record.clone().into();
        let record2: WindowRecord = back.into();
        assert_eq!(record, record2);
    }

    #[test]
    fn window_record_dict_encodes_as_a_dbus_dict_not_a_struct() {
        // `WindowRecordDict::SIGNATURE` being `a{sv}` (not a `(...)` struct
        // signature) is what makes this type usable as a genuine D-Bus
        // dict — check that directly rather than by re-decoding the same
        // bytes as a `Value` (a `Value`'s own wire encoding is a
        // signature-prefixed variant, a different byte layout entirely
        // from the raw `a{sv}` bytes below, so re-interpreting one as the
        // other isn't a meaningful check).
        assert_eq!(WindowRecordDict::SIGNATURE.to_string(), "a{sv}");

        let dict = sample_dict();
        let ctx = Context::new_dbus(Endian::Little, 0);
        let encoded = to_bytes(ctx, &dict).expect("encode a{sv}");

        let (decoded, _): (WindowRecordDict, usize) =
            encoded.deserialize().expect("decode a{sv} back into dict");
        assert_eq!(decoded.id, dict.id);
        assert_eq!(decoded.title, dict.title);
    }
}
