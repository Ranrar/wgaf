//! One window, one session, every subsystem in sequence.
//!
//! Focus it, resize it, put the pointer on its button, click the button, scroll
//! it — each step verified from the application's own report before the next
//! one starts, with a running log so that watching it tells you where it got to.
//!
//! # Why a combined test earns its place beside the focused ones
//!
//! `window_management.rs`, `keyboard_coverage.rs` and `pointer.rs` each drive
//! one subsystem in isolation, which is the right shape for locating a fault.
//! None of them covers the thing a user actually does, which is to combine them
//! — and combinations are where state left behind by one step breaks the next.
//! A window that is focused when resized, a pointer aimed at a button whose
//! position moved because of that resize, a scroll that needs the pointer to
//! still be over the window: each of those couplings is invisible to a suite
//! that does one thing.
//!
//! It is deliberately *not* a replacement for the focused suites. When this
//! fails, it says the sequence broke; it takes one of the others to say why.
//!
//! # Rule 1 holds throughout, including for the click
//!
//! Every assertion is read from `input-test`'s JSON report. wgaf's own replies
//! are used only where they are the *instruction* rather than the evidence: the
//! window id to act on, and the window's origin to convert a window-relative
//! coordinate into a global one.
//!
//! The click step deserves a note, because it looks closer to the line than it
//! is. The application reports where its button is; wgaf aims there; the
//! application reports that the button was activated. Both ends are GTK, and
//! nothing asks wgaf whether wgaf succeeded. Hardcoding a coordinate instead
//! would be *worse* than either: it would couple this test to the fixture's
//! current margins, and a layout change would land the click on empty space,
//! increment `clicks`, leave `button_activations` at zero, and blame wgaf.
//!
//! **This step does lean on one property it does not itself prove** — that
//! wgaf's coordinate space agrees with the application's. `tests/pointer.rs`
//! proves that independently and exactly (delta `(0.0, 0.0)`). If that suite
//! ever goes away, this one silently becomes weaker, so the click is also
//! asserted against the coordinates the application reports for it rather than
//! only against the activation count.
//!
//! # This test is incomplete on purpose — extend it when the gaps close
//!
//! Two of the steps a full round trip would include **cannot be written yet**,
//! because the operations do not exist:
//!
//! - **Minimise and maximise.** Nothing in wgaf sets either. The window record
//!   *reports* `maximized`, and `window-test` reports `maximized`/`fullscreen`
//!   too, so both fixtures are already able to verify it — the missing half is
//!   entirely on wgaf's side: no `MinimizeWindow`/`MaximizeWindow` on the
//!   extension, no proxy method, no capability, no CLI command.
//! - **Scrolling content.** `wgaf mouse scroll` synthesizes the event and
//!   `input-test` counts it arriving, but the application has nothing
//!   scrollable, so a test can prove the scroll *arrived* and not that anything
//!   *moved*. Closing this needs a `GtkScrolledWindow` in the fixture reporting
//!   its adjustment value — at which point the scroll step below should assert
//!   on the position rather than on the counter.
//!
//! **When either lands, extend this test rather than writing a new one.** The
//! value here is in the sequence being unbroken; a second suite covering
//! "minimise, then the same five steps" would duplicate the setup and still not
//! test the coupling. The steps are numbered and logged, so inserting one is
//! meant to be easy. Tracked in `backlog.md`.

mod harness;

use std::time::Duration;

use harness::TestApp;
use wgaf_common::WindowRecord;
use wgaf_common::dict::WindowRecordDict;

const SUITE: &str = "combined";

/// The size the window is resized to mid-test.
///
/// Deliberately different from the fixture's 640x480 starting size in both
/// dimensions, so a resize that changed only one would fail rather than pass by
/// half. Comfortably above the content's minimum size, which would otherwise
/// clamp the result and look like wgaf ignoring the request — the trap W9.1
/// recorded after the dialog that asked for 320 wide and got 451.
const RESIZED: (i32, i32) = (720, 560);

