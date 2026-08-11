//! Window management driven against a real GNOME Shell, verified by the
//! application's own account of itself.
//!
//! # What is new here
//!
//! `tests/windows_stub.rs` fakes the GNOME Shell Extension entirely — it
//! implements `org.gnome.Shell.Extensions.Wgaf.V1` in Rust and points the daemon
//! at it. That is the right way to test the *daemon* without a GNOME session,
//! and it says nothing whatsoever about whether the extension works: not one
//! line of `extension/windows.js` executes in it.
//!
//! **This suite is the first thing in the project that runs that GJS.** The
//! whole path is real: the daemon's `org.wgaf.Windows1`, the D-Bus hop to the
//! extension, the extension's Mutter calls, and a real window on a real
//! compositor. What it verifies is not the extension's reply — that would be
//! checking wgaf against wgaf — but what the application on the other side
//! reports about itself afterwards.
//!
//! # What cannot be tested this way, and why
//!
//! **`wgaf window move` has no honest end-to-end test and is deliberately
//! absent from this file.** A Wayland client is never told where the compositor
//! placed it — there is no getter in GTK4 and no request in the protocol — so
//! `window-test` cannot report its position, and the only other source of one is
//! the extension's own reply. Asserting on that would be verifying the mover
//! with the mover. The absence is a property of Wayland, not an oversight, and
//! it should not be "fixed" by comparing `ListWindows` against itself.
//!
//! **Initial focus is not deterministic.** Three identical runs of
//! `window-test` gave focus to three different windows; the transient-for
//! relationship competes with presentation order. So the focus test asserts the
//! *transition* it causes, after reading which window happened to start focused.
//!
//! # Matching windows to the application
//!
//! By title, not by `app_id`. Two of `window-test`'s three windows report
//! `dev.wgaf.WindowTest` and the dialog reports `window-test`, because a plain
//! `gtk4::Window` is never associated with the `GtkApplication` and GTK falls
//! back to the program name (filed in `issues.md`). A filter on `app_id` would
//! silently drop exactly the window that exists to be the awkward case, and
//! would look like it worked.

mod harness;

use std::time::Duration;

use harness::{Report, TestApp};
use wgaf_common::WindowRecord;
use wgaf_common::dict::WindowRecordDict;

/// `window-test`'s fixed titles. Changing one of these in the application
/// breaks this suite by design — they are its contract with these tests.
const MAIN_TITLE: &str = "wgaf window-test";
const SECONDARY_TITLE: &str = "wgaf window-test — secondary";
const DIALOG_TITLE: &str = "wgaf window-test — dialog";

/// How long a window operation gets to show up in the application's report.
const SETTLE: Duration = Duration::from_secs(5);

/// Serializes this file's tests against each other.
///
/// Every instance of `window-test` draws windows with identical titles, and
/// these tests find their windows by title. Two instances running at once would
/// let one test resize or close the other's window and then wait for a change
/// its own application never made. The same hazard `tests/accessibility.rs`
/// documents for AT-SPI application names, for the same reason: the namespace is
/// global and the application has no per-instance identifier to hand out.
async fn lock_window_test() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// Everything a test needs: a daemon, a connection, and a running application.
struct Fixture {
    _daemon: harness::DaemonGuard,
    connection: zbus::Connection,
    bus_name: String,
    app: TestApp,
}

impl Fixture {
    /// Starts the daemon and `window-test`, and refuses to continue unless the
    /// extension is actually answering.
    async fn start(suite: &str) -> Self {
        harness::require_wayland_session();

        let pid = std::process::id();
        let bus_name = format!("org.wgaf.Test.Windows{suite}{pid}");
        // `extension_bus_name` is deliberately left at its default: the point of
        // this suite is the real extension.
        let daemon = harness::spawn_daemon(suite, &bus_name, "");
        let connection = harness::wait_for_daemon(&bus_name).await;

        // The precondition, stated as one line naming what to install rather
        // than left to surface as whichever assertion happens to fail first.
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

        wait_for_a_clean_slate(&connection, &bus_name).await;

        let app = TestApp::spawn("window-test").await;

        let fixture = Self {
            _daemon: daemon,
            connection,
            bus_name,
            app,
        };
        fixture.wait_until_every_window_is_mapped().await;
        fixture
    }

