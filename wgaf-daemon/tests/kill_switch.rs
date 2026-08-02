//! The kill switch: `org.wgaf.Daemon1.Stop` / `Release`, against a really
//! spawned daemon and a real `uinput` device.
//!
//! # What these tests are careful about
//!
//! Every positive check here — "synthesis works again after `Release`" — is a
//! `MouseMove(0, 0)`. It travels the entire path a keystroke does (rate limiter,
//! device creation, blocking emit) while moving the pointer nowhere, so a test
//! run cannot disturb the desktop it borrowed. The refusals are safe by
//! definition: a refused call synthesizes nothing.
//!
//! `input_device_settle_ms = 0` is the same **safety** setting `tests/input.rs`
//! documents at length, for the same reason: it restores the property that the
//! events go nowhere, by writing to the device before the compositor has opened
//! it. It is a mitigation and not a guarantee, and the real fix is a nested
//! compositor.
//!
//! # What is deliberately not tested here
//!
//! That stopping *aborts input already in flight* — the property the whole
//! feature rests on. Nothing observable from this process distinguishes "the
//! flood stopped" from "the flood finished", because these tests have no
//! application receiving the keystrokes. The test that can tell the difference
//! is `stop_during_a_long_type_text_aborts_it` at the bottom of this file, which
//! drives a real window and is `#[ignore]`d for it.

mod harness;

use std::process::{Child, Command};
use std::time::Duration;

use wgaf_common::DaemonStatus;
use wgaf_common::dict::DaemonStatusDict;
use zbus::Connection;

/// A spawned daemon with a policy file of this test's choosing, killed on drop.
///
/// Local rather than `harness::spawn_daemon` because one of these tests needs a
/// *deny-everything* policy, and the harness deliberately writes an
/// allow-everything one.
struct DaemonGuard {
    child: Child,
    config_path: std::path::PathBuf,
    permissions_path: std::path::PathBuf,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_file(&self.permissions_path);
    }
}

/// Spawns a daemon with a private bus name, a private virtual-device name, and
/// `policy` as its `permissions.toml`.
fn spawn_daemon(tag: &str, policy: &str, extra_config: &str) -> (DaemonGuard, String, String) {
    let nonce = format!("{}-{tag}", std::process::id());
    let bus_name = format!("org.wgaf.Test.KillSwitch.{tag}{}", std::process::id());
    // `/proc/bus/input/devices` is machine-global and records nothing about
    // which process created an entry, so the device is named per test.
    let device_name = format!("wgaf-kill-switch-{nonce}");

    let config_path = std::env::temp_dir().join(format!("wgaf-kill-switch-config-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{bus_name}\"\n\
             log_level = \"error\"\n\
             input_device_name = \"{device_name}\"\n\
             {extra_config}"
        ),
    )
    .expect("failed to write the test config");

    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-kill-switch-permissions-{nonce}.toml"));
    std::fs::write(&permissions_path, policy).expect("failed to write the test policy");

    // The daemon refuses a group- or world-writable config or policy file, and
    // `fs::write` honours the umask — 002 on many distributions, giving 0664.
    for path in [&config_path, &permissions_path] {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .expect("failed to tighten test file permissions");
    }

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");

    (
        DaemonGuard {
            child,
            config_path,
            permissions_path,
        },
        bus_name,
        device_name,
    )
}

/// An empty capability table: the explicit way to say "allow everything".
const ALLOW_EVERYTHING: &str = "[capabilities]\n";

/// Config that keeps a test's synthesized events off the developer's desktop,
/// by writing them to the device before the compositor has opened it. See the
/// module docs, and `tests/input.rs`, which documents the same setting at
/// length: it is a mitigation and not a guarantee.
///
/// Every test here uses it except the one that needs delivery.
const EVENTS_GO_NOWHERE: &str = "input_device_settle_ms = 0\n";

/// Every capability this daemon has, denied. The point of D3 is that the kill
/// switch survives even this.
const DENY_EVERYTHING: &str = "[capabilities]\n\
     FocusWindow = \"Deny\"\n\
     MoveWindow = \"Deny\"\n\
     ResizeWindow = \"Deny\"\n\
     CloseWindow = \"Deny\"\n\
     TypeText = \"Deny\"\n\
     KeyPress = \"Deny\"\n\
     KeyRelease = \"Deny\"\n\
     MouseMove = \"Deny\"\n\
     MouseClick = \"Deny\"\n\
     MouseScroll = \"Deny\"\n\
     InvokeAction = \"Deny\"\n\
     SetText = \"Deny\"\n\
     FocusElement = \"Deny\"\n";

/// Calls `Stop` or `Release` on `org.wgaf.Daemon1`.
async fn daemon_call(connection: &Connection, bus_name: &str, method: &str) -> zbus::Result<()> {
    harness::call(
        connection,
        bus_name,
        wgaf_common::OBJECT_PATH,
        wgaf_common::INTERFACE_NAME,
        method,
        &(),
    )
    .await
}

