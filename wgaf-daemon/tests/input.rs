//! Integration tests for the daemon's `org.wgaf.Input1` D-Bus API.
//!
//! Unlike `windows_stub.rs` (which has to fake the GNOME Shell
//! Extension because no real GNOME Shell session is available in this
//! sandbox), `/dev/uinput` in this environment is confirmed writable by the
//! current user (a POSIX ACL grants `rw-` directly), so these tests
//! exercise a **real** `uinput` virtual device created by a **real**
//! spawned `wgaf-daemon` process, not a stub.
//!
//! Each test gives its spawned daemon a unique `input_device_name` (via
//! `Config::input_device_name`, see `wgaf-daemon/src/config.rs`) rather than
//! the real production name — `/proc/bus/input/devices` is a machine-global
//! namespace with no notion of "which process created this device", and
//! these `#[tokio::test]`s run concurrently by default, so without a unique
//! name per test one test's device would be indistinguishable from
//! another's (or from a real daemon's, if one happens to be running).
//!
//! What is verified for real:
//!   - The daemon's `org.wgaf.Input1` methods (`TypeText`, `KeyPress`/
//!     `KeyRelease`, `MouseMove`, `MouseClick`, `MouseScroll`) all complete
//!     without error against the real device — if any `ioctl`/`write` call
//!     in `input/device.rs` failed, these would surface as D-Bus errors.
//!   - The virtual device genuinely registers with the kernel: it appears
//!     in `/proc/bus/input/devices` under its own (test-unique) name with
//!     the expected `EV`/`KEY`/`REL` capability bits, for as long as the
//!     daemon process is alive, and is torn down (disappears from
//!     `/proc/bus/input/devices`) once the daemon exits — proving
//!     `UI_DEV_CREATE`/`UI_DEV_DESTROY` are both actually taking effect in
//!     the kernel, not just returning success from a mocked layer.
//!
//! What is NOT verified here (a documented gap, not silently skipped):
//! reading back the individual synthesized `struct input_event`s from the
//! device's own `/dev/input/eventN` node. This was investigated directly —
//! the resulting event node is created with the standard udev-managed
//! permissions (`root:input`, mode `0660`), and this sandbox's user is
//! *not* a member of the `input` group
//! (only `/dev/uinput` itself has a one-off ACL granting this user direct
//! access, for creating the device — not for reading back the events of
//! devices it creates). Opening `/dev/input/eventN` for reading fails with
//! `EACCES` as a result, confirmed by direct experiment. Per-event
//! readback would need either `input` group membership or a VM/CI runner
//! configured for it — out of scope for this sandboxed test run, same
//! category of gap as `windows_stub.rs`'s "no real GNOME Shell session"
//! limitation.

use std::process::{Child, Command};
use std::time::Duration;

use zbus::Connection;

/// Kills the spawned daemon even if an assertion panics mid-test, and keeps
/// its config file alive until then (removing it earlier would race the
/// child's own async `Config::load`, per the existing convention in
/// `windows_stub.rs`/`ping.rs`).
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

/// Spawns the real `wgaf-daemon` binary with a test-private bus name and a
/// test-private, unique `input_device_name` (see module docs for why the
/// latter matters).
fn spawn_daemon(daemon_bus_name: &str, device_name: &str, nonce: &str) -> DaemonGuard {
    spawn_daemon_with_config(daemon_bus_name, device_name, nonce, "")
}