/// Scroll amount for the final step. Positive `dy` is up in wgaf's convention,
/// which arrives at GDK as negative — the sign flip `input-test` documents.
const SCROLL: (i32, i32) = (0, 3);

/// Prints a step banner, so a run reads as a sequence rather than a silence.
///
/// `cargo test` hides this unless the test fails or `--nocapture` is passed,
/// which is the right default: it is diagnostic output, and a passing run
/// should be quiet.
fn step(n: u32, what: &str) {
    println!("  [{n}] {what}");
}

fn detail(what: &str) {
    println!("      {what}");
}

/// The single sequence. One test rather than several, because the point is that
/// the steps follow one another against one window — splitting them would give
/// each a fresh window and test nothing that the focused suites do not.
#[tokio::test]
#[ignore = "needs a live GNOME Wayland session with the wgaf extension installed"]
async fn a_window_is_focused_resized_clicked_and_scrolled_in_sequence() {
    harness::require_wayland_session();
    harness::require_uinput();
    harness::require_test_app("input-test");

    let bus_name = format!("org.wgaf.Test.Combined{}", std::process::id());
    let _daemon = harness::spawn_daemon(SUITE, &bus_name, "");
    let connection = harness::wait_for_daemon(&bus_name).await;

    if let Err(err) =
        harness::windows::<Vec<WindowRecordDict>, _>(&connection, &bus_name, "ListWindows", &())
            .await
    {
        panic!(
            "this suite needs the wgaf GNOME Shell Extension installed and enabled \
             (`make install`, then log out and back in once — Wayland has no in-session \
             Shell restart). ListWindows failed: {err}"
        );
    }

    // The first synthesized input command after a daemon starts can be lost
    // while the virtual device is still settling. Paying that cost here, moving
    // nothing, keeps it out of the click step where it would look like a miss.
    harness::warm_up_input_device(&connection, &bus_name).await;

    println!("\ncombined sequence:");

    let app = TestApp::spawn("input-test").await;
    let window = wait_for_mapped_window(&connection, &bus_name).await;
    step(
        0,
        &format!(
            "input-test is up: window {} at ({}, {}) {}x{}",
            window.id, window.x, window.y, window.width, window.height
        ),
    );

    // --- 1. Focus ---------------------------------------------------------
    step(1, "focus the window");
    harness::windows::<(), _>(&connection, &bus_name, "FocusWindow", &(window.id,))
        .await
        .expect("FocusWindow should succeed");
    let report = app
        .wait_for("the application to report itself focused", |r| {
            r.bool("window_focused")
        })
        .await;
    detail(&format!(
        "window_focused = {}",
        report.bool("window_focused")
    ));

    // --- 2. Resize --------------------------------------------------------
    step(2, &format!("resize to {}x{}", RESIZED.0, RESIZED.1));
    harness::windows::<(), _>(
        &connection,
        &bus_name,
        "ResizeWindow",
        &(window.id, RESIZED.0, RESIZED.1),
    )
    .await
    .expect("ResizeWindow should succeed");
    let report = app
        .wait_for(
            &format!("the application to report {}x{}", RESIZED.0, RESIZED.1),
            |r| r.i64("width") == i64::from(RESIZED.0) && r.i64("height") == i64::from(RESIZED.1),
        )
        .await;
    detail(&format!(
        "application reports {}x{}",
        report.i64("width"),
        report.i64("height")
    ));

    // --- 3. Aim at the button --------------------------------------------
    //
    // Re-read the window rather than reusing step 0's rectangle: the resize has
    // changed its size, and `wgaf window resize` returns before that change is
    // readable — see `aim_at_button`, which is why this is a converging loop
    // rather than a single measurement.
    step(3, "move the pointer onto the button");
    let (button, report) = aim_at_button(&connection, &bus_name, &app).await;
    detail(&format!(
        "application sees the pointer at ({:.0}, {:.0}), inside the button at {button}",
        report.f64("pointer_x"),
        report.f64("pointer_y")
    ));

    // --- 4. Click the button ---------------------------------------------
    step(4, "click the left button");
    let activations_before = report.u64("button_activations");
    harness::input::<(), _>(&connection, &bus_name, "MouseClick", &("left",))
        .await
        .expect("MouseClick should succeed");
    let report = app
        .wait_for("the button to be activated", |r| {
            r.u64("button_activations") > activations_before
        })
        .await;
    detail(&format!(
        "button_activations {} -> {}",
        activations_before,
        report.u64("button_activations")
    ));

    // The activation count alone would pass even if the click had landed
    // somewhere else and GTK had activated the button for an unrelated reason
    // (a keyboard default, say). Checking where the application says the click
    // landed makes the aim part of the assertion instead of an assumption —
    // see this module's note about what the click step leans on.
    let last_click = report
        .array("clicks")
        .last()
        .expect("a click should have been recorded")
        .clone();
    let (click_x, click_y) = (
        last_click["x"].as_f64().expect("click x"),
        last_click["y"].as_f64().expect("click y"),
    );
    detail(&format!("click recorded at ({click_x:.0}, {click_y:.0})"));
    assert!(
        button.contains(click_x, click_y),
        "the click was recorded at ({click_x:.0}, {click_y:.0}), which is outside the \
         button at {button} — the button was activated, but not by a click that landed \
         on it"
    );

    // --- 5. Scroll --------------------------------------------------------
    //
    // Asserted on the counter, not on any content moving: `input-test` has
    // nothing scrollable. See this module's header — this step is the one to
    // strengthen when the fixture grows a GtkScrolledWindow.
    step(5, &format!("scroll by {SCROLL:?}"));
    let scrolls_before = report.u64("scroll_count");
    harness::input::<(), _>(&connection, &bus_name, "MouseScroll", &SCROLL)
        .await
        .expect("MouseScroll should succeed");
    let report = app
        .wait_for("the scroll to arrive", |r| {
            r.u64("scroll_count") > scrolls_before
        })
        .await;
    detail(&format!(
        "scroll_count {} -> {}, accumulated dy {:.1}",
        scrolls_before,
        report.u64("scroll_count"),
        report.f64("scroll_dy")
    ));

    // wgaf's positive dy is up (the kernel's REL_WHEEL convention); GDK's
    // positive dy is down. A test that expected them to agree would fail on a
    // correct implementation, so the flip is asserted rather than tolerated.
    assert!(
        report.f64("scroll_dy") < 0.0,
        "scrolling up in wgaf's convention must arrive as a negative GDK dy, got {}",
        report.f64("scroll_dy")
    );

    // --- Still coherent at the end ---------------------------------------
    //
    // Five subsystems have acted on this window. The cheapest way to catch one
    // of them having left it in a bad state is to ask whether the things
    // established earlier are still true.
    step(
        6,
        "the window is still focused and still the size it was set to",
    );
    let report = app.read().expect("a final report");
    assert!(
        report.bool("window_focused"),
        "the window lost focus during the sequence — later steps were acting on \
         something else"
    );
    assert_eq!(
        (report.i64("width"), report.i64("height")),
        (i64::from(RESIZED.0), i64::from(RESIZED.1)),
        "the window is no longer the size step 2 set it to"
    );
    detail("sequence complete");
}