    /// Blocks until the compositor knows about all three windows.
    ///
    /// **The application's own report is not a readiness signal here, and
    /// looked like one.** `window_count` is 3 in `window-test`'s very first
    /// report, because the array describes all three windows from the moment
    /// they are constructed — whether or not the compositor has mapped any of
    /// them. Gating on it let the tests run while Mutter had seen exactly one
    /// window, and every one of them failed on the first run for that reason.
    ///
    /// Which one it had seen is the tell: `window-test` presents the dialog
    /// first, and the dialog was the only window `ListWindows` returned.
    ///
    /// So the readiness signal has to come from the compositor's side, which
    /// means asking wgaf. That is not verifying wgaf with wgaf — nothing is
    /// asserted here, and every assertion in this file still terminates in the
    /// application's own report. It is a gate, and it fails loudly rather than
    /// silently proceeding, so a genuine enumeration bug still surfaces as a
    /// timeout naming the windows that never appeared.
    ///
    /// **A window being listed is not a readiness signal either, and that
    /// looked like one too.** Mutter reports a window — correct id, title and
    /// `app_id` — before its frame rectangle exists, and `get_frame_rect()`
    /// answers `0x0 at (0,0)` until the surface has been committed. Measured,
    /// not inferred: a probe that gated on titles alone and then listed
    /// immediately caught all three windows at `0x0` in 2 runs out of 6, with
    /// every window correct again 100 ms later.
    ///
    /// That was half of this suite's flakiness — the other half was a missing
    /// `unmap` watcher in `window-test`, see the close test. Gating on titles
    /// let a test read a zero geometry and compare it against the size the
    /// application reports, which is how
    /// `list_windows_reports_every_window_the_application_has` failed roughly a
    /// third of the time with `left: 0, right: 640` — a number that came from
    /// the compositor, not from any defect in wgaf. So the gate waits for a
    /// non-zero size as well, which is the point at which the window genuinely
    /// has one.
    async fn wait_until_every_window_is_mapped(&self) {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let windows = self.list().await;
            let seen: Vec<&str> = windows.iter().map(|window| window.title.as_str()).collect();
            // A window with no size yet is not ready, so it counts as missing
            // rather than as present-but-odd: the two are the same wait.
            let missing: Vec<&str> = [MAIN_TITLE, SECONDARY_TITLE, DIALOG_TITLE]
                .into_iter()
                .filter(|title| {
                    !windows
                        .iter()
                        .any(|w| w.title == *title && w.width > 0 && w.height > 0)
                })
                .collect();

            if missing.is_empty() {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "the compositor never reported {missing:?} at a non-zero size within \
                     {SETTLE:?}, though window-test says it has them. \
                     `wgaf window list` saw: {seen:?}"
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn list(&self) -> Vec<WindowRecord> {
        list_windows(&self.connection, &self.bus_name).await
    }

    /// The window record for one of `window-test`'s titles, failing with what
    /// wgaf did see when it is not there.
    async fn window(&self, title: &str) -> WindowRecord {
        let windows = self.list().await;
        windows
            .iter()
            .find(|window| window.title == title)
            .cloned()
            .unwrap_or_else(|| {
                let seen: Vec<&str> = windows.iter().map(|w| w.title.as_str()).collect();
                panic!("`wgaf window list` did not report `{title}`. It reported: {seen:?}")
            })
    }
}

async fn list_windows(connection: &zbus::Connection, bus_name: &str) -> Vec<WindowRecord> {
    let records: Vec<WindowRecordDict> = harness::windows(connection, bus_name, "ListWindows", &())
        .await
        .expect("ListWindows failed against the real extension");
    records.into_iter().map(Into::into).collect()
}

/// Blocks until no `window-test` window is left over from an earlier run.
///
/// **Killing the application does not immediately remove its windows.**
/// `TestApp`'s `Drop` sends a signal and reaps the process, which is as much as
/// a synchronous `Drop` can do; Mutter unmaps the surfaces afterwards, on its
/// own schedule. So a run that starts promptly after another one finds the
/// previous instance's three windows still listed — with valid titles and valid
/// geometry, which is precisely what makes them indistinguishable from this
/// run's.
///
/// **This is precautionary, and it is worth being clear that it did not fix
/// anything.** It was added on 2026-08-01 while chasing this suite's
/// flakiness, on the theory that leftover windows were making tests drive a
/// dead instance. That theory was wrong — the flakiness was a missing `unmap`
/// watcher in `window-test` plus a gate that accepted zero geometry, both fixed
/// separately — and the wait has never once tripped since.
///
/// It stays because the hazard it guards is real even though it was not the
/// one being hunted: every window in this file is found by *title*, and two
/// instances of `window-test` present identical titles, so a leftover instance
/// would silently hand a test another process's window id. Cheap when the slate
/// is already clean, which is the normal case.
///
/// It has to happen here rather than in `TestApp`'s `Drop`, because it needs the
/// bus and a running daemon.
async fn wait_for_a_clean_slate(connection: &zbus::Connection, bus_name: &str) {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let leftover: Vec<String> = list_windows(connection, bus_name)
            .await
            .into_iter()
            .filter(|window| {
                [MAIN_TITLE, SECONDARY_TITLE, DIALOG_TITLE].contains(&window.title.as_str())
            })
            .map(|window| window.title)
            .collect();

        if leftover.is_empty() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{leftover:?} were still listed {SETTLE:?} after an earlier run should have \
                 finished with them. Every window here is found by title, so starting now \
                 would let this test drive another instance's windows. \
                 Close any stray `window-test` and re-run."
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The application's own report of the window playing `role`.
fn reported<'a>(report: &'a Report, role: &str) -> &'a serde_json::Value {
    report
        .array("windows")
        .iter()
        .find(|window| window.get("role").and_then(serde_json::Value::as_str) == Some(role))
        .unwrap_or_else(|| panic!("window-test reported no window with role `{role}`"))
}

fn reported_bool(report: &Report, role: &str, field: &str) -> bool {
    reported(report, role)
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| panic!("the `{role}` window has no `{field}` flag"))
}