async fn status(connection: &Connection, bus_name: &str) -> DaemonStatus {
    let dict: DaemonStatusDict = harness::call(
        connection,
        bus_name,
        wgaf_common::OBJECT_PATH,
        wgaf_common::INTERFACE_NAME,
        "Status",
        &(),
    )
    .await
    .expect("Status must answer");
    dict.into()
}

/// Moves the pointer by nothing, which exercises the whole synthesis path
/// without touching the desktop. See the module docs.
async fn synthesize(connection: &Connection, bus_name: &str) -> zbus::Result<()> {
    harness::input::<(), _>(connection, bus_name, "MouseMove", &(0i32, 0i32)).await
}

/// The D-Bus error name and description of a failed call.
fn method_error(err: &zbus::Error) -> (String, String) {
    match err {
        zbus::Error::MethodError(name, description, _) => (
            name.as_str().to_string(),
            description.clone().unwrap_or_default(),
        ),
        other => panic!("expected a D-Bus method error, got {other}"),
    }
}

/// Asserts a call was refused by the kill switch specifically.
fn assert_stopped(result: zbus::Result<()>, what: &str) {
    let err = result.expect_err(&format!("{what} must be refused while stopped"));
    let (name, description) = method_error(&err);
    assert_eq!(
        name,
        wgaf_common::INPUT_ERROR_STOPPED,
        "{what} was refused, but not as `Stopped` — a caller cannot tell an \
         emergency stop from a policy denial by reading `{name}`"
    );
    // The state outlives the emergency, so the failure has to carry the way
    // back — see `InputError::Stopped`.
    assert!(
        description.contains("wgaf release"),
        "the refusal must name `wgaf release`, got: {description}"
    );
}

#[tokio::test]
async fn stop_refuses_every_kind_of_synthesis_and_release_restores_it() {
    let (_daemon, bus_name, _device) = spawn_daemon("Basic", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis works before the kill switch is engaged");
    assert!(!status(&connection, &bus_name).await.input_stopped);

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed");

    // Every entry point, not just the one that motivated the feature: a brake
    // that stops typing while leaving the pointer free is not a brake.
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "TypeText", &("hello",)).await,
        "TypeText",
    );
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "KeyPress", &("a",)).await,
        "KeyPress",
    );
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "Hotkey", &(vec!["ctrl", "t"],)).await,
        "Hotkey",
    );
    assert_stopped(synthesize(&connection, &bus_name).await, "MouseMove");
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "MouseClick", &("left",)).await,
        "MouseClick",
    );
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "MouseScroll", &(0i32, 1i32)).await,
        "MouseScroll",
    );

    assert!(
        status(&connection, &bus_name).await.input_stopped,
        "`wgaf status` is where a user goes to find out why nothing is happening"
    );

    // A second Stop is not a toggle. Someone in a panic presses the shortcut
    // repeatedly, and the second press must not restart what the first stopped.
    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must be idempotent");
    assert_stopped(synthesize(&connection, &bus_name).await, "MouseMove");

    daemon_call(&connection, &bus_name, "Release")
        .await
        .expect("Release must succeed");
    assert!(!status(&connection, &bus_name).await.input_stopped);
    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis works again after Release");
}

