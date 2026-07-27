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
use zbus::zvariant::Type;

pub mod dict;

// ---------------------------------------------------------------------------
// Daemon's own public D-Bus API (org.wgaf.Daemon1).
// ---------------------------------------------------------------------------

/// Well-known session bus name the daemon registers on startup.
pub const BUS_NAME: &str = "org.wgaf.Daemon";

/// Object path the daemon's root interface is served at.
pub const OBJECT_PATH: &str = "/org/wgaf/Daemon";

/// Versioned interface name, following the `org.freedesktop.*1` convention
/// so a future `Daemon2` can be introduced without breaking existing clients.
pub const INTERFACE_NAME: &str = "org.wgaf.Daemon1";

// ---------------------------------------------------------------------------
// Daemon's own public window-management D-Bus API (org.wgaf.Windows1).
// Served on the same bus name/connection as `org.wgaf.Daemon1` above, at a
// sibling object path.
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

/// D-Bus error name returned by `org.wgaf.Windows1`'s mutating methods
/// (`FocusWindow`/`MoveWindow`/`ResizeWindow`/`CloseWindow`) when the
/// permission policy (`permissions.toml`) denies the call, or the caller
/// declined an interactive `Prompt`. See `wgaf-daemon/src/permissions/`.
pub const WINDOWS_ERROR_PERMISSION_DENIED: &str = "org.wgaf.Windows1.Error.PermissionDenied";

// ---------------------------------------------------------------------------
// Daemon's own public input-automation D-Bus API (org.wgaf.Input1).
// Served on the same bus name/connection as `org.wgaf.Daemon1`/
// `org.wgaf.Windows1` above, at a sibling object path. Unlike Windows1, this
// interface has no upstream GNOME Shell Extension dependency — it talks
// directly to a `uinput`-backed virtual input device owned by the daemon
// (see `wgaf-daemon/src/input/`), since GJS/Mutter has no `uinput` access.
// ---------------------------------------------------------------------------

/// Object path the daemon's input-automation interface is served at.
pub const INPUT_OBJECT_PATH: &str = "/org/wgaf/Input";

/// Versioned interface name for the daemon's own input-automation API.
pub const INPUT_INTERFACE_NAME: &str = "org.wgaf.Input1";

/// D-Bus error name returned by `org.wgaf.Input1` methods when the daemon's
/// `uinput` virtual device could not be opened/created — typically a
/// permissions problem (`/dev/uinput` not accessible), not a code bug. See
/// `wgaf-daemon/src/input/device.rs` for the exact diagnostic text and the
/// required udev rule.
pub const INPUT_ERROR_DEVICE_UNAVAILABLE: &str = "org.wgaf.Input1.Error.DeviceUnavailable";

/// D-Bus error name returned when a `KeyPress`/`KeyRelease`/`TypeText` call
/// references a key name or character this daemon's ASCII/US-QWERTY mapping
/// table (see `wgaf-daemon/src/input/codes.rs`) doesn't know how to
/// synthesize.
pub const INPUT_ERROR_UNKNOWN_KEY: &str = "org.wgaf.Input1.Error.UnknownKey";

/// D-Bus error name returned when `MouseClick` is given a button name other
/// than `left`, `right`, or `middle`.
pub const INPUT_ERROR_INVALID_BUTTON: &str = "org.wgaf.Input1.Error.InvalidButton";

/// D-Bus error name returned by `org.wgaf.Input1`'s mutating methods
/// (`TypeText`/`KeyPress`/`KeyRelease`/`MouseMove`/`MouseClick`/
/// `MouseScroll`) when the permission policy (`permissions.toml`)
/// denies the call, or the caller declined an interactive `Prompt`. See
/// `wgaf-daemon/src/permissions/`.
pub const INPUT_ERROR_PERMISSION_DENIED: &str = "org.wgaf.Input1.Error.PermissionDenied";

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
// Daemon's own public accessibility-automation D-Bus API
// (org.wgaf.Accessibility1). Served on the same bus name/
// connection as `org.wgaf.Daemon1`/`org.wgaf.Windows1`/`org.wgaf.Input1`
// above, at a sibling object path. Like Input1 (and unlike Windows1), there
// is no GNOME Shell Extension hop — the daemon talks directly to the
// separate AT-SPI accessibility bus (`org.a11y.Bus`, a distinct bus from the
// session bus, reached via `atspi::AccessibilityConnection`) since GJS/Mutter
// has no role in AT-SPI at all.
// ---------------------------------------------------------------------------

/// Object path the daemon's accessibility-automation interface is served at.
pub const ACCESSIBILITY_OBJECT_PATH: &str = "/org/wgaf/Accessibility";

/// Versioned interface name for the daemon's own accessibility-automation API.
pub const ACCESSIBILITY_INTERFACE_NAME: &str = "org.wgaf.Accessibility1";

