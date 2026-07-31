//! Every key wgaf advertises, pressed for real and read back from a real
//! application — the regression test for the 105-key audit.
//!
//! # What this exists to catch
//!
//! `wgaf key press a` resolves a name to an evdev keycode, writes it to a
//! virtual `uinput` device, and reports success. Between that write and an
//! application receiving anything sit the kernel, libinput, the compositor, the
//! xkb keymap and the toolkit — none of which the daemon can see past, and all
//! of which the daemon's own tests stop short of.
//!
//! `tests/input.rs` reads the synthesized events back from the kernel and
//! compares them against the same table that produced them, so it verifies that
//! wgaf agrees with itself. That is how `wgaf type` shipped for seven phases
//! pressing the wrong key for 25 of the 26 letters: the letter codes were
//! computed as `KEY_A + (c - b'a')`, evdev numbers letter keys by physical
//! position instead, and every test agreed with the mistake. It took an
//! application reporting what it actually received to expose it.
//!
//! This suite is that readback, made permanent. The 105-key audit was verified
//! by hand once, from a script in a scratchpad; a hand-run script guards
//! nothing against the next edit to the key table.
//!
//! # What the assertions rest on
//!
//! **The keycode is the load-bearing one.** `input-test` reports the hardware
//! keycode GTK4 received, which is the evdev code plus the 8 that X11 added and
//! Wayland kept. Comparing it against the code the key *should* have proves the
//! right physical key was pressed, all the way through the stack, and does so
//! identically on every keyboard layout in the world.
//!
//! **Those expected codes are transcribed from `<linux/input-event-codes.h>`,
//! never imported from `wgaf_daemon::input::codes`.** An assertion against
//! wgaf's own table would agree with any typo in it — which is the precise
//! mechanism by which the letter bug survived, restated as a rule.
//!
//! The GDK key names are a secondary check, and deliberately partial. They are
//! asserted only where the name does not depend on the session's keyboard
//! layout: `Escape` and `Left` are what they are anywhere, while the name of the
//! key beside the left shift is whatever the local layout prints on it. Where
//! the layout has a say, the keycode still carries the assertion.
//!
//! # Keys this deliberately does not press
//!
//! Seven of the keys the daemon advertises are excluded, each for a reason that
//! is a property of the desktop rather than a gap in the test:
//!
//! | Key | Why not |
//! |---|---|
//! | `leftmeta`, `rightmeta` (`super`) | GNOME Shell consumes them to open the overview, which takes focus and swallows everything after it. The application never sees them, and that is the compositor working correctly. |
//! | `sysrq` (`printscreen`) | GNOME binds it to the screenshot UI, which takes focus and would leave a file behind. |
//! | `capslock`, `numlock`, `scrolllock` | They toggle **persistent session state**. A test that leaves Caps Lock on inverts every letter the user types afterwards; a test must not change the state of the desktop it borrowed. |
//! | `compose` (`menu`) | Opens the entry's context menu, a popover that grabs input until dismissed. |
//!
//! That they cannot be verified this way is worth stating explicitly rather than
//! leaving them quietly absent from a table that claims to be exhaustive.

mod harness;

use std::time::Duration;

use harness::{Report, TestApp};

/// The X11 offset between an evdev keycode and the hardware keycode a toolkit
/// reports. Wayland inherited it; every GDK `keycode` is `evdev + 8`.
const EVDEV_TO_HARDWARE: u32 = 8;

/// How long one key gets to arrive before it is recorded as a failure.
///
/// Short on purpose. This suite makes about ninety checks in a loop, and a
/// systematic breakage fails all of them — at the harness's usual ten seconds
/// that would be a quarter of an hour of waiting to be told the same thing
/// ninety times.
const KEY_TIMEOUT: Duration = Duration::from_secs(2);

/// How many keys may fail to arrive before the run gives up.
///
/// A handful of failures is a table bug worth enumerating in full; everything
/// failing means the window lost focus or the device died, and the remaining
/// eighty checks would only restate that at two seconds each.
const CONSECUTIVE_FAILURE_LIMIT: usize = 5;

/// One key, and the physical key press it must produce.
struct KeyCase {
    /// The name given to `wgaf key press` / `KeyPress`.
    name: &'static str,
    /// Its evdev code, from `<linux/input-event-codes.h>`. See the module docs
    /// for why this is transcribed rather than imported.
    code: u32,
    /// GDK key names, any one of which is an acceptable resolution. Empty means
    /// arrival-only: the key's name depends on the keyboard layout, so only the
    /// keycode can be asserted.
    ///
    /// Several keypad entries list two names because the keypad's meaning
    /// depends on Num Lock, which this suite does not control and must not
    /// change.
    names: &'static [&'static str],
}