fn reported_i64(report: &Report, role: &str, field: &str) -> i64 {
    reported(report, role)
        .get(field)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_else(|| panic!("the `{role}` window has no `{field}`"))
}

/// The role `window-test` gives the window with `title`.
fn role_for(title: &str) -> &'static str {
    match title {
        MAIN_TITLE => "main",
        SECONDARY_TITLE => "secondary",
        DIALOG_TITLE => "dialog",
        other => panic!("`{other}` is not one of window-test's windows"),
    }
}

/// `ListWindows` sees every window the application says it has, at the size the
/// application says it is.
///
/// The size comparison is only sound because these are client-side-decorated
/// GTK4 windows, where the frame rectangle Mutter tracks *is* the client's
/// surface — measured, not assumed, and not a property to rely on for a
/// server-side-decorated window.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn list_windows_reports_every_window_the_application_has() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("list").await;

    let report = fixture.app.read().expect("window-test stopped reporting");

    for title in [MAIN_TITLE, SECONDARY_TITLE, DIALOG_TITLE] {
        let window = fixture.window(title).await;
        let role = role_for(title);

        assert_eq!(
            i64::from(window.width),
            reported_i64(&report, role, "width"),
            "`wgaf window list` and the application disagree about the `{role}` window's width"
        );
        assert_eq!(
            i64::from(window.height),
            reported_i64(&report, role, "height"),
            "`wgaf window list` and the application disagree about the `{role}` window's height"
        );
    }
}

/// `FocusWindow` moves focus, verified by the application rather than by asking
/// wgaf what it just did.
///
/// Targets whichever window did *not* start focused, because which one does is
/// not deterministic — asserting on the starting state would fail roughly two
/// runs in three.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn focus_window_moves_focus_to_the_window_it_names() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("focus").await;

    let before = fixture.app.read().expect("window-test stopped reporting");
    let target_title = [MAIN_TITLE, SECONDARY_TITLE, DIALOG_TITLE]
        .into_iter()
        .find(|title| !reported_bool(&before, role_for(title), "focused"))
        .expect("all three windows claim focus at once, which cannot happen");
    let target_role = role_for(target_title);

    let target = fixture.window(target_title).await;
    harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "FocusWindow",
        &(target.id,),
    )
    .await
    .expect("FocusWindow failed against the real extension");

    fixture
        .app
        .try_wait_for(SETTLE, |report| {
            reported_bool(report, target_role, "focused")
        })
        .await
        .unwrap_or_else(|last| {
            panic!(
                "the `{target_role}` window never reported gaining focus after FocusWindow({}). \
                 Last report:\n{}",
                target.id,
                last.map(|r| format!("{:#}", r.json()))
                    .unwrap_or_else(|| "<none>".into())
            )
        });
}