/// D-Bus error name returned when the daemon could not connect to the AT-SPI
/// accessibility bus at all (e.g. accessibility is not enabled for the
/// session, or `org.a11y.Bus` isn't reachable). Distinct from
/// [`ACCESSIBILITY_ERROR_APP_NOT_FOUND`]/[`ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND`]:
/// this means the whole a11y stack is unavailable, not just one lookup.
pub const ACCESSIBILITY_ERROR_BUS_UNAVAILABLE: &str =
    "org.wgaf.Accessibility1.Error.BusUnavailable";

/// D-Bus error name returned when `FindElements`/`GetTree` is given an `app`
/// filter that doesn't match any currently-registered accessible
/// application's name.
pub const ACCESSIBILITY_ERROR_APP_NOT_FOUND: &str = "org.wgaf.Accessibility1.Error.AppNotFound";

/// D-Bus error name returned when an [`ElementRef`] passed to
/// `GetElementInfo`/`InvokeAction`/`SetText`/`FocusElement` no longer
/// corresponds to a live accessible object (the application exited, or the
/// widget was destroyed after it was found).
pub const ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND: &str =
    "org.wgaf.Accessibility1.Error.ElementNotFound";

/// D-Bus error name returned when `InvokeAction`/`SetText`/`FocusElement` is
/// called on an element that doesn't implement the AT-SPI interface the
/// operation requires (`Action`, `EditableText`, or `Component`
/// respectively).
pub const ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED: &str =
    "org.wgaf.Accessibility1.Error.ActionNotSupported";

/// D-Bus error name returned by `org.wgaf.Accessibility1`'s mutating methods
/// (`InvokeAction`/`SetText`/`FocusElement`) when the permission
/// policy (`permissions.toml`) denies the call, or the caller declined an
/// interactive `Prompt`. See `wgaf-daemon/src/permissions/`.
pub const ACCESSIBILITY_ERROR_PERMISSION_DENIED: &str =
    "org.wgaf.Accessibility1.Error.PermissionDenied";

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

// ---------------------------------------------------------------------------
// Accessibility DTOs.
//
// Unlike `WindowRecord`/`WorkspaceRecord` above, these derive `zvariant::Type`
// directly and are used as-is on `org.wgaf.Accessibility1`'s method
// signatures — no separate `a{sv}`-shaped "wire" struct in `dict.rs`. That
// split existed for Windows1/the extension bridge because the *extension*
// side (GJs) already emits `a{sv}` GVariant dicts of its own accord
// (`windowRecordToVariantDict`), and `a{sv}`'s `Variant`-wrapping isn't
// JSON-compatible (see this module's top doc comment). Here there is no
// external emitter to match — `wgaf-daemon` is the sole author of both the
// D-Bus server and (via `wgaf-cli`) the only client — so a plain
// `#[derive(Serialize, Deserialize, Type)]` struct is used directly: zvariant
// encodes it as an ordinary positional D-Bus struct (e.g. `(ssss)`), while
// plain `serde_json` still serializes/deserializes it as an ordinary JSON
// object with named fields. Confirmed by the round-trip tests below.

/// A stable reference to one AT-SPI accessible object.
///
/// This is AT-SPI's own native object-reference scheme — the `(so)`
/// bus-name/object-path tuple every `org.a11y.atspi.Accessible` method
/// already deals in (e.g. `GetChildren() -> a(so)`) — not an invented id
/// scheme. `bus_name` is the owning application's D-Bus *unique* connection
/// name (e.g. `:1.87`), stable for the lifetime of that application process,
/// which is also the lifetime of its accessible tree; `object_path` is the
/// object's path within that application (e.g.
/// `/org/a11y/atspi/accessible/1234`).
///
/// CLI serialization: `wgaf-cli` renders/parses this as a single
/// `bus_name#object_path` string (see `wgaf-cli/src/commands/accessibility.rs`)
/// — `#` cannot appear in either a D-Bus unique name or an object path, so
/// there's no ambiguity splitting on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ElementRef {
    pub bus_name: String,
    pub object_path: String,
}

impl std::fmt::Display for ElementRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.bus_name, self.object_path)
    }
}

/// Error returned by [`ElementRef`]'s `FromStr` impl when a CLI-supplied
/// `bus_name#object_path` string is missing its `#` separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseElementRefError;

impl std::fmt::Display for ParseElementRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid element reference: expected `bus_name#object_path` (e.g. \
             `:1.87#/org/a11y/atspi/accessible/1234`, as printed by `wgaf a11y list-apps`/`find`/`tree`)"
        )
    }
}

impl std::error::Error for ParseElementRefError {}

