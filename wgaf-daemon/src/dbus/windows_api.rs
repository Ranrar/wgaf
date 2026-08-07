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

use wgaf_common::Stacking;
use wgaf_common::dict::{
    MonitorRecordDict, WindowRecordDict, WorkspaceLayoutDict, WorkspaceRecordDict,
};

use futures_util::StreamExt;

use crate::permissions::{Capability, PermissionError, PermissionGate};
use crate::windows::{WindowEvent, WindowManager, WindowsError};

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
    /// No workspace exists at the given index.
    WorkspaceNotFound(String),
    /// The extension issued the operation and the compositor did not carry it
    /// out — so the desktop is as it was.
    ///
    /// Its own name rather than a generic failure because it is the one
    /// outcome a script most needs to branch on: nothing broke, and nothing
    /// changed.
    OperationNotApplied(String),
    /// The window itself refused, before anything was attempted — a dialog
    /// that cannot be maximized, a window the compositor keeps on every
    /// workspace.
    ///
    /// Kept apart from [`Self::OperationNotApplied`] because the two call for
    /// opposite responses: that one may work on a retry, this one never will.
    OperationNotSupported(String),
    /// A named-choice argument — `RestackWindow`'s `stacking` — that names
    /// none of the accepted values.
    ///
    /// The only error here about the call rather than the desktop.
    InvalidArgument(String),
    /// `GetMonitors` could not read the layout from Mutter.
    ///
    /// Kept distinct from [`Self::ExtensionUnavailable`] because the two send
    /// a user to different places: the layout comes from
    /// `org.gnome.Mutter.DisplayConfig`, which is present on any GNOME session
    /// whether or not the wgaf extension is installed.
    MonitorLayoutUnavailable(String),
}

impl From<WindowsError> for WindowsApiError {
    fn from(err: WindowsError) -> Self {
        match err {
            WindowsError::WindowNotFound(id) => {
                Self::WindowNotFound(format!("window {id} not found"))
            }
            WindowsError::WorkspaceNotFound(index) => {
                Self::WorkspaceNotFound(format!("workspace {index} not found"))
            }
            WindowsError::OperationNotApplied(reason) => Self::OperationNotApplied(reason),
            WindowsError::OperationNotSupported(reason) => Self::OperationNotSupported(reason),
            WindowsError::ExtensionUnavailable { .. } => {
                Self::ExtensionUnavailable(err.to_string())
            }
            WindowsError::DBus(e) => Self::ZBus(e),

            // Only `org.wgaf.Input1`'s targeted methods can reach this, via
            // `ensure_focused`, and that interface has its own named error for
            // it. No method on `org.wgaf.Windows1` calls `ensure_focused` at
            // all, so this is unreachable here — the catch-all keeps the
            // message intact rather than inventing a window-ish name for it,
            // the same treatment `OutOfBounds` gets below and for the same
            // reason.
            err @ WindowsError::WindowMinimized(_) => {
                Self::ZBus(zbus::Error::Failure(err.to_string()))
            }

            // `GetMonitors` reaches this one, and it is the only method on this
            // interface that can — every other caller of the display
            // configuration is on `org.wgaf.Input1`'s pointer path.
            err @ WindowsError::DisplayConfig(_) => Self::MonitorLayoutUnavailable(err.to_string()),

            // `WindowManager` owns the pointer because only the extension can
            // reach Clutter's seat, but no method on `org.wgaf.Windows1` moves
            // it — `MouseMoveAbsolute` and `GetPointerPosition` live on
            // `org.wgaf.Input1`, which translates this into a named error of
            // its own. It is unreachable here, and mapping it to a window-ish
            // error name would be a lie; the catch-all keeps the message intact
            // for the impossible case.
            err @ WindowsError::OutOfBounds { .. } => {
                Self::ZBus(zbus::Error::Failure(err.to_string()))
            }
        }
    }
}

