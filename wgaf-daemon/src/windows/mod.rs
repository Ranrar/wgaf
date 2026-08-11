//! Daemon-side window management: a `zbus` client of the GNOME Shell
//! Extension's window bridge (`org.gnome.Shell.Extensions.Wgaf.V1`), with
//! extension-availability discovery and translation of the extension's
//! D-Bus errors into daemon-level errors.
//! Exposed to the CLI via the daemon's own `org.wgaf.Windows1` interface,
//! see `crate::dbus::windows_api`.

pub mod display_config;
mod proxy;

use std::time::Duration;

use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::{OnceCell, RwLock};
use wgaf_common::dict::{WindowRecordDict, WorkAreaDict, WorkspaceRecordDict};
use wgaf_common::{MonitorRecord, Rect, Stacking, WindowRecord, WorkspaceLayout, WorkspaceRecord};

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
    "GetWorkspaceLayout",
    "SwitchWorkspace",
    "AddWorkspace",
    "RemoveWorkspace",
    "ReorderWorkspace",
    "MoveWindowToWorkspace",
    "SetWindowMinimized",
    "SetWindowMaximized",
    "SetWindowFullscreen",
    "SetWindowAbove",
    "SetWindowOnAllWorkspaces",
    "RestackWindow",
    "GetWorkAreas",
    "WarpPointer",
    "GetPointer",
    "GetWindowAtPointer",
];

/// Signals the extension must declare, checked alongside the methods above.
///
/// **A missing signal fails worse than a missing method**, which is why
/// ADR-0002 requires this list as well. An outdated extension without these
/// answers every method call perfectly and simply never emits: the daemon
/// subscribes successfully, `wgaf window watch` starts and prints nothing, and
/// on an idle desktop that is exactly what a working watch looks like. A missing
/// *method* at least fails at the moment it is called.
const REQUIRED_EXTENSION_SIGNALS: &[&str] =
    &["WindowCreated", "WindowClosed", "WindowFocusChanged"];

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

    /// The extension reported that no workspace at this index exists.
    #[error("workspace {0} not found")]
    WorkspaceNotFound(i32),

    /// The extension issued the operation, re-read the state, and the change
    /// never became readable — so the desktop is as it was.
    ///
    /// The extension's own message is carried through rather than rewritten:
    /// it is the side that knows what was expected and what was found, and it
    /// already reads as a sentence.
    #[error("{0}")]
    OperationNotApplied(String),

    /// The window itself will not do what was asked, and Mutter said so before
    /// anything was attempted.
    ///
    /// The extension asks `can_minimize()` / `can_maximize()` /
    /// `is_always_on_all_workspaces()` first, so this is the window's own
    /// answer. Its message is carried through for the same reason
    /// [`Self::OperationNotApplied`]'s is.
    ///
    /// **Nearly the opposite of that variant**, and the difference is what a
    /// caller needs in order to decide whether to try again: an unapplied
    /// operation was issued and may land next time, this one was never issued
    /// and never will.
    #[error("{0}")]
    OperationNotSupported(String),

    /// The target window is minimized, so nothing typed at it could reach it.
    ///
    /// Raised by [`WindowManager::ensure_focused`] **before** any focus change
    /// is attempted, and the window is left minimized — restoring it is
    /// `SetWindowMinimized`'s job and carries its own capability. See the
    /// method's doc comment for why this is refused rather than corrected.
    #[error(
        "window {0} is minimized, so nothing typed at it would reach it — run \
         `wgaf window unminimize {0}` first"
    )]
    WindowMinimized(u32),

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

/// One thing that happened to a window, as the extension reported it.
///
/// Deliberately mirrors the extension's three signals one-for-one rather than
/// inventing a richer vocabulary on top. The daemon does not know about window
/// states the extension does not announce, and a variant the compositor cannot
/// produce would be a promise nothing keeps.
///
/// # Every variant carries an id and nothing else — measured, not assumed
///
/// `Created` originally carried the whole [`WindowRecord`], on the reasoning
/// that a consumer nearly always wants the title and geometry and that fetching
/// them afterwards is a race the window can lose by closing first.
///
/// **That was wrong, and a live run against GNOME Shell 50.1 showed it.** The
/// extension emits `WindowCreated` synchronously inside Mutter's
/// `window-created` handler, which is the earliest moment a window exists —
/// before the client has set a title or committed a surface. Every record
/// arrived as `title: "", app_id: "", 0x0 at (0,0)`, on all three of
/// `window-test`'s windows. The payload cost bytes and carried nothing.
///
/// So the record is dropped and the id kept. A consumer that wants detail calls
/// `ListWindows`, which is a round trip it controls — and which can honestly
/// answer "that window is already gone", where a pre-filled record could only
/// have lied.
///
/// **The alternative was rejected deliberately.** Having the daemon poll
/// `ListWindows` until the record filled would produce a better payload and
/// reintroduce the readiness-gate race this project has been bitten by twice,
/// with a new ordering hazard on top: a short-lived window could emit `Closed`
/// before its own `Created`, or a `Created` that never arrives at all. Ordering
/// is worth more than convenience the caller can get for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    /// A window appeared. Its detail is not available yet — see above.
    Created(u32),
    /// The window with this id went away.
    Closed(u32),
    /// Keyboard focus moved to the window with this id.
    FocusChanged(u32),
}