/// `ResizeWindow` changes the size the application itself reports.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn resize_window_changes_the_size_the_application_reports() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("resize").await;

    // Comfortably above the main window's minimum content size: a request below
    // that is silently clamped, and would look like wgaf ignoring it.
    const NEW_WIDTH: i32 = 720;
    const NEW_HEIGHT: i32 = 560;

    let main = fixture.window(MAIN_TITLE).await;
    assert_ne!(
        (main.width, main.height),
        (NEW_WIDTH, NEW_HEIGHT),
        "the window already has the size this test resizes it to, so it would prove nothing"
    );

    harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "ResizeWindow",
        &(main.id, NEW_WIDTH, NEW_HEIGHT),
    )
    .await
    .expect("ResizeWindow failed against the real extension");

    // **Read back immediately, with no waiting at all.** This is the assertion
    // the open S3 was about: `ResizeWindow` used to reply as soon as Mutter
    // accepted the request, so for ~30 ms afterwards `ListWindows` still
    // described the old rectangle and a script computing a centre point from
    // it aimed at the wrong place. A `wait_for` here would pass either way and
    // prove nothing.
    let after = fixture.window(MAIN_TITLE).await;
    assert_eq!(
        (after.width, after.height),
        (NEW_WIDTH, NEW_HEIGHT),
        "ResizeWindow returned before the new size was readable — `wgaf window list` still \
         reports {}x{}. The reply is supposed to mean the resize has landed.",
        after.width,
        after.height
    );

    fixture
        .app
        .try_wait_for(SETTLE, |report| {
            reported_i64(report, "main", "width") == i64::from(NEW_WIDTH)
                && reported_i64(report, "main", "height") == i64::from(NEW_HEIGHT)
        })
        .await
        .unwrap_or_else(|last| {
            panic!(
                "the main window never reported {NEW_WIDTH}x{NEW_HEIGHT} after ResizeWindow. \
                 Last report:\n{}",
                last.map(|r| format!("{:#}", r.json()))
                    .unwrap_or_else(|| "<none>".into())
            )
        });
}

/// Does `MoveWindow` reply before the new position is readable, the way
/// `ResizeWindow` used to?
///
/// # This is a measurement, and it is the one the resize issue asked for
///
/// That issue's checklist said to apply the same confirmation to `MoveWindow`
/// **only if it has the same lag**, and to measure rather than assume — so the
/// question stayed open through two desktop runs. This answers it, and stays as
/// a guard afterwards either way.
///
/// # Why it compares wgaf against wgaf, which is normally forbidden here
///
/// A Wayland client is never told where it is on screen: there is no GTK getter
/// and nothing in the protocol, so `window-test` cannot report its own position
/// the way it reports its own size. `wgaf window list` is the only source that
/// exists, which is why this suite has no test that `MoveWindow` moves a window
/// to the right place, and why it cannot have one.
///
/// That does not block *this* question. The defect is not "the window ends up
/// somewhere wrong" — it is "the reply arrives before the new value is
/// readable", and both readings come from the same source by construction. So
/// the test compares the position read **immediately after the call returns**
/// against the position once everything has settled. If they differ, the reply
/// was premature. Nothing here claims the window went where it was asked; that
/// remains unprovable from outside the compositor.
///
/// Comparing settled-against-immediate rather than immediate-against-requested
/// also survives Mutter constraining the move — `move_frame` is issued as a
/// user-directed operation, so an off-screen or edge-clamped request is
/// adjusted, and a test demanding the exact requested coordinate would fail for
/// a reason that has nothing to do with lag.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn move_window_reports_its_new_position_without_lag() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("movelag").await;

    let before = fixture.window(MAIN_TITLE).await;
    // A modest displacement, on-screen from wherever the window opened, so
    // Mutter has no reason to constrain it.
    let (target_x, target_y) = (before.x + 120, before.y + 90);

    harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "MoveWindow",
        &(before.id, target_x, target_y),
    )
    .await
    .expect("MoveWindow failed against the real extension");

    let immediately = fixture.window(MAIN_TITLE).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = fixture.window(MAIN_TITLE).await;

    // Without this the test proves nothing: a move that did not happen at all
    // would read identically at both instants and pass.
    assert_ne!(
        (settled.x, settled.y),
        (before.x, before.y),
        "the window never moved, so this measured nothing — it was at \
         ({}, {}) before and after a move to ({target_x}, {target_y})",
        before.x,
        before.y
    );

    assert_eq!(
        (immediately.x, immediately.y),
        (settled.x, settled.y),
        "MoveWindow returned before the new position was readable: `wgaf window list` \
         said ({}, {}) immediately after the call and ({}, {}) once settled. That is the \
         same defect `ResizeWindow` had — put `moveWindow` on `_confirmGeometrySettled` \
         too, and update the archived resize issue's open MoveWindow item.",
        immediately.x,
        immediately.y,
        settled.x,
        settled.y
    );
}