impl From<String> for WindowsApiError {
    /// A `stacking` argument that names nothing.
    ///
    /// The parse error's own message names both the bad value and the accepted
    /// ones, so it is carried through unchanged.
    ///
    /// **The standard `org.freedesktop.DBus.Error.InvalidArgs` would have been
    /// the better name and is not reachable from here.** Returning it through
    /// the `#[zbus(error)]` catch-all flattens it to
    /// `org.freedesktop.zbus.Error`, which a client cannot branch on at all —
    /// measured, not assumed: an integration test asserted the standard name
    /// and got that instead. A wgaf-prefixed name is also what every other
    /// error on this interface uses, so the fallback is the consistent choice
    /// rather than merely the available one.
    fn from(message: String) -> Self {
        Self::InvalidArgument(message)
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
    manager: Arc<WindowManager>,
    permissions: Arc<PermissionGate>,
}

impl WindowsApi {
    pub fn new(manager: Arc<WindowManager>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            manager,
            permissions,
        }
    }
}

// Interface name must match `wgaf_common::WINDOWS_INTERFACE_NAME` (zbus
/// How long to wait before re-subscribing after the extension's event stream
/// ends or could not be established.
///
/// Seconds rather than milliseconds: the thing being waited for is a human
/// enabling an extension or logging back in, not a transient blip, and a tight
/// retry would spend the daemon's life introspecting a bus name that is not
/// there.
const RESUBSCRIBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Re-emits the extension's window signals on `org.wgaf.Windows1`, forever.
///
/// # Why the daemon re-emits instead of the CLI subscribing upstream
///
/// The CLI only ever talks to the daemon. Letting it subscribe to the extension
/// directly would duplicate the availability check, the error translation and
/// the `WindowRecordDict` → `WindowRecord` conversion in a second place, and
/// would mean a `wgaf` command that works without the daemon running — which is
/// not a shape this project has anywhere else.
///
/// # The re-subscribe loop replaces a bet on `NameOwnerChanged`
///
/// Disabling and re-enabling the extension changes its unique bus name. `zbus`
/// is documented to track `NameOwnerChanged` for well-known-name destinations,
/// so the streams *should* survive that — but "should" is how a watch silently
/// stops working, and it is the single most likely source of that failure. This
/// loop re-subscribes whenever the stream ends, which is correct whether or not
/// the tracking works, and costs one wakeup every few seconds in the case where
/// the extension is genuinely absent.
///
/// It also covers the extension not being installed when the daemon starts and
/// appearing later, which is the same no-restart-required behaviour
/// `ensure_extension_available` already gives every other window call.
///
/// # No bounded channel, deliberately
///
/// The plan called for one with a drop-and-warn policy, on the reasoning that a
/// burst of window events must not block the daemon's other interfaces. It is
/// not needed here and would be a queue in front of a queue: **emitting a D-Bus
/// signal does not wait for any consumer** — it is fire-and-forget from the
/// sender — so a slow or absent `wgaf window watch` cannot apply back-pressure.
/// The only place a burst can pile up is the upstream stream from the
/// extension, and `zbus` already bounds that and drops from it. Adding a channel
/// would move where events are discarded without changing whether they are.
pub async fn forward_window_events(
    manager: Arc<WindowManager>,
    emitter: zbus::object_server::SignalEmitter<'static>,
) {
    loop {
        match manager.subscribe_events().await {
            Ok(stream) => {
                tracing::debug!("subscribed to the extension's window signals");
                pump_window_events(stream, &emitter).await;
                // Reached only when the stream ends, which means the extension
                // went away. Logged at info because it is a real change in what
                // the daemon can report, and a user chasing a dead `watch`
                // needs it to be visible at the default level.
                tracing::info!(
                    "the extension's window signal stream ended; will re-subscribe in {}s",
                    RESUBSCRIBE_INTERVAL.as_secs()
                );
            }
            Err(err) => {
                // Debug, not warn: on a session with no extension installed this
                // is the steady state, and warning every few seconds forever
                // would make the log useless for finding real faults.
                tracing::debug!(
                    error = %err,
                    "cannot subscribe to window events yet; will retry in {}s",
                    RESUBSCRIBE_INTERVAL.as_secs()
                );
            }
        }

        tokio::time::sleep(RESUBSCRIBE_INTERVAL).await;
    }
}

