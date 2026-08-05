//! D-Bus naming and shared DTOs used across `wgaf-daemon` and `wgaf-cli`
//! (and, for naming purposes only, the GNOME Shell Extension's D-Bus
//! contract in `extension/dbusInterface.js`, which `wgaf-daemon` is a client
//! of).
//!
//! The DTOs here are deliberately plain `serde` types with no `zvariant`
//! coupling: zvariant's `a{sv}` dict (de)serialization wraps every field in
//! a `Variant` marker, which is not interchangeable with plain JSON (a
//! `--json` CLI field would come out as `{"signature": "u", "value": 42}`
//! instead of `42`). The matching `a{sv}`-shaped "wire" structs live in this
//! crate's own [`dict`] module, purely for the two D-Bus hops
//! (extension -> daemon, daemon -> CLI); both `wgaf-daemon` and `wgaf-cli`
//! convert to/from the plain DTOs here at the edges. These DTOs are what
//! `wgaf-cli` actually serializes for `--json` output.
//!
//! Note this split applies only to the window/workspace types, which have to
//! match the GNOME Shell Extension's self-authored `a{sv}` dicts. The
//! accessibility DTOs further down derive `zvariant::Type` directly and need
//! no wire counterpart — see their own section comment for why.

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

/// D-Bus error name returned when `TypeText`/`TypeTextAt` is given a
/// character the active keyboard layout has no key sequence for.
///
/// Distinct from [`INPUT_ERROR_UNKNOWN_KEY`] on purpose: that one means
/// "there is no such key", this means "this layout cannot produce that
/// character". A script that branches on it can substitute or skip the
/// character; one that cannot tell them apart has to guess which. See
/// `wgaf-daemon/src/input/keyboard.rs`.
pub const INPUT_ERROR_CHARACTER_NOT_TYPEABLE: &str = "org.wgaf.Input1.Error.CharacterNotTypeable";

/// D-Bus error name returned when the session's keyboard layout could not be
/// determined, or the configured one is invalid, so `TypeText` does not know
/// what its keystrokes would produce.
///
/// Environmental rather than a bad request — no Wayland session reachable,
/// or no keyboard on any seat — see `wgaf-daemon/src/input/mod.rs`.
pub const INPUT_ERROR_KEYBOARD_LAYOUT_UNAVAILABLE: &str =
    "org.wgaf.Input1.Error.KeyboardLayoutUnavailable";

/// D-Bus error name returned when `MouseClick` is given a button name other
/// than `left`, `right`, or `middle`.
pub const INPUT_ERROR_INVALID_BUTTON: &str = "org.wgaf.Input1.Error.InvalidButton";

/// D-Bus error name returned when `TypeText` is given more characters than
/// `config.toml`'s `input_max_type_text_chars` allows.
///
/// Named rather than generic because the limit is the user's to choose: a
/// caller that sets it to 256 will meet this routinely, and a script should be
/// able to branch on "too long" without string-matching a description. See
/// `wgaf-daemon/src/input/mod.rs`.
pub const INPUT_ERROR_TEXT_TOO_LONG: &str = "org.wgaf.Input1.Error.TextTooLong";

/// D-Bus error name returned when so much synthetic input is queued that the
/// call would have waited past the daemon's runaway threshold.
///
/// **Not the ordinary over-budget response.** Exceeding
/// `config.toml`'s `input_max_events_per_second` normally just slows a call
/// down, so a legitimate long script still completes; this error means the
/// backlog got large enough to indicate a caller stuck in a loop. See
/// `wgaf-daemon/src/input/rate_limit.rs`.
pub const INPUT_ERROR_RATE_LIMITED: &str = "org.wgaf.Input1.Error.RateLimited";

/// D-Bus error name returned by every `org.wgaf.Input1` method while the kill
/// switch is engaged (`org.wgaf.Daemon1.Stop`, or the desktop shortcut the
/// GNOME Shell Extension installs).
///
/// Distinct from [`INPUT_ERROR_PERMISSION_DENIED`] on purpose: policy is a
/// standing decision about what wgaf may ever do, while this is a live
/// emergency stop the user can lift with `wgaf release`. A script that cannot
/// tell them apart cannot tell "you are not allowed to do this" from "wait,
/// then try again".
pub const INPUT_ERROR_STOPPED: &str = "org.wgaf.Input1.Error.Stopped";

