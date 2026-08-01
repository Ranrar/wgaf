//! `wgaf type` against the session's real keyboard layout, checked by reading
//! the text a real application actually received.
//!
//! # What this exists to catch
//!
//! wgaf synthesizes key *positions*; the compositor applies the session's keymap
//! to decide what character each position produces. For seven phases wgaf
//! assumed those positions were US-QWERTY, so on a Danish keyboard
//! `wgaf type "user@example.com"` wrote `user"example.com` and reported success.
//! `@ $ { } [ ] | \` — every one of them an AltGr combination there — could not
//! be produced at all.
//!
//! Unit tests cover the mapping itself against keymaps compiled from names, and
//! they cover eight layouts rather than the one this machine has. What they
//! cannot cover is the rest of the path: `uinput`, the kernel, libinput, the
//! compositor, the input method and the toolkit all sit between a resolved
//! keystroke and a character appearing in an entry, and every one of them can
//! turn the right key into the wrong character.
//!
//! # This suite asserts on the entry's text, not on key events
//!
//! That is the opposite of `keyboard_coverage.rs`, deliberately. There, the
//! character is incidental and the hardware keycode is the load-bearing fact,
//! because the key's *name* depends on the layout. Here the character **is** the
//! point: the whole feature is "the text you asked for is the text that
//! arrives", and only the entry's contents can say whether that happened.
//!
//! # It runs against whatever layout this machine has
//!
//! There is no fixture keymap here and there cannot be — the compositor decides
//! the layout, not the test. So the suite asserts things that must hold on
//! *every* layout: ASCII text arrives verbatim, a character the layout cannot
//! produce is refused by name, and a combination releases everything it pressed.
//!
//! On a US session this passes trivially, since the old table was already
//! correct there. On the maintainer's Danish session it is a real test, and it
//! is the one that would have caught the original defect.

mod harness;

use std::time::Duration;

use harness::TestApp;

/// How long text gets to travel from `uinput` to the entry.
///
/// Longer than `keyboard_coverage.rs`'s per-key budget because a whole string
/// is in flight, and because a composed character is two keystrokes the input
/// method has to see in order.
const TYPE_TIMEOUT: Duration = Duration::from_secs(3);

/// Sets up a daemon and a focused `input-test` window.
///
/// Returns the guard, the connection and the app; the guard must be held for
/// the test's lifetime or the daemon is killed underneath it.
async fn session(suite: &str) -> (harness::DaemonGuard, zbus::Connection, TestApp, String) {
    harness::require_wayland_session();
    harness::require_uinput();

    let pid = std::process::id();
    let bus_name = format!("org.wgaf.Test.{suite}{pid}");
    let daemon = harness::spawn_daemon(
        suite,
        &bus_name,
        &format!("input_device_name = \"wgaf-{suite}-{pid}\"\n"),
    );
    let connection = harness::wait_for_daemon(&bus_name).await;

    let app = TestApp::spawn("input-test").await;

    // A precondition, not an assertion. Input goes wherever keyboard focus is;
    // if that is not this window then every check below fails while wgaf is
    // working perfectly, and the keystrokes land somewhere they were never
    // meant to go.
    app.wait_for("the test window to take keyboard focus", |report| {
        report.bool("window_focused")
    })
    .await;

    harness::warm_up_input_device(&connection, &bus_name).await;

    (daemon, connection, app, bus_name)
}

/// Types `text` and returns whatever ended up in the entry.
async fn type_and_read(
    connection: &zbus::Connection,
    bus_name: &str,
    app: &TestApp,
    text: &str,
) -> String {
    let baseline = app.seq();

    harness::input::<(), _>(connection, bus_name, "TypeText", &(text,))
        .await
        .unwrap_or_else(|err| panic!("TypeText({text:?}) failed: {err}"));

    // Wait for the entry to hold the expected text rather than for any report:
    // the characters arrive one keystroke at a time, so an early report shows a
    // prefix, which would look like a truncation bug.
    let settled = app
        .try_wait_for(TYPE_TIMEOUT, |report| {
            report.seq() > baseline && report.str("typed").ends_with(text)
        })
        .await;

    match settled {
        Ok(report) => report.str("typed").to_string(),
        // Return what did arrive so the assertion can show the difference,
        // rather than failing here with a timeout that hides it.
        Err(Some(report)) => report.str("typed").to_string(),
        Err(None) => panic!("input-test stopped reporting"),
    }
}

/// The headline case, and the exact string from the defect report.
///
/// On a Danish session the old table produced `user"example.com`: `@` is
/// AltGr+2 there and Shift+2 is `"`. Every character here is ASCII, so this must
/// hold on any layout whatsoever.
#[tokio::test]
#[ignore = "takes over the desktop: types real text into a real session. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn an_email_address_arrives_exactly_as_asked() {
    let (_daemon, connection, app, bus_name) = session("KeyboardLayout").await;

    let typed = type_and_read(&connection, &bus_name, &app, "user@example.com").await;

    assert_eq!(
        typed, "user@example.com",
        "the entry received something other than what was typed — this is the \
         layout defect if `@` came out as `\"`"
    );
}

