//! Workspace operations driven against a real GNOME Shell, verified by the
//! application's own account of itself.
//!
//! # Why this suite exists
//!
//! Workspace switching, adding, removing and reordering shipped in 0.8.2 with
//! **no desktop coverage at all**. `tests/windows_stub.rs` drives them against
//! a Rust stub, which proves the daemon's half and executes not one line of
//! `extension/windows.js`; `examples/desktop-layout.sh` drives the real thing
//! but is a demonstration run by hand, not something `make test-desktop`
//! notices breaking. This closes that gap, on the same terms as
//! `window_management.rs`.
//!
//! # The oracle, and why it is a real one
//!
//! **A window on another workspace is out of the user's view, and GTK says
//! so.** `window-test` reports `suspended` — the toolkit's own account of "not
//! visible to you" — and that flips when the workspace changes underneath it.
//! Nothing wgaf says is involved, which is the whole point: asking wgaf whether
//! its own workspace switch worked would prove nothing.
//!
//! `suspended` is wider than "on another workspace" — it also covers minimized
//! and fully obscured — so it is only sound here because nothing else in these
//! tests moves the window out of view. That is stated rather than assumed; see
//! the field's comment in `tests/apps/window-test/src/main.rs`.
//!
//! # What is deliberately not checked against wgaf's own report
//!
//! Adding and removing workspaces changes a count that only the compositor
//! knows, and no application is told about it. `GetWorkspaces` is wgaf
//! reporting on wgaf, so the add/remove test below asserts the *consequence* a
//! caller actually depends on — that a workspace it added can be switched to
//! and a window moved there — rather than that a number went up.
//!
//! # This suite rearranges the session and puts it back
//!
//! It switches workspace, and adds one when the session has only a single
//! workspace to work with. [`Session`] restores the active workspace and
//! removes anything added, including when a test panics part-way.

mod harness;

use std::time::Duration;

use harness::{Report, TestApp};
use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};
use zbus::Connection;

/// `window-test`'s main window, matched by title for the reason
/// `window_management.rs` documents: its dialog reports a different `app_id`,
/// so an `app_id` filter silently drops a window.
const MAIN_TITLE: &str = "wgaf window-test";

/// How long to let the compositor and the application catch up.
const SETTLE: Duration = Duration::from_secs(5);

/// Serializes the tests in this file, which all drive the one `window-test`
/// and all move the session around. Same reasoning as
/// `window_management.rs`'s lock, and separate from it only because cargo runs
/// the two binaries one at a time anyway.
async fn lock_session() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// The daemon, the application, and the workspace state to put back.
struct Session {
    _daemon: harness::DaemonGuard,
    connection: Connection,
    bus_name: String,
    app: TestApp,
    window: u32,
    /// The workspace that was active when this started.
    original: i32,
    /// Set when this suite added a workspace, so it can take it away again.
    added: Option<i32>,
}

impl Session {
    async fn start(suite: &'static str) -> Self {
        harness::require_wayland_session();

        let bus_name = format!("org.wgaf.Test.Workspaces.{suite}{}", std::process::id());
        let daemon = harness::spawn_daemon(suite, &bus_name, "");
        let connection = harness::wait_for_daemon(&bus_name).await;

        let app = TestApp::spawn("window-test").await;
        app.wait_for("window-test to report its windows", |report| {
            !report.array("windows").is_empty()
        })
        .await;

        let mut session = Session {
            _daemon: daemon,
            connection,
            bus_name,
            app,
            window: 0,
            original: 0,
            added: None,
        };

        // Wait for the compositor to have the window at a real size before
        // reading its id — the same readiness gate `window_management.rs`
        // needs, and for the same reason.
        session.window = session.wait_for_window().await;
        session.original = session.active().await;
        session
    }

