//! Integration tests for the daemon's `org.wgaf.Windows1` D-Bus API against
//! a hand-written stub implementation of the GNOME Shell Extension's
//! `org.gnome.Shell.Extensions.Wgaf.V1` interface, per the roadmap's
//! "Mocked/stub GNOME Shell Extension D-Bus service for daemon-side
//! window-management tests that don't require a real GNOME Shell session"
//! testing strategy (Section 11).
//!
//! These tests exercise the *real* `wgaf-daemon` binary and its *real*
//! `org.wgaf.Windows1` D-Bus interface end-to-end — only the extension it
//! depends on is faked, via `Config::extension_bus_name`. This matches
//! `ping.rs`'s existing pattern (spawn the real binary, talk to it over
//! D-Bus) rather than reaching into internal Rust modules, so the D-Bus
//! contract itself stays covered by the test, not just the code behind it.
//!
//! What this does NOT cover (documented gap, not silently skipped): this
//! sandboxed environment has no `gnome-shell`/nested-Wayland-session
//! available, so there is no test here against the *real* GNOME Shell
//! Extension — that remains a manual/VM testing gap per the roadmap's
//! Section 11 "GNOME Extension Tests"/"Virtual Machine Testing" notes.

use std::process::{Child, Command};
use std::time::Duration;

use wgaf_common::dict::{
    MonitorRecordDict, WindowRecordDict, WorkAreaDict, WorkspaceLayoutDict, WorkspaceRecordDict,
};
use wgaf_common::{MonitorRecord, WindowRecord, WorkspaceLayout, WorkspaceRecord};
use zbus::interface;

/// Kills the spawned daemon even if an assertion panics mid-test, and cleans
/// up its config file — kept alive until here (not removed right after
/// `spawn()`) since the child process reads it asynchronously and a
/// same-tick removal races the daemon's own `Config::load`.
struct DaemonGuard {
    child: Child,
    config_path: std::path::PathBuf,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config_path);
    }
}

/// D-Bus error the stub returns for unknown window ids, matching
/// `wgaf_common::EXTENSION_ERROR_WINDOW_NOT_FOUND`'s fully qualified name
/// exactly — this is what lets `WindowManager::translate_window_error`
/// (and, further up, the daemon's own `org.wgaf.Windows1` error
/// translation) be exercised for real, not just unit-tested in isolation.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.gnome.Shell.Extensions.Wgaf.Error")]
enum StubExtensionError {
    #[zbus(error)]
    ZBus(zbus::Error),
    WindowNotFound(String),
    WorkspaceNotFound(String),
    OperationNotApplied(String),
}

fn canned_window() -> WindowRecord {
    WindowRecord {
        id: 1,
        title: "Stub Terminal".to_string(),
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

fn canned_workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        index: 0,
        active: true,
        n_windows: 1,
    }
}

/// The stub's workspace state: how many there are and which is active.
///
/// Real state rather than canned answers, because the workspace methods are
/// the first ones on this interface whose *effect* is what a caller checks.
/// A stub that accepted `SwitchWorkspace` and always reported workspace 0 as
/// active would pass a test asserting the call succeeds and prove nothing.
///
/// It models the two behaviours the real extension has to get right and the
/// daemon has to translate: an out-of-range index is `WorkspaceNotFound`, and
/// removing the last remaining workspace is refused rather than silently
/// declined — which is what Mutter does and what makes it worth naming.
struct StubWorkspaces {
    count: i32,
    active: i32,
}

impl Default for StubWorkspaces {
    fn default() -> Self {
        StubWorkspaces {
            count: 1,
            active: 0,
        }
    }
}

/// Stub implementation of `org.gnome.Shell.Extensions.Wgaf.V1`: canned data
/// for window id 1, `WindowNotFound` for anything else.
///
/// Holds a pointer position so the two pointer methods behave like the real
/// extension does — `WarpPointer` moves it and reports where it landed,
/// `GetPointer` reads it back. Landing exactly on the requested coordinate is
/// the measured behaviour of the real `warp_pointer` (5/5 exact, 2026-08-02),
/// so the stub is not being more obliging than the thing it stands in for.
///
/// It deliberately does **not** clamp out-of-range coordinates the way Mutter
/// does. Nothing should ever hand it one: the daemon's bounds check is supposed
/// to reject those first, and a stub that silently accepted them would hide
/// exactly the bug the check exists to prevent.
///
/// Its workspace state is real too — see [`StubWorkspaces`].
#[derive(Default)]
struct StubExtension {
    pointer: std::sync::Mutex<(i32, i32)>,
    workspaces: std::sync::Mutex<StubWorkspaces>,
}

#[interface(name = "org.gnome.Shell.Extensions.Wgaf.V1")]
impl StubExtension {
    /// The three window signals, declared so the stub looks like a *current*
    /// extension to the daemon's member-presence check.
    ///
    /// Not optional decoration: `REQUIRED_EXTENSION_SIGNALS` makes an extension
    /// without these fail `ensure_extension_available`, so a stub that omitted
    /// them would be rejected as outdated and every test here would fail with
    /// `ExtensionUnavailable`. That is the check doing its job — it caught this
    /// stub the moment the signal requirement landed.
    ///
    /// Declaring them also makes them emittable, which is what lets a test drive
    /// the daemon's re-emission path with no GNOME Shell anywhere.
    #[zbus(signal)]
    async fn window_created(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        window: WindowRecordDict,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn window_closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn window_focus_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
    ) -> zbus::Result<()>;

    fn warp_pointer(&self, x: i32, y: i32) -> (i32, i32) {
        let mut pointer = self.pointer.lock().expect("stub pointer lock");
        *pointer = (x, y);
        *pointer
    }

    fn get_pointer(&self) -> (i32, i32) {
        *self.pointer.lock().expect("stub pointer lock")
    }

    fn list_windows(&self) -> Vec<WindowRecordDict> {
        vec![canned_window().into()]
    }

    fn focus_window(&self, id: u32) -> Result<(), StubExtensionError> {
        self.check_known(id)
    }

    fn move_window(&self, id: u32, _x: i32, _y: i32) -> Result<(), StubExtensionError> {
        self.check_known(id)
    }

    fn resize_window(&self, id: u32, _width: i32, _height: i32) -> Result<(), StubExtensionError> {
        self.check_known(id)
    }

    fn close_window(&self, id: u32) -> Result<(), StubExtensionError> {
        self.check_known(id)
    }

    fn get_workspaces(&self) -> Vec<WorkspaceRecordDict> {
        let ws = self.workspaces.lock().expect("stub workspace lock");
        (0..ws.count)
            .map(|index| {
                WorkspaceRecord {
                    index,
                    active: index == ws.active,
                    // Only workspace 0 holds the canned window.
                    n_windows: if index == 0 { 1 } else { 0 },
                }
                .into()
            })
            .collect()
    }

