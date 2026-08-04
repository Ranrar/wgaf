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
//! is `stop_during_a_long_type_text_aborts_it`, which drives a real window and
//! is `#[ignore]`d for it.
//!
//! # What runs unattended, and what cannot
//!
//! Most of this file runs in CI. Two tests do not, for different reasons, and
//! both are `#[ignore]`d:
//!
//! | Test | Needs |
//! |---|---|
//! | `stop_during_a_long_type_text_aborts_it` | a real window to type into |
//! | `a_synthesized_escape_is_ignored_but_a_physical_one_stops_wgaf` | a person to press a key |
//!
//! The second one's requirement is not an oversight to be engineered away. It
//! asserts that the *same key* from two different keyboards produces two
//! different outcomes, and nothing in this process can press a physical one.
//! Asserting only the automatable half is worse than not testing at all — that
//! was tried, and it passed while the compositor grab was never armed.
//!
//! What *is* automatable of that feature is the daemon's half: that
//! `InputDeviceActive` follows the virtual device and announces both edges.
//! Those two tests run in CI and would catch most ways the emergency key could
//! silently stop being armed.

mod harness;

use std::io::Write;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

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

/// Asserts a call was **not** refused by the kill switch.
///
/// Deliberately tolerant of the call failing for other reasons. On a machine
/// with no accessibility bus or no GNOME Shell Extension — CI, for one — these
/// calls fail with `BusUnavailable` or `ExtensionUnavailable`, and that is
/// fine. The property under test is *which* error, not whether one occurred, so
/// this stays meaningful everywhere instead of needing a desktop.
fn assert_not_stopped<T>(result: zbus::Result<T>, what: &str) {
    let Err(err) = result else {
        return;
    };

    if let zbus::Error::MethodError(name, _, _) = &err {
        assert_ne!(
            name.as_str(),
            wgaf_common::INPUT_ERROR_STOPPED,
            "{what} was refused by the kill switch. The brake is for synthesized input only; \
             the user guide promises window and accessibility commands keep working while input \
             is stopped, and a user whose script has just been stopped needs them to — they are \
             how the desktop is inspected, and how a dialog gets dismissed when `Escape` cannot \
             reach it"
        );
    }
}