/// A resize the window will not take is reported, not reported as success.
///
/// The old behaviour returned immediately and unconditionally, so a clamped
/// request was indistinguishable from an honoured one — the caller was told
/// "resized" and got a window of some other size. Now the confirmation waits
/// for the size that was asked for, and a size the window refuses comes back
/// as `OperationNotApplied` (ADR-0007 `Unverified`, exit 4) naming what it
/// actually is.
///
/// 1x1 rather than a size near the real minimum: GTK's minimum depends on the
/// window's own content and this test has no business knowing it, but nothing
/// with a title bar is one pixel wide.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn a_resize_the_window_refuses_is_reported_rather_than_claimed() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("resizeclamped").await;

    let main = fixture.window(MAIN_TITLE).await;
    let err = harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "ResizeWindow",
        &(main.id, 1i32, 1i32),
    )
    .await
    .expect_err("a 1x1 resize cannot be honoured and must not be reported as success");

    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_OPERATION_NOT_APPLIED,
            "a clamped resize should be OperationNotApplied"
        ),
        other => panic!("expected a named MethodError, got {other:?}"),
    }

    // The window is still usable — a refused resize is not a broken one.
    let after = fixture.window(MAIN_TITLE).await;
    assert!(
        after.width > 1 && after.height > 1,
        "the window was left at {}x{} after a refused resize",
        after.width,
        after.height
    );
}

