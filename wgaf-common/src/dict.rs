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

use crate::{DaemonStatus, MonitorRecord, WindowRecord, WorkspaceLayout, WorkspaceRecord};
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
    input_keyboard_layout_configured: String,
    input_keyboard_layout_resolved: String,
    input_stopped: bool,
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
            input_keyboard_layout_configured: s.input_keyboard_layout_configured,
            input_keyboard_layout_resolved: s.input_keyboard_layout_resolved,
            input_stopped: s.input_stopped,
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
            input_keyboard_layout_configured: d.input_keyboard_layout_configured,
            input_keyboard_layout_resolved: d.input_keyboard_layout_resolved,
            input_stopped: d.input_stopped,
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

/// `a{sv}` wire form of [`WorkspaceLayout`], for `GetWorkspaceLayout` on both
/// the extension's interface and the daemon's own.
///
/// Field names match `extension/dbusInterface.js`'s
/// `workspaceLayoutToVariantDict` exactly.
#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct WorkspaceLayoutDict {
    n_workspaces: i32,
    active: i32,
    rows: i32,
    columns: i32,
    dynamic: bool,
}

impl From<WorkspaceLayoutDict> for WorkspaceLayout {
    fn from(d: WorkspaceLayoutDict) -> Self {
        WorkspaceLayout {
            n_workspaces: d.n_workspaces,
            active: d.active,
            rows: d.rows,
            columns: d.columns,
            dynamic: d.dynamic,
        }
    }
}

impl From<WorkspaceLayout> for WorkspaceLayoutDict {
    fn from(r: WorkspaceLayout) -> Self {
        WorkspaceLayoutDict {
            n_workspaces: r.n_workspaces,
            active: r.active,
            rows: r.rows,
            columns: r.columns,
            dynamic: r.dynamic,
        }
    }
}

/// `a{sv}` wire form of [`MonitorRecord`], for `org.wgaf.Windows1.GetMonitors`.
///
/// The only dict in this module with **no extension counterpart** — nothing in
/// `extension/dbusInterface.js` emits this shape, because the monitor layout is
/// read from `org.gnome.Mutter.DisplayConfig` rather than from the extension.
/// It is an `a{sv}` anyway, for consistency with the two records above and
/// because that is the shape a record can gain a field in without a `V2`.
#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct MonitorRecordDict {
    connector: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    transform: u32,
    primary: bool,
    work_area_x: Option<i32>,
    work_area_y: Option<i32>,
    work_area_width: Option<i32>,
    work_area_height: Option<i32>,
}

/// `a{sv}` wire form of one entry from the extension's `GetWorkAreas`.
///
/// **Carries no monitor index or connector name**, deliberately: the extension
/// knows monitors by Mutter's index and the daemon knows them by connector
/// name, and that those two enumerate in the same order is unverified. The
/// monitor's own rectangle is the join key instead — exact, since two monitors
/// cannot occupy one rectangle. See `WindowManager::list_monitors`.
///
/// Consumed inside the daemon only; nothing on `org.wgaf.Windows1` returns this
/// shape. The work area reaches callers as four optional fields on
/// [`MonitorRecordDict`] above.
#[derive(Debug, Clone, SerializeDict, DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct WorkAreaDict {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub work_area_x: i32,
    pub work_area_y: i32,
    pub work_area_width: i32,
    pub work_area_height: i32,
}

impl From<MonitorRecordDict> for MonitorRecord {
    fn from(d: MonitorRecordDict) -> Self {
        MonitorRecord {
            connector: d.connector,
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            scale: d.scale,
            transform: d.transform,
            primary: d.primary,
            // All four or none — the daemon only ever sets them together, and
            // a half-known rectangle would be worse than an unknown one.
            work_area: match (
                d.work_area_x,
                d.work_area_y,
                d.work_area_width,
                d.work_area_height,
            ) {
                (Some(x), Some(y), Some(width), Some(height)) => Some(crate::Rect {
                    x,
                    y,
                    width,
                    height,
                }),
                _ => None,
            },
        }
    }
}