/// D-Bus error name returned by `org.wgaf.Input1`'s mutating methods
/// (`TypeText`/`KeyPress`/`KeyRelease`/`MouseMove`/`MouseClick`/
/// `MouseScroll`) when the permission policy (`permissions.toml`)
/// denies the call, or the caller declined an interactive `Prompt`. See
/// `wgaf-daemon/src/permissions/`.
pub const INPUT_ERROR_PERMISSION_DENIED: &str = "org.wgaf.Input1.Error.PermissionDenied";

/// D-Bus error name returned by `org.wgaf.Input1.MouseMoveAbsolute` when the
/// requested coordinate is not on any monitor.
///
/// Worth branching on rather than treating as a generic failure: a desktop
/// whose monitors differ in size or alignment has coordinates inside its
/// overall bounding box that sit on no screen, so a caller computing a target
/// can reach one without having done anything wrong. The error's description
/// lists the monitors and their rectangles, which is usually enough to see why.
///
/// Nothing is moved when this is returned. The pointer is deliberately **not**
/// clamped to the nearest valid position — see the error's documentation in
/// `wgaf-daemon/src/windows/mod.rs`.
pub const INPUT_ERROR_OUT_OF_BOUNDS: &str = "org.wgaf.Input1.Error.OutOfBounds";

/// D-Bus error name returned by `org.wgaf.Input1.MouseMoveAbsolute` when the
/// monitor layout cannot be read from the compositor at all, so no coordinate
/// can be validated.
///
/// Environmental rather than a bad request: absolute positioning needs both the
/// wgaf GNOME Shell Extension and `org.gnome.Mutter.DisplayConfig`, and this
/// says the second one is missing. Relative `MouseMove` is unaffected.
pub const INPUT_ERROR_MONITOR_LAYOUT_UNAVAILABLE: &str =
    "org.wgaf.Input1.Error.MonitorLayoutUnavailable";

/// D-Bus error name returned by `org.wgaf.Input1`'s targeted methods
/// (`TypeTextAt`/`KeyPressAt`/`KeyReleaseAt`/`HotkeyAt`) when the given
/// window id does not correspond to any currently open window.
///
/// Distinct from [`WINDOWS_ERROR_WINDOW_NOT_FOUND`] only in which interface
/// returns it — the message shape mirrors it deliberately, since both name
/// the same kind of caller mistake on their own interface.
pub const INPUT_ERROR_WINDOW_NOT_FOUND: &str = "org.wgaf.Input1.Error.WindowNotFound";

/// D-Bus error name returned by `org.wgaf.Input1`'s targeted methods when the
/// named window could not be confirmed focused before the timeout, after the
/// daemon attempted to correct it — the pre-condition half of action
/// verification (see `wgaf-daemon/src/windows/mod.rs`'s
/// `FocusOutcome::TimedOut`).
///
/// **Not a policy denial and not a fault.** Nothing was refused and nothing
/// malfunctioned — most often this is the compositor's own focus-stealing
/// prevention declining the request. A script should be able to branch on
/// this without mistaking it for either `PermissionDenied` (no
/// `permissions.toml` rule fired) or a generic failure (nothing is broken).
pub const INPUT_ERROR_VERIFICATION_FAILED: &str = "org.wgaf.Input1.Error.VerificationFailed";

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

/// D-Bus error name returned when an [`ElementRef`] is not a well-formed
/// D-Bus `(bus name, object path)` pair at all — e.g. `wgaf a11y info
/// 'nosuch#/x'`, where `nosuch` is not a valid bus name.
///
/// Deliberately distinct from [`ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND`],
/// because the two call for different remedies: a *malformed* reference is a
/// caller mistake to be fixed at the call site, while a *stale* one was valid
/// when it was issued and simply needs re-querying via `FindElements`.
/// Collapsing them would tell a user to re-run a query that was never going
/// to help.
pub const ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF: &str =
    "org.wgaf.Accessibility1.Error.InvalidElementRef";

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