/// As [`spawn_daemon`], plus `extra_config` appended verbatim to the
/// generated `config.toml` — for tests that need a non-default setting.
fn spawn_daemon_with_config(
    daemon_bus_name: &str,
    device_name: &str,
    nonce: &str,
    extra_config: &str,
) -> DaemonGuard {
    let config_path = std::env::temp_dir().join(format!("wgaf-daemon-input-test-{nonce}.toml"));
    // `extension_bus_name` is deliberately left at the default — these tests
    // never touch `org.wgaf.Windows1`, so it doesn't matter whether the
    // (nonexistent, in this sandbox) extension is reachable.
    // `input_device_settle_ms = 0` is a **safety setting here, not a
    // performance one**, and must not be removed without reading this.
    //
    // These tests synthesize real keystrokes through a real kernel device.
    // They verify that synthesis reaches the *kernel*; they do not verify that
    // anything receives it, and they have no window of their own to aim at. So
    // if the device is live, every character they type lands in whatever window
    // currently has focus on the developer's desktop — and one of them types
    // 4096 characters with the rate limiter switched off.
    //
    // The settle wait exists so the first real command is not discarded (see
    // `input::DEFAULT_DEVICE_SETTLE_MS`). Switching it off restores, for these
    // tests only, the property that the events go nowhere: the device is
    // created and written to before udev has published it and the compositor
    // has opened it. That is exactly what this suite wants and exactly what a
    // user does not.
    //
    // **This is a mitigation, not a guarantee.** It is still a race, so a slow
    // enough test run could still deliver. The real fix is to run this suite
    // against a nested compositor rather than the developer's own session; the
    // day that exists, this line should be reconsidered rather than kept out of
    // habit.
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{daemon_bus_name}\"\nlog_level = \"error\"\ninput_device_name = \"{device_name}\"\ninput_device_settle_ms = 0\n{extra_config}"
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
    let permissions_path = std::env::temp_dir().join(format!(
        "wgaf-input-permissions-{}.toml",
        std::process::id()
    ));
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

async fn wait_for_daemon(bus_name: &str) -> Connection {
    let connection = Connection::session().await.expect("connect to session bus");
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

async fn call_input<R, A>(
    connection: &Connection,
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

/// Finds the virtual device block in `/proc/bus/input/devices` (blocks are
/// blank-line-separated) whose name is exactly `device_name` — the
/// test-unique name `spawn_daemon` gave this particular test's daemon (see
/// module docs), so unlike a fixed name this is safe to use for "does *my*
/// device exist" assertions even with other tests' daemons running
/// concurrently.
fn find_device_block(device_name: &str) -> Option<String> {
    let data = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    let needle = format!("Name=\"{device_name}\"");
    data.split("\n\n")
        .find(|block| block.contains(&needle))
        .map(|s| s.to_string())
}

async fn poll_until<F: Fn() -> bool>(predicate: F, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if predicate() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn input_methods_succeed_against_a_real_uinput_device() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.Ok{pid}");
    let device_name = format!("wgaf test device ok {pid}");

    let _daemon = spawn_daemon(&daemon_bus_name, &device_name, &format!("ok{pid}"));
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // The device is created lazily on first use (see
    // `InputBackend::device`), so it shouldn't exist yet.
    assert!(
        find_device_block(&device_name).is_none(),
        "virtual device should not exist before any Input1 call"
    );

    // ...calling any Input1 method creates it.
    call_input::<(), _>(&connection, &daemon_bus_name, "MouseMove", &(5i32, 7i32))
        .await
        .expect("MouseMove should succeed against the real uinput device");

    let created = poll_until(
        || find_device_block(&device_name).is_some(),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        created,
        "virtual device should appear in /proc/bus/input/devices"
    );

    let block = find_device_block(&device_name).expect("device block");
    // EV=7 is EV_SYN(1) | EV_KEY(2) | EV_REL(4) as a bitmask — confirms the
    // device actually advertises both keyboard and relative-pointer
    // capabilities to the kernel, not just "some device with this name".
    assert!(
        block.contains("EV=7"),
        "expected EV=7 (SYN|KEY|REL) in device block:\n{block}"
    );
    assert!(
        block
            .lines()
            .any(|l| l.starts_with('H') && l.contains("event")),
        "expected an eventN handler in device block:\n{block}"
    );

    // Exercise every remaining Input1 method for real.
    call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &("ab1!",))
        .await
        .expect("TypeText should succeed");
    call_input::<(), _>(&connection, &daemon_bus_name, "KeyPress", &("a",))
        .await
        .expect("KeyPress should succeed");
    call_input::<(), _>(&connection, &daemon_bus_name, "KeyRelease", &("a",))
        .await
        .expect("KeyRelease should succeed");
    call_input::<(), _>(&connection, &daemon_bus_name, "MouseClick", &("left",))
        .await
        .expect("MouseClick should succeed");
    call_input::<(), _>(&connection, &daemon_bus_name, "MouseScroll", &(0i32, 1i32))
        .await
        .expect("MouseScroll should succeed");
}

#[tokio::test]
async fn virtual_device_is_destroyed_when_daemon_exits() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.Teardown{pid}");
    let device_name = format!("wgaf test device teardown {pid}");

    let daemon = spawn_daemon(&daemon_bus_name, &device_name, &format!("teardown{pid}"));
    let connection = wait_for_daemon(&daemon_bus_name).await;

    call_input::<(), _>(&connection, &daemon_bus_name, "MouseMove", &(1i32, 1i32))
        .await
        .expect("MouseMove should succeed");
    assert!(
        find_device_block(&device_name).is_some(),
        "device should be registered while the daemon is running"
    );

    drop(daemon); // kills the child, which drops UinputDevice -> UI_DEV_DESTROY

    let destroyed = poll_until(
        || find_device_block(&device_name).is_none(),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        destroyed,
        "device should disappear from /proc/bus/input/devices after the daemon exits"
    );
}

#[tokio::test]
async fn unknown_key_reports_unknown_key_error() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.UnknownKey{pid}");
    let device_name = format!("wgaf test device unknownkey {pid}");

    let _daemon = spawn_daemon(&daemon_bus_name, &device_name, &format!("unknownkey{pid}"));
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_input::<(), _>(
        &connection,
        &daemon_bus_name,
        "KeyPress",
        &("not_a_real_key",),
    )
    .await
    .expect_err("pressing an unknown key should fail");

    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.as_str(), wgaf_common::INPUT_ERROR_UNKNOWN_KEY);
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_mouse_button_reports_invalid_button_error() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.InvalidButton{pid}");
    let device_name = format!("wgaf test device invalidbutton {pid}");

    let _daemon = spawn_daemon(
        &daemon_bus_name,
        &device_name,
        &format!("invalidbutton{pid}"),
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let err = call_input::<(), _>(&connection, &daemon_bus_name, "MouseClick", &("nope",))
        .await
        .expect_err("clicking an invalid button should fail");

    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.as_str(), wgaf_common::INPUT_ERROR_INVALID_BUTTON);
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }
}