/// Stopping must leave nothing registered with the kernel — a virtual keyboard
/// that survives the panic key is exactly what the user was trying to be rid of.
#[tokio::test]
async fn stopping_takes_the_virtual_device_away_from_the_kernel() {
    let (_daemon, bus_name, device_name) =
        spawn_daemon("Device", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    synthesize(&connection, &bus_name)
        .await
        .expect("the first synthesis creates the device");
    assert!(
        device_is_registered(&device_name),
        "the daemon should hold a live virtual device after synthesizing"
    );
    assert!(status(&connection, &bus_name).await.input_device_created);

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed");

    assert!(
        poll_until(
            || !device_is_registered(&device_name),
            Duration::from_secs(5)
        )
        .await,
        "`{device_name}` is still in /proc/bus/input/devices after Stop — \
         the device was not destroyed"
    );
    assert!(
        !status(&connection, &bus_name).await.input_device_created,
        "status must agree that no device is held"
    );

    // And the next synthesis after `Release` creates a fresh one, rather than
    // leaving the daemon permanently unable to type.
    daemon_call(&connection, &bus_name, "Release")
        .await
        .expect("Release must succeed");
    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis must recreate the device after Release");
    assert!(device_is_registered(&device_name));
}

/// D3: policy governs what wgaf may do to the desktop; the kill switch governs
/// wgaf. A policy file that could deny someone their own brake would be an
/// unsafe design, and one that allowed `Stop` while denying `Release` would
/// strand them on the far side of it.
#[tokio::test]
async fn stop_and_release_are_not_gated_by_the_permission_policy() {
    let (_daemon, bus_name, _device) = spawn_daemon("Policy", DENY_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    let (name, _) = method_error(
        &synthesize(&connection, &bus_name)
            .await
            .expect_err("the policy denies everything"),
    );
    assert_eq!(
        name,
        wgaf_common::INPUT_ERROR_PERMISSION_DENIED,
        "precondition: this daemon's policy must actually be denying calls"
    );

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed under a deny-everything policy");
    assert!(status(&connection, &bus_name).await.input_stopped);

    daemon_call(&connection, &bus_name, "Release")
        .await
        .expect("Release must succeed under a deny-everything policy");
    assert!(!status(&connection, &bus_name).await.input_stopped);

    // And lifting the kill switch grants nothing: the policy is still the
    // policy. `Release` un-stops wgaf, it does not un-deny it.
    let (name, _) = method_error(
        &synthesize(&connection, &bus_name)
            .await
            .expect_err("the policy still denies everything"),
    );
    assert_eq!(name, wgaf_common::INPUT_ERROR_PERMISSION_DENIED);
}

/// **The assertion the feature exists for**: a stop issued while a long
/// `TypeText` is being emitted aborts it, rather than taking effect once the
/// flood has finished.
///
/// The check is on **how much text the application actually received**, never on
/// the D-Bus call returning an error. A daemon that reported `Stopped` while
/// still typing the remaining ten thousand characters would satisfy the second
/// and fail the user.
///
/// The string is deliberately enormous, and `input_max_type_text_chars` is
/// raised to allow it. Nothing in wgaf paces keystrokes — the emit loop writes
/// as fast as the kernel accepts — so a default-length call is over in a few
/// tens of milliseconds, faster than another process can react to it. Length is
/// the only thing that buys a window to interrupt.
#[tokio::test]
#[ignore = "takes over the desktop: synthesizes real keystrokes into a real session. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn stop_during_a_long_type_text_aborts_it() {
    harness::require_wayland_session();
    harness::require_uinput();

    /// How many characters the interrupted call asks for.
    const REQUESTED: usize = 20_000;

    let (_daemon, bus_name, _device) = spawn_daemon(
        "Abort",
        ALLOW_EVERYTHING,
        // The settle wait is restored here, unlike the tests above: this one
        // needs the keystrokes to actually arrive somewhere. The rate limiter
        // is switched off so that the call is not refused as a runaway before
        // it starts — being a flood is the point of it.
        &format!(
            "input_max_type_text_chars = {REQUESTED}\n\
             input_max_events_per_second = 0\n\
             input_device_settle_ms = 300\n"
        ),
    );
    let connection = harness::wait_for_daemon(&bus_name).await;

    let app = harness::TestApp::spawn("input-test").await;
    app.wait_for("the test window to take keyboard focus", |report| {
        report.bool("window_focused")
    })
    .await;
    harness::warm_up_input_device(&connection, &bus_name).await;

    let text: String = std::iter::repeat_n('a', REQUESTED).collect();
    let typing = {
        let connection = connection.clone();
        let bus_name = bus_name.clone();
        tokio::spawn(async move {
            harness::input::<(), _>(&connection, &bus_name, "TypeText", &(text.as_str(),)).await
        })
    };

    // Long enough that typing is unmistakably under way — the assertion below
    // requires having interrupted a flood, not having beaten it to the start.
    tokio::time::sleep(Duration::from_millis(250)).await;
    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed while a TypeText is in flight");

    let result = typing.await.expect("the TypeText task panicked");
    assert_stopped(result, "the in-flight TypeText");

    // The application is still draining whatever reached it before the stop, so
    // give it a moment to settle before reading the count. Growth after this
    // point would mean the daemon kept typing, which is what the assertion
    // below rejects.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let typed = app
        .read()
        .expect("input-test stopped reporting")
        .str("typed")
        .chars()
        .count();

    // Printed rather than only asserted: how far the flood got before it was
    // cut off is the interesting number, and a run that reports 19,998 is
    // technically passing while telling you the stop barely arrived in time.
    println!("the application received {typed} of {REQUESTED} characters before the stop");

    assert!(
        typed > 0,
        "nothing arrived at all, so this run proves only that a stopped daemon \
         does not type — not that a stop interrupts one that is typing"
    );
    assert!(
        typed < REQUESTED,
        "the application received all {REQUESTED} characters: the stop was \
         accepted but the flood ran to completion anyway"
    );
}

/// Whether a device with this name is currently registered with the kernel.
fn device_is_registered(device_name: &str) -> bool {
    let Ok(data) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return false;
    };
    data.contains(&format!("Name=\"{device_name}\""))
}

async fn poll_until<F: Fn() -> bool>(predicate: F, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