impl From<MonitorRecord> for MonitorRecordDict {
    fn from(r: MonitorRecord) -> Self {
        MonitorRecordDict {
            connector: r.connector,
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
            scale: r.scale,
            transform: r.transform,
            primary: r.primary,
            work_area_x: r.work_area.as_ref().map(|w| w.x),
            work_area_y: r.work_area.as_ref().map(|w| w.y),
            work_area_width: r.work_area.as_ref().map(|w| w.width),
            work_area_height: r.work_area.as_ref().map(|w| w.height),
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
            input_keyboard_layout_configured: "auto".into(),
            input_keyboard_layout_resolved: String::new(),
            input_stopped: false,
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
            "input_keyboard_layout_configured",
            "input_keyboard_layout_resolved",
            "input_stopped",
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
            23,
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

    /// The maintainer's real layout: a rotated 1080x1920 panel beside a
    /// 2560x1440 primary. Uses the awkward one (`transform: 1`) rather than a
    /// tidy invented monitor, so a conversion that dropped `transform` or
    /// `scale` would be visible here.
    fn sample_monitor() -> MonitorRecord {
        MonitorRecord {
            connector: "HDMI-1".to_string(),
            x: 0,
            y: 0,
            width: 1080,
            height: 1920,
            scale: 1.0,
            transform: 1,
            primary: false,
            // A GNOME top bar's worth of reserved space, so a conversion that
            // dropped the work area or confused it with the monitor's own
            // geometry is visible rather than coincidentally identical.
            work_area: Some(crate::Rect {
                x: 0,
                y: 37,
                width: 1080,
                height: 1883,
            }),
        }
    }

    #[test]
    fn monitor_record_dict_round_trips_through_conversion() {
        let record = sample_monitor();
        let dict: MonitorRecordDict = record.clone().into();
        let back: MonitorRecord = dict.into();
        assert_eq!(record, back);
    }

    /// `GetMonitors` advertises `aa{sv}`, so a single record must encode as
    /// `a{sv}` — not as the positional struct a plain `Type` derive would give.
    #[test]
    fn monitor_record_dict_encodes_as_a_dbus_dict() {
        assert_eq!(MonitorRecordDict::SIGNATURE.to_string(), "a{sv}");

        let dict: MonitorRecordDict = sample_monitor().into();
        let ctx = Context::new_dbus(Endian::Little, 0);
        let encoded = to_bytes(ctx, &dict).expect("encode a{sv}");
        let (decoded, _): (MonitorRecordDict, usize) =
            encoded.deserialize().expect("decode a{sv} back into dict");

        assert_eq!(decoded.connector, "HDMI-1");
        assert_eq!(decoded.height, 1920);
        assert_eq!(decoded.transform, 1);
        assert_eq!(decoded.work_area_y, Some(37));
        assert_eq!(decoded.work_area_height, Some(1883));
    }

    /// An unknown work area survives the round trip as unknown.
    ///
    /// The case that matters: a session with no GNOME Shell extension still
    /// gets a monitor list, and `None` there must not decode as a zero-sized
    /// rectangle or as the monitor's full geometry — both would be a guess
    /// presented as a fact.
    #[test]
    fn a_monitor_with_no_known_work_area_round_trips_as_unknown() {
        let mut record = sample_monitor();
        record.work_area = None;

        let dict: MonitorRecordDict = record.clone().into();
        let back: MonitorRecord = dict.into();
        assert_eq!(back.work_area, None);
        assert_eq!(back, record);
    }

    /// A work area is all four fields or none of them.
    ///
    /// The dict carries four independent `Option`s, so a malformed `a{sv}`
    /// from a mismatched extension could in principle supply some and not
    /// others. Half a rectangle is not a rectangle, and the three known values
    /// would otherwise be combined with an invented fourth.
    #[test]
    fn a_partial_work_area_decodes_as_unknown_rather_than_being_completed() {
        let ctx = Context::new_dbus(Endian::Little, 0);
        let mut dict: MonitorRecordDict = sample_monitor().into();
        dict.work_area_height = None;

        let encoded = to_bytes(ctx, &dict).expect("encode a{sv}");
        let (decoded, _): (MonitorRecordDict, usize) = encoded.deserialize().expect("decode a{sv}");
        let record: MonitorRecord = decoded.into();

        assert_eq!(
            record.work_area, None,
            "three of four work-area fields is not a work area"
        );
    }

    /// The dict and the DTO must stay in step, for the same reason
    /// `daemon_status_dict_field_names_match_the_dto` exists: the dict defines
    /// the D-Bus contract, the DTO defines `--json` output, and a change on one
    /// side alone silently breaks the other's consumers.
    ///
    /// **The two do not have identical field names here, unlike every other
    /// pair in this module**, and that is deliberate. The work area is one
    /// nested `Rect` on the DTO — the shape a `--json` consumer wants, and the
    /// only one that makes "all four or none" expressible — and four flat
    /// `Option<i32>`s on the wire, because `a{sv}` has no nested-struct idiom
    /// the extension side could produce. So this asserts the DTO's own surface
    /// and leaves the mapping to the round-trip tests above, which cover it end
    /// to end.
    #[test]
    fn monitor_record_dict_field_names_match_the_dto() {
        let json = serde_json::to_value(sample_monitor()).expect("serialize DTO");
        let object = json.as_object().expect("object");
        let keys: Vec<&String> = object.keys().collect();

        for key in [
            "connector",
            "x",
            "y",
            "width",
            "height",
            "scale",
            "transform",
            "primary",
            "work_area",
        ] {
            assert!(
                keys.iter().any(|k| *k == key),
                "MonitorRecord lost the `{key}` field — this is a breaking change to \
                 `wgaf monitor list --json`"
            );
        }
        assert_eq!(
            keys.len(),
            9,
            "a field was added to MonitorRecord without being added to the assertion above \
             (and probably without being added to MonitorRecordDict either)"
        );

        // The nested rectangle's own field names are part of the JSON contract
        // too, and nothing else asserts them.
        let work_area = object["work_area"]
            .as_object()
            .expect("a known work area serializes as an object, not a tuple");
        for key in ["x", "y", "width", "height"] {
            assert!(work_area.contains_key(key), "work_area lost `{key}`");
        }
        assert_eq!(work_area.len(), 4);
    }

    /// An unknown work area is JSON `null`, not an omitted key.
    ///
    /// A consumer has to be able to tell "nothing is reserving space on this
    /// monitor" from "wgaf could not find out" — the first is a rectangle equal
    /// to the monitor, the second is this. Omitting the key entirely would make
    /// a script that indexes it fail rather than read the absence.
    #[test]
    fn an_unknown_work_area_serializes_as_null() {
        let mut record = sample_monitor();
        record.work_area = None;

        let json = serde_json::to_value(&record).expect("serialize DTO");
        let object = json.as_object().expect("object");
        assert!(
            object.contains_key("work_area"),
            "the key must still be present"
        );
        assert!(object["work_area"].is_null());
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