/// The runaway ceiling: a call whose backlog exceeds `MAX_THROTTLE_DELAY` is
/// refused with a named error rather than throttled.
///
/// **This test needs no `/dev/uinput`, deliberately.** The rate-limit check
/// in `InputBackend::run` happens *before* the device is resolved, so the
/// refusal never touches the kernel. It lives here for topical reasons rather
/// than environmental ones — if this suite is ever split by environment (see
/// the S4 preconditions issue), this test belongs on the device-free side.
///
/// It is also deterministic and instant: at one event per second, a
/// 4096-character `TypeText` is 8,192 events, so the projected wait is over
/// two hours and the refusal is immediate. Nothing here sleeps or races.
#[tokio::test]
async fn a_runaway_flood_is_refused_with_the_rate_limited_error() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.RateLimit{pid}");
    let device_name = format!("wgaf test device ratelimit {pid}");

    let _daemon = spawn_daemon_with_config(
        &daemon_bus_name,
        &device_name,
        &format!("ratelimit{pid}"),
        "input_max_events_per_second = 1\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    let text: String = std::iter::repeat_n('a', 4096).collect();
    let err = call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &(text,))
        .await
        .expect_err("a flood far beyond the budget should be refused, not queued");

    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.as_str(), wgaf_common::INPUT_ERROR_RATE_LIMITED);
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }

    // Refusing must not have created the device — the limiter sits in front
    // of device resolution, and a refused call should leave no trace.
    assert!(
        find_device_block(&device_name).is_none(),
        "a rate-limited call must not have created the uinput device"
    );
}

/// A rate of `0` switches the limiter off entirely, so the documented escape
/// hatch in `config.toml` actually works.
#[tokio::test]
async fn a_rate_of_zero_disables_the_limiter() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.NoLimit{pid}");
    let device_name = format!("wgaf test device nolimit {pid}");

    let _daemon = spawn_daemon_with_config(
        &daemon_bus_name,
        &device_name,
        &format!("nolimit{pid}"),
        "input_max_events_per_second = 0\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // The same call the previous test refuses at a rate of 1. This one needs
    // a real device, since with the limiter disabled it proceeds to synthesis.
    let text: String = std::iter::repeat_n('a', 4096).collect();
    call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &(text,))
        .await
        .expect("with the limiter disabled, a large TypeText should be accepted");
}

/// `input_max_type_text_chars` actually caps a single `TypeText`, and the
/// error names the configured limit rather than the built-in default.
///
/// Like the rate-limit refusal, this needs no `/dev/uinput`: the length check
/// runs before the limiter and before the device is resolved.
#[tokio::test]
async fn type_text_is_capped_by_the_configured_character_limit() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Input.TextCap{pid}");
    let device_name = format!("wgaf test device textcap {pid}");

    let _daemon = spawn_daemon_with_config(
        &daemon_bus_name,
        &device_name,
        &format!("textcap{pid}"),
        "input_max_type_text_chars = 10\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // Ten is fine; eleven is not. Testing the boundary rather than a wildly
    // oversized string, so an off-by-one in the comparison is caught.
    let err = call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &("12345678901",))
        .await
        .expect_err("text over the configured cap should be refused");

    let zbus::Error::MethodError(name, description, _) = err else {
        panic!("expected a MethodError");
    };
    assert_eq!(
        name.as_str(),
        wgaf_common::INPUT_ERROR_TEXT_TOO_LONG,
        "over-length text must report a named error a script can branch on, \
         not a generic failure"
    );
    let description = description.unwrap_or_default();
    assert!(
        description.contains("max 10"),
        "the error should name the configured limit, not the built-in default: {description}"
    );
    assert!(
        description.contains("input_max_type_text_chars"),
        "the error should name the setting to change: {description}"
    );

    assert!(
        find_device_block(&device_name).is_none(),
        "an over-length call must not have created the uinput device"
    );
}
