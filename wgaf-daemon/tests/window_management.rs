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
    async fn wait_until_every_window_is_mapped(&self) {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let seen: Vec<String> = self
                .list()
                .await
                .into_iter()
                .map(|window| window.title)
                .collect();
            let missing: Vec<&str> = [MAIN_TITLE, SECONDARY_TITLE, DIALOG_TITLE]
                .into_iter()
                .filter(|title| !seen.iter().any(|seen| seen == title))
                .collect();

            if missing.is_empty() {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "the compositor never reported {missing:?} within {SETTLE:?}, \
                     though window-test says it has them. `wgaf window list` saw: {seen:?}"
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn list(&self) -> Vec<WindowRecord> {
        let records: Vec<WindowRecordDict> =
            harness::windows(&self.connection, &self.bus_name, "ListWindows", &())
                .await
                .expect("ListWindows failed against the real extension");
        records.into_iter().map(Into::into).collect()
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

/// `CloseWindow` closes the window it names, and only that one.
///
/// The dialog is the interesting target: it is the transient, the window whose
/// `app_id` differs from its siblings', and the one a naive filter loses.
///
/// # This test currently fails about half the time, and the test is not the
/// # thing that is wrong
///
/// It reproduces an open defect, filed in `issues.md`: **the window disappears
/// from the compositor's list while the application is never told it closed.**
/// Both halves were observed in the same failing run — `ListWindows` no longer
/// returns the dialog, and the application, still running, has written no
/// report and goes on describing the window as visible.
///
/// **Do not weaken this test to make it pass.** Asserting only that the
/// compositor stopped listing the window would check wgaf against wgaf and
/// would hide exactly the symptom a user would meet. The assertion below is the
/// correct one; it is the behaviour underneath that is wrong.
///
/// What has been ruled out, so nobody repeats it: it is not a mapping race
/// (waiting longer makes it *worse*), not stale windows from an earlier run
/// (checked before spawning), and not specific to the dialog (targeting the
/// main `ApplicationWindow` failed five times out of five). `window-test` also
/// now watches `destroy` in addition to `close-request` and `notify::visible`,
/// which did **not** change the outcome.
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
            // application thinks — because the two disagreeing is the whole
            // shape of the open defect this test currently reproduces, and a
            // failure that shows only one side sends the next person looking
            // in the wrong place.
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
                 If the dialog is absent there, this is the known defect in `issues.md`: \
                 the window is gone from the compositor and the application was never told.\n\
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