    fn get_workspace_layout(&self) -> WorkspaceLayoutDict {
        let ws = self.workspaces.lock().expect("stub workspace lock");
        WorkspaceLayout {
            n_workspaces: ws.count,
            active: ws.active,
            // A standard vertical GNOME layout: one column, one row per
            // workspace.
            rows: ws.count,
            columns: 1,
            // Static, matching the default this stub starts in. The dynamic
            // case is a property of the session's GSettings, not of anything
            // the daemon does, so there is nothing here to exercise by
            // flipping it.
            dynamic: false,
        }
        .into()
    }

    fn switch_workspace(&self, index: i32) -> Result<(), StubExtensionError> {
        let mut ws = self.workspaces.lock().expect("stub workspace lock");
        Self::check_index(&ws, index)?;
        ws.active = index;
        Ok(())
    }

    fn add_workspace(&self) -> i32 {
        let mut ws = self.workspaces.lock().expect("stub workspace lock");
        ws.count += 1;
        ws.count - 1
    }

    fn remove_workspace(&self, index: i32) -> Result<(), StubExtensionError> {
        let mut ws = self.workspaces.lock().expect("stub workspace lock");
        Self::check_index(&ws, index)?;
        if ws.count <= 1 {
            return Err(StubExtensionError::OperationNotApplied(
                "the last workspace cannot be removed: expected at least 2 workspaces, got 1"
                    .to_string(),
            ));
        }
        ws.count -= 1;
        // Mutter moves the active workspace when the active one goes away;
        // clamping is enough to model that here.
        ws.active = ws.active.min(ws.count - 1);
        Ok(())
    }

    /// Work areas for a *deliberately partial* match against the stub display
    /// configuration below.
    ///
    /// The first entry's rectangle is exactly `DP-3`'s, so the daemon's
    /// geometry join finds it. The second matches no monitor at all. Together
    /// they exercise both halves of that join in one stub: a monitor that gets
    /// its usable area, and a monitor whose work area stays unknown because
    /// nothing described it — which is what must happen rather than the daemon
    /// pairing a leftover entry with whatever monitor is left over.
    fn get_work_areas(&self) -> Vec<WorkAreaDict> {
        vec![
            WorkAreaDict {
                x: DP3_RECT.0,
                y: DP3_RECT.1,
                width: DP3_RECT.2,
                height: DP3_RECT.3,
                // A GNOME top bar's worth off the top.
                work_area_x: DP3_RECT.0,
                work_area_y: DP3_RECT.1 + TOP_BAR_HEIGHT,
                work_area_width: DP3_RECT.2,
                work_area_height: DP3_RECT.3 - TOP_BAR_HEIGHT,
            },
            WorkAreaDict {
                x: 9000,
                y: 9000,
                width: 640,
                height: 480,
                work_area_x: 9000,
                work_area_y: 9000,
                work_area_width: 640,
                work_area_height: 480,
            },
        ]
    }

    fn reorder_workspace(&self, index: i32, new_index: i32) -> Result<(), StubExtensionError> {
        let ws = self.workspaces.lock().expect("stub workspace lock");
        Self::check_index(&ws, index)?;
        Self::check_index(&ws, new_index)?;
        // Nothing distinguishes the stub's workspaces from one another, so
        // there is no order to permute — validating both indices is the whole
        // of what this stub can meaningfully model.
        Ok(())
    }

    /// Validates **both** arguments, which is the point of stubbing it.
    ///
    /// This is the only method on the interface that can fail for two unrelated
    /// reasons — an unknown window or an unknown workspace — and the daemon has
    /// to map each onto a different named error. Checking the window first
    /// mirrors the real extension, where `_requireWindow` runs before
    /// `_requireWorkspace`.
    fn move_window_to_workspace(&self, id: u32, index: i32) -> Result<(), StubExtensionError> {
        self.check_known(id)?;
        let ws = self.workspaces.lock().expect("stub workspace lock");
        Self::check_index(&ws, index)?;
        Ok(())
    }
}

impl StubExtension {
    fn check_known(&self, id: u32) -> Result<(), StubExtensionError> {
        if id == canned_window().id {
            Ok(())
        } else {
            Err(StubExtensionError::WindowNotFound(format!(
                "no window with id {id}"
            )))
        }
    }

    fn check_index(ws: &StubWorkspaces, index: i32) -> Result<(), StubExtensionError> {
        if index >= 0 && index < ws.count {
            Ok(())
        } else {
            Err(StubExtensionError::WorkspaceNotFound(format!(
                "no workspace at index {index} (there are {})",
                ws.count
            )))
        }
    }
}

// --- Stub for org.gnome.Mutter.DisplayConfig -------------------------------
//
// A second stub, because it is a different service. It exists at all because
// the real Mutter already owns `org.gnome.Mutter.DisplayConfig` on any live
// session, so nothing else can take that name — `Config::display_config_bus_name`
// is what lets a test daemon read its monitor layout from here instead.

/// The stub layout's two logical monitors as `(x, y, width, height)`, in the
/// logical coordinates the daemon computes from `GetCurrentState`.
///
/// Named because **two independent stubs have to agree on them**: the display
/// configuration below produces them from mode sizes and transforms, and
/// `StubExtension::get_work_areas` reports a work area keyed on `DP-3`'s
/// rectangle. The daemon joins those two on geometry, so a literal drifting on
/// one side alone would silently turn the join into a no-match — and the
/// work-area test would then be asserting the failure path while looking like
/// it passed the success one.
const DP3_RECT: (i32, i32, i32, i32) = (1080, 0, 2560, 1440);
const HDMI1_RECT: (i32, i32, i32, i32) = (0, 0, 1080, 1920);

/// Height of the notional top bar the stub reserves on `DP-3`.
const TOP_BAR_HEIGHT: i32 = 37;

/// One display mode, as `GetCurrentState` reports it: id, width, height,
/// refresh rate, preferred scale, supported scales, properties.
type StubMode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
);

type StubMonitor = (
    (String, String, String, String),
    Vec<StubMode>,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
);

type StubLogicalMonitor = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, String, String)>,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
);

type StubCurrentState = (
    u32,
    Vec<StubMonitor>,
    Vec<StubLogicalMonitor>,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
);

