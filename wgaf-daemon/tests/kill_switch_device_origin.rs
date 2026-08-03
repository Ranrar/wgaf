//! Can the kill switch tell wgaf's own `Escape` from the developer's?
//!
//! This is the acceptance test for W13. It exercises the one property that
//! makes keeping the handbrake on bare `Escape` survivable: the same key,
//! arriving from two different keyboards, must produce two different outcomes.
//!
//! | Where the `Escape` came from | What must happen |
//! |---|---|
//! | wgaf's own virtual `uinput` device | nothing — the run continues |
//! | the developer's physical keyboard | the handbrake engages |
//!
//! # Why this cannot be automated
//!
//! Half of it can. Synthesizing an `Escape` is what the daemon does for a
//! living. But nothing in this process — or in CI — can press a *physical*
//! key, and a physical press is the entire point of the second half. So the
//! test asks, and waits.
//!
//! Asserting only the virtual half would be worse than not testing at all: a
//! kill switch that ignores every `Escape` passes it perfectly.
//!
//! # This test is expected to FAIL until W13 ships
//!
//! On current `master` the virtual half fails, and that failure is the S1 in
//! `issues.md`: the compositor cannot distinguish a synthesized `Escape` from
//! a real one *unless somebody asks it to*, nothing in the extension asks, so
//! `wgaf key press escape` trips wgaf's own handbrake and stops the run.
//!
//! Until now that has been reasoned rather than measured. Running this test is
//! the measurement.
//!
//! # Running it
//!
//! ```text
//! cargo test --test kill_switch_device_origin -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The test says when it is your turn with a desktop notification, so you can
//! watch the screen rather than the terminal — which matters, because the key
//! press has to go to some *other* window anyway.
//!
//! `--nocapture` keeps the same prompts in the terminal. Worth passing, but the
//! notifications are what you will actually act on.
//!
//! # What it does to the desktop it borrows
//!
//! It owns the production bus name and synthesizes one `Escape`. If the grab
//! is armed that `Escape` is consumed by the compositor and reaches nothing;
//! if it is not, the `Escape` lands in whatever window has focus, where it is
//! about as harmless as a synthesized key gets. Nothing is typed.

mod harness;

use std::io::Write;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use zbus::Connection;

/// The extension's UUID, as installed.
const EXTENSION_UUID: &str = "wgaf@wgaf.dev";

/// How long to wait for a physical key press before giving up.
const HUMAN_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait *after* synthesizing, before concluding that no handbrake
/// engaged.
///
/// This is a one-sided wait and it is deliberately generous. Waiting longer can
/// only ever turn a pass into a failure — it gives a `Stop` that is on its way
/// more time to arrive — so the failure mode of a too-long wait is a *correct*
/// failure, not a false one. Too short is the dangerous direction.
const SETTLE_FOR_A_STOP: Duration = Duration::from_millis(2_000);

/// A daemon owning the **production** bus name, killed on drop.
///
/// Unlike every other suite here, this one cannot use a private bus name. The
/// extension's kill-switch handler calls a hardcoded `org.wgaf.Daemon`
/// (`extension/extension.js:42`), so a daemon on a test-private name would
/// never hear the `Stop` this test exists to observe.
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

fn spawn_production_daemon() -> DaemonGuard {
    let nonce = std::process::id();
    let device_name = format!("wgaf-device-origin-{nonce}");

    let config_path = std::env::temp_dir().join(format!("wgaf-device-origin-config-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{}\"\n\
             log_level = \"error\"\n\
             input_device_name = \"{device_name}\"\n\
             # Unlike the other suites, this one needs the synthesized Escape to\n\
             # actually reach the compositor, so the settle wait stays.\n\
             input_device_settle_ms = 300\n",
            wgaf_common::BUS_NAME
        ),
    )
    .expect("failed to write the test config");

    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-device-origin-permissions-{nonce}.toml"));
    std::fs::write(&permissions_path, "[capabilities]\n").expect("failed to write the test policy");

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

    DaemonGuard {
        child,
        config_path,
        permissions_path,
    }
}

// ---------------------------------------------------------------------------
// Preconditions
//
// Each one panics with the command that fixes it. This suite is `#[ignore]`d
// and run deliberately, so an unmet precondition is a setup mistake worth
// naming precisely rather than a reason to quietly pass.
// ---------------------------------------------------------------------------

/// Fails unless nothing already owns `org.wgaf.Daemon`.
///
/// A daemon the developer started themselves would answer this test's D-Bus
/// calls while a *different* daemon received the extension's `Stop`, and the
/// halves would disagree for a reason that has nothing to do with the code.
fn require_production_bus_free() {
    let owned = Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains(wgaf_common::BUS_NAME))
        .unwrap_or(false);

    if owned {
        panic!(
            "something already owns {}. This test must be the daemon the \
             extension talks to, so stop the running one first: \
             `systemctl --user stop wgaf-daemon`",
            wgaf_common::BUS_NAME
        );
    }
}