/// Puts the pointer on the button, re-reading and re-aiming until the
/// application confirms it is there.
///
/// **Why this is a loop and not a single warp.** `wgaf window resize` returns
/// before the new geometry is readable — for about 30ms afterwards
/// `ListWindows` still reports the *old* rectangle while the application
/// already has the new one, and the layout inside it is still reflowing (the
/// button grows from 616 to 696 wide as the window widens). Aiming once from
/// whatever is readable at that moment misses; measured, the pointer landed
/// above the button entirely, roughly one run in five.
///
/// The lag is filed as an S3 in `issues.md`. **This loop is a workaround for a
/// test, not a fix** — a user writing the same resize-then-compute-a-coordinate
/// script has no equivalent, which is the argument in that issue for making the
/// daemon confirm the resize the way `warpPointer` confirms a warp.
///
/// Note the window does **not** move when resized: the origin was identical
/// before and after in every measurement. That was asserted during diagnosis
/// and is false, and it is written down here because it was believed for long
/// enough to send the investigation the wrong way.
///
/// Retrying converges without needing to know how long any of that takes, and
/// without a sleep that would be too short on a loaded machine and wasted time
/// on an idle one. Each attempt re-reads **both** rectangles, because either can
/// still be moving.
///
/// Returns the button rectangle that the successful aim used, so the caller can
/// assert the click against the same geometry rather than re-reading it.
async fn aim_at_button(
    connection: &zbus::Connection,
    bus_name: &str,
    app: &TestApp,
) -> (ButtonRect, harness::Report) {
    let mut last = String::new();
    for _ in 0..50 {
        let window = wait_for_mapped_window(connection, bus_name).await;
        let button = ButtonRect::read(app);
        let (cx, cy) = button.centre();
        let target = (window.x + cx.round() as i32, window.y + cy.round() as i32);

        let reached: (i32, i32) =
            harness::input(connection, bus_name, "MouseMoveAbsolute", &target)
                .await
                .expect("MouseMoveAbsolute should succeed");
        assert_eq!(
            reached, target,
            "the pointer did not reach the coordinate it was sent to"
        );

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Some(report) = app.read()
            && report.bool("pointer_in_window")
            && button.contains(report.f64("pointer_x"), report.f64("pointer_y"))
        {
            return (button, report);
        }
        if let Some(report) = app.read() {
            // Read through the raw JSON rather than the typed accessor: before
            // the pointer has ever entered the window these are `null`, and
            // `f64` would panic here — turning a useful diagnostic into a
            // second, less informative failure.
            let seen = |field: &str| {
                report
                    .json()
                    .get(field)
                    .and_then(serde_json::Value::as_f64)
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "never in window".to_string())
            };
            last = format!(
                "aimed at window ({}, {}) + button centre ({cx:.0}, {cy:.0}); \
                 the application saw the pointer at ({}, {}) with the button at {button}",
                window.x,
                window.y,
                seen("pointer_x"),
                seen("pointer_y")
            );
        }
    }
    panic!("the pointer never landed on the button. Last attempt: {last}");
}