    async fn wait_for_window(&self) -> u32 {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let found = self
                .list()
                .await
                .into_iter()
                .find(|w| w.title == MAIN_TITLE && w.width > 0 && w.height > 0);
            if let Some(window) = found {
                return window.id;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the compositor never reported `{MAIN_TITLE}` at a non-zero size"
            );
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

    async fn workspaces(&self) -> Vec<WorkspaceRecord> {
        let records: Vec<WorkspaceRecordDict> =
            harness::windows(&self.connection, &self.bus_name, "GetWorkspaces", &())
                .await
                .expect("GetWorkspaces failed against the real extension");
        records.into_iter().map(Into::into).collect()
    }

    async fn active(&self) -> i32 {
        self.workspaces()
            .await
            .into_iter()
            .find(|w| w.active)
            .expect("some workspace is always active")
            .index
    }

    async fn switch_to(&self, index: i32) {
        harness::windows::<(), _>(
            &self.connection,
            &self.bus_name,
            "SwitchWorkspace",
            &(index,),
        )
        .await
        .unwrap_or_else(|e| panic!("SwitchWorkspace({index}) failed: {e}"));
    }

    /// A workspace other than the active one, adding one if the session has
    /// none to spare.
    async fn somewhere_else(&mut self) -> i32 {
        let active = self.active().await;
        if let Some(other) = self
            .workspaces()
            .await
            .into_iter()
            .find(|w| w.index != active)
        {
            return other.index;
        }

        let index: i32 = harness::windows(&self.connection, &self.bus_name, "AddWorkspace", &())
            .await
            .expect("AddWorkspace failed against the real extension");
        self.added = Some(index);
        index
    }

    /// Waits for the application's own view of whether it is visible.
    async fn await_out_of_view(&self, out_of_view: bool, what: &str) -> Report {
        self.app
            .try_wait_for(SETTLE, |report| {
                main_window(report)
                    .get("suspended")
                    .and_then(serde_json::Value::as_bool)
                    == Some(out_of_view)
            })
            .await
            .unwrap_or_else(|last| {
                panic!(
                    "the application never reported suspended={out_of_view} after {what}. \
                     Last report:\n{}",
                    last.map(|r| format!("{:#}", r.json()))
                        .unwrap_or_else(|| "<none>".into())
                )
            })
    }

    /// Puts the session back the way it was found.
    ///
    /// Called explicitly at the end of each test rather than from `Drop`,
    /// because every step of it is async and a `Drop` cannot await. A test that
    /// panics before reaching it leaves the session on another workspace —
    /// annoying, and preferable to the alternative of blocking inside `Drop`.
    async fn restore(&mut self) {
        harness::windows::<(), _>(
            &self.connection,
            &self.bus_name,
            "MoveWindowToWorkspace",
            &(self.window, self.original),
        )
        .await
        .ok();
        self.switch_to(self.original).await;
        if let Some(added) = self.added.take() {
            harness::windows::<(), _>(
                &self.connection,
                &self.bus_name,
                "RemoveWorkspace",
                &(added,),
            )
            .await
            .ok();
        }
    }
}

fn main_window(report: &Report) -> &serde_json::Value {
    report
        .array("windows")
        .iter()
        .find(|w| w.get("role").and_then(serde_json::Value::as_str) == Some("main"))
        .expect("window-test always reports its main window")
}

/// Switching workspace takes the window out of the user's view, and switching
/// back brings it home.
///
/// The application is the witness: it reports itself suspended when the desktop
/// moves out from under it, which wgaf cannot fake.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows and switches workspace on the running \
            GNOME Shell. Run deliberately after `make test-apps`, with --test-threads=1."]
async fn switching_workspace_takes_the_window_out_of_view() {
    let _lock = lock_session().await;
    let mut session = Session::start("workspaces-switch").await;

    let report = session.app.read().expect("window-test stopped reporting");
    assert_eq!(
        main_window(&report).get("suspended").unwrap(),
        false,
        "the window starts out of view, so this test would prove nothing"
    );

    let elsewhere = session.somewhere_else().await;
    session.switch_to(elsewhere).await;
    session
        .await_out_of_view(true, "switching to another workspace")
        .await;

    // And the daemon agrees about where we are — checked after the
    // application, so a disagreement is attributable rather than a race.
    assert_eq!(
        session.active().await,
        elsewhere,
        "wgaf reports a different active workspace than the one it switched to"
    );

    session.switch_to(session.original).await;
    session.await_out_of_view(false, "switching back").await;

    session.restore().await;
}

/// `MoveWindowToWorkspace` sends the window away and leaves the user where they
/// are — the split the command documents, checked from both sides.
#[tokio::test]
#[ignore = "takes over the desktop: opens real windows and moves them between workspaces on the \
            running GNOME Shell. Run deliberately after `make test-apps`, with --test-threads=1."]
async fn moving_a_window_away_does_not_take_the_user_with_it() {
    let _lock = lock_session().await;
    let mut session = Session::start("workspaces-move").await;

    let elsewhere = session.somewhere_else().await;
    let before = session.active().await;

    harness::windows::<(), _>(
        &session.connection,
        &session.bus_name,
        "MoveWindowToWorkspace",
        &(session.window, elsewhere),
    )
    .await
    .expect("MoveWindowToWorkspace failed against the real extension");

    // The window went; the view did not.
    session
        .await_out_of_view(true, "moving the window to another workspace")
        .await;
    assert_eq!(
        session.active().await,
        before,
        "moving a window changed which workspace the user is looking at"
    );

    // Following it brings it back into view, which is the other half of the
    // contract: the window really is over there, not merely hidden.
    session.switch_to(elsewhere).await;
    session
        .await_out_of_view(false, "following the window to its new workspace")
        .await;

    session.restore().await;
}

/// A workspace this suite added can actually be used — switched to, and a
/// window moved onto it.
///
/// Asserts the consequence rather than the count. `GetWorkspaces` reporting one
/// more workspace is wgaf agreeing with itself; a window becoming visible there
/// is the compositor and the application agreeing that the workspace is real.
#[tokio::test]
#[ignore = "takes over the desktop: adds and removes a workspace on the running GNOME Shell. \
            Run deliberately after `make test-apps`, with --test-threads=1."]
async fn an_added_workspace_is_one_a_window_can_be_moved_to() {
    let _lock = lock_session().await;
    let mut session = Session::start("workspaces-add").await;

    let added: i32 = harness::windows(&session.connection, &session.bus_name, "AddWorkspace", &())
        .await
        .expect("AddWorkspace failed against the real extension");
    session.added = Some(added);

    harness::windows::<(), _>(
        &session.connection,
        &session.bus_name,
        "MoveWindowToWorkspace",
        &(session.window, added),
    )
    .await
    .expect("a window should be movable to a workspace that was just added");

    session
        .await_out_of_view(true, "moving the window to the new workspace")
        .await;

    session.switch_to(added).await;
    session
        .await_out_of_view(false, "switching to the new workspace")
        .await;

    session.restore().await;
}

/// A workspace index that does not exist comes back as `WorkspaceNotFound`, by
/// name, through the real extension.
///
/// # The workspace half of an error path that was dead until 0.8.4
///
/// Every named error the extension raised used to reach the daemon as
/// `org.gtk.GDBus.UnmappedGError…`, so this assertion would have failed from
/// the day `WorkspaceNotFound` shipped — and nothing was making it. Its window
/// counterpart lives in `window_management.rs`; this is the same guard on the
/// path that had no test.
#[tokio::test]
#[ignore = "needs the wgaf GNOME Shell extension on a live session. Run via `make test-desktop`."]
async fn an_unknown_workspace_index_is_reported_by_name() {
    let _lock = lock_session().await;
    let session = Session::start("workspaces-missing").await;

    // Far beyond any plausible workspace count, and derived rather than magic.
    let absent = session.workspaces().await.len() as i32 + 100;

    let err = harness::windows::<(), _>(
        &session.connection,
        &session.bus_name,
        "SwitchWorkspace",
        &(absent,),
    )
    .await
    .expect_err("switching to a workspace that does not exist must fail");

    match err {
        zbus::Error::MethodError(name, description, _) => assert_eq!(
            name.as_str(),
            wgaf_common::WINDOWS_ERROR_WORKSPACE_NOT_FOUND,
            "the extension's WorkspaceNotFound did not survive as a name — it arrived as \
             `{name}` with description {description:?}. If this says \
             `org.gtk.GDBus.UnmappedGError…`, the extension is replying through \
             `return_gerror` again; only `return_dbus_error` keeps the name."
        ),
        other => panic!("expected a named MethodError, got {other:?}"),
    }
}