/// Fails unless the wgaf extension is installed and enabled.
fn require_extension_enabled() {
    let output = Command::new("gnome-extensions")
        .args(["info", EXTENSION_UUID])
        .output()
        .unwrap_or_else(|err| {
            panic!("could not run `gnome-extensions` ({err}); this suite needs a GNOME session")
        });

    let info = String::from_utf8_lossy(&output.stdout);

    // `State: ACTIVE` rather than `Enabled: Yes`: an extension can be enabled
    // and still not be running, having thrown on load. Only the loaded one
    // holds the grab this test is here to observe.
    if !output.status.success() || !info.contains("State: ACTIVE") {
        let state = info
            .lines()
            .find_map(|line| line.trim().strip_prefix("State: "))
            .unwrap_or("not installed");
        panic!(
            "the wgaf extension must be installed and running for this test — \
             it is the half that grabs the key. Its state is `{state}`. Run \
             `gnome-extensions enable {EXTENSION_UUID}`, and if it is enabled \
             but not ACTIVE, check `journalctl --user -b _COMM=gnome-shell` for \
             the error it threw on load."
        );
    }
}

/// Fails unless the kill switch is bound to bare `Escape`.
///
/// The test synthesizes exactly one key, so the binding has to be that key.
/// A combination would need the modifiers synthesized around it, which is a
/// different test with different failure modes.
fn require_kill_switch_on_bare_escape() {
    let schemadir = dirs_extension_schemas();
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

    if value != "['Escape']" {
        panic!(
            "this test needs the kill switch bound to bare `Escape`, but it is {value}.\n\
             Set it for the duration of the test with:\n  \
             gsettings{} set org.gnome.shell.extensions.wgaf kill-switch \"['Escape']\"\n\
             and remember to put it back afterwards — while it is bare `Escape`, \
             no application on the desktop receives that key.",
            schemadir
                .as_ref()
                .map(|d| format!(" --schemadir {}", d.display()))
                .unwrap_or_default()
        );
    }
}

/// The extension's own schema directory, for a per-user install.
///
/// A system-wide install compiles its schema into the global set, where plain
/// `gsettings` finds it, so `None` is a valid answer rather than a failure.
fn dirs_extension_schemas() -> Option<std::path::PathBuf> {
    let candidate = std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join(".local/share/gnome-shell/extensions")
            .join(EXTENSION_UUID)
            .join("schemas")
    })?;
    candidate.is_dir().then_some(candidate)
}

// ---------------------------------------------------------------------------
// D-Bus helpers
// ---------------------------------------------------------------------------

/// Whether the handbrake is engaged, asked of the input subsystem directly.
///
/// **Deliberately not `Status`.** That method probes every subsystem the daemon
/// has, including accessibility, and the accessibility probe can hang
/// indefinitely on a session whose a11y bus is otherwise healthy — see
/// `issues.md`. A test about input has no business failing because of that, so
/// it asks the thing it actually cares about.
///
/// A zero-distance pointer move is the question: it travels the whole synthesis
/// path, is refused with `Stopped` exactly when the handbrake is on, and moves
/// the pointer nowhere on the desktop it borrowed.
async fn is_stopped(connection: &Connection) -> bool {
    match harness::input::<(), _>(
        connection,
        wgaf_common::BUS_NAME,
        "MouseMove",
        &(0i32, 0i32),
    )
    .await
    {
        Ok(()) => false,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == wgaf_common::INPUT_ERROR_STOPPED =>
        {
            true
        }
        Err(other) => panic!("could not tell whether input is stopped: {other}"),
    }
}

async fn release(connection: &Connection) {
    harness::call::<(), _>(
        connection,
        wgaf_common::BUS_NAME,
        wgaf_common::OBJECT_PATH,
        wgaf_common::INTERFACE_NAME,
        "Release",
        &(),
    )
    .await
    .expect("Release must succeed");
}