/// The maintainer's real two-monitor layout, measured from Mutter on
/// 2026-08-02 and reproduced here rather than invented.
///
/// ```text
///   x=0        x=1080                      x=3640
///   +----------+---------------------------+ y=0
///   | HDMI-1   | DP-3                      |
///   | 1080x1920| 2560x1440  (primary)      |
///   | rotated  |                           |
///   |          +---------------------------+ y=1440
///   |          |  <-- not on any monitor
///   +----------+                             y=1920
/// ```
///
/// The notch in the bottom right is the whole reason this shape was chosen: it
/// is inside the layout's bounding box and on no screen, so a bounds check
/// written against a bounding box passes every test on a tidy side-by-side
/// layout and still lets the pointer be aimed off-screen here.
fn stub_monitor(connector: &str, width: i32, height: i32) -> StubMonitor {
    let mut mode_props = std::collections::HashMap::new();
    mode_props.insert(
        "is-current".to_string(),
        zbus::zvariant::OwnedValue::from(true),
    );
    (
        (
            connector.to_string(),
            "StubVendor".to_string(),
            "StubProduct".to_string(),
            "0".to_string(),
        ),
        vec![(
            format!("{width}x{height}@60"),
            width,
            height,
            60.0,
            1.0,
            vec![1.0],
            mode_props,
        )],
        std::collections::HashMap::new(),
    )
}

fn stub_logical_monitor(
    connector: &str,
    x: i32,
    y: i32,
    transform: u32,
    primary: bool,
) -> StubLogicalMonitor {
    (
        x,
        y,
        1.0,
        transform,
        primary,
        vec![(
            connector.to_string(),
            "StubVendor".to_string(),
            "StubProduct".to_string(),
            "0".to_string(),
        )],
        std::collections::HashMap::new(),
    )
}

struct StubDisplayConfig;

#[interface(name = "org.gnome.Mutter.DisplayConfig")]
impl StubDisplayConfig {
    fn get_current_state(&self) -> StubCurrentState {
        (
            1,
            vec![
                // HDMI-1's mode is landscape; the 90-degree transform on its
                // logical monitor is what makes it present as 1080x1920. The
                // daemon has to do that swap itself, and getting it wrong would
                // put the tall monitor's bounds at 1920x1080. Its mode is
                // therefore its logical size *transposed* — the one place these
                // constants cannot be used directly.
                stub_monitor("HDMI-1", HDMI1_RECT.3, HDMI1_RECT.2),
                stub_monitor("DP-3", DP3_RECT.2, DP3_RECT.3),
            ],
            vec![
                stub_logical_monitor("HDMI-1", HDMI1_RECT.0, HDMI1_RECT.1, 1, false),
                stub_logical_monitor("DP-3", DP3_RECT.0, DP3_RECT.1, 0, true),
            ],
            std::collections::HashMap::new(),
        )
    }
}

async fn start_stub_display_config(bus_name: &str) {
    let connection = zbus::connection::Builder::session()
        .expect("session bus builder")
        .name(bus_name)
        .expect("valid bus name")
        .serve_at("/org/gnome/Mutter/DisplayConfig", StubDisplayConfig)
        .expect("serve stub display config")
        .build()
        .await
        .expect("stub display config registers on session bus");
    std::mem::forget(connection);
}

/// Starts the stub extension service on a private, unique bus name (so it
/// never collides with a real GNOME Shell Extension on a shared session
/// bus) and leaks the connection for the test's lifetime — the process
/// exiting cleans it up.
async fn start_stub_extension(bus_name: &str) {
    // Leak: keeps the stub alive for the rest of this test process. Each
    // test uses its own unique bus name, so leaking across tests in the
    // same binary is harmless.
    std::mem::forget(start_stub_extension_with_handle(bus_name).await);
}

/// As [`start_stub_extension`], but hands the connection back so a test can
/// *emit* from the stub rather than only answer calls on it.
///
/// The caller must hold the returned connection for as long as the stub is
/// needed — dropping it unregisters the bus name, and the daemon then reports
/// the extension as unavailable rather than as silent, which is a confusing way
/// for a signal test to fail.
async fn start_stub_extension_with_handle(bus_name: &str) -> zbus::Connection {
    zbus::connection::Builder::session()
        .expect("session bus builder")
        .name(bus_name)
        .expect("valid bus name")
        .serve_at(wgaf_common::EXTENSION_OBJECT_PATH, StubExtension::default())
        .expect("serve stub extension")
        .build()
        .await
        .expect("stub extension registers on session bus")
}

/// Spawns the real `wgaf-daemon` binary with a config pointing its own bus
/// name and its expected extension bus name at test-private, unique names.
fn spawn_daemon(daemon_bus_name: &str, extension_bus_name: &str, nonce: &str) -> DaemonGuard {
    spawn_daemon_with_display_config(daemon_bus_name, extension_bus_name, None, nonce, ALLOW_ALL)
}

/// An empty capability table — the explicit way to say "allow everything".
const ALLOW_ALL: &str = "[capabilities]\n";

/// As [`spawn_daemon`], but with a policy of the test's choosing.
fn spawn_daemon_with_policy(
    daemon_bus_name: &str,
    extension_bus_name: &str,
    nonce: &str,
    policy: &str,
) -> DaemonGuard {
    spawn_daemon_with_display_config(daemon_bus_name, extension_bus_name, None, nonce, policy)
}

/// As [`spawn_daemon`], but also pointing the daemon's monitor-layout lookup at
/// a private bus name.
///
/// `None` leaves the default (`org.gnome.Mutter.DisplayConfig`), which is what
/// the non-pointer tests want: they never read the layout, and pointing them at
/// a stub that does not exist would be misleading rather than harmless.
fn spawn_daemon_with_display_config(
    daemon_bus_name: &str,
    extension_bus_name: &str,
    display_config_bus_name: Option<&str>,
    nonce: &str,
    policy: &str,
) -> DaemonGuard {
    let config_path = std::env::temp_dir().join(format!("wgaf-daemon-windows-test-{nonce}.toml"));
    let display_config_line = display_config_bus_name
        .map(|name| format!("display_config_bus_name = \"{name}\"\n"))
        .unwrap_or_default();
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{daemon_bus_name}\"\nlog_level = \"error\"\nextension_bus_name = \"{extension_bus_name}\"\n{display_config_line}"
        ),
    )
    .expect("failed to write test config");
    // The daemon requires both files to exist and to not be group/world
    // writable; `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test config permissions");

    // The daemon requires a policy file; an empty [capabilities] table is the
    // explicit way to say "allow everything". Given its own unique path
    // rather than relying on the config sibling, because every test config
    // lives directly in the temp dir and would otherwise share one file.
    let permissions_path = std::env::temp_dir().join(format!("wgaf-stub-permissions-{nonce}.toml"));
    std::fs::write(&permissions_path, policy).expect("failed to write test policy");
    // Explicit mode: the daemon rejects a group/world-writable policy file,
    // and `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &permissions_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test policy permissions");

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    DaemonGuard { child, config_path }
}