/// The other half of the kill switch's contract: it brakes **input**, and
/// nothing else.
///
/// `CHANGELOG.md` states this as a user-facing promise — "Window, workspace and
/// accessibility commands keep working" — and nothing tested it. The stop is
/// enforced in `input/mod.rs` alone, which is correct and also easy to
/// "harden" later by moving the check up to the D-Bus layer, at which point the
/// promise would quietly become false. This is the test that would object.
///
/// It matters beyond documentation accuracy. The recovery route from a runaway
/// script is to stop it and then look at what happened, and both halves of
/// looking — `wgaf window list` and the `wgaf a11y` queries — are exactly the
/// commands this asserts still answer.
///
/// **The mutating accessibility actions keep working too, and that is
/// load-bearing rather than an oversight.** While a run is in progress the
/// emergency key is held by wgaf, so a script cannot press `Escape` at a dialog
/// and the dialog stays open. The documented remedy is to press the dialog's
/// own Cancel or Close button with `wgaf a11y` — which only works because
/// `InvokeAction` is outside the brake. A "hardening" that gated the mutating
/// accessibility actions would take away the escape route the user guide sends
/// people to, so this asserting only the read-only calls is a deliberate
/// boundary, not a gap left to fill in later.
#[tokio::test]
async fn stopping_does_not_refuse_window_or_accessibility_commands() {
    let (_daemon, bus_name, _device) = spawn_daemon("Scope", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed");

    // The stop is engaged — established against input, so a passing assertion
    // below cannot be explained by the brake never having been applied.
    assert_stopped(
        harness::input::<(), _>(&connection, &bus_name, "TypeText", &("hello",)).await,
        "TypeText",
    );

    assert_not_stopped(
        harness::accessibility::<Vec<wgaf_common::AppRecord>, _>(
            &connection,
            &bus_name,
            "ListApps",
            &(),
        )
        .await,
        "a11y ListApps",
    );
    assert_not_stopped(
        harness::accessibility::<Vec<wgaf_common::ElementRecord>, _>(
            &connection,
            &bus_name,
            "FindElements",
            &("no-such-application-xyz", "", "", "", 0i32),
        )
        .await,
        "a11y FindElements",
    );
    assert_not_stopped(
        harness::windows::<Vec<wgaf_common::dict::WindowRecordDict>, _>(
            &connection,
            &bus_name,
            "ListWindows",
            &(),
        )
        .await,
        "ListWindows",
    );

    // Transparency is ungated by design — a status query that the emergency
    // state could suppress would defeat the point of having one.
    assert!(
        status(&connection, &bus_name).await.input_stopped,
        "`wgaf status` must keep answering, and must say the stop is engaged"
    );
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

// ---------------------------------------------------------------------------
// `InputDeviceActive` — the signal the emergency key is armed from
//
// The GNOME Shell Extension registers its shortcut while this is true and
// gives the key back when it goes false. That is what keeps `Escape` out of
// wgaf's hands between runs, so the property lying in either direction is a
// safety fault: stuck true holds the key hostage for the session, stuck false
// leaves a running script with no panic key.
//
// The extension half cannot be tested from here — it is GJS in the Shell's
// process, and proving the *key* works needs a physical press (see
// `a_synthesized_escape_is_ignored_but_a_physical_one_stops_wgaf` below).
// The daemon half is entirely testable, and these run in CI.
// ---------------------------------------------------------------------------

/// Reads `org.wgaf.Daemon1.InputDeviceActive`.
async fn input_device_active(connection: &Connection, bus_name: &str) -> bool {
    let value: zbus::zvariant::OwnedValue = harness::call(
        connection,
        bus_name,
        wgaf_common::OBJECT_PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(wgaf_common::INTERFACE_NAME, "InputDeviceActive"),
    )
    .await
    .expect("reading InputDeviceActive must succeed");

    bool::try_from(value).expect("InputDeviceActive must be a boolean")
}

#[tokio::test]
async fn input_device_active_follows_the_virtual_device() {
    let (_daemon, bus_name, _device) = spawn_daemon("Active", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    // False at startup, and reading it must not be what makes it true: the
    // daemon deliberately does not touch /dev/uinput until asked to synthesize.
    assert!(
        !input_device_active(&connection, &bus_name).await,
        "no device exists until something synthesizes, so this must start false"
    );
    assert!(
        !input_device_active(&connection, &bus_name).await,
        "reading the property must not create a device as a side effect"
    );

    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis must succeed");
    assert!(
        input_device_active(&connection, &bus_name).await,
        "a synthesis creates the virtual device, so this must now be true"
    );

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed");
    assert!(
        !input_device_active(&connection, &bus_name).await,
        "Stop destroys the device, so this must go false — an extension that \
         believed otherwise would hold the emergency key for the whole session"
    );

    // Release alone does not bring the device back; the next synthesis does.
    // The property must describe what is, not what is permitted.
    daemon_call(&connection, &bus_name, "Release")
        .await
        .expect("Release must succeed");
    assert!(
        !input_device_active(&connection, &bus_name).await,
        "Release lifts the refusal but recreates nothing, so this stays false"
    );

    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis must succeed again after Release");
    assert!(
        input_device_active(&connection, &bus_name).await,
        "the device is recreated lazily by the next synthesis"
    );
}

/// The value being right is not enough: the extension is driven by the
/// *change notification*, not by polling. A property that never announced
/// itself would leave the key armed until something else happened to ask.
#[tokio::test]
async fn input_device_active_announces_both_edges() {
    use futures_util::StreamExt;

    let (_daemon, bus_name, _device) = spawn_daemon("Edges", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    let properties = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(bus_name.as_str())
        .expect("the test daemon's bus name must be valid")
        .path(wgaf_common::OBJECT_PATH)
        .expect("the daemon object path must be valid")
        .build()
        .await
        .expect("building a Properties proxy must succeed");

    // Subscribed before anything changes, so no edge can be missed between
    // setup and the first assertion.
    let mut changes = properties
        .receive_properties_changed()
        .await
        .expect("subscribing to PropertiesChanged must succeed");

    /// Waits for the next announced value of `InputDeviceActive`.
    ///
    /// Bounded rather than awaited forever: a missing signal is the fault
    /// under test, and a test that hangs reports it as a stuck suite instead
    /// of a failure.
    async fn next_value(
        changes: &mut (impl StreamExt<Item = zbus::fdo::PropertiesChanged> + Unpin),
    ) -> bool {
        let signal = tokio::time::timeout(Duration::from_secs(5), changes.next())
            .await
            .expect("no PropertiesChanged arrived within 5s")
            .expect("the PropertiesChanged stream ended unexpectedly");

        let args = signal.args().expect("PropertiesChanged must carry args");
        let value = args
            .changed_properties()
            .get("InputDeviceActive")
            .expect("the signal must name InputDeviceActive");

        bool::try_from(value.try_clone().expect("cloning the value must succeed"))
            .expect("InputDeviceActive must be announced as a boolean")
    }

    synthesize(&connection, &bus_name)
        .await
        .expect("synthesis must succeed");
    assert!(
        next_value(&mut changes).await,
        "creating the device must announce true — this is the edge the \
         extension arms the emergency key on"
    );

    daemon_call(&connection, &bus_name, "Stop")
        .await
        .expect("Stop must succeed");
    assert!(
        !next_value(&mut changes).await,
        "destroying the device must announce false — without this edge the \
         extension never gives Escape back to your applications"
    );
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

// ---------------------------------------------------------------------------
// Does the daemon still provide what the extension asks it for?
// ---------------------------------------------------------------------------

/// The extension's source, read at compile time.
const EXTENSION_SOURCE: &str = include_str!("../../extension/extension.js");

/// Every `name="…"` inside the extension's `DAEMON_INTERFACE_XML` block.
///
/// Deliberately crude: it collects method and property names without caring
/// which is which, because the failure being guarded against is a member
/// disappearing or being renamed, and that reads the same either way.
fn members_the_extension_expects() -> Vec<String> {
    let block = EXTENSION_SOURCE
        .split_once("const DAEMON_INTERFACE_XML")
        .expect("extension.js must declare DAEMON_INTERFACE_XML")
        .1
        .split_once("`;")
        .expect("DAEMON_INTERFACE_XML must be a terminated template literal")
        .0;

    let names: Vec<String> = block
        .match_indices("name=\"")
        .filter_map(|(at, needle)| {
            let rest = &block[at + needle.len()..];
            rest.find('"').map(|end| rest[..end].to_string())
        })
        // The interface's own name="org.wgaf.Daemon1" is not a member.
        .filter(|name| !name.contains('.'))
        .collect();

    assert!(
        !names.is_empty(),
        "parsed no members out of the extension's DAEMON_INTERFACE_XML — the \
         parser has drifted from the file's shape and is now guarding nothing"
    );
    names
}

/// **The daemon must provide every member the extension asks it for.**
///
/// The extension is a *client* of `org.wgaf.Daemon1` for exactly two things:
/// `Stop`, which is the emergency key, and `InputDeviceActive`, which tells it
/// when to grab that key. This is the opposite direction from the drift test in
/// `windows/proxy.rs`, which guards the daemon's use of the extension — and
/// until now nothing guarded this way round.
///
/// **Why it matters more than a normal contract test.** Rename or remove
/// `InputDeviceActive` on the daemon side and nothing shouts. The extension
/// reads the property, gets nothing, treats that as "wgaf cannot type right
/// now", and so never registers the shortcut. The emergency key silently stops
/// existing. The only test that would notice needs a human to press a key, so
/// CI never runs it.
///
/// **If you are changing `org.wgaf.Daemon1` and this test failed:** the fix is
/// to change `DAEMON_INTERFACE_XML` in `extension/extension.js` to match, not
/// to relax this test.
#[tokio::test]
async fn the_daemon_provides_every_member_the_extension_asks_for() {
    let (_daemon, bus_name, _device) =
        spawn_daemon("Contract", ALLOW_EVERYTHING, EVENTS_GO_NOWHERE);
    let connection = harness::wait_for_daemon(&bus_name).await;

    // Asked of a running daemon rather than parsed out of the Rust source, so
    // that what is checked is what a client actually sees on the bus.
    let xml: String = harness::call(
        &connection,
        &bus_name,
        wgaf_common::OBJECT_PATH,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        &(),
    )
    .await
    .expect("the daemon must answer Introspect");

    for member in members_the_extension_expects() {
        assert!(
            xml.contains(&format!("name=\"{member}\"")),
            "extension/extension.js asks {} for `{member}`, and the daemon does \
             not provide it.\n\n\
             The extension uses these to know when to grab the emergency key. A \
             missing member does not raise an error there — it reads as \"wgaf \
             is not typing\", so the key is never grabbed and the emergency stop \
             silently stops existing.\n\n\
             Fix extension.js's DAEMON_INTERFACE_XML to match the daemon, rather \
             than relaxing this test.",
            wgaf_common::INTERFACE_NAME
        );
    }
}

// ===========================================================================
// Does the emergency key tell wgaf's own `Escape` from the developer's?
//
// Everything above runs unattended. This last section does not, and cannot.
//
// The property under test is a *difference* between two origins of the same
// key: one synthesized by wgaf, one pressed by a person. Nothing in this
// process — or in CI — can press a physical key, so the test asks and waits.
//
// **Do not "fix" this by asserting only the automatable half.** That was tried
// on 2026-08-03 and passed while the compositor grab was never armed at all: a
// kill switch that fires on nothing satisfies "wgaf's own Escape did not stop
// it" perfectly. Only the physical press proves the grab existed, which is what
// makes the synthesized half mean anything. The two assertions are one test for
// that reason.
//
// What *is* automatable is the daemon half — that `InputDeviceActive` follows
// the device and announces both edges — and that is covered above, in CI.
//
// See `adr/adr-0006-emergency-key-armed-on-device-and-checked-by-origin.md`.
// ===========================================================================

/// The extension's UUID, as installed.
const EXTENSION_UUID: &str = "wgaf@wgaf.dev";

/// How long to wait for a physical key press before giving up.
const HUMAN_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait after synthesizing before concluding no handbrake engaged.
///
/// One-sided and deliberately generous: waiting longer can only give a `Stop`
/// that is on its way more time to arrive, so the failure mode of too long is a
/// *correct* failure. Too short is the dangerous direction.
const SETTLE_FOR_A_STOP: Duration = Duration::from_millis(2_000);

/// A daemon owning the **production** bus name, killed on drop.
///
/// Unlike every other daemon in this file, this one cannot use a private bus
/// name: the extension's emergency-key handler calls a hardcoded
/// `org.wgaf.Daemon` (`extension/extension.js`), so a daemon on a test-private
/// name would never hear the `Stop` this test exists to observe.
fn spawn_production_daemon() -> (DaemonGuard, String) {
    let nonce = std::process::id();
    let device_name = format!("wgaf-device-origin-{nonce}");

    let config_path = std::env::temp_dir().join(format!("wgaf-device-origin-config-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{}\"\n\
             log_level = \"error\"\n\
             input_device_name = \"{device_name}\"\n\
             # Unlike the suites above, this one needs the synthesized Escape to\n\
             # actually reach the compositor, so the settle wait stays.\n\
             input_device_settle_ms = 300\n",
            wgaf_common::BUS_NAME
        ),
    )
    .expect("failed to write the test config");

    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-device-origin-permissions-{nonce}.toml"));
    std::fs::write(&permissions_path, ALLOW_EVERYTHING).expect("failed to write the test policy");

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
        device_name,
    )
}

/// Fails unless nothing already owns `org.wgaf.Daemon`.
///
/// A daemon the developer started themselves would answer this test's D-Bus
/// calls while a *different* daemon received the extension's `Stop`, and the
/// halves would disagree for a reason unrelated to the code.
fn require_production_bus_free() {
    let owned = Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains(wgaf_common::BUS_NAME))
        .unwrap_or(false);

    assert!(
        !owned,
        "something already owns {}. This test must be the daemon the extension \
         talks to, so stop the running one first: \
         `systemctl --user stop wgaf-daemon`",
        wgaf_common::BUS_NAME
    );
}

/// Fails unless the wgaf extension is installed and actually running.
fn require_extension_running() {
    let output = Command::new("gnome-extensions")
        .args(["info", EXTENSION_UUID])
        .output()
        .unwrap_or_else(|err| {
            panic!("could not run `gnome-extensions` ({err}); this test needs a GNOME session")
        });

    let info = String::from_utf8_lossy(&output.stdout);

    // `State: ACTIVE` rather than `Enabled: Yes`: an extension can be enabled
    // and still not be running, having thrown on load. Only the loaded one
    // holds the grab this test observes. Note also that the Shell cannot reload
    // an extension's code in-session — after editing extension.js you must log
    // out and back in, and nothing warns you that you did not.
    if !output.status.success() || !info.contains("State: ACTIVE") {
        let state = info
            .lines()
            .find_map(|line| line.trim().strip_prefix("State: "))
            .unwrap_or("not installed");
        panic!(
            "the wgaf extension must be installed and running — it is the half \
             that grabs the key. Its state is `{state}`. Run \
             `gnome-extensions enable {EXTENSION_UUID}`, and if it is enabled but \
             not ACTIVE, check `journalctl --user -b _COMM=gnome-shell` for what \
             it threw on load."
        );
    }
}

/// Fails unless the emergency key is bound to bare `Escape`.
///
/// The test synthesizes exactly one key, so the binding has to be that key. A
/// combination would need modifiers synthesized around it, which is a different
/// test with different failure modes.
fn require_kill_switch_on_bare_escape() {
    let schemadir = extension_schema_dir();
    let mut command = Command::new("gsettings");
    if let Some(dir) = &schemadir {
        command.arg("--schemadir").arg(dir);
    }
    let output = command
        .args(["get", "org.gnome.shell.extensions.wgaf", "kill-switch"])
        .output()
        .expect("failed to run `gsettings`");

    let value = String::from_utf8_lossy(&output.stdout);
    let value = value.trim();

    assert!(
        value == "['Escape']",
        "this test needs the emergency key bound to bare `Escape`, but it is \
         {value}. That is the shipped default, so the likely cause is a local \
         override; clear it with:\n  gsettings{} reset \
         org.gnome.shell.extensions.wgaf kill-switch",
        schemadir
            .as_ref()
            .map(|d| format!(" --schemadir {}", d.display()))
            .unwrap_or_default()
    );
}

/// The extension's own schema directory, for a per-user install.
///
/// A system-wide install compiles its schema into the global set where plain
/// `gsettings` finds it, so `None` is a valid answer rather than a failure.
fn extension_schema_dir() -> Option<std::path::PathBuf> {
    let candidate = std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".local/share/gnome-shell/extensions")
            .join(EXTENSION_UUID)
            .join("schemas")
    })?;
    candidate.is_dir().then_some(candidate)
}