const fn key(name: &'static str, code: u32, names: &'static [&'static str]) -> KeyCase {
    KeyCase { name, code, names }
}

/// Every key the daemon advertises, minus the seven the module documentation
/// explains, in evdev code order so that a gap is visible by eye.
const KEYS: &[KeyCase] = &[
    key("escape", 1, &["Escape"]),
    key("1", 2, &[]),
    key("2", 3, &[]),
    key("3", 4, &[]),
    key("4", 5, &[]),
    key("5", 6, &[]),
    key("6", 7, &[]),
    key("7", 8, &[]),
    key("8", 9, &[]),
    key("9", 10, &[]),
    key("0", 11, &[]),
    key("minus", 12, &[]),
    key("equal", 13, &[]),
    key("backspace", 14, &["BackSpace"]),
    key("tab", 15, &["Tab"]),
    key("q", 16, &[]),
    key("w", 17, &[]),
    key("e", 18, &[]),
    key("r", 19, &[]),
    key("t", 20, &[]),
    key("y", 21, &[]),
    key("u", 22, &[]),
    key("i", 23, &[]),
    key("o", 24, &[]),
    key("p", 25, &[]),
    key("leftbrace", 26, &[]),
    key("rightbrace", 27, &[]),
    key("enter", 28, &["Return"]),
    key("leftctrl", 29, &["Control_L"]),
    key("a", 30, &[]),
    key("s", 31, &[]),
    key("d", 32, &[]),
    key("f", 33, &[]),
    key("g", 34, &[]),
    key("h", 35, &[]),
    key("j", 36, &[]),
    key("k", 37, &[]),
    key("l", 38, &[]),
    key("semicolon", 39, &[]),
    key("apostrophe", 40, &[]),
    key("grave", 41, &[]),
    key("leftshift", 42, &["Shift_L"]),
    key("backslash", 43, &[]),
    key("z", 44, &[]),
    key("x", 45, &[]),
    key("c", 46, &[]),
    key("v", 47, &[]),
    key("b", 48, &[]),
    key("n", 49, &[]),
    key("m", 50, &[]),
    key("comma", 51, &[]),
    key("dot", 52, &[]),
    key("slash", 53, &[]),
    key("rightshift", 54, &["Shift_R"]),
    key("kpasterisk", 55, &["KP_Multiply"]),
    key("leftalt", 56, &["Alt_L"]),
    key("space", 57, &["space"]),
    key("f1", 59, &["F1"]),
    key("f2", 60, &["F2"]),
    key("f3", 61, &["F3"]),
    key("f4", 62, &["F4"]),
    key("f5", 63, &["F5"]),
    key("f6", 64, &["F6"]),
    key("f7", 65, &["F7"]),
    key("f8", 66, &["F8"]),
    key("f9", 67, &["F9"]),
    key("f10", 68, &["F10"]),
    key("kp7", 71, &["KP_7", "KP_Home"]),
    key("kp8", 72, &["KP_8", "KP_Up"]),
    key("kp9", 73, &["KP_9", "KP_Page_Up", "KP_Prior"]),
    key("kpminus", 74, &["KP_Subtract"]),
    key("kp4", 75, &["KP_4", "KP_Left"]),
    key("kp5", 76, &["KP_5", "KP_Begin"]),
    key("kp6", 77, &["KP_6", "KP_Right"]),
    key("kpplus", 78, &["KP_Add"]),
    key("kp1", 79, &["KP_1", "KP_End"]),
    key("kp2", 80, &["KP_2", "KP_Down"]),
    key("kp3", 81, &["KP_3", "KP_Page_Down", "KP_Next"]),
    key("kp0", 82, &["KP_0", "KP_Insert"]),
    key("kpdot", 83, &["KP_Decimal", "KP_Delete", "KP_Separator"]),
    // Arrival-only, and the clearest illustration of why the keycode is the real
    // assertion: this key is reported by the character the layout prints on it,
    // which is `less` on a Danish keymap and something else on the next one.
    key("102nd", 86, &[]),
    key("f11", 87, &["F11"]),
    key("f12", 88, &["F12"]),
    key("kpenter", 96, &["KP_Enter"]),
    key("rightctrl", 97, &["Control_R"]),
    key("kpslash", 98, &["KP_Divide"]),
    // `Alt_R` on a US keymap; `ISO_Level3_Shift` wherever the layout makes it
    // the third-level modifier, which is most of Europe. Both are correct, and
    // which one appears says nothing about wgaf.
    key("rightalt", 100, &["Alt_R", "ISO_Level3_Shift"]),
    key("home", 102, &["Home"]),
    key("up", 103, &["Up"]),
    key("pageup", 104, &["Page_Up", "Prior"]),
    key("left", 105, &["Left"]),
    key("right", 106, &["Right"]),
    key("end", 107, &["End"]),
    key("down", 108, &["Down"]),
    key("pagedown", 109, &["Page_Down", "Next"]),
    key("insert", 110, &["Insert"]),
    key("delete", 111, &["Delete"]),
    key("pause", 119, &["Pause"]),
];

/// evdev code for `KEY_LEFTSHIFT`, from `<linux/input-event-codes.h>` — the
/// modifier `TypeText` wraps around a capital.
const KEY_LEFTSHIFT: u32 = 42;
/// evdev code for `KEY_A`.
const KEY_A: u32 = 30;

/// One key event as `input-test` reported it.
#[derive(Debug)]
struct Observed {
    name: Option<String>,
    pressed: bool,
    keycode: u32,
    shift: bool,
}

fn observed_keys(report: &Report) -> Vec<Observed> {
    report
        .array("keys")
        .iter()
        .map(|event| Observed {
            name: event
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            pressed: event
                .get("pressed")
                .and_then(serde_json::Value::as_bool)
                .expect("a reported key event has a `pressed` flag"),
            keycode: event
                .get("keycode")
                .and_then(serde_json::Value::as_u64)
                .expect("a reported key event has a `keycode`") as u32,
            shift: event
                .get("shift")
                .and_then(serde_json::Value::as_bool)
                .expect("a reported key event has a `shift` flag"),
        })
        .collect()
}

/// Drives every advertised key and checks what the application received.
///
/// One test rather than one per key, and one application instance rather than
/// ninety: spawning a daemon and a GTK4 window per key would take minutes to
/// tell you what this tells you in seconds. Failures are collected rather than
/// asserted immediately, so a broken table reports every key it broke — fixing
/// them one failing run at a time is how a ninety-entry table takes an afternoon.
#[tokio::test]
#[ignore = "takes over the desktop: synthesizes real keystrokes into a real session. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn every_advertised_key_presses_the_physical_key_it_names() {
    harness::require_wayland_session();
    harness::require_uinput();

    let pid = std::process::id();
    let bus_name = format!("org.wgaf.Test.KeyboardCoverage{pid}");
    // `/proc/bus/input/devices` is machine-global and records nothing about
    // which process created an entry, so the device is named per test run.
    let _daemon = harness::spawn_daemon(
        "keyboard-coverage",
        &bus_name,
        &format!("input_device_name = \"wgaf-keyboard-coverage-{pid}\"\n"),
    );
    let connection = harness::wait_for_daemon(&bus_name).await;

    let app = TestApp::spawn("input-test").await;

    // A precondition, not an assertion. Input goes to whatever holds keyboard
    // focus; if that is not this window then every check below fails while wgaf
    // is working perfectly, and the keystrokes land somewhere they were never
    // meant to go.
    app.wait_for("the test window to take keyboard focus", |report| {
        report.bool("window_focused")
    })
    .await;

    harness::warm_up_input_device(&connection, &bus_name).await;

    let mut failures: Vec<String> = Vec::new();
    let mut consecutive_missing = 0usize;

    for case in KEYS {
        let before = app
            .read()
            .expect("input-test stopped reporting")
            .u64("key_event_count");

        harness::input::<(), _>(&connection, &bus_name, "KeyPress", &(case.name,))
            .await
            .unwrap_or_else(|err| panic!("KeyPress({}) failed: {err}", case.name));
        harness::input::<(), _>(&connection, &bus_name, "KeyRelease", &(case.name,))
            .await
            .unwrap_or_else(|err| panic!("KeyRelease({}) failed: {err}", case.name));

        // Both halves, not just the press: a key that is pressed and never
        // released leaves a modifier stuck down, which would break every key
        // after it and the session along with them.
        let arrived = app
            .try_wait_for(KEY_TIMEOUT, |report| {
                report.u64("key_event_count") >= before + 2
            })
            .await;

        let report = match arrived {
            Ok(report) => {
                consecutive_missing = 0;
                report
            }
            Err(_) => {
                failures.push(format!(
                    "`{}` (evdev {}): no press/release reached the application",
                    case.name, case.code
                ));
                consecutive_missing += 1;
                if consecutive_missing >= CONSECUTIVE_FAILURE_LIMIT {
                    panic!(
                        "{consecutive_missing} keys in a row never arrived, so the run stopped. \
                         The window most likely lost focus, or the device went away. \
                         Failures so far:\n{}",
                        failures.join("\n")
                    );
                }
                continue;
            }
        };

        let events = observed_keys(&report);
        let Some((release, press)) = events.last().and_then(|release| {
            events
                .get(events.len().wrapping_sub(2))
                .map(|p| (release, p))
        }) else {
            failures.push(format!(
                "`{}` (evdev {}): fewer than two events in the report",
                case.name, case.code
            ));
            continue;
        };

        let expected = case.code + EVDEV_TO_HARDWARE;

        if !press.pressed || release.pressed {
            failures.push(format!(
                "`{}` (evdev {}): expected a press then a release, got pressed={} then pressed={}",
                case.name, case.code, press.pressed, release.pressed
            ));
        }

        for (event, half) in [(press, "press"), (release, "release")] {
            if event.keycode != expected {
                failures.push(format!(
                    "`{}` (evdev {}): {half} arrived as hardware keycode {} (evdev {}), expected {} — \
                     wgaf pressed a different physical key",
                    case.name,
                    case.code,
                    event.keycode,
                    event.keycode.saturating_sub(EVDEV_TO_HARDWARE),
                    expected
                ));
            }
        }

        if !case.names.is_empty() {
            let reported = press.name.as_deref().unwrap_or("<none>");
            if !case.names.contains(&reported) {
                failures.push(format!(
                    "`{}` (evdev {}): the toolkit resolved it to `{reported}`, expected one of {:?}",
                    case.name, case.code, case.names
                ));
            }
        }
    }

    // Checked last as well as first: a focus loss part-way through explains a
    // block of failures far better than the failures themselves do.
    let still_focused = app
        .read()
        .expect("input-test stopped reporting")
        .bool("window_focused");

    assert!(
        failures.is_empty(),
        "{} of {} keys did not arrive as the physical key they name{}:\n{}",
        failures.len(),
        KEYS.len(),
        if still_focused {
            ""
        } else {
            " (and the window had lost focus by the end of the run, which may be the whole cause)"
        },
        failures.join("\n")
    );
}