/// Every character the S2 defect named, in one string.
///
/// All eight are AltGr combinations on Danish and Shift combinations on US, so
/// this exercises both the third level and the ordinary shifted one depending on
/// where it runs.
#[tokio::test]
#[ignore = "takes over the desktop: types real text into a real session."]
async fn the_characters_the_ascii_table_could_not_produce_all_arrive() {
    let (_daemon, connection, app, bus_name) = session("KeyboardLayoutSymbols").await;

    const SYMBOLS: &str = "@$ { } [ ] | \\";
    let typed = type_and_read(&connection, &bus_name, &app, SYMBOLS).await;

    assert_eq!(
        typed, SYMBOLS,
        "at least one third-level character came out wrong"
    );
}

/// A character behind a dead key, which is two keystrokes and needs the
/// application's input method to compose them.
///
/// `~` is `dead_tilde` then Space on a Danish layout and a plain shifted key on
/// US; either way the text is the same, which is the whole point. The harness
/// runs the application with `GTK_IM_MODULE=gtk-im-context-simple`, and that is
/// what makes the composition happen — wgaf presses two keys and composes
/// nothing itself.
#[tokio::test]
#[ignore = "takes over the desktop: types real text into a real session."]
async fn a_character_behind_a_dead_key_arrives_composed() {
    let (_daemon, connection, app, bus_name) = session("KeyboardLayoutDeadKey").await;

    let typed = type_and_read(&connection, &bus_name, &app, "~/notes.md").await;

    assert_eq!(
        typed, "~/notes.md",
        "a dead-key character did not compose — `~` is in every shell path, so \
         this is not an edge case"
    );
}

/// A character no ordinary layout can produce is refused **by name**, and
/// nothing is typed.
///
/// The refusal matters as much as the message. `TypeText` resolves the whole
/// string before pressing anything, so a string containing one impossible
/// character leaves the entry untouched rather than half-filled — half a command
/// in a terminal is worse than none.
#[tokio::test]
#[ignore = "takes over the desktop: types real text into a real session."]
async fn an_impossible_character_types_nothing_and_says_why() {
    let (_daemon, connection, app, bus_name) = session("KeyboardLayoutRefusal").await;

    // Establish that typing works at all, so a failure below is about the
    // refusal rather than about the session.
    let typed = type_and_read(&connection, &bus_name, &app, "before").await;
    assert_eq!(typed, "before");

    let err = harness::input::<(), _>(&connection, &bus_name, "TypeText", &("a 😀 b",))
        .await
        .expect_err("an emoji has no key sequence on any ordinary layout");

    let message = err.to_string();
    assert!(
        message.contains("CharacterNotTypeable"),
        "expected a named error, got: {message}"
    );
    assert!(
        message.contains('😀'),
        "the error must name the character it could not type: {message}"
    );

    // Nothing after the refusal, and nothing before it either: the entry still
    // holds exactly what the successful call put there.
    let report = app.read().expect("input-test stopped reporting");
    assert_eq!(
        report.str("typed"),
        "before",
        "the refused call typed part of its string"
    );
}

/// A key combination presses everything and — the part that matters — releases
/// everything.
///
/// A modifier left held down makes the session behave as though the user is
/// leaning on Ctrl, with nothing on screen to say why, and it would break every
/// keystroke after it. Asserted on key events rather than on text, because a
/// combination is physical keys and produces no character.
#[tokio::test]
#[ignore = "takes over the desktop: synthesizes real keystrokes into a real session."]
async fn a_combination_releases_every_key_it_pressed() {
    let (_daemon, connection, app, bus_name) = session("KeyboardLayoutCombo").await;

    let before = app
        .read()
        .expect("input-test stopped reporting")
        .u64("key_event_count");

    // Three keys chosen for being unbound in GNOME: `super` opens the overview
    // and `printscreen` opens the screenshot UI, either of which would take
    // focus and swallow the rest of the run.
    let keys = vec![
        "leftctrl".to_string(),
        "leftshift".to_string(),
        "k".to_string(),
    ];
    harness::input::<(), _>(&connection, &bus_name, "Hotkey", &(&keys,))
        .await
        .expect("Hotkey failed");

    let report = app
        .try_wait_for(TYPE_TIMEOUT, |report| {
            report.u64("key_event_count") >= before + 6
        })
        .await
        .expect("six key events (three presses, three releases) should have arrived");

    let events = report.array("keys");
    let tail: Vec<_> = events.iter().rev().take(6).rev().collect();

    // evdev codes, transcribed from `<linux/input-event-codes.h>` rather than
    // imported from wgaf's own table — an assertion against our own constants
    // would agree with any typo in them.
    const EVDEV_TO_HARDWARE: u64 = 8;
    let pressed: Vec<u64> = tail
        .iter()
        .filter(|e| e["pressed"].as_bool().unwrap_or(false))
        .map(|e| e["keycode"].as_u64().unwrap_or_default() - EVDEV_TO_HARDWARE)
        .collect();
    let released: Vec<u64> = tail
        .iter()
        .filter(|e| !e["pressed"].as_bool().unwrap_or(true))
        .map(|e| e["keycode"].as_u64().unwrap_or_default() - EVDEV_TO_HARDWARE)
        .collect();

    assert_eq!(
        pressed,
        vec![29, 42, 37],
        "keys should be pressed in order: leftctrl, leftshift, k. Events: {tail:#?}"
    );
    assert_eq!(
        released,
        vec![37, 42, 29],
        "keys must be released in reverse order, so the combination never passes \
         through one it was not asked for. Events: {tail:#?}"
    );
}