/// Drains one subscription, emitting each event, until the stream ends.
async fn pump_window_events(
    stream: impl futures_util::Stream<Item = WindowEvent>,
    emitter: &zbus::object_server::SignalEmitter<'_>,
) {
    let mut stream = std::pin::pin!(stream);

    while let Some(event) = stream.next().await {
        let emitted = match event {
            WindowEvent::Created(id) => WindowsApi::window_created(emitter, id).await,
            WindowEvent::Closed(id) => WindowsApi::window_closed(emitter, id).await,
            WindowEvent::FocusChanged(id) => WindowsApi::window_focus_changed(emitter, id).await,
        };

        // A failed emission is not worth ending the subscription over: the next
        // event may well succeed, and dropping the whole stream because one
        // signal could not be written would turn a transient bus problem into a
        // permanently dead watch.
        if let Err(err) = emitted {
            tracing::warn!(
                error = %err,
                kind = event.kind(),
                window = event.window_id(),
                "failed to re-emit a window event"
            );
        }
    }
}

// requires a string literal here, so it can't reference the constant
// directly) — see the existing convention in `dbus/mod.rs`.
#[interface(name = "org.wgaf.Windows1")]
impl WindowsApi {
    async fn list_windows(&self) -> Result<Vec<WindowRecordDict>, WindowsApiError> {
        let windows = self.manager.list_windows().await?;
        Ok(windows.into_iter().map(WindowRecordDict::from).collect())
    }

    /// Asks permission to watch, before a caller starts consuming the signals
    /// below. `wgaf window watch` calls this first and reports what it says.
    ///
    /// # Why a method exists at all for a signal-based feature
    ///
    /// D-Bus signals are **broadcast**: any client on the session bus can add a
    /// match rule and receive them without the daemon being asked. So there is
    /// nowhere to put a check on the delivery path, and this method is where the
    /// policy is consulted and the audit line written instead.
    ///
    /// **That makes `WatchWindows` a policy statement, not an enforcement
    /// boundary**, and it must not be described as one. A process that ignores
    /// this method and subscribes to the bus directly still receives the events.
    /// The same is true of `input_max_events_per_second` — anything running as
    /// this user can open `/dev/uinput` without involving wgaf — and the honest
    /// framing is the same: it makes the intent explicit, writes it down in the
    /// audit trail, and lets a user say "no" in a file, rather than making the
    /// events unreachable.
    ///
    /// What it does buy is real: a denied watch **fails loudly and names the
    /// file**, instead of a caller sitting on a silent stream that is
    /// indistinguishable from an idle desktop.
    async fn watch_windows(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::WatchWindows, connection, &header)
            .await?;