/// `CloseWindow` closes the window it names, and only that one.
///
/// The dialog is the interesting target: it is the transient, the window whose
/// `app_id` differs from its siblings', and the one a naive filter loses.
///
/// # This test used to fail about half the time, and wgaf was never the reason
///
/// It was recorded in `issues.md` as reproducing a compositor defect — "the
/// window disappears from the compositor's list while the application is never
/// told it closed". **That was wrong, and the diagnosis is worth keeping so it
/// is not repeated.** The application is told every single time. What it lacked
/// was a watcher on any signal that fires once the close has *completed*, so it
/// never wrote a report describing the state afterwards, and this test waited
/// for a report that was never coming.
///
/// The intermittency was the tell, read backwards. The runs that passed did so
/// because an unrelated `is-active` change happened to trigger a report after
/// the close, which then observed the new state by luck. Nothing about closing
/// varied between runs.
///
/// Fixed in `window-test` by watching `unmap`, which is emitted on every close
/// with `is_visible()` already `false`; the full signal measurements are in the
/// comment on that watcher. Two other defects were found and fixed alongside
/// it, and both had been masking this one: the readiness gate accepted windows
/// Mutter was still reporting as `0x0`, and nothing waited for a previous
/// instance's windows to go away.
///
/// **Do not weaken this test.** Asserting only that the compositor stopped
/// listing the window would check wgaf against wgaf and would hide exactly the
/// symptom a user would meet. The assertion below is the correct one, and it
/// now holds deterministically — measured at 20 consecutive passes.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn close_window_closes_the_one_it_names() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("close").await;

    let dialog = fixture.window(DIALOG_TITLE).await;
    harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "CloseWindow",
        &(dialog.id,),
    )
    .await
    .expect("CloseWindow failed against the real extension");

    // A closed window is reported as **not visible**, not as absent.
    // `window-test` tracks a fixed set of three windows for its whole life and
    // describes all of them in every report, so `window_count` is 3 from the
    // first report to the last. It is the number of windows the application
    // has, not the number still open — despite its own documentation claiming
    // that "a window vanished" is visible in it, which is filed in `issues.md`.
    let attempt = fixture
        .app
        .try_wait_for(SETTLE, |report| !reported_bool(report, "dialog", "visible"))
        .await;
    let after = match attempt {
        Ok(report) => report,
        Err(last) => {
            // What the compositor thinks, printed alongside what the
            // application thinks. A failure that shows only one side sends the
            // next person looking in the wrong place — which is precisely what
            // happened the first time this failed.
            let titles: Vec<String> = fixture
                .list()
                .await
                .into_iter()
                .map(|window| window.title)
                .filter(|title| title.starts_with(MAIN_TITLE))
                .collect();
            panic!(
                "the dialog never reported becoming invisible after CloseWindow({}).\n\
                 The compositor now lists: {titles:?}\n\
                 If the dialog is absent there, the close itself worked and the \
                 application did not report it — check that `window-test` still watches \
                 `unmap`, which is the only signal that fires on a compositor close.\n\
                 Last report:\n{}",
                dialog.id,
                last.map(|r| format!("{:#}", r.json()))
                    .unwrap_or_else(|| "<none>".into())
            )
        }
    };

    // The other two must be untouched — closing one window by id must not take
    // its siblings, and the dialog's transient-for relationship to the main
    // window is exactly the sort of link that could.
    for role in ["main", "secondary"] {
        assert!(
            reported_bool(&after, role, "visible"),
            "closing the dialog also closed the `{role}` window. Report:\n{:#}",
            after.json()
        );
    }

    // The compositor must agree, from its own side: a window the application
    // considers gone but that Mutter still lists would be a leak in the
    // extension's enumeration, and `wgaf window close` would be lying.
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let titles: Vec<String> = fixture
            .list()
            .await
            .into_iter()
            .map(|window| window.title)
            .collect();
        if !titles.iter().any(|title| title == DIALOG_TITLE) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "`wgaf window list` still reports the closed dialog after {SETTLE:?}: {titles:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// --- Window state (W18.1) ---------------------------------------------------
//
// Each of these asserts against what `window-test` says about itself, not
// against wgaf's reply, for the reason this file's header gives. That is what
// limits which of the six can be covered here at all:
//
//   maximize, fullscreen  — GTK reports both directly.
//   minimize              — via GTK's `suspended`, its only account of "not
//                           visible to the user"; see the field's comment in
//                           window-test.
//   above, stick, restack — NOT COVERED, and not by oversight. A Wayland client
//                           is told nothing about its stacking order, its layer,
//                           or which workspaces it appears on; there is no
//                           getter in GTK4 and nothing in the protocol. The only
//                           other source is `wgaf window list`, and asserting on
//                           that would check the extension's own report against
//                           the extension — the same trap `wgaf window move` is
//                           deliberately absent for. If a real oracle ever turns
//                           up, this is where those tests go.