/// The daemon's self-report: which subsystems are usable right now, and what
/// policy it is enforcing. Returned by `org.wgaf.Daemon1.Status` and rendered
/// by `wgaf status`.
///
/// Two design rules this type exists to serve, both of which are easy to
/// violate later:
///
/// 1. **Reporting must not change anything.** Every field here is answerable
///    without creating the `uinput` device, opening a cached AT-SPI
///    connection, or populating any availability cache. A status query that
///    initializes the thing it reports on is not a status query.
/// 2. **Nothing here is a secret.** The interface is deliberately ungated —
///    a transparency mechanism that policy can switch off defeats its own
///    purpose — so it must never carry anything a caller could not already
///    learn by attempting the operations it describes. Paths and policy are
///    the user's own configuration; do not add tokens, keys, or window
///    titles.
///
/// The layout is flat with subsystem-prefixed keys rather than nested
/// sub-objects, matching [`WindowRecord`]/[`WorkspaceRecord`]'s existing
/// shape and keeping the `a{sv}` encoding (see [`crate::dict`]) simple. The
/// `_detail` fields carry the daemon's own actionable error text when the
/// corresponding `_available` flag is false, and are empty otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    /// Daemon crate version (`CARGO_PKG_VERSION`).
    pub daemon_version: String,
    /// Well-known bus name this daemon actually owns.
    pub daemon_bus_name: String,
    /// The daemon's process id — the same identity its audit log records.
    pub daemon_pid: u32,
    /// Seconds since the daemon finished starting up.
    pub daemon_uptime_seconds: u64,
    /// The config file location in effect — the explicit `--config` path, or
    /// the resolved XDG default. Always populated, **whether or not the file
    /// exists**: when it does not, this is where to create one, which is the
    /// single most useful thing to tell someone asking "where does config
    /// go?". Pair with [`Self::config_present`] to tell the cases apart.
    pub config_path: String,
    /// Whether a file was actually read from [`Self::config_path`]. A missing
    /// file is not an error — the built-in defaults apply — but reporting the
    /// path without this flag would imply a config is in effect when none is.
    pub config_present: bool,

    /// Whether the GNOME Shell Extension bridge is reachable *right now*
    /// (freshly checked, never read from the availability cache).
    pub extension_available: bool,
    /// Bus name the extension is expected on.
    pub extension_bus_name: String,
    /// On failure, the same actionable text `ExtensionUnavailable` carries.
    pub extension_detail: String,

    /// Whether `/dev/uinput` is currently openable for writing. Answered by
    /// opening and immediately closing it, issuing no ioctls — so this never
    /// creates a virtual device as a side effect of asking.
    pub uinput_accessible: bool,
    /// On failure, the udev-rule/`input`-group guidance from
    /// `InputError::DeviceUnavailable`.
    pub uinput_detail: String,
    /// Name the daemon's virtual device reports to the kernel.
    pub input_device_name: String,
    /// Whether the daemon currently holds a live virtual input device. This
    /// is an *activity* signal, not a health one: false simply means nothing
    /// has synthesized input yet this run.
    pub input_device_created: bool,
    /// The configured `input_keyboard_layout` value, verbatim — `auto`, a
    /// layout code or name, or `us-ascii`.
    pub input_keyboard_layout_configured: String,
    /// The layout that value actually resolved to, as the keymap names it
    /// (`Danish`, `English (Dvorak)`). Empty until it has been resolved.
    ///
    /// Reported separately from the configured value because `auto` says
    /// nothing about *which* layout was chosen, and "why did my text come out
    /// wrong" is answered by this field — the layout is read once at daemon
    /// start, so a desktop whose layout changed since then shows the old one
    /// here.
    pub input_keyboard_layout_resolved: String,
    /// Whether the kill switch is engaged: every input-synthesis call is being
    /// refused until `wgaf release`.
    ///
    /// Not a fault, and not persisted — it means someone deliberately stopped
    /// wgaf this run. It is reported here because "nothing happens when I run
    /// `wgaf type`" has no other visible explanation.
    pub input_stopped: bool,

    /// Whether the AT-SPI accessibility bus is reachable right now.
    pub accessibility_available: bool,
    /// On failure, the guidance from `AccessibilityError::BusUnavailable` —
    /// which names the specific cause (no accessibility bus, a stale address
    /// left by one that exited, an unreachable session bus) and its remedy,
    /// rather than a single generic hint.
    pub accessibility_detail: String,
    /// Whether the daemon currently holds an open AT-SPI connection —
    /// again activity, not health.
    pub accessibility_connected: bool,

    /// The policy file location in effect, always populated on the same terms
    /// as [`Self::config_path`] — including when absent, so a user wanting to
    /// restrict a capability is told exactly where to write it.
    pub permissions_path: String,
    /// Whether a policy file was actually read. When false every capability
    /// defaults to `Allow` and [`Self::permissions_restricted`] is empty.
    pub permissions_present: bool,
    /// Capabilities whose configured value is *not* the `Allow` default,
    /// formatted `Capability=Value`. Empty means nothing is restricted.
    pub permissions_restricted: Vec<String>,
    /// Interactive `Prompt` decisions cached for this daemon run, formatted
    /// `Capability=allowed`/`Capability=denied`. Lost on restart, by design.
    pub permissions_prompt_decisions: Vec<String>,
}

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