/// Whether the handbrake is engaged, asked of the input subsystem directly.
///
/// **Deliberately not `Status`.** That method probes every subsystem including
/// accessibility, and the accessibility probe can hang indefinitely on a
/// session whose a11y bus is otherwise healthy — see the S2 in `issues.md`. A
/// test about input has no business failing because of that.
async fn handbrake_engaged(connection: &Connection) -> bool {
    match synthesize(connection, wgaf_common::BUS_NAME).await {
        Ok(()) => false,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == wgaf_common::INPUT_ERROR_STOPPED =>
        {
            true
        }
        Err(other) => panic!("could not tell whether input is stopped: {other}"),
    }
}

/// Polls until the handbrake is engaged, or `timeout` elapses.
async fn wait_for_handbrake(connection: &Connection, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if handbrake_engaged(connection).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// Puts a prompt where the developer will actually see it.
fn prompt(lines: &[&str]) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr);
    let _ = writeln!(stderr, "  ┌─────────────────────────────────────────────");
    for line in lines {
        let _ = writeln!(stderr, "  │ {line}");
    }
    let _ = writeln!(stderr, "  └─────────────────────────────────────────────");
    let _ = stderr.flush();
}

/// Puts the same thing on screen, because the terminal is the one place the
/// developer is *not* looking during this test — the key press has to go to
/// some other window.
///
/// A notification rather than a dialog, deliberately: a dialog steals the focus
/// the press is aimed at, and `zenity` and friends close themselves on
/// `Escape`, which is the exact key under test. A notification can do neither.
///
/// Best-effort — a machine without `notify-send` still runs the test, it just
/// runs it the noisy way.
fn on_screen(urgency: &str, summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=wgaf test")
        .arg(format!("--urgency={urgency}"))
        .arg(summary)
        .arg(body)
        .status();
}