/// `SetWindowMaximized` maximizes and unmaximizes, verified by the application
/// and cross-checked against the geometry the compositor reports.
///
/// # This test had a hole, and it is worth knowing what it was
///
/// It used to drive a `directions` argument and assert only on GTK's
/// `is_maximized()`, in the sequence both → un-horizontal → horizontal → un-both.
/// **It passed against a build where the direction was completely ignored**,
/// because `is_maximized()` means *both* axes: a horizontal request that really
/// maximized both satisfies every step. The defect was found by running the
/// command by hand and reading the window's size.
///
/// The argument is gone — Mutter 18 has no per-axis maximize to offer (see
/// `setWindowMaximized` in `extension/windows.js` for the measurements) — so
/// the hole is closed by removing the thing it hid. What remains is the check
/// that both axes really move, and the geometry cross-check below is what makes
/// "maximized" mean something beyond a flag the toolkit set.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn maximizing_changes_what_the_application_reports() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("maximize").await;

    let main = fixture.window(MAIN_TITLE).await;
    let report = fixture.app.read().expect("window-test stopped reporting");
    assert!(
        !reported_bool(&report, "main", "maximized"),
        "the main window starts maximized, so this test would prove nothing"
    );

    let set = async |maximized: bool| {
        harness::windows::<(), _>(
            &fixture.connection,
            &fixture.bus_name,
            "SetWindowMaximized",
            &(main.id, maximized),
        )
        .await
        .unwrap_or_else(|e| panic!("SetWindowMaximized({maximized}) failed: {e}"));
    };

    let await_maximized = async |expected: bool, what: &str| {
        fixture
            .app
            .try_wait_for(SETTLE, |report| {
                reported_bool(report, "main", "maximized") == expected
            })
            .await
            .unwrap_or_else(|last| {
                panic!(
                    "the main window never reported maximized={expected} after {what}. \
                     Last report:\n{}",
                    last.map(|r| format!("{:#}", r.json()))
                        .unwrap_or_else(|| "<none>".into())
                )
            });
    };

    set(true).await;
    await_maximized(true, "maximizing").await;

    // The application only knows a boolean — xdg-shell carries one `maximized`
    // state and no geometry claim — so the compositor's frame rect is what
    // says the window actually grew. A toolkit flag set without a resize
    // behind it would pass the assertion above and fail this one.
    let maximized = fixture.window(MAIN_TITLE).await;
    assert!(
        maximized.width > main.width && maximized.height > main.height,
        "the window reports maximized but is still {}x{} (it was {}x{})",
        maximized.width,
        maximized.height,
        main.width,
        main.height
    );

    set(false).await;
    await_maximized(false, "unmaximizing").await;

    let restored = fixture.window(MAIN_TITLE).await;
    assert_eq!(
        (restored.width, restored.height),
        (main.width, main.height),
        "unmaximizing did not return the window to the size it started at"
    );
}

/// `SetWindowFullscreen` puts a window fullscreen and takes it back out,
/// verified by the application.
///
/// Kept apart from the maximize test rather than folded in with it: the two are
/// different states and a window can be in both, so a test that set them in
/// sequence could pass while confusing one for the other.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn fullscreen_changes_what_the_application_reports() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("fullscreen").await;

    let main = fixture.window(MAIN_TITLE).await;
    let report = fixture.app.read().expect("window-test stopped reporting");
    assert!(
        !reported_bool(&report, "main", "fullscreen"),
        "the main window starts fullscreen, so this test would prove nothing"
    );

    for (fullscreen, what) in [(true, "going fullscreen"), (false, "leaving fullscreen")] {
        harness::windows::<(), _>(
            &fixture.connection,
            &fixture.bus_name,
            "SetWindowFullscreen",
            &(main.id, fullscreen),
        )
        .await
        .unwrap_or_else(|e| panic!("SetWindowFullscreen({fullscreen}) failed: {e}"));

        fixture
            .app
            .try_wait_for(SETTLE, |report| {
                reported_bool(report, "main", "fullscreen") == fullscreen
            })
            .await
            .unwrap_or_else(|last| {
                panic!(
                    "the main window never reported fullscreen={fullscreen} after {what}. \
                     Last report:\n{}",
                    last.map(|r| format!("{:#}", r.json()))
                        .unwrap_or_else(|| "<none>".into())
                )
            });
    }

    // Fullscreen must not have been implemented as a maximize. They are
    // different states with different geometry — a maximized window stops at
    // the work area — and a script placing other windows depends on the
    // difference.
    let after = fixture.app.read().expect("window-test stopped reporting");
    assert!(
        !reported_bool(&after, "main", "maximized"),
        "leaving fullscreen left the window maximized, which nothing asked for. Report:\n{:#}",
        after.json()
    );
}