/// The button's rectangle in window-relative logical pixels, as the application
/// itself reports it.
///
/// Read from the fixture rather than hardcoded, so the aim tracks the layout.
/// See this module's header for why that is not the test marking its own
/// homework: this is where to aim, and the assertions are the activation count
/// and the click coordinates GTK records.
struct ButtonRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl ButtonRect {
    fn read(app: &TestApp) -> Self {
        let report = app.read().expect("a report");
        let json = report.json();
        let value = |field: &str| {
            json.get(field)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_else(|| {
                    panic!(
                        "input-test did not report `{field}` — it must publish its button's \
                         bounds for a coordinate click to be aimed honestly. Full report: \
                         {json:#}"
                    )
                })
        };
        Self {
            x: value("button_x"),
            y: value("button_y"),
            width: value("button_width"),
            height: value("button_height"),
        }
    }

    fn centre(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

impl std::fmt::Display for ButtonRect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({:.0}, {:.0}) {:.0}x{:.0}",
            self.x, self.y, self.width, self.height
        )
    }
}

/// Waits until `input-test`'s window is listed with a real size.
///
/// Mutter lists a window before its frame rectangle exists, answering `0x0` at
/// `(0,0)` until the surface is committed.
async fn wait_for_mapped_window(connection: &zbus::Connection, bus_name: &str) -> WindowRecord {
    for _ in 0..100 {
        let windows: Vec<WindowRecordDict> =
            harness::windows(connection, bus_name, "ListWindows", &())
                .await
                .expect("ListWindows should succeed");
        let found = windows
            .into_iter()
            .map(WindowRecordDict::into)
            .find(|w: &WindowRecord| w.title == "wgaf input-test" && w.width > 0 && w.height > 0);
        if let Some(window) = found {
            return window;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("input-test's window was never listed with a non-zero size");
}