/// `TypeText` holds shift around a capital, and lets go of it afterwards.
///
/// Verified here rather than in `tests/input.rs` for the reason this whole suite
/// exists: that suite reads the events back from the kernel, so it can only
/// confirm wgaf emitted what wgaf decided to emit. This confirms an application
/// saw the capital arrive *with the shift modifier actually applied*, which is a
/// different claim.
///
/// Asserted on keycodes and the modifier flag, never on the entry's text: the
/// character `A` depends on the session's keyboard layout, and the physical key
/// wgaf pressed does not.
#[tokio::test]
#[ignore = "takes over the desktop: synthesizes real keystrokes into a real session. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn type_text_wraps_a_capital_in_shift_and_releases_it() {
    harness::require_wayland_session();
    harness::require_uinput();

    let pid = std::process::id();
    let bus_name = format!("org.wgaf.Test.KeyboardShift{pid}");
    let _daemon = harness::spawn_daemon(
        "keyboard-shift",
        &bus_name,
        &format!("input_device_name = \"wgaf-keyboard-shift-{pid}\"\n"),
    );
    let connection = harness::wait_for_daemon(&bus_name).await;

    let app = TestApp::spawn("input-test").await;
    app.wait_for("the test window to take keyboard focus", |report| {
        report.bool("window_focused")
    })
    .await;
    harness::warm_up_input_device(&connection, &bus_name).await;

    // Lowercase then uppercase, same physical key, so the difference between
    // them is the modifier and nothing else.
    harness::input::<(), _>(&connection, &bus_name, "TypeText", &("aA",))
        .await
        .expect("TypeText failed");

    // Six events: `a` down/up, shift down, `A` down/up, shift up.
    let report = app
        .wait_for("six key events from typing `aA`", |report| {
            report.u64("key_event_count") >= 6
        })
        .await;

    let events = observed_keys(&report);
    let hardware = |code: u32| code + EVDEV_TO_HARDWARE;

    let lowercase = events
        .iter()
        .position(|e| e.keycode == hardware(KEY_A) && e.pressed && !e.shift)
        .expect("no unshifted `a` press was reported");
    let shift_down = events
        .iter()
        .position(|e| e.keycode == hardware(KEY_LEFTSHIFT) && e.pressed)
        .expect("no shift press was reported — `TypeText` did not hold shift for the capital");
    let capital = events
        .iter()
        .position(|e| e.keycode == hardware(KEY_A) && e.pressed && e.shift)
        .expect("no `a` press carrying the shift modifier was reported");
    let shift_up = events
        .iter()
        .position(|e| e.keycode == hardware(KEY_LEFTSHIFT) && !e.pressed)
        .expect("shift was pressed and never released, which would leave the session stuck");

    assert!(
        lowercase < shift_down,
        "the lowercase `a` must be typed before shift goes down; events: {events:#?}"
    );
    assert!(
        shift_down < capital,
        "shift must go down before the capital, not after it; events: {events:#?}"
    );
    assert!(
        capital < shift_up,
        "shift must stay down until the capital has been released; events: {events:#?}"
    );
}