impl std::str::FromStr for ElementRef {
    type Err = ParseElementRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (bus_name, object_path) = s.split_once('#').ok_or(ParseElementRefError)?;
        if bus_name.is_empty() || object_path.is_empty() {
            return Err(ParseElementRefError);
        }
        Ok(ElementRef {
            bus_name: bus_name.to_string(),
            object_path: object_path.to_string(),
        })
    }
}

/// A registered accessible application, as returned by
/// `org.wgaf.Accessibility1.ListApps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AppRecord {
    pub element: ElementRef,
    pub name: String,
}

/// Summary information about one accessible element, as returned by
/// `FindElements`/`GetElementInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ElementRecord {
    pub element: ElementRef,
    pub name: String,
    pub role: String,
    pub description: String,
    pub child_count: i32,
    /// AT-SPI state names (e.g. `Focused`, `Enabled`, `Visible`), `Debug`-formatted
    /// from `atspi::State` — a human-readable diagnostic list, not a stable
    /// wire contract of its own.
    pub states: Vec<String>,
}

/// One node in a `GetTree` traversal: [`ElementRecord`]'s fields plus
/// `depth`, the node's nesting level relative to the application's root
/// object (the root itself is `depth = 0`). `GetTree` returns these as a
/// flat, depth-first-ordered list rather than a recursive structure — a
/// genuinely recursive D-Bus struct type isn't practical to encode with
/// zvariant's static signature computation, and a flat list with a depth
/// column is trivial for `wgaf-cli` to render as an indented tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TreeNode {
    pub element: ElementRef,
    pub name: String,
    pub role: String,
    pub description: String,
    pub child_count: i32,
    pub states: Vec<String>,
    pub depth: u32,
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

    // -----------------------------------------------------------------------
    // Accessibility DTOs
    // -----------------------------------------------------------------------

    fn sample_element_ref() -> ElementRef {
        ElementRef {
            bus_name: ":1.87".to_string(),
            object_path: "/org/a11y/atspi/accessible/1234".to_string(),
        }
    }

    fn sample_element_record() -> ElementRecord {
        ElementRecord {
            element: sample_element_ref(),
            name: "Save".to_string(),
            role: "push button".to_string(),
            description: String::new(),
            child_count: 0,
            states: vec!["Enabled".to_string(), "Visible".to_string()],
        }
    }

    #[test]
    fn element_ref_display_then_parse_round_trips() {
        let element = sample_element_ref();
        let text = element.to_string();
        assert_eq!(text, ":1.87#/org/a11y/atspi/accessible/1234");
        let back: ElementRef = text.parse().expect("parse");
        assert_eq!(element, back);
    }

    #[test]
    fn element_ref_parse_rejects_missing_separator() {
        assert!("no-hash-here".parse::<ElementRef>().is_err());
    }

    #[test]
    fn element_ref_parse_rejects_empty_halves() {
        assert!(
            "#/org/a11y/atspi/accessible/1234"
                .parse::<ElementRef>()
                .is_err()
        );
        assert!(":1.87#".parse::<ElementRef>().is_err());
    }

    #[test]
    fn element_ref_json_round_trips_as_plain_object() {
        // The whole point of using a plain `Type` derive instead of the
        // `a{sv}`-dict derive (see this module's doc comment on the
        // accessibility DTOs) is that this ALSO works as ordinary JSON, not
        // a Variant-wrapped map — assert that directly.
        let element = sample_element_ref();
        let value = serde_json::to_value(&element).expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({
                "bus_name": ":1.87",
                "object_path": "/org/a11y/atspi/accessible/1234",
            })
        );
        let back: ElementRef = serde_json::from_value(value).expect("deserialize");
        assert_eq!(element, back);
    }

    #[test]
    fn element_ref_encodes_as_a_dbus_struct_not_a_dict() {
        use zbus::zvariant::{Endian, serialized::Context, to_bytes};

        // A plain positional struct signature (`(ss)`), not `a{sv}` — this is
        // what lets the same derive round-trip through serde_json too.
        assert_eq!(ElementRef::SIGNATURE.to_string(), "(ss)");

        let element = sample_element_ref();
        let ctx = Context::new_dbus(Endian::Little, 0);
        let encoded = to_bytes(ctx, &element).expect("encode (ss)");
        let (decoded, _): (ElementRef, usize) = encoded
            .deserialize()
            .expect("decode (ss) back into ElementRef");
        assert_eq!(decoded, element);
    }

    #[test]
    fn element_record_json_round_trips() {
        let record = sample_element_record();
        let json = serde_json::to_string(&record).expect("serialize");
        let back: ElementRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    #[test]
    fn tree_node_json_round_trips() {
        let node = TreeNode {
            element: sample_element_ref(),
            name: "Save".to_string(),
            role: "push button".to_string(),
            description: String::new(),
            child_count: 0,
            states: vec!["Enabled".to_string()],
            depth: 2,
        };
        let json = serde_json::to_string(&node).expect("serialize");
        let back: TreeNode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(node, back);
    }
}