        // Surfaces an absent extension here rather than leaving the caller on a
        // stream that will never carry anything. Without this, "the extension is
        // not installed" and "nothing has happened yet" look identical.
        self.manager.probe_available().await?;
        Ok(())
    }

    /// A window appeared. Carries its id only.
    ///
    /// **Not the record**, though the extension's own signal does carry one.
    /// Measured against GNOME Shell 50.1: the extension emits inside Mutter's
    /// `window-created` handler, before the client has set a title or committed
    /// a surface, so every record arrives blank — `title: ""`, `app_id: ""`,
    /// `0x0 at (0,0)`. Call `ListWindows` for detail, which also answers
    /// honestly when the window has already gone. See
    /// [`crate::windows::WindowEvent`] for the full reasoning.
    #[zbus(signal)]
    pub async fn window_created(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
    ) -> zbus::Result<()>;

    /// The window with this id went away.
    #[zbus(signal)]
    pub async fn window_closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
    ) -> zbus::Result<()>;

    /// Keyboard focus moved to the window with this id.
    #[zbus(signal)]
    pub async fn window_focus_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
    ) -> zbus::Result<()>;

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

    /// How the workspaces are arranged, and whether GNOME is managing their
    /// number itself. Read-only, so ungated.
    async fn get_workspace_layout(&self) -> Result<WorkspaceLayoutDict, WindowsApiError> {
        Ok(self.manager.get_workspace_layout().await?.into())
    }

    /// Make the workspace at `index` active.
    ///
    /// Returning `Ok` means it *is* active: the extension confirms the switch
    /// is readable before replying, so a caller can follow this with
    /// `ListWindows` without racing the switch.
    async fn switch_workspace(
        &self,
        index: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SwitchWorkspace, connection, &header)
            .await?;
        Ok(self.manager.switch_workspace(index).await?)
    }

    /// Append a workspace, returning its index.
    ///
    /// The new workspace is **not** activated. Adding a workspace and moving
    /// the user's view to it are two decisions, and a caller who wants both can
    /// follow this with `SwitchWorkspace`.
    ///
    /// On a session with dynamic workspaces — GNOME's default — the Shell may
    /// reclaim the new workspace as soon as it is left empty. It is genuinely
    /// created either way; `GetWorkspaceLayout`'s `dynamic` field is how a
    /// caller tells which mode the session is in.
    async fn add_workspace(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<i32, WindowsApiError> {
        self.permissions
            .check(Capability::AddWorkspace, connection, &header)
            .await?;
        Ok(self.manager.add_workspace().await?)
    }

    /// Remove the workspace at `index`.
    ///
    /// Windows on it are **not** closed — Mutter moves them to a neighbouring
    /// workspace, the same thing that happens when a user removes one from the
    /// overview. Removing the last remaining workspace is refused.
    async fn remove_workspace(
        &self,
        index: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::RemoveWorkspace, connection, &header)
            .await?;
        Ok(self.manager.remove_workspace(index).await?)
    }

    /// Move the workspace at `index` to `new_index`, shifting the others to
    /// make room.
    ///
    /// **Every workspace index a caller read before this call is stale
    /// afterwards.** That is Mutter's model, not a choice made here.
    async fn reorder_workspace(
        &self,
        index: i32,
        new_index: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::ReorderWorkspace, connection, &header)
            .await?;
        Ok(self.manager.reorder_workspace(index, new_index).await?)
    }

    /// Send a window to the workspace at `index`.
    ///
    /// **The window moves; you do not.** The active workspace is unchanged, so
    /// this sends a window out of sight rather than taking the caller with it.
    /// Follow with `SwitchWorkspace` to do both.
    ///
    /// Returning `Ok` means the window is on that workspace — the extension
    /// confirms before replying. A workspace index that does not exist is
    /// refused rather than created; creating one is `AddWorkspace`'s job and
    /// carries its own capability.
    async fn move_window_to_workspace(
        &self,
        id: u32,
        index: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::MoveWindowToWorkspace, connection, &header)
            .await?;
        Ok(self.manager.move_window_to_workspace(id, index).await?)
    }

    /// Minimize a window, or restore it.
    ///
    /// Restoring does **not** focus the window — `FocusWindow` is a separate
    /// method with a separate capability. Returning `Ok` means the window is in
    /// that state, not that the request was sent.
    async fn set_window_minimized(
        &self,
        id: u32,
        minimized: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SetWindowMinimized, connection, &header)
            .await?;
        Ok(self.manager.set_window_minimized(id, minimized).await?)
    }

    /// Maximize or unmaximize a window.
    ///
    /// **Both axes.** There is deliberately no per-axis argument: Mutter's
    /// `maximize()` takes no direction and overwrites the flags that appear to
    /// supply one, measured inside the Shell — see `setWindowMaximized` in
    /// `extension/windows.js`. An argument that could not be honoured would be
    /// worse than none.
    async fn set_window_maximized(
        &self,
        id: u32,
        maximized: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SetWindowMaximized, connection, &header)
            .await?;
        Ok(self.manager.set_window_maximized(id, maximized).await?)
    }

    /// Make a window fullscreen, or return it to its previous size.
    ///
    /// Not a synonym for maximizing: a fullscreen window covers the top bar and
    /// any dock, a maximized one stops at the work area.
    async fn set_window_fullscreen(
        &self,
        id: u32,
        fullscreen: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SetWindowFullscreen, connection, &header)
            .await?;
        Ok(self.manager.set_window_fullscreen(id, fullscreen).await?)
    }

    /// Keep a window above other windows, or stop doing so.
    ///
    /// This changes the window's stack layer, so it outranks `RestackWindow`
    /// entirely — a raised ordinary window still sits below one of these.
    async fn set_window_above(
        &self,
        id: u32,
        above: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SetWindowAbove, connection, &header)
            .await?;
        Ok(self.manager.set_window_above(id, above).await?)
    }

    /// Show a window on every workspace, or return it to one.
    ///
    /// **Turning this off leaves the window on the *active* workspace**, not on
    /// whichever one it was on before — nothing remembers that. So a caller that
    /// sticks a window, switches workspace, and unsticks it has moved that
    /// window. Measured, not assumed; see `setWindowOnAllWorkspaces` in
    /// `extension/windows.js`.
    ///
    /// A window the compositor puts on every workspace for its own reasons
    /// cannot be moved off them, and is refused with `OperationNotSupported`
    /// naming that reason.
    async fn set_window_on_all_workspaces(
        &self,
        id: u32,
        on_all_workspaces: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        self.permissions
            .check(Capability::SetWindowOnAllWorkspaces, connection, &header)
            .await?;
        Ok(self
            .manager
            .set_window_on_all_workspaces(id, on_all_workspaces)
            .await?)
    }

    /// Raise a window to the top of its stack layer, or lower it to the bottom.
    /// `stacking` is `raise` or `lower`.
    ///
    /// **Within its layer**, which is Mutter's model: raising cannot lift a
    /// window past an always-on-top one. Raising does not focus, though
    /// focusing does raise.
    async fn restack_window(
        &self,
        id: u32,
        stacking: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), WindowsApiError> {
        let stacking: Stacking = stacking.parse().map_err(WindowsApiError::from)?;
        self.permissions
            .check(Capability::RestackWindow, connection, &header)
            .await?;
        Ok(self.manager.restack_window(id, stacking).await?)
    }

    /// The logical monitors making up the desktop.
    ///
    /// # Ungated, and it needs no extension
    ///
    /// Read-only, so it has no [`Capability`] variant — the same rule
    /// `ListWindows` and `GetWorkspaces` follow. Unlike those two it does not
    /// go through the GNOME Shell extension at all: the layout comes from
    /// Mutter's own `org.gnome.Mutter.DisplayConfig`, so this answers on a
    /// session where the extension is not installed.
    ///
    /// # Why this exists
    ///
    /// The daemon has read the monitor layout since W5, to refuse an
    /// out-of-bounds `MouseMoveAbsolute` — and never told anyone what the
    /// bounds were. A caller could be told `(2000, 1700) is not on any
    /// monitor` and have no way to ask which coordinates are.
    async fn get_monitors(&self) -> Result<Vec<MonitorRecordDict>, WindowsApiError> {
        let monitors = self.manager.list_monitors().await?;
        Ok(monitors.into_iter().map(MonitorRecordDict::from).collect())
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
        let no_layout = WindowsApiError::MonitorLayoutUnavailable("no layout".to_string());
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
        assert_eq!(
            no_layout.name().as_str(),
            wgaf_common::WINDOWS_ERROR_MONITOR_LAYOUT_UNAVAILABLE
        );
    }

    /// A missing monitor layout must not be reported as a missing extension.
    ///
    /// These are different faults with different fixes, and conflating them
    /// would tell a user to install the wgaf extension when the actual problem
    /// is that they are not on a GNOME session at all. The catch-all `ZBus`
    /// variant would be no better — it produces
    /// `org.freedesktop.DBus.Error.Failed`, which a script cannot branch on.
    #[test]
    fn a_display_config_failure_maps_to_its_own_named_error() {
        use crate::windows::display_config::DisplayConfigError;

        let err = WindowsApiError::from(WindowsError::DisplayConfig(
            DisplayConfigError::Unavailable {
                bus_name: "org.gnome.Mutter.DisplayConfig".to_string(),
            },
        ));

        assert_eq!(
            err.name().as_str(),
            wgaf_common::WINDOWS_ERROR_MONITOR_LAYOUT_UNAVAILABLE,
            "GetMonitors must not report a missing layout as a missing extension"
        );
    }
}