/// The pair, asserted together. See the section comment above for why this is
/// one test and not two, and why it cannot be automated.
#[tokio::test]
#[ignore = "needs a human: asks the developer to press Escape on a real keyboard, \
            and needs the extension installed and running. Run with --ignored \
            --nocapture --test-threads=1; press the key in a window other than \
            the terminal running the test."]
async fn a_synthesized_escape_is_ignored_but_a_physical_one_stops_wgaf() {
    harness::require_wayland_session();
    harness::require_uinput();
    require_production_bus_free();
    require_extension_running();
    require_kill_switch_on_bare_escape();

    let (_daemon, _device_name) = spawn_production_daemon();
    let connection = harness::wait_for_daemon(wgaf_common::BUS_NAME).await;

    // Creates the virtual device, which is what arms the grab. Also means the
    // `Escape` below is not the call that pays for device creation.
    harness::warm_up_input_device(&connection, wgaf_common::BUS_NAME).await;

    assert!(
        !handbrake_engaged(&connection).await,
        "the daemon must start with the handbrake off"
    );

    // -- Half one: wgaf's own Escape. Automatable. ------------------------
    prompt(&[
        "Part 1 of 2 — no action needed.",
        "wgaf is about to press Escape on its own virtual keyboard.",
        "Please do not touch the keyboard for the next few seconds.",
    ]);
    on_screen(
        "normal",
        "wgaf test — part 1 of 2: hands off",
        "wgaf is pressing Escape on its own keyboard. Do not touch the keyboard \
         until the next notification.",
    );

    harness::input::<(), _>(&connection, wgaf_common::BUS_NAME, "KeyPress", &("escape",))
        .await
        .expect("KeyPress escape must be accepted while the handbrake is off");

    // **The release is not optional, and leaving it out cost an afternoon.** A
    // press with no matching release leaves Escape held down on wgaf's virtual
    // keyboard exactly as a stuck physical key would, and the developer's press
    // in part two then produces no fresh press-edge for the shortcut to fire
    // on. Two full runs failed at the hardware half and blamed the extension
    // for a fault of the test's own making.
    harness::input::<(), _>(
        &connection,
        wgaf_common::BUS_NAME,
        "KeyRelease",
        &("escape",),
    )
    .await
    .expect("KeyRelease escape must be accepted while the handbrake is off");

    tokio::time::sleep(SETTLE_FOR_A_STOP).await;

    assert!(
        !handbrake_engaged(&connection).await,
        "wgaf stopped itself.\n\n\
         The Escape that engaged the handbrake was one wgaf synthesized on its \
         own virtual keyboard — vendor 0x57ae, product 0x0001 (input/device.rs).\n\n\
         The extension's handler reads `get_source_device()` on the triggering \
         event to tell wgaf's keystrokes from the user's. If that check is gone \
         or the ids no longer match, `wgaf key press escape` — a documented \
         command, annotated \"dismissing a dialog\" — aborts the run that issued \
         it. `extension_agrees_with_the_ids_this_device_advertises` in \
         input/device.rs guards the ids specifically."
    );

    // -- Half two: the developer's Escape. Needs a human. -----------------
    prompt(&[
        "Part 2 of 2 — over to you.",
        "",
        "Press Escape now, once, on your physical keyboard.",
        "Press it in some window other than this terminal.",
        "",
        &format!("Waiting up to {} seconds…", HUMAN_TIMEOUT.as_secs()),
    ]);
    // Critical so it stays on screen until acted on: one that faded after a few
    // seconds would be no better than the terminal line nobody is reading.
    on_screen(
        "critical",
        "wgaf test — YOUR TURN",
        &format!(
            "Press Escape once, on your real keyboard, in any window except the \
             terminal running the test. Waiting up to {} seconds.",
            HUMAN_TIMEOUT.as_secs()
        ),
    );

    let stopped = wait_for_handbrake(&connection, HUMAN_TIMEOUT).await;

    on_screen(
        if stopped { "normal" } else { "critical" },
        if stopped {
            "wgaf test — passed"
        } else {
            "wgaf test — FAILED"
        },
        if stopped {
            "Your Escape stopped wgaf, and wgaf's own Escape did not. You can \
             stop watching now."
        } else {
            "No Escape arrived within the time limit, so the handbrake was never \
             tested. See the terminal."
        },
    );

    assert!(
        stopped,
        "no handbrake after a physical Escape (waited {}s).\n\n\
         If you did press it, the grab was not armed. Check that the extension \
         is ACTIVE, that it has been reloaded since extension.js last changed \
         (the Shell cannot do that in-session — log out and back in), and that \
         `InputDeviceActive` went true.\n\n\
         Note the direction of this failure: it is the dangerous one. It means \
         the handbrake does not work.",
        HUMAN_TIMEOUT.as_secs()
    );

    prompt(&["Both halves passed. Releasing the handbrake."]);

    daemon_call(&connection, wgaf_common::BUS_NAME, "Release")
        .await
        .expect("Release must succeed");
    assert!(
        !handbrake_engaged(&connection).await,
        "Release must lift the handbrake"
    );
}