async fn wait_for_daemon(bus_name: &str) -> zbus::Connection {
    let connection = zbus::Connection::session()
        .await
        .expect("connect to session bus");
    for _ in 0..50 {
        let reply = connection
            .call_method(
                Some(bus_name),
                wgaf_common::OBJECT_PATH,
                Some(wgaf_common::INTERFACE_NAME),
                "Ping",
                &(),
            )
            .await;
        if reply.is_ok() {
            return connection;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("daemon did not respond to Ping in time");
}

async fn call_windows<R, A>(
    connection: &zbus::Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            method,
            args,
        )
        .await?;
    reply.body().deserialize()
}

#[tokio::test]
async fn list_windows_and_get_workspaces_via_stub() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.Ok{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.Ok{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(&daemon_bus_name, &extension_bus_name, &format!("ok{pid}"));
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let windows: Vec<WindowRecordDict> =
        call_windows(&connection, &daemon_bus_name, "ListWindows", &())
            .await
            .expect("ListWindows should succeed");
    let windows: Vec<WindowRecord> = windows.into_iter().map(Into::into).collect();
    assert_eq!(windows, vec![canned_window()]);

    let workspaces: Vec<WorkspaceRecordDict> =
        call_windows(&connection, &daemon_bus_name, "GetWorkspaces", &())
            .await
            .expect("GetWorkspaces should succeed");
    let workspaces: Vec<WorkspaceRecord> = workspaces.into_iter().map(Into::into).collect();
    assert_eq!(workspaces, vec![canned_workspace()]);
}

/// Every signal the extension emits comes back out on `org.wgaf.Windows1`.
///
/// This is the whole of W6's forwarding path exercised with **no GNOME Shell
/// anywhere**: stub emits → daemon's proxy subscription → typed conversion →
/// re-emission. The equivalent against a real Shell needs a live session and so
/// can never run in CI, which is the argument for testing at the daemon
/// boundary rather than only end to end.
#[tokio::test]
async fn the_daemon_re_emits_every_window_signal_the_extension_sends() {
    use futures_util::StreamExt;

    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.Watch{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.Watch{pid}");

    // Held, not leaked: emitting needs the connection the stub is served on.
    let stub = start_stub_extension_with_handle(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("watch{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // Subscribed before anything is emitted. The daemon's forwarding task
    // retries on an interval, so it may not have subscribed upstream yet — the
    // retry loop below covers that, and is why this asserts on *eventually
    // arriving* rather than on a single emission.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(daemon_bus_name.as_str())
        .expect("valid sender")
        .path(wgaf_common::WINDOWS_OBJECT_PATH)
        .expect("valid path")
        .interface(wgaf_common::WINDOWS_INTERFACE_NAME)
        .expect("valid interface")
        .build();
    let mut events = zbus::MessageStream::for_match_rule(rule, &connection, None)
        .await
        .expect("subscribe to the daemon's window signals");

    let emitter =
        zbus::object_server::SignalEmitter::new(&stub, wgaf_common::EXTENSION_OBJECT_PATH)
            .expect("emitter for the stub's object path");

    let mut seen: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);

    while seen.len() < 3 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the daemon never re-emitted all three signals; saw {seen:?}"
        );

        // Re-emitted every round rather than once, because the daemon's
        // subscription is established by a retrying background task and a
        // signal sent before it subscribes is simply gone — D-Bus signals are
        // fire-and-forget, so there is nothing to replay.
        StubExtension::window_created(&emitter, canned_window().into())
            .await
            .expect("emit WindowCreated");
        StubExtension::window_closed(&emitter, 4242)
            .await
            .expect("emit WindowClosed");
        StubExtension::window_focus_changed(&emitter, 777)
            .await
            .expect("emit WindowFocusChanged");

        let collect_until = tokio::time::Instant::now() + std::time::Duration::from_millis(700);
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout_at(collect_until, events.next()).await
        {
            let Some(member) = message.header().member().map(|m| m.to_string()) else {
                continue;
            };
            if let Ok(id) = message.body().deserialize::<u32>() {
                seen.insert(member, id);
            }
        }
    }

    assert_eq!(
        seen.get("WindowCreated").copied(),
        Some(canned_window().id),
        "WindowCreated must carry the created window's id"
    );
    assert_eq!(seen.get("WindowClosed").copied(), Some(4242));
    assert_eq!(seen.get("WindowFocusChanged").copied(), Some(777));
}

/// A denied watch fails loudly instead of returning an empty stream.
///
/// ADR-0003 requires this and nothing else in the suite would catch it: a
/// `WatchWindows` that quietly succeeded under a `Deny` policy would leave the
/// caller subscribed to a feed that never carries anything, which on an idle
/// desktop is indistinguishable from a watch that is working.
#[tokio::test]
async fn a_denied_watch_is_refused_rather_than_silently_empty() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WatchDeny{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WatchDeny{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("watchdeny{pid}"),
        "[capabilities]\nWatchWindows = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_windows::<(), _>(&connection, &daemon_bus_name, "WatchWindows", &())
        .await
        .expect_err("a denied WatchWindows must fail");

    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED,
            "expected PermissionDenied, got {name}"
        ),
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

/// The same call succeeds when the policy allows it.
///
/// Paired with the test above deliberately: a `WatchWindows` that failed for
/// some unrelated reason — no extension, a typo in the method name — would
/// satisfy the denial test on its own while proving nothing about the policy.
#[tokio::test]
async fn an_allowed_watch_is_accepted() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WatchAllow{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WatchAllow{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("watchallow{pid}"),
        "[capabilities]\nWatchWindows = \"Allow\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    call_windows::<(), _>(&connection, &daemon_bus_name, "WatchWindows", &())
        .await
        .expect("an allowed WatchWindows should succeed");
}

#[tokio::test]
async fn focus_known_window_succeeds_via_stub() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.Focus{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.Focus{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("focus{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    call_windows::<(), _>(&connection, &daemon_bus_name, "FocusWindow", &(1u32,))
        .await
        .expect("focusing the known window should succeed");
}

#[tokio::test]
async fn unknown_window_id_reports_window_not_found() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.NotFound{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.NotFound{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("notfound{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_windows::<(), _>(&connection, &daemon_bus_name, "FocusWindow", &(999u32,))
        .await
        .expect_err("focusing an unknown window should fail");

    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.as_str(), wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND);
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_extension_reports_extension_unavailable() {
    let pid = std::process::id();
    // Deliberately never started — nobody owns this bus name.
    let extension_bus_name = format!("org.wgaf.Test.Extension.Missing{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.Missing{pid}");

    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("missing{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err =
        call_windows::<Vec<WindowRecordDict>, _>(&connection, &daemon_bus_name, "ListWindows", &())
            .await
            .expect_err("ListWindows should fail when the extension isn't running");

    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(
                name.as_str(),
                wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE
            );
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

// --- Workspace operations (W18.2) ------------------------------------------

/// Asserts a D-Bus failure carries the expected named error, returning its
/// description so a test can also check the wording reached the caller.
fn assert_named_error(err: zbus::Error, expected: &str) -> String {
    match err {
        zbus::Error::MethodError(name, description, _) => {
            assert_eq!(name.as_str(), expected);
            description.unwrap_or_default()
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

/// Switching, adding and removing all take effect, and each is visible to the
/// *next* call rather than only reported as having succeeded.
///
/// Reading the state back through `GetWorkspaces`/`GetWorkspaceLayout` after
/// each mutation is the point: a method that replied successfully without
/// changing anything would pass an assertion on its own return value, and this
/// is the defect the extension's confirm-before-replying design exists to
/// prevent.
#[tokio::test]
async fn workspace_mutations_are_visible_to_the_next_call() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WsMutate{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WsMutate{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("wsmutate{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let layout = |connection: zbus::Connection, bus: String| async move {
        let dict: WorkspaceLayoutDict = call_windows(&connection, &bus, "GetWorkspaceLayout", &())
            .await
            .expect("GetWorkspaceLayout should succeed");
        WorkspaceLayout::from(dict)
    };

    let before = layout(connection.clone(), daemon_bus_name.clone()).await;
    assert_eq!(before.n_workspaces, 1);
    assert_eq!(before.active, 0);

    let added: i32 = call_windows(&connection, &daemon_bus_name, "AddWorkspace", &())
        .await
        .expect("AddWorkspace should succeed");
    assert_eq!(added, 1, "the appended workspace's index is reported");
    assert_eq!(
        layout(connection.clone(), daemon_bus_name.clone())
            .await
            .n_workspaces,
        2,
        "the added workspace must be visible to the next call"
    );

    call_windows::<(), _>(&connection, &daemon_bus_name, "SwitchWorkspace", &(1i32,))
        .await
        .expect("SwitchWorkspace should succeed");
    assert_eq!(
        layout(connection.clone(), daemon_bus_name.clone())
            .await
            .active,
        1,
        "SwitchWorkspace returning Ok must mean the workspace is already active"
    );

    // Reordering within bounds is accepted; the stub has nothing to permute,
    // so this covers the call and its validation rather than Mutter's shuffle.
    call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "ReorderWorkspace",
        &(1i32, 0i32),
    )
    .await
    .expect("ReorderWorkspace should succeed for two valid indices");

    call_windows::<(), _>(&connection, &daemon_bus_name, "RemoveWorkspace", &(1i32,))
        .await
        .expect("RemoveWorkspace should succeed");
    let after = layout(connection.clone(), daemon_bus_name.clone()).await;
    assert_eq!(after.n_workspaces, 1);
    assert_eq!(
        after.active, 0,
        "removing the active workspace must leave a valid one active"
    );
}

/// An index that does not exist is `WorkspaceNotFound`, on every method that
/// takes one — including both of `ReorderWorkspace`'s.
///
/// The daemon does not know how many workspaces exist; the extension raises
/// this and the daemon translates it. Covering every method means a future one
/// added without the translation fails here rather than leaking the
/// extension's own error name to clients.
#[tokio::test]
async fn an_unknown_workspace_index_reports_workspace_not_found() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WsMissing{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WsMissing{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("wsmissing{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // The stub starts with exactly one workspace, so 7 and -1 are both absent.
    for (method, args) in [
        ("SwitchWorkspace", vec![7i32]),
        ("RemoveWorkspace", vec![7i32]),
        // The index being moved...
        ("ReorderWorkspace", vec![7i32, 0i32]),
        // ...and the destination, which is just as invalid a request.
        ("ReorderWorkspace", vec![0i32, 7i32]),
        ("SwitchWorkspace", vec![-1i32]),
    ] {
        let err = match args.len() {
            1 => call_windows::<(), _>(&connection, &daemon_bus_name, method, &(args[0],)).await,
            _ => {
                call_windows::<(), _>(&connection, &daemon_bus_name, method, &(args[0], args[1]))
                    .await
            }
        }
        .expect_err(&format!("{method}{args:?} should be rejected"));

        assert_named_error(err, wgaf_common::WINDOWS_ERROR_WORKSPACE_NOT_FOUND);
    }
}

/// Removing the last remaining workspace is refused by name, and the refusal
/// says why.
///
/// Mutter declines this silently, so without the extension's explicit check a
/// caller would get a successful reply and an unchanged desktop. That it comes
/// back as `OperationNotApplied` rather than an error is the ADR-0007
/// classification: nothing malfunctioned and no policy refused — the state
/// simply did not change.
#[tokio::test]
async fn removing_the_last_workspace_is_refused_and_says_why() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WsLast{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WsLast{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("wslast{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_windows::<(), _>(&connection, &daemon_bus_name, "RemoveWorkspace", &(0i32,))
        .await
        .expect_err("removing the only workspace must be refused");

    let description = assert_named_error(err, wgaf_common::WINDOWS_ERROR_OPERATION_NOT_APPLIED);
    assert!(
        description.contains("last workspace"),
        "the extension's own explanation must survive translation, got: {description}"
    );

    // And the desktop is as it was — the refusal did not half-apply.
    let dict: WorkspaceLayoutDict =
        call_windows(&connection, &daemon_bus_name, "GetWorkspaceLayout", &())
            .await
            .expect("GetWorkspaceLayout should succeed");
    assert_eq!(WorkspaceLayout::from(dict).n_workspaces, 1);
}

/// Each workspace mutation is gated by its own capability, and denying one
/// leaves the others working.
///
/// The point of four variants rather than one `ManageWorkspaces`: an operator
/// can let automation move around their desktop without letting it rearrange
/// the session. A single capability could not express that, and this is what
/// proves the split is real rather than decorative.
#[tokio::test]
async fn each_workspace_capability_is_gated_separately() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WsDeny{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WsDeny{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("wsdeny{pid}"),
        // Rearranging denied, navigating allowed.
        "[capabilities]\nAddWorkspace = \"Deny\"\nRemoveWorkspace = \"Deny\"\n\
         ReorderWorkspace = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    for (method, args) in [
        ("AddWorkspace", vec![]),
        ("RemoveWorkspace", vec![0i32]),
        ("ReorderWorkspace", vec![0i32, 0i32]),
    ] {
        let err = match args.len() {
            0 => call_windows::<i32, _>(&connection, &daemon_bus_name, method, &())
                .await
                .map(|_| ()),
            1 => call_windows::<(), _>(&connection, &daemon_bus_name, method, &(args[0],)).await,
            _ => {
                call_windows::<(), _>(&connection, &daemon_bus_name, method, &(args[0], args[1]))
                    .await
            }
        }
        .expect_err(&format!("{method} is denied by this policy"));

        assert_named_error(err, wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED);
    }

    // The one that was not denied still works, so the policy is doing exactly
    // what it says and not simply refusing everything.
    call_windows::<(), _>(&connection, &daemon_bus_name, "SwitchWorkspace", &(0i32,))
        .await
        .expect("SwitchWorkspace is not denied by this policy");
}

/// Read-only workspace calls are never gated, matching `ListWindows` and
/// `GetWorkspaces`, and they keep working under the strictest policy.
#[tokio::test]
async fn reading_the_workspace_layout_is_not_gated() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.WsRead{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.WsRead{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("wsread{pid}"),
        "[capabilities]\nSwitchWorkspace = \"Deny\"\nAddWorkspace = \"Deny\"\n\
         RemoveWorkspace = \"Deny\"\nReorderWorkspace = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let dict: WorkspaceLayoutDict =
        call_windows(&connection, &daemon_bus_name, "GetWorkspaceLayout", &())
            .await
            .expect("reading the layout must not need a capability");
    let layout = WorkspaceLayout::from(dict);
    assert_eq!(layout.n_workspaces, 1);
    assert_eq!(layout.columns, 1);
}

/// Asserts a call failed with a particular named D-Bus error.
///
/// The three `MoveWindowToWorkspace` tests below each check a different error
/// name from the same method, which is exactly the case where hand-written
/// `match` blocks drift into saying different things about the same failure.
/// `what` names the case so a failure says which of them broke.
fn assert_error_name(err: zbus::Error, expected: &str, what: &str) {
    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            expected,
            "{what}: expected {expected}, got {name}"
        ),
        other => panic!("{what}: expected a MethodError, got {other:?}"),
    }
}

/// `MoveWindowToWorkspace` names *which* argument was wrong.
///
/// It is the only method on this interface taking both a window id and a
/// workspace index, so it is the only one where a caller can be wrong in two
/// ways — and a single generic failure would leave a script guessing which. The
/// daemon has to translate two different extension errors coming back from one
/// call, which is a translation nothing else exercises.
#[tokio::test]
async fn moving_a_window_to_a_workspace_distinguishes_a_bad_window_from_a_bad_workspace() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.MoveWs{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.MoveWs{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("movews{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // Both arguments valid: the canned window, and workspace 0.
    call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "MoveWindowToWorkspace",
        &(canned_window().id, 0i32),
    )
    .await
    .expect("a known window and a known workspace should succeed");

    let unknown_window = canned_window().id + 999;
    let err = call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "MoveWindowToWorkspace",
        &(unknown_window, 0i32),
    )
    .await
    .expect_err("an unknown window id must be refused");
    assert_error_name(
        err,
        wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND,
        "bad window",
    );

    let err = call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "MoveWindowToWorkspace",
        &(canned_window().id, 7i32),
    )
    .await
    .expect_err("an unknown workspace index must be refused");
    assert_error_name(
        err,
        wgaf_common::WINDOWS_ERROR_WORKSPACE_NOT_FOUND,
        "bad workspace",
    );
}

/// `MoveWindowToWorkspace` is gated by its own capability, not by the
/// workspace ones.
///
/// Denying every workspace capability must leave it working: it changes neither
/// how many workspaces exist nor which is active. An operator who denied
/// "rearrange my workspaces" has not asked for "never move a window between
/// them", and folding the two together would take away a capability nobody
/// refused.
#[tokio::test]
async fn moving_a_window_between_workspaces_is_gated_separately_from_the_workspaces_themselves() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.MoveWsPerm{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.MoveWsPerm{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("movewsperm{pid}"),
        "[capabilities]\nSwitchWorkspace = \"Deny\"\nAddWorkspace = \"Deny\"\n\
         RemoveWorkspace = \"Deny\"\nReorderWorkspace = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "MoveWindowToWorkspace",
        &(canned_window().id, 0i32),
    )
    .await
    .expect("MoveWindowToWorkspace is not denied by a workspace-only policy");

    // And the reverse, so this is not passing because the policy was ignored.
    call_windows::<(), _>(&connection, &daemon_bus_name, "SwitchWorkspace", &(0i32,))
        .await
        .expect_err("SwitchWorkspace must still be denied");
}

/// Denying it specifically must refuse it.
#[tokio::test]
async fn a_denied_move_to_workspace_is_refused() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.MoveWsDeny{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.MoveWsDeny{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &extension_bus_name,
        &format!("movewsdeny{pid}"),
        "[capabilities]\nMoveWindowToWorkspace = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_windows::<(), _>(
        &connection,
        &daemon_bus_name,
        "MoveWindowToWorkspace",
        &(canned_window().id, 0i32),
    )
    .await
    .expect_err("a denied MoveWindowToWorkspace must fail");
    assert_error_name(
        err,
        wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED,
        "denied move",
    );
}

// --- Monitor listing (W18.2) -----------------------------------------------

/// The layout the daemon has been reading privately since W5 comes back out
/// intact, in the same logical coordinates every other wgaf command uses.
///
/// The rotated monitor is the assertion that matters. `HDMI-1`'s *mode* is
/// 1920x1080; it reports 1080x1920 only if the daemon applied the 90-degree
/// transform, and reporting the mode size instead would tell a user the pointer
/// can go to (1500, 500) on a monitor that is 1080 pixels wide.
#[tokio::test]
async fn get_monitors_reports_the_layout_in_logical_coordinates() {
    let (_daemon, connection, bus) = pointer_test_daemon("Monitors").await;

    let dicts: Vec<MonitorRecordDict> = call_windows(&connection, &bus, "GetMonitors", &())
        .await
        .expect("GetMonitors should succeed against the stub display config");
    let monitors: Vec<MonitorRecord> = dicts.into_iter().map(Into::into).collect();

    assert_eq!(monitors.len(), 2, "both stub monitors should be reported");

    let rotated = &monitors[0];
    assert_eq!(rotated.connector, "HDMI-1");
    assert_eq!((rotated.x, rotated.y), (0, 0));
    assert_eq!(
        (rotated.width, rotated.height),
        (1080, 1920),
        "the 90-degree transform must be applied to the mode size"
    );
    assert_eq!(rotated.transform, 1);
    assert!(!rotated.primary);

    let primary = &monitors[1];
    assert_eq!(primary.connector, "DP-3");
    assert_eq!((primary.x, primary.y), (1080, 0));
    assert_eq!((primary.width, primary.height), (2560, 1440));
    assert_eq!(primary.transform, 0);
    assert!(primary.primary);
}

/// Every rectangle `GetMonitors` reports must be one `MouseMoveAbsolute`
/// accepts, and the gap between them must be refused by both.
///
/// The two answers come from the same `MonitorLayout`, so this is not testing
/// the arithmetic twice — it is testing that the *reported* bounds are the
/// *enforced* bounds. A monitor list that disagreed with the bounds check would
/// be worse than no list: it would tell a user exactly where to aim and then
/// refuse them.
#[tokio::test]
async fn the_reported_bounds_are_the_bounds_that_are_enforced() {
    let (_daemon, connection, bus) = pointer_test_daemon("MonitorsAgree").await;

    let dicts: Vec<MonitorRecordDict> = call_windows(&connection, &bus, "GetMonitors", &())
        .await
        .expect("GetMonitors should succeed");
    let monitors: Vec<MonitorRecord> = dicts.into_iter().map(Into::into).collect();

    for m in &monitors {
        // Half-open on the far edges, so the last addressable pixel is one
        // short of the reported size — the same convention `MonitorRect`
        // documents.
        for (x, y) in [
            (m.x, m.y),
            (m.x + m.width - 1, m.y + m.height - 1),
            (m.x + m.width / 2, m.y + m.height / 2),
        ] {
            let landed: (i32, i32) = call_input(&connection, &bus, "MouseMoveAbsolute", &(x, y))
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "({x}, {y}) is inside {}'s reported {}x{} rectangle at ({}, {}) but was \
                         refused: {e}",
                        m.connector, m.width, m.height, m.x, m.y
                    )
                });
            assert_eq!(landed, (x, y));
        }
    }

    // The notch: inside the layout's bounding box, on neither reported
    // rectangle, and refused. If a future change merged the monitors into one
    // bounding box, the loop above would still pass and this would not.
    assert!(
        !monitors
            .iter()
            .any(|m| 2000 >= m.x && 2000 < m.x + m.width && 1700 >= m.y && 1700 < m.y + m.height),
        "(2000, 1700) must be on no reported monitor"
    );
    call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(2000i32, 1700i32))
        .await
        .expect_err("a coordinate on no reported monitor must be refused");
}

/// The work area is joined onto the right monitor by geometry, and a monitor
/// nothing described gets no work area rather than someone else's.
///
/// The stub reports two entries: one whose rectangle is exactly `DP-3`'s, and
/// one matching nothing. The failure this guards against is the tempting
/// implementation — pairing the two lists by position — which would hand
/// `HDMI-1` the leftover 9000,9000 rectangle and look entirely plausible in the
/// output.
#[tokio::test]
async fn work_areas_are_matched_to_monitors_by_geometry() {
    let (_daemon, connection, bus) = pointer_test_daemon("WorkAreas").await;

    let dicts: Vec<MonitorRecordDict> = call_windows(&connection, &bus, "GetMonitors", &())
        .await
        .expect("GetMonitors should succeed");
    let monitors: Vec<MonitorRecord> = dicts.into_iter().map(Into::into).collect();

    let dp3 = monitors
        .iter()
        .find(|m| m.connector == "DP-3")
        .expect("DP-3 should be reported");
    let work_area = dp3
        .work_area
        .expect("DP-3's rectangle is described exactly, so its work area must be found");
    assert_eq!(work_area.x, DP3_RECT.0);
    assert_eq!(work_area.y, DP3_RECT.1 + TOP_BAR_HEIGHT);
    assert_eq!(work_area.width, DP3_RECT.2);
    assert_eq!(work_area.height, DP3_RECT.3 - TOP_BAR_HEIGHT);
    assert_ne!(
        (work_area.x, work_area.y, work_area.width, work_area.height),
        (dp3.x, dp3.y, dp3.width, dp3.height),
        "the work area must not simply be the monitor's own geometry"
    );

    let hdmi1 = monitors
        .iter()
        .find(|m| m.connector == "HDMI-1")
        .expect("HDMI-1 should be reported");
    assert_eq!(
        hdmi1.work_area, None,
        "no reported work area describes HDMI-1's rectangle, so its usable area is unknown — \
         it must not inherit the unmatched entry"
    );
}

/// `GetMonitors` answers with no extension on the bus at all.
///
/// This is the property that made `display_config.rs` read Mutter directly
/// instead of adding a third extension method, and it is worth a test rather
/// than a comment: a user whose extension is not installed still gets a useful
/// answer to "where are my screens", and the failure they see for
/// `wgaf window list` does not spread to this command.
#[tokio::test]
async fn get_monitors_works_without_the_extension() {
    let pid = std::process::id();
    // Deliberately never started — nobody owns this bus name.
    let extension_bus_name = format!("org.wgaf.Test.Extension.NoExtMonitors{pid}");
    let display_config_bus_name = format!("org.wgaf.Test.DisplayConfig.NoExtMonitors{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.NoExtMonitors{pid}");

    start_stub_display_config(&display_config_bus_name).await;
    let _daemon = spawn_daemon_with_display_config(
        &daemon_bus_name,
        &extension_bus_name,
        Some(&display_config_bus_name),
        &format!("noextmonitors{pid}"),
        ALLOW_ALL,
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // The premise: this session genuinely has no extension.
    call_windows::<Vec<WindowRecordDict>, _>(&connection, &daemon_bus_name, "ListWindows", &())
        .await
        .expect_err("ListWindows must fail without the extension, or this test proves nothing");

    let dicts: Vec<MonitorRecordDict> =
        call_windows(&connection, &daemon_bus_name, "GetMonitors", &())
            .await
            .expect("GetMonitors must not need the extension");
    let monitors: Vec<MonitorRecord> = dicts.into_iter().map(Into::into).collect();
    assert_eq!(monitors.len(), 2);

    // The one thing that genuinely does need the extension degrades to
    // "unknown" rather than to a guess or to a failed call.
    for m in &monitors {
        assert_eq!(
            m.work_area, None,
            "{}'s usable area cannot be known without the extension, and must not be \
             defaulted to its full geometry",
            m.connector
        );
    }
}

/// A missing display configuration is reported as its own named error, not as
/// a missing extension and not as a bare `org.freedesktop.DBus.Error.Failed`.
///
/// A script that cannot tell these apart tells the user to install an
/// extension when the real problem is that they are not on a GNOME session.
#[tokio::test]
async fn a_missing_display_config_reports_monitor_layout_unavailable() {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.NoLayout{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.NoLayout{pid}");
    // Deliberately never started, so the daemon's layout lookup finds no owner.
    let display_config_bus_name = format!("org.wgaf.Test.DisplayConfig.NoLayout{pid}");

    start_stub_extension(&extension_bus_name).await;
    let _daemon = spawn_daemon_with_display_config(
        &daemon_bus_name,
        &extension_bus_name,
        Some(&display_config_bus_name),
        &format!("nolayout{pid}"),
        ALLOW_ALL,
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_windows::<Vec<MonitorRecordDict>, _>(
        &connection,
        &daemon_bus_name,
        "GetMonitors",
        &(),
    )
    .await
    .expect_err("GetMonitors should fail when no display configuration is on the bus");

    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_MONITOR_LAYOUT_UNAVAILABLE,
            "a missing layout must not be reported as a missing extension"
        ),
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

// --- Absolute pointer positioning (W5) -------------------------------------
//
// These call `org.wgaf.Input1`, not `org.wgaf.Windows1`, even though they live
// in this file: absolute positioning is served by `WindowManager` through the
// extension, but it is presented to users as a mouse operation. The stubs it
// needs are here, so the tests are too.
//
// Nothing in this section synthesizes real input. `MouseMoveAbsolute` never
// touches `/dev/uinput` — it goes to the extension — so unlike `tests/input.rs`
// these are safe to run on a developer's desktop, and the stub means no pointer
// moves anywhere at all.

async fn call_input<R, A>(
    connection: &zbus::Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            method,
            args,
        )
        .await?;
    reply.body().deserialize()
}

/// Spawns a daemon wired to both stubs, on names unique to `label`.
async fn pointer_test_daemon(label: &str) -> (DaemonGuard, zbus::Connection, String) {
    let pid = std::process::id();
    let extension_bus_name = format!("org.wgaf.Test.Extension.{label}{pid}");
    let display_config_bus_name = format!("org.wgaf.Test.DisplayConfig.{label}{pid}");
    let daemon_bus_name = format!("org.wgaf.Test.Daemon.{label}{pid}");

    start_stub_extension(&extension_bus_name).await;
    start_stub_display_config(&display_config_bus_name).await;

    let daemon = spawn_daemon_with_display_config(
        &daemon_bus_name,
        &extension_bus_name,
        Some(&display_config_bus_name),
        &format!("{label}{pid}"),
        ALLOW_ALL,
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;
    (daemon, connection, daemon_bus_name)
}

#[tokio::test]
async fn absolute_move_reports_where_the_pointer_landed() {
    let (_daemon, connection, bus) = pointer_test_daemon("PtrOk").await;

    let landed: (i32, i32) = call_input(&connection, &bus, "MouseMoveAbsolute", &(1500i32, 700i32))
        .await
        .expect("a coordinate on the primary monitor should be accepted");
    assert_eq!(landed, (1500, 700));

    // The reply is the position reached, and a subsequent read must agree with
    // it. On the real extension these are two different mechanisms — the warp's
    // own confirmation and a fresh query — so agreement is worth asserting.
    let read_back: (i32, i32) = call_input(&connection, &bus, "GetPointerPosition", &())
        .await
        .expect("GetPointerPosition should succeed");
    assert_eq!(read_back, (1500, 700));
}

/// The rotated monitor sits at the origin and is *taller* than it is wide, which
/// is only true if the daemon applied the 90-degree transform to the mode size.
/// Without that it would compute 1920x1080 and reject this as off-screen.
#[tokio::test]
async fn a_rotated_monitors_bounds_use_its_rotated_size() {
    let (_daemon, connection, bus) = pointer_test_daemon("PtrRot").await;

    let landed: (i32, i32) = call_input(&connection, &bus, "MouseMoveAbsolute", &(500i32, 1700i32))
        .await
        .expect("(500,1700) is on the rotated monitor and must be accepted");
    assert_eq!(landed, (500, 1700));

    // 1500 is beyond the rotated monitor's width, and below the primary's
    // height — off-screen despite both numbers looking individually plausible.
    let err =
        call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(1500i32, 1700i32))
            .await
            .expect_err("(1500,1700) is on neither monitor");
    assert_out_of_bounds(err);
}

/// The case the whole bounds check exists for: a coordinate inside the layout's
/// bounding box and on no monitor. A bounding-box check accepts it, and the
/// compositor would then silently clamp the pointer somewhere else.
#[tokio::test]
async fn the_notch_between_two_monitors_is_refused() {
    let (_daemon, connection, bus) = pointer_test_daemon("PtrNotch").await;

    let err =
        call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(2000i32, 1700i32))
            .await
            .expect_err("(2000,1700) is inside the bounding box but on no monitor");
    let description = assert_out_of_bounds(err);

    // The message has to be actionable: a user told only "out of bounds" has to
    // go and work out what the bounds were.
    assert!(
        description.contains("HDMI-1") && description.contains("DP-3"),
        "the error should name the monitors, got: {description}"
    );
}

/// Nothing may move when the coordinate is refused. If the daemon warped first
/// and validated afterwards, this would still pass its error assertion while
/// having already moved the pointer — so the position is checked too.
#[tokio::test]
async fn a_refused_move_leaves_the_pointer_where_it_was() {
    let (_daemon, connection, bus) = pointer_test_daemon("PtrIntact").await;

    call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(1200i32, 400i32))
        .await
        .expect("setup move should succeed");

    let err =
        call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(9999i32, 9999i32))
            .await
            .expect_err("(9999,9999) is off every monitor");
    assert_out_of_bounds(err);

    let position: (i32, i32) = call_input(&connection, &bus, "GetPointerPosition", &())
        .await
        .expect("GetPointerPosition should succeed");
    assert_eq!(
        position,
        (1200, 400),
        "a refused move must not have moved the pointer"
    );
}

/// Edge coordinates are the ones an off-by-one gets wrong, and both directions
/// matter: the last pixel of a monitor must be reachable, and the first pixel
/// past it must not be.
#[tokio::test]
async fn monitor_edges_are_half_open() {
    let (_daemon, connection, bus) = pointer_test_daemon("PtrEdge").await;

    for (x, y) in [(0i32, 0i32), (1079, 1919), (1080, 0), (3639, 1439)] {
        let landed: (i32, i32) = call_input(&connection, &bus, "MouseMoveAbsolute", &(x, y))
            .await
            .unwrap_or_else(|e| panic!("({x},{y}) is on a monitor and should be accepted: {e}"));
        assert_eq!(landed, (x, y));
    }

    for (x, y) in [(-1i32, 0i32), (0, 1920), (3640, 0), (1080, 1440)] {
        let err = call_input::<(i32, i32), _>(&connection, &bus, "MouseMoveAbsolute", &(x, y))
            .await
            .unwrap_err();
        assert_out_of_bounds(err);
    }
}

/// Asserts the error is `OutOfBounds` and returns its description.
fn assert_out_of_bounds(err: zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(name, description, _) => {
            assert_eq!(
                name.as_str(),
                wgaf_common::INPUT_ERROR_OUT_OF_BOUNDS,
                "expected OutOfBounds, got {name}"
            );
            description.unwrap_or_default()
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}