impl WindowEvent {
    /// The event name as it appears on `org.wgaf.Windows1` and in `--json`.
    ///
    /// One definition, so the D-Bus signal, the CLI's human output and its JSON
    /// cannot drift into three spellings of the same event.
    pub const fn kind(&self) -> &'static str {
        match self {
            WindowEvent::Created(_) => "created",
            WindowEvent::Closed(_) => "closed",
            WindowEvent::FocusChanged(_) => "focus-changed",
        }
    }

    /// The id of the window the event is about.
    pub const fn window_id(&self) -> u32 {
        match self {
            WindowEvent::Created(id) | WindowEvent::Closed(id) | WindowEvent::FocusChanged(id) => {
                *id
            }
        }
    }
}

/// The result of asking [`WindowManager::ensure_focused`] to make sure a
/// window has keyboard focus before an action runs against it.
///
/// Every variant is a normal, successful outcome. `ensure_focused` never
/// treats "the window still isn't focused" as an error — the window not
/// gaining focus in time is not a malfunction, see [`FocusOutcome::TimedOut`].
/// Whether that verdict should go on to block the action it was checked for
/// is a policy question for the caller, decided elsewhere; this type only
/// reports what actually happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOutcome {
    /// The window already had focus. Nothing was done: no `FocusWindow`
    /// call, no waiting.
    AlreadyFocused,
    /// The window did not have focus, `FocusWindow` was called, and a
    /// matching `WindowFocusChanged` arrived before the timeout.
    Corrected,
    /// The window did not have focus, `FocusWindow` was called, and no
    /// matching `WindowFocusChanged` arrived before the timeout.
    ///
    /// Most often this is Mutter's focus-stealing prevention legitimately
    /// declining the request — expected, normal behaviour, not a bug in wgaf
    /// and not a fault worth an error variant. Classifying it as a
    /// policy-level verification failure belongs to a later phase; this type
    /// just states the fact plainly.
    TimedOut,
}