/// `SetWindowMinimized` minimizes and restores, verified by the application
/// rather than by wgaf's own record.
///
/// Restoring is asserted as well as minimizing, and it is the half worth
/// having: a run that left the maintainer's window minimized would be a test
/// that damaged the session it borrowed.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn minimizing_hides_the_window_from_the_application_and_restoring_brings_it_back() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("minimize").await;

    let main = fixture.window(MAIN_TITLE).await;
    let report = fixture.app.read().expect("window-test stopped reporting");
    assert!(
        !reported_bool(&report, "main", "suspended"),
        "the main window is already out of view, so this test would prove nothing. \
         Report:\n{:#}",
        report.json()
    );

    for (minimized, what) in [(true, "minimizing"), (false, "restoring")] {
        harness::windows::<(), _>(
            &fixture.connection,
            &fixture.bus_name,
            "SetWindowMinimized",
            &(main.id, minimized),
        )
        .await
        .unwrap_or_else(|e| panic!("SetWindowMinimized({minimized}) failed: {e}"));

        fixture
            .app
            .try_wait_for(SETTLE, |report| {
                reported_bool(report, "main", "suspended") == minimized
            })
            .await
            .unwrap_or_else(|last| {
                panic!(
                    "the main window never reported suspended={minimized} after {what}. \
                     GTK reports `suspended` rather than `minimized` because a Wayland client \
                     is never told which it is — see the field's comment in window-test. \
                     Last report:\n{}",
                    last.map(|r| format!("{:#}", r.json()))
                        .unwrap_or_else(|| "<none>".into())
                )
            });
    }

    // Restoring must not have focused it as a side effect. Nothing here asked
    // for that, and doing it would be `FocusWindow`'s capability being spent
    // without being checked.
    let after = fixture.window(MAIN_TITLE).await;
    assert!(
        !after.minimized,
        "`wgaf window list` still reports the window minimized after restoring it"
    );
}

/// The extension's named errors reach the daemon by name, not as a generic
/// D-Bus failure.
///
/// # This is the test whose absence hid a live bug for three releases
///
/// Every daemon-side error translation was unit-tested against
/// `tests/windows_stub.rs`, which implements the extension in Rust and emits
/// names correctly — so the daemon half was proven and the *extension* half was
/// never executed by anything. It turned out the extension's reply path
/// discarded the name entirely (`return_gerror` re-encodes it as
/// `org.gtk.GDBus.UnmappedGError…`), which meant `WindowNotFound`,
/// `WorkspaceNotFound` and `OperationNotApplied` had never once arrived
/// intact. Found by inspection, not by a failing test. See the S2 in
/// `issues.md`.
///
/// So this asserts the one thing no stub can: that a name survives the real
/// GJS. Closing a window and then acting on its id is the cheapest way to
/// provoke one.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn a_named_error_from_the_extension_survives_the_trip_to_the_daemon() {
    let _lock = lock_window_test().await;
    let fixture = Fixture::start("errorname").await;

    // An id no window has. Derived from a real one so it is plausibly shaped
    // rather than a magic number, and high enough that Mutter's stable
    // sequence will not have reached it.
    let absent = fixture.window(MAIN_TITLE).await.id + 100_000;

    let err = harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "FocusWindow",
        &(absent,),
    )
    .await
    .expect_err("focusing a window that does not exist must fail");

    match err {
        zbus::Error::MethodError(name, description, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND,
            "the extension's WindowNotFound did not survive as a name — it arrived as \
             `{name}` with description {description:?}. If this says \
             `org.gtk.GDBus.UnmappedGError…`, the extension is replying through \
             `return_gerror` again; only `return_dbus_error` keeps the name."
        ),
        other => panic!("expected a named MethodError, got {other:?}"),
    }

    // And the same through a window-state method, which is the newer path and
    // the one with a second named error of its own.
    let err = harness::windows::<(), _>(
        &fixture.connection,
        &fixture.bus_name,
        "SetWindowMinimized",
        &(absent, true),
    )
    .await
    .expect_err("minimizing a window that does not exist must fail");

    match err {
        zbus::Error::MethodError(name, _, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND,
            "SetWindowMinimized lost the error name"
        ),
        other => panic!("expected a named MethodError, got {other:?}"),
    }
}
