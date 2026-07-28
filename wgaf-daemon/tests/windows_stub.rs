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
struct StubExtension;

#[interface(name = "org.gnome.Shell.Extensions.Wgaf.V1")]
impl StubExtension {
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

/// Starts the stub extension service on a private, unique bus name (so it
/// never collides with a real GNOME Shell Extension on a shared session
/// bus) and leaks the connection for the test's lifetime — the process
/// exiting cleans it up.
async fn start_stub_extension(bus_name: &str) {
    let connection = zbus::connection::Builder::session()
        .expect("session bus builder")
        .name(bus_name)
        .expect("valid bus name")
        .serve_at(wgaf_common::EXTENSION_OBJECT_PATH, StubExtension)
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
    let config_path = std::env::temp_dir().join(format!("wgaf-daemon-windows-test-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{daemon_bus_name}\"\nlog_level = \"error\"\nextension_bus_name = \"{extension_bus_name}\"\n"
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
