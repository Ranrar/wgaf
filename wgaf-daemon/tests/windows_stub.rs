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

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};
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
#[derive(Default)]
struct StubExtension {
    pointer: std::sync::Mutex<(i32, i32)>,
}

#[interface(name = "org.gnome.Shell.Extensions.Wgaf.V1")]
impl StubExtension {
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
        vec![canned_workspace().into()]
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
}

// --- Stub for org.gnome.Mutter.DisplayConfig -------------------------------
//
// A second stub, because it is a different service. It exists at all because
// the real Mutter already owns `org.gnome.Mutter.DisplayConfig` on any live
// session, so nothing else can take that name — `Config::display_config_bus_name`
// is what lets a test daemon read its monitor layout from here instead.

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
                // put the tall monitor's bounds at 1920x1080.
                stub_monitor("HDMI-1", 1920, 1080),
                stub_monitor("DP-3", 2560, 1440),
            ],
            vec![
                stub_logical_monitor("HDMI-1", 0, 0, 1, false),
                stub_logical_monitor("DP-3", 1080, 0, 0, true),
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
    let connection = zbus::connection::Builder::session()
        .expect("session bus builder")
        .name(bus_name)
        .expect("valid bus name")
        .serve_at(wgaf_common::EXTENSION_OBJECT_PATH, StubExtension::default())
        .expect("serve stub extension")
        .build()
        .await
        .expect("stub extension registers on session bus");
    // Leak: keeps the stub alive for the rest of this test process. Each
    // test uses its own unique bus name, so leaking across tests in the
    // same binary is harmless.
    std::mem::forget(connection);
}

/// Spawns the real `wgaf-daemon` binary with a config pointing its own bus
/// name and its expected extension bus name at test-private, unique names.
fn spawn_daemon(daemon_bus_name: &str, extension_bus_name: &str, nonce: &str) -> DaemonGuard {
    spawn_daemon_with_display_config(daemon_bus_name, extension_bus_name, None, nonce)
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
    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-stub-permissions-{}.toml", std::process::id()));
    std::fs::write(&permissions_path, "[capabilities]\n").expect("failed to write test policy");
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
