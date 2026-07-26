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

use crate::{WindowRecord, WorkspaceRecord};
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

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