/// Upper bound on how long [`WindowManager::ensure_focused`] waits for a
/// `WindowFocusChanged` signal after asking the extension to focus a window.
///
/// **This is not the guessing pattern flagged elsewhere for
/// `input_device_settle_ms`.** That constant stands in for a readiness
/// condition the daemon never actually observes, so every caller pays the
/// full wait regardless of how soon the device was really ready. This
/// constant guards a wait that is already event-driven: `ensure_focused`
/// returns the instant the real `WindowFocusChanged` signal arrives,
/// however fast that is. The constant only exists to end the wait if the
/// signal never arrives at all — which is expected, not exceptional, since
/// focus-stealing prevention can legitimately and silently sit on a focus
/// request forever.
///
/// Two seconds is generous on purpose: a safety bound, not a tuned latency
/// figure. It has not been measured against real focus-stealing-prevention
/// behaviour and could be revisited if that ever matters.
pub(crate) const DEFAULT_FOCUS_CONFIRM_TIMEOUT: Duration = Duration::from_secs(2);

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

        if let Some(missing) = REQUIRED_EXTENSION_SIGNALS
            .iter()
            .find(|signal| !xml.contains(&format!("signal name=\"{signal}\"")))
        {
            return Err(unavailable(format!(
                "extension is running but its `{}` interface declares no `{missing}` signal — the \
                 installed wgaf GNOME Shell Extension is older than this daemon and needs \
                 updating. Window watching would start and silently report nothing until it does",
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

    /// Subscribes to the extension's window signals as one typed stream.
    ///
    /// # All three are subscribed before this returns
    ///
    /// Not an implementation detail — it is the contract. `zbus` installs the
    /// bus match rule when the signal stream is *created*, so anything emitted
    /// between subscribing to the first stream and the third would reach only
    /// some of them. Subscribing all three up front and merging afterwards
    /// closes that window. `permissions/notify.rs` shipped the opposite ordering
    /// once — it subscribed after the call that triggers the reply — and the
    /// symptom was a user's deliberate click being silently dropped.
    ///
    /// # The creation record is discarded on purpose
    ///
    /// The extension's `WindowCreated` carries a full `WindowRecordDict`, and it
    /// is empty at the moment it fires — see [`WindowEvent`] for the
    /// measurement. Only the id survives the conversion. Reading the record here
    /// and passing it on would hand callers blank titles that look like a wgaf
    /// bug rather than a property of when the signal is emitted.
    ///
    /// # No replay, and no buffering before the first poll
    ///
    /// D-Bus signals are fire-and-forget. A caller that subscribes late has
    /// missed everything prior and there is no way to ask for it. `zbus`'s
    /// streams do buffer once created, so events are not lost between this
    /// returning and the first `next()`, but nothing before it exists at all.
    pub async fn subscribe_events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = WindowEvent> + Send + use<>, WindowsError> {
        self.ensure_extension_available().await?;

        let created = self.proxy.receive_window_created().await?;
        let closed = self.proxy.receive_window_closed().await?;
        let focus_changed = self.proxy.receive_window_focus_changed().await?;

        // A signal whose body does not decode is dropped rather than ending the
        // stream. The alternative — treating it as fatal — would let one
        // malformed emission from a mismatched extension silently stop every
        // later event, which is the "watch quietly stopped working" failure this
        // whole item has to avoid. The drift test is what catches a genuine
        // shape change, at `cargo test` rather than at runtime.
        let created = created.filter_map(|signal| async move {
            signal.args().ok().map(|args| {
                // Only the id is kept; the rest of the record is blank this
                // early. Converting through `WindowRecord` rather than reaching
                // into the dict keeps one definition of how the wire shape maps.
                WindowEvent::Created(WindowRecord::from(args.window.clone()).id)
            })
        });
        let closed = closed.filter_map(|signal| async move {
            signal.args().ok().map(|a| WindowEvent::Closed(a.id))
        });
        let focus_changed = focus_changed.filter_map(|signal| async move {
            signal.args().ok().map(|a| WindowEvent::FocusChanged(a.id))
        });

        Ok(futures_util::stream::select(
            futures_util::stream::select(created, closed),
            focus_changed,
        ))
    }

    pub async fn focus_window(&self, id: u32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .focus_window(id)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    /// Makes sure a window has keyboard focus before an action runs against
    /// it, correcting focus if it does not and *confirming* the correction
    /// actually took — rather than trusting [`Self::focus_window`]'s
    /// fire-and-forget reply, which cannot tell a real focus change from a
    /// request Mutter silently declined.
    ///
    /// # Why `focus_window` alone is not enough
    ///
    /// The extension's `FocusWindow` asks Mutter to activate a window and
    /// returns as soon as the request is sent; it does not and cannot confirm
    /// the window actually became focused. Mutter's focus-stealing prevention
    /// can legitimately and silently decline the request, so a caller that
    /// treated a successful `FocusWindow` reply as "focus is now there" would
    /// be wrong in exactly that case.
    ///
    /// # The subscribe-before-act ordering is load-bearing
    ///
    /// The focus-changed signal stream is subscribed to **before**
    /// [`Self::focus_window`] is called, not after. `zbus` installs the bus
    /// match rule the instant that subscription's `.await` completes, and
    /// from that moment the connection queues any matching signal whether or
    /// not anything is polling the stream yet — the same guarantee
    /// [`Self::subscribe_events`] relies on for the same reason. Subscribing
    /// only after calling `focus_window` would race a fast, real focus
    /// change: a signal fired before the subscription exists is gone, D-Bus
    /// signals being fire-and-forget, and every such loss would surface as a
    /// spurious [`FocusOutcome::TimedOut`] under perfectly normal conditions.
    ///
    /// # `TimedOut` is a normal outcome, not a fault
    ///
    /// See [`FocusOutcome::TimedOut`]. This method reports the fact plainly
    /// through `Ok((_, FocusOutcome::TimedOut))` rather than an `Err`.
    ///
    /// # A minimized window is refused, not restored
    ///
    /// A minimized window cannot hold keyboard focus, so input aimed at one
    /// would land in whatever window does — the exact hazard this guard exists
    /// to prevent, reached by a different route. The check happens before any
    /// focus change is attempted and raises
    /// [`WindowsError::WindowMinimized`].
    ///
    /// **Restoring it instead was considered and rejected.** Unminimizing is
    /// [`Self::set_window_minimized`]'s job and carries its own capability;
    /// doing it from inside a targeted `TypeText` would let one capability
    /// quietly perform another's work, and would un-hide a window the user
    /// deliberately hid. It is the same split that stops
    /// [`Self::move_window_to_workspace`] from switching workspace and
    /// [`Self::add_workspace`] from activating what it adds. The two-step is
    /// explicit and composes:
    /// `wgaf window unminimize <id> && wgaf type --window <id> …`.
    ///
    /// Leaving it to the focus timeout was also rejected: it fails safe
    /// already — focus never confirms, so nothing is typed — but reports
    /// "focus could not be confirmed" for a cause that was knowable up front,
    /// after waiting out the timeout to say so.
    ///
    /// # No permission awareness here, deliberately
    ///
    /// This method does not know about `Capability`/`PermissionGate`, and
    /// never should — see `permissions/mod.rs`'s module doc for why
    /// permission checks stay out of domain logic like this one. A caller
    /// wiring this into the input path is responsible for checking
    /// `Capability::FocusWindow` *before* calling this: calling it
    /// unconditionally would let any targeted input silently move focus
    /// regardless of policy.
    pub async fn ensure_focused(
        &self,
        id: u32,
        timeout: Duration,
    ) -> Result<(WindowRecord, FocusOutcome), WindowsError> {
        let record = self
            .list_windows()
            .await?
            .into_iter()
            .find(|window| window.id == id)
            .ok_or(WindowsError::WindowNotFound(id))?;

        // Before the focused check, not after: a minimized window cannot be
        // focused, so `focused` is already false and the ordering only decides
        // which of the two answers the caller gets. "It is minimized" is the
        // one that says what to do about it.
        if record.minimized {
            return Err(WindowsError::WindowMinimized(id));
        }

        if record.focused {
            return Ok((record, FocusOutcome::AlreadyFocused));
        }

        // Subscribed before `focus_window` is called below — see this
        // method's doc comment for why the order is the entire point.
        // `list_windows` above already went through
        // `ensure_extension_available`, so this does not need to repeat it.
        let mut focus_changed = self.proxy.receive_window_focus_changed().await?;

        self.focus_window(id).await?;

        // `true` only if a signal naming this id was actually seen; a closed
        // stream (e.g. the extension going away mid-wait) must not be
        // mistaken for confirmation just because the loop ended without
        // hitting the timeout.
        let matched = tokio::time::timeout(timeout, async {
            loop {
                match focus_changed.next().await {
                    Some(signal) => {
                        if let Ok(args) = signal.args()
                            && args.id == id
                        {
                            return true;
                        }
                    }
                    None => return false,
                }
            }
        })
        .await;

        if !matches!(matched, Ok(true)) {
            return Ok((record, FocusOutcome::TimedOut));
        }

        // A fresh fetch rather than trusting the signal's bare id: the
        // window could in principle have changed further — or closed —
        // between the snapshot above and the signal arriving, and
        // `WindowEvent`'s doc comment already established this file's
        // preference for a caller-controlled round trip over a payload that
        // can only ever describe the wrong moment. If it is gone by the time
        // this asks, that is `WindowNotFound`, not a fabricated record.
        let record = self
            .list_windows()
            .await?
            .into_iter()
            .find(|window| window.id == id)
            .ok_or(WindowsError::WindowNotFound(id))?;
        Ok((record, FocusOutcome::Corrected))
    }

    pub async fn move_window(&self, id: u32, x: i32, y: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .move_window(id, x, y)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    /// Resizes the window, returning once the new size is readable.
    ///
    /// **`translate_window_state_error`, not `translate_window_error`.** Resize
    /// confirms its own effect now, so it is the first non-state operation that
    /// can produce `OperationNotApplied` — a size the window will not take. With
    /// the narrower translation the extension's error name arrived and fell
    /// through to a generic failure, which is what a live desktop run found
    /// (2026-08-11) and what the unit test beside it now pins. `MoveWindow`
    /// above stays on the narrow one because it does not confirm and cannot
    /// raise that error; move it too if it ever does.
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
            .map_err(|e| translate_window_state_error(e, id))
    }

    pub async fn close_window(&self, id: u32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .close_window(id)
            .await
            .map_err(|e| translate_window_error(e, id))
    }

    // --- Window state -------------------------------------------------------
    //
    // Six operations, all the same shape: delegate to the extension, which
    // confirms the state actually changed before replying, and translate its
    // two named refusals. `Ok` therefore means the window IS in that state, not
    // that the request was sent — the same promise the workspace mutations
    // make, and the reason `ResizeWindow` above is the odd one out (it still
    // returns early; see the open S3 in `issues.md`).
    //
    // None of them does a neighbouring operation's job: restoring does not
    // focus, maximizing does not raise, un-fullscreening does not resize.

    /// Minimizes a window, or restores it.
    ///
    /// Restoring deliberately does **not** focus. That is
    /// [`Self::focus_window`], gated by its own capability, and a caller
    /// wanting both says so.
    pub async fn set_window_minimized(&self, id: u32, minimized: bool) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .set_window_minimized(id, minimized)
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    /// Maximizes or unmaximizes a window, on both axes.
    ///
    /// There is no per-axis option because Mutter 18 has none to offer:
    /// `maximize()` takes no direction and overwrites the flags that look like
    /// they would supply one. Measured inside the Shell — the numbers are in
    /// `setWindowMaximized`'s comment in `extension/windows.js`, and the
    /// rejected alternative is in `backlog.md`.
    pub async fn set_window_maximized(&self, id: u32, maximized: bool) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .set_window_maximized(id, maximized)
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    /// Makes a window fullscreen, or returns it to its previous size.
    ///
    /// Not the same as maximizing: a fullscreen window covers the top bar and
    /// any dock, where a maximized one stops at the work area. A script
    /// positioning other windows afterwards cares about the difference.
    pub async fn set_window_fullscreen(
        &self,
        id: u32,
        fullscreen: bool,
    ) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .set_window_fullscreen(id, fullscreen)
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    /// Keeps a window above other windows, or stops doing so.
    pub async fn set_window_above(&self, id: u32, above: bool) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .set_window_above(id, above)
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    /// Shows a window on every workspace, or returns it to one.
    ///
    /// **Un-sticking leaves the window on the active workspace**, not on the
    /// one it was on beforehand — a window on all workspaces is on the current
    /// one too, and that is the one it keeps. For a caller that switched
    /// workspace in between, this call is therefore also a move.
    ///
    /// A window the compositor puts on every workspace for its own reasons
    /// cannot be moved off them, and comes back as
    /// [`WindowsError::OperationNotSupported`] naming that reason rather than
    /// as a change that silently never happened.
    pub async fn set_window_on_all_workspaces(
        &self,
        id: u32,
        on_all_workspaces: bool,
    ) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .set_window_on_all_workspaces(id, on_all_workspaces)
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    /// Raises a window to the top of its stack layer, or lowers it to the
    /// bottom.
    ///
    /// **Within its layer**, which is Mutter's model rather than a limit here:
    /// a raised ordinary window still sits below an always-on-top one, and
    /// changing that is [`Self::set_window_above`]. Raising does not focus.
    pub async fn restack_window(&self, id: u32, stacking: Stacking) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .restack_window(id, stacking.as_str())
            .await
            .map_err(|e| translate_window_state_error(e, id))
    }

    pub async fn get_workspaces(&self) -> Result<Vec<WorkspaceRecord>, WindowsError> {
        self.ensure_extension_available().await?;
        let dicts = self.proxy.get_workspaces().await?;
        Ok(dicts.into_iter().map(WorkspaceRecordDict::into).collect())
    }

    pub async fn get_workspace_layout(&self) -> Result<WorkspaceLayout, WindowsError> {
        self.ensure_extension_available().await?;
        Ok(self.proxy.get_workspace_layout().await?.into())
    }

    /// Makes the workspace at `index` active.
    ///
    /// The extension confirms the switch took effect before replying, so this
    /// returning `Ok` means the workspace is active — not merely that the
    /// request was sent. A switch that never landed arrives here as
    /// [`WindowsError::OperationNotApplied`].
    pub async fn switch_workspace(&self, index: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .switch_workspace(index)
            .await
            .map_err(|e| translate_workspace_error(e, index))
    }

    /// Appends a workspace, returning its index.
    ///
    /// See [`WorkspaceLayout::dynamic`] for why that index may be short-lived
    /// on a stock GNOME session.
    pub async fn add_workspace(&self) -> Result<i32, WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .add_workspace()
            .await
            // No index to attribute a `WorkspaceNotFound` to, and this call
            // takes none — only the not-applied translation can apply.
            .map_err(|e| translate_workspace_error(e, -1))
    }

    pub async fn remove_workspace(&self, index: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .remove_workspace(index)
            .await
            .map_err(|e| translate_workspace_error(e, index))
    }

    pub async fn reorder_workspace(&self, index: i32, new_index: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .reorder_workspace(index, new_index)
            .await
            .map_err(|e| translate_workspace_error(e, index))
    }

    /// Sends a window to another workspace, leaving the active workspace alone.
    ///
    /// The extension confirms the window reports itself on the target before
    /// replying, so `Ok` means it is there — not that the request was sent.
    ///
    /// Two error names are reachable and they mean different things: the window
    /// id is unknown ([`WindowsError::WindowNotFound`]), or the workspace index
    /// is ([`WindowsError::WorkspaceNotFound`]). Both are translated, so a
    /// caller does not have to read the message to tell which argument was
    /// wrong.
    pub async fn move_window_to_workspace(&self, id: u32, index: i32) -> Result<(), WindowsError> {
        self.ensure_extension_available().await?;
        self.proxy
            .move_window_to_workspace(id, index)
            .await
            .map_err(|e| {
                // The window check first, then the workspace one — the two
                // translators cannot be composed because each consumes the
                // `zbus::Error`, and this is the only call that can raise
                // either.
                if let zbus::Error::MethodError(name, _, _) = &e
                    && name.as_str() == wgaf_common::EXTENSION_ERROR_WINDOW_NOT_FOUND
                {
                    return WindowsError::WindowNotFound(id);
                }
                translate_workspace_error(e, index)
            })
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

    /// The monitors making up the desktop, for `org.wgaf.Windows1.GetMonitors`.
    ///
    /// # Always a fresh read, unlike [`Self::monitor_layout`]
    ///
    /// The cache exists to keep a D-Bus round trip off the pointer path, which
    /// consults the layout on every warp and only needs it to answer "is this
    /// coordinate on a screen" — a question a stale answer degrades gracefully
    /// (see the `monitor_layout` field's refresh-policy note). A caller asking
    /// *what the monitors are* is asking for the current state directly, and
    /// handing them a layout from before the projector was plugged in would be
    /// a wrong answer rather than a slow one. So this refreshes, and the
    /// pointer path gets the newer cache as a side benefit.
    ///
    /// # Not behind `ensure_extension_available`
    ///
    /// Same reason [`Self::display_config`] is not: this reads Mutter's own
    /// `org.gnome.Mutter.DisplayConfig`, so it answers on a session with no
    /// wgaf extension installed.
    /// # The work area comes from the extension, and is optional
    ///
    /// `DisplayConfig` knows the monitors and nothing about struts, so the
    /// usable area has to come from Mutter's window manager — which only the
    /// extension can reach. A session without the extension therefore gets the
    /// monitor list with `work_area: None`, rather than no list at all.
    pub async fn list_monitors(&self) -> Result<Vec<MonitorRecord>, WindowsError> {
        let layout = self.refresh_monitor_layout().await?;
        let work_areas = self.work_areas().await;

        Ok(layout
            .monitors()
            .iter()
            .map(|m| MonitorRecord {
                connector: m.connector.clone(),
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                scale: m.scale,
                transform: m.transform,
                primary: m.primary,
                // Matched on the monitor's rectangle rather than on a position
                // in either list — see `work_areas` for why an index would be
                // a guess.
                work_area: work_areas
                    .iter()
                    .find(|w| {
                        w.x == m.x && w.y == m.y && w.width == m.width && w.height == m.height
                    })
                    .map(|w| Rect {
                        x: w.work_area_x,
                        y: w.work_area_y,
                        width: w.work_area_width,
                        height: w.work_area_height,
                    }),
            })
            .collect())
    }

    /// The extension's work-area report, or an empty list if it cannot be had.
    ///
    /// # Failing to an empty list rather than an error, deliberately
    ///
    /// This is the only part of [`Self::list_monitors`] that needs the
    /// extension, and the monitor layout is useful without it. Propagating the
    /// failure would make "the extension is not installed" take out a command
    /// that reads Mutter directly and would otherwise work — the exact coupling
    /// `display_config.rs` was written to avoid.
    ///
    /// # Why the join is on geometry and not on an index
    ///
    /// `get_work_area_for_monitor()` takes a Mutter monitor index;
    /// `DisplayConfig` reports connectors. That the two enumerate monitors in
    /// the same order is plausible and **unmeasured** — GNOME Shell's `Eval`
    /// endpoint is disabled, so it could not be checked from outside the
    /// compositor. Rather than assume it, the extension reports each monitor's
    /// own rectangle and this matches on that: two monitors cannot occupy one
    /// rectangle, so a match is exact and a mismatch is visible instead of
    /// silently pairing the wrong work area with the wrong screen.
    async fn work_areas(&self) -> Vec<WorkAreaDict> {
        if self.ensure_extension_available().await.is_err() {
            return Vec::new();
        }
        match self.proxy.get_work_areas().await {
            Ok(areas) => areas,
            Err(err) => {
                // Debug rather than warn: on a session with no extension this
                // is the steady state, and the caller still gets their
                // monitors. `forward_window_events` makes the same call.
                tracing::debug!(error = %err, "could not read work areas from the extension");
                Vec::new()
            }
        }
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

    /// Which window the pointer is over right now, or `None` if it is over
    /// none of them.
    ///
    /// `None` is an ordinary answer, not a failure: the desktop background,
    /// the top bar and the gap between two windows are all real places for a
    /// pointer to be. A caller deciding whether to synthesize a click treats it
    /// as "not the target", which it is.
    ///
    /// See [`WindowsProxy::get_window_at_pointer`] for why the extension reads
    /// the pointer itself instead of being handed a coordinate.
    pub async fn window_at_pointer(&self) -> Result<Option<u32>, WindowsError> {
        self.ensure_extension_available().await?;
        let (found, id) = self.proxy.get_window_at_pointer().await?;
        Ok(found.then_some(id))
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

/// [`translate_window_error`] plus the two outcomes only the window-state
/// operations can produce: the compositor not applying a change it was asked
/// for, and the window refusing one outright.
///
/// Both keep the extension's own description, for the reason
/// [`translate_workspace_error`] gives — the extension is the side that saw
/// what was expected and what was there instead, and the daemon would only be
/// guessing at it.
fn translate_window_state_error(err: zbus::Error, id: u32) -> WindowsError {
    if let zbus::Error::MethodError(name, description, _) = &err {
        if name.as_str() == wgaf_common::EXTENSION_ERROR_OPERATION_NOT_APPLIED {
            return WindowsError::OperationNotApplied(
                description
                    .clone()
                    .unwrap_or_else(|| "the compositor did not apply the operation".to_string()),
            );
        }
        if name.as_str() == wgaf_common::EXTENSION_ERROR_OPERATION_NOT_SUPPORTED {
            return WindowsError::OperationNotSupported(
                description
                    .clone()
                    .unwrap_or_else(|| format!("window {id} will not do that")),
            );
        }
    }
    translate_window_error(err, id)
}

/// The workspace-path equivalent of [`translate_window_error`], mapping the
/// extension's two named workspace errors onto daemon-level ones.
///
/// `OperationNotApplied` keeps the extension's own description rather than
/// discarding it: unlike `WindowNotFound` — where the daemon already knows the
/// id and the text adds nothing — the interesting part here is *what was
/// expected and what was found*, which only the extension observed. A
/// description-less reply falls back to naming the operation, so the error is
/// never empty.
fn translate_workspace_error(err: zbus::Error, index: i32) -> WindowsError {
    if let zbus::Error::MethodError(name, description, _) = &err {
        if name.as_str() == wgaf_common::EXTENSION_ERROR_WORKSPACE_NOT_FOUND {
            return WindowsError::WorkspaceNotFound(index);
        }
        if name.as_str() == wgaf_common::EXTENSION_ERROR_OPERATION_NOT_APPLIED {
            return WindowsError::OperationNotApplied(
                description
                    .clone()
                    .unwrap_or_else(|| "the compositor did not apply the operation".to_string()),
            );
        }
    }
    WindowsError::from(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use wgaf_common::dict::WorkspaceLayoutDict;
    use zbus::object_server::SignalEmitter;

    // These tests need a session bus — the whole `wgaf-daemon` integration
    // suite already does (see `tests/ping.rs`), and they deliberately do NOT
    // skip when one is absent, the same call `permissions/notify.rs`'s tests
    // make.

    /// The id `StubExtension`'s one canned window answers to.
    const STUB_WINDOW_ID: u32 = 1;

    /// A stand-in extension sized for exactly what `ensure_focused` needs,
    /// not for the whole surface `tests/windows_stub.rs`'s `StubExtension`
    /// covers.
    ///
    /// It still has to implement every member `ensure_extension_available`'s
    /// introspection check requires — see `REQUIRED_EXTENSION_METHODS`/
    /// `REQUIRED_EXTENSION_SIGNALS` — or `WindowManager` rejects it as an
    /// outdated extension before any test gets to exercise `ensure_focused`
    /// at all. Everything below `list_windows`/`focus_window` is a stub only
    /// to satisfy that check.
    struct StubExtension {
        /// Whether the canned window currently has focus, read back through
        /// `ListWindows`.
        focused: AtomicBool,
        /// If set, a `FocusWindow` call whose `id` equals this value marks
        /// the canned window focused and emits `WindowFocusChanged` for it —
        /// **synchronously, inside the same method call, before its D-Bus
        /// reply** — the fastest a real signal could ever arrive, and
        /// exactly the case that would be lost if `ensure_focused`
        /// subscribed after calling `FocusWindow` instead of before.
        ///
        /// A `FocusWindow` call for a *different* id than this one still
        /// emits — for that other id — but leaves `focused` untouched,
        /// which is what the timeout test uses to also exercise "ignore
        /// focus-changed events for other ids" in the same test.
        ///
        /// `None` models focus-stealing prevention silently declining the
        /// request: `FocusWindow` succeeds and nothing else happens.
        focus_succeeds_and_emits_for: Option<u32>,
        /// Whether the canned window reports itself minimized.
        ///
        /// A plain field rather than an `AtomicBool` because nothing here ever
        /// changes it: `ensure_focused` is supposed to refuse a minimized
        /// window outright, so a test that saw this flip would be watching the
        /// bug it exists to catch.
        minimized: bool,
    }

    #[zbus::interface(name = "org.gnome.Shell.Extensions.Wgaf.V1")]
    impl StubExtension {
        fn list_windows(&self) -> Vec<WindowRecordDict> {
            vec![
                WindowRecord {
                    id: STUB_WINDOW_ID,
                    title: "Stub".to_string(),
                    app_id: "org.wgaf.Stub".to_string(),
                    workspace: 0,
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                    focused: self.focused.load(Ordering::SeqCst),
                    maximized: false,
                    minimized: self.minimized,
                    fullscreen: false,
                    above: false,
                    on_all_workspaces: false,
                }
                .into(),
            ]
        }

        async fn focus_window(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
            let Some(emit_for) = self.focus_succeeds_and_emits_for else {
                return;
            };
            if emit_for == id {
                self.focused.store(true, Ordering::SeqCst);
            }
            // Reported, not swallowed: a failed emission would make the
            // affected test fail anyway (with an unexpected `TimedOut`), and
            // this makes that failure legible instead of mysterious.
            if let Err(e) = Self::window_focus_changed(&emitter, emit_for).await {
                eprintln!("stub failed to emit WindowFocusChanged: {e}");
            }
        }

        fn move_window(&self, _id: u32, _x: i32, _y: i32) {}
        fn resize_window(&self, _id: u32, _width: i32, _height: i32) {}
        fn close_window(&self, _id: u32) {}
        fn get_workspaces(&self) -> Vec<WorkspaceRecordDict> {
            vec![]
        }
        fn get_workspace_layout(&self) -> WorkspaceLayoutDict {
            WorkspaceLayout {
                n_workspaces: 1,
                active: 0,
                rows: 1,
                columns: 1,
                dynamic: false,
            }
            .into()
        }
        fn switch_workspace(&self, _index: i32) {}
        fn add_workspace(&self) -> i32 {
            0
        }
        fn remove_workspace(&self, _index: i32) {}
        fn reorder_workspace(&self, _index: i32, _new_index: i32) {}
        fn move_window_to_workspace(&self, _id: u32, _index: i32) {}
        fn set_window_minimized(&self, _id: u32, _minimized: bool) {}
        fn set_window_maximized(&self, _id: u32, _directions: &str, _maximized: bool) {}
        fn set_window_fullscreen(&self, _id: u32, _fullscreen: bool) {}
        fn set_window_above(&self, _id: u32, _above: bool) {}
        fn set_window_on_all_workspaces(&self, _id: u32, _on_all_workspaces: bool) {}
        fn restack_window(&self, _id: u32, _stacking: &str) {}
        fn get_work_areas(&self) -> Vec<WorkAreaDict> {
            vec![]
        }
        fn warp_pointer(&self, x: i32, y: i32) -> (i32, i32) {
            (x, y)
        }
        fn get_pointer(&self) -> (i32, i32) {
            (0, 0)
        }
        /// Nothing under the pointer. This stub exists for `ensure_focused`
        /// and never exercises the pointer guard; the stub that does is
        /// `tests/windows_stub.rs`'s, which tracks a real position.
        fn get_window_at_pointer(&self) -> (bool, u32) {
            (false, 0)
        }

        #[zbus(signal)]
        async fn window_created(
            emitter: &SignalEmitter<'_>,
            window: WindowRecordDict,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn window_closed(emitter: &SignalEmitter<'_>, id: u32) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn window_focus_changed(emitter: &SignalEmitter<'_>, id: u32) -> zbus::Result<()>;
    }

    /// Starts a `StubExtension` on a private, unique bus name (leaked for the
    /// test process's lifetime, same rationale as
    /// `tests/windows_stub.rs`'s `start_stub_extension`) and returns a
    /// `WindowManager` freshly connected to it.
    ///
    /// `tag` keeps each test's bus name distinct, since these run
    /// concurrently by default and a bus name is a session-global resource —
    /// the same convention `tests/windows_stub.rs` and
    /// `permissions/notify.rs`'s tests already use.
    async fn manager_against_stub(
        tag: &str,
        focused: bool,
        focus_succeeds_and_emits_for: Option<u32>,
    ) -> WindowManager {
        manager_against_stub_window(tag, focused, focus_succeeds_and_emits_for, false).await
    }

    /// [`manager_against_stub`] with the canned window's minimized state
    /// spelled out. Separate so the four existing tests stay readable — only
    /// the minimize test cares.
    async fn manager_against_stub_window(
        tag: &str,
        focused: bool,
        focus_succeeds_and_emits_for: Option<u32>,
        minimized: bool,
    ) -> WindowManager {
        let bus_name = format!("org.wgaf.Test.EnsureFocused{tag}{}", std::process::id());
        let stub_connection = zbus::connection::Builder::session()
            .expect("session bus builder")
            .name(bus_name.as_str())
            .expect("valid bus name")
            .serve_at(
                wgaf_common::EXTENSION_OBJECT_PATH,
                StubExtension {
                    focused: AtomicBool::new(focused),
                    focus_succeeds_and_emits_for,
                    minimized,
                },
            )
            .expect("serve stub extension")
            .build()
            .await
            .expect("stub extension registers on the session bus");
        std::mem::forget(stub_connection);

        let client_connection = zbus::Connection::session()
            .await
            .expect("connect to session bus");
        WindowManager::connect_to(
            client_connection,
            &bus_name,
            wgaf_common::EXTENSION_OBJECT_PATH,
            wgaf_common::EXTENSION_INTERFACE_NAME,
            // Never read by `ensure_focused`; a real Mutter or another
            // stub is not needed for these tests to be meaningful.
            "org.gnome.Mutter.DisplayConfig",
        )
        .await
        .expect("connect to the stub extension")
    }

    /// A minimized window is refused by name, immediately, without trying to
    /// focus it and without restoring it.
    ///
    /// The stub's `focus_succeeds_and_emits_for` is set to the window's own id,
    /// so if `ensure_focused` were to attempt the focus anyway it would
    /// *succeed* and this test would see `Corrected` — which is the failure
    /// worth catching, since it is the one that ends with keystrokes going to
    /// whatever window really had focus.
    ///
    /// The timeout is deliberately long enough to be noticed: refusing up front
    /// means not waiting it out, and a regression to "let the focus confirmation
    /// time out" would take two seconds to reach a worse-worded answer.
    #[tokio::test]
    async fn a_minimized_window_is_refused_before_focus_is_attempted() {
        let manager =
            manager_against_stub_window("Minimized", false, Some(STUB_WINDOW_ID), true).await;

        let started = std::time::Instant::now();
        let error = manager
            .ensure_focused(STUB_WINDOW_ID, Duration::from_secs(2))
            .await
            .expect_err("a minimized window must not be reported as focusable");

        assert!(
            matches!(error, WindowsError::WindowMinimized(id) if id == STUB_WINDOW_ID),
            "expected WindowMinimized, got {error:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the refusal must not wait out the focus timeout — took {:?}",
            started.elapsed()
        );

        // Left minimized. Restoring it is `SetWindowMinimized`'s job and carries
        // its own capability; doing it here would be one capability performing
        // another's work, which is the whole reason this refuses.
        let record = manager
            .list_windows()
            .await
            .expect("stub answers ListWindows")
            .into_iter()
            .find(|w| w.id == STUB_WINDOW_ID)
            .expect("the canned window is still there");
        assert!(record.minimized, "the window must be left minimized");
    }

    #[tokio::test]
    async fn already_focused_short_circuits_with_no_signal_and_no_wait() {
        let manager = manager_against_stub("AlreadyFocused", true, None).await;

        let (record, outcome) = manager
            .ensure_focused(STUB_WINDOW_ID, Duration::from_millis(50))
            .await
            .expect("ensure_focused should succeed against a known, focused window");

        assert_eq!(outcome, FocusOutcome::AlreadyFocused);
        assert_eq!(record.id, STUB_WINDOW_ID);
        assert!(record.focused);
    }

    #[tokio::test]
    async fn a_signal_arriving_before_the_timeout_reports_corrected() {
        let manager = manager_against_stub("Corrected", false, Some(STUB_WINDOW_ID)).await;

        let (record, outcome) = manager
            .ensure_focused(STUB_WINDOW_ID, DEFAULT_FOCUS_CONFIRM_TIMEOUT)
            .await
            .expect("ensure_focused should succeed once the signal arrives");

        assert_eq!(outcome, FocusOutcome::Corrected);
        assert_eq!(record.id, STUB_WINDOW_ID);
        assert!(
            record.focused,
            "the record returned alongside Corrected must reflect the new state"
        );
    }

    #[tokio::test]
    async fn no_matching_signal_times_out_within_the_given_duration() {
        // Emits for a different id than the one being focused, so this also
        // exercises "ignore focus-changed events for other ids" alongside
        // the timeout path itself.
        const OTHER_WINDOW_ID: u32 = STUB_WINDOW_ID + 41;
        let manager = manager_against_stub("TimedOut", false, Some(OTHER_WINDOW_ID)).await;

        let short_timeout = Duration::from_millis(50);
        let started = tokio::time::Instant::now();
        let (record, outcome) = manager
            .ensure_focused(STUB_WINDOW_ID, short_timeout)
            .await
            .expect("a timeout is a normal Ok outcome, not an error");

        assert_eq!(outcome, FocusOutcome::TimedOut);
        assert_eq!(record.id, STUB_WINDOW_ID);
        assert!(!record.focused);
        assert!(
            started.elapsed() < DEFAULT_FOCUS_CONFIRM_TIMEOUT,
            "must time out using the given duration, not fall back to the 2s default"
        );
    }

    #[tokio::test]
    async fn unknown_window_id_reports_window_not_found() {
        let manager = manager_against_stub("NotFound", false, None).await;
        let unknown_id = STUB_WINDOW_ID + 999;

        let err = manager
            .ensure_focused(unknown_id, Duration::from_millis(50))
            .await
            .expect_err("an id absent from ListWindows must not be treated as focusable");

        assert!(matches!(err, WindowsError::WindowNotFound(id) if id == unknown_id));
    }
}