/// Polls until the handbrake is engaged, or `timeout` elapses.
async fn wait_until_stopped(connection: &Connection, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_stopped(connection).await {
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
/// developer is *not* looking during this test.
///
/// A notification rather than a dialog window, deliberately. A dialog would
/// take keyboard focus, and — worse — `zenity` and friends close themselves on
/// `Escape`, which is the exact key under test. A notification cannot swallow
/// it, and cannot steal the focus the press is aimed at.
///
/// Best-effort: a machine without `notify-send` still runs the test, it just
/// runs it the noisy way.
fn on_screen(urgency: &str, summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=wgaf test")
        .arg(format!("--urgency={urgency}"))
        .arg(summary)
        .arg(body)
        .status();
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// The pair, asserted together.
///
/// Deliberately one test and not two. The property under test is a
/// *difference* between two origins, and either half alone is satisfied by a
/// kill switch that is simply broken — one by a switch that never fires, the
/// other by one that always does.
#[tokio::test]
#[ignore = "needs a human: asks the developer to press Escape on a real keyboard. \
            Run with --ignored --nocapture --test-threads=1, and see the module docs \
            for the desktop state it requires."]
async fn a_synthesized_escape_is_ignored_but_a_physical_one_stops_wgaf() {
    harness::require_wayland_session();
    harness::require_uinput();
    require_production_bus_free();
    require_extension_enabled();
    require_kill_switch_on_bare_escape();

    let _daemon = spawn_production_daemon();
    let connection = harness::wait_for_daemon(wgaf_common::BUS_NAME).await;

    // Creates the virtual device, which is what arms the grab under W13. Also
    // means the `Escape` below is not the call that pays for device creation.
    harness::warm_up_input_device(&connection, wgaf_common::BUS_NAME).await;

    assert!(
        !is_stopped(&connection).await,
        "the daemon must start with the handbrake off"
    );

    // -----------------------------------------------------------------
    // Half one: wgaf's own Escape. Automatable.
    // -----------------------------------------------------------------
    prompt(&[
        "Part 1 of 2 — no action needed.",
        "wgaf is about to press Escape on its own virtual keyboard.",
        "Please do not touch the keyboard for the next few seconds.",
    ]);
    on_screen(
        "normal",
        "wgaf test — part 1 of 2: hands off",
        "wgaf is pressing Escape on its own keyboard. Do not touch the \
         keyboard until the next notification.",
    );

    harness::input::<(), _>(&connection, wgaf_common::BUS_NAME, "KeyPress", &("escape",))
        .await
        .expect("KeyPress escape must be accepted while the handbrake is off");

    // **The release is not optional, and leaving it out cost an afternoon.**
    // A press with no matching release leaves Escape held down on wgaf's
    // virtual keyboard for as long as the device exists — a stuck key, exactly
    // as the CLI reference warns. The developer's press in part two then
    // produces no fresh press-edge for the shortcut to fire on, so the
    // handbrake never engages and the test blames the extension for a fault of
    // its own making.
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
        !is_stopped(&connection).await,
        "wgaf stopped itself.\n\n\
         The Escape that engaged the handbrake was one wgaf synthesized on its \
         own virtual keyboard — vendor 0x57ae, product 0x0001 (input/device.rs).\n\n\
         The kill-switch handler receives the ClutterEvent and can read \
         `get_source_device()` to tell wgaf's keystrokes from the user's. Until \
         it does, `wgaf key press escape` — a documented command, annotated \
         \"dismissing a dialog\" — aborts the run that issued it.\n\n\
         This is the S1 in issues.md, and W13.1 is the fix."
    );

    // -----------------------------------------------------------------
    // Half two: the developer's Escape. Needs a human.
    // -----------------------------------------------------------------
    prompt(&[
        "Part 2 of 2 — over to you.",
        "",
        "Press Escape now, once, on your physical keyboard.",
        "Press nothing else.",
        "",
        &format!("Waiting up to {} seconds…", HUMAN_TIMEOUT.as_secs()),
    ]);
    // Critical so it stays on screen until acted on: a notification that
    // faded after a few seconds would be no better than the terminal line.
    on_screen(
        "critical",
        "wgaf test — YOUR TURN",
        &format!(
            "Press Escape once, on your real keyboard. Press nothing else. \
             Waiting up to {} seconds.",
            HUMAN_TIMEOUT.as_secs()
        ),
    );

    let stopped = wait_until_stopped(&connection, HUMAN_TIMEOUT).await;

    on_screen(
        if stopped { "normal" } else { "critical" },
        if stopped {
            "wgaf test — passed"
        } else {
            "wgaf test — FAILED"
        },
        if stopped {
            "Your Escape stopped wgaf, and wgaf's own Escape did not. \
             You can stop watching now."
        } else {
            "No Escape arrived within the time limit, so the handbrake was \
             never tested. See the terminal."
        },
    );

    assert!(
        stopped,
        "no handbrake after a physical Escape (waited {}s).\n\n\
         If you did press it, the grab is not armed. Check that the extension \
         is enabled, that `kill-switch` is still ['Escape'], and — once W13.1 \
         lands — that the grab is armed while a virtual device exists.\n\n\
         Note the direction of this failure: it is the dangerous one. It means \
         the handbrake does not work.",
        HUMAN_TIMEOUT.as_secs()
    );

    prompt(&["Both halves passed. Releasing the handbrake."]);

    // Leaves the daemon in the state it was found in, for the moment between
    // here and the guard's `Drop`.
    release(&connection).await;
    assert!(
        !is_stopped(&connection).await,
        "Release must lift the handbrake"
    );
}
