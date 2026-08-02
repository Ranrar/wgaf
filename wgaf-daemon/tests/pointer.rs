//! Absolute pointer positioning against a real GNOME Shell, verified by the
//! application the pointer is moved onto.
//!
//! # Why this can exist when `wgaf window move` has no equivalent
//!
//! The plan recorded, for a long time, that no automated coverage of real
//! pointer movement was possible — carried over from window management, where
//! it is true: a Wayland client is never told where the compositor placed its
//! window, so the only source of a window's position is wgaf's own reply, and
//! asserting on that verifies wgaf with wgaf.
//!
//! Pointer position is the opposite case. A client **is** told where the
//! pointer is, in its own surface coordinates, because it needs that to draw
//! hover states and route clicks. So `input-test` can report a position wgaf
//! never told it, and Rule 1 is satisfied: wgaf drives, the application
//! reports, and the assertion terminates outside wgaf.
//!
//! # What the assertion rests on
//!
//! `MouseMoveAbsolute` takes global coordinates; `input-test` reports the
//! pointer in coordinates relative to its own window. Bridging them needs the
//! window's position, which comes from `ListWindows` — wgaf's own answer, and
//! therefore *not* something this suite may assert on. It is used as an input,
//! not as an expectation: the claim under test is that the client's report
//! matches `target - window origin`, and a wrong window origin would break that
//! equality rather than silently satisfying it.
//!
//! Measured during the W5.1 spike on 2026-08-02: that equality held exactly,
//! delta `(0.0, 0.0)`, across seven positions. So this asserts equality rather
//! than a tolerance. If it ever needs a tolerance, something has changed about
//! the coordinate spaces and the right response is to find out what, not to
//! widen the bound.
//!
//! # This suite does not synthesize input
//!
//! Unlike every other `#[ignore]`d suite here, nothing is typed and no `uinput`
//! device is used: absolute positioning goes through the Shell extension, not
//! the kernel. It moves the real pointer, which is disruptive to watch but
//! cannot corrupt anything, so it carries none of the risk that keeps
//! `keyboard_coverage.rs` and `keyboard_layout.rs` out of `cargo test`.
//!
//! It is still `#[ignore]`d, for a different reason: it needs a live GNOME
//! Wayland session with the extension installed, which CI does not have.

mod harness;

use harness::TestApp;
use wgaf_common::WindowRecord;
use wgaf_common::dict::WindowRecordDict;

const SUITE: &str = "pointer";

/// Offsets inside `input-test`'s window to move the pointer to.
///
/// Well inside the frame on every side: a position on the very edge is
/// ambiguous about which window is under the pointer, and this suite is testing
/// coordinate arithmetic rather than edge-case window stacking.
const OFFSETS: &[(i32, i32)] = &[(320, 240), (100, 100), (500, 400), (60, 60), (580, 420)];

/// Where the pointer is parked while waiting for the window to settle. The
/// centre, so it is unambiguously inside the window whatever the animation is
/// currently doing to its size.
const SETTLE_OFFSET: (i32, i32) = (320, 240);

struct Fixture {
    _daemon: harness::DaemonGuard,
    connection: zbus::Connection,
    bus_name: String,
    app: TestApp,
}

impl Fixture {
    async fn setup() -> Self {
        harness::require_wayland_session();
        harness::require_test_app("input-test");

        let bus_name = format!("org.wgaf.Test.Pointer{}", std::process::id());
        let daemon = harness::spawn_daemon(SUITE, &bus_name, "");
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

        let app = TestApp::spawn("input-test").await;

        // Mutter lists a window before its frame rectangle exists, answering
        // 0x0 at (0,0) until the surface is committed — the same trap
        // `window_management.rs` records. A zero-sized window here would make
        // every offset below land outside it and every assertion fail for a
        // reason that has nothing to do with the pointer.
        wait_for_mapped_window(&connection, &bus_name).await;

        let fixture = Fixture {
            _daemon: daemon,
            connection,
            bus_name,
            app,
        };
        fixture.settle().await;
        fixture
    }

    /// Waits until the window has stopped being animated onto the screen.
    ///
    /// **Why a non-zero size is not enough.** GNOME scales a window up as it
    /// opens, and GTK reports pointer coordinates through the widget transform
    /// that animation is driving. A warp during it produces a report that is
    /// *nearly* right — measured at 319.691 for an expected 320.0, a ratio of
    /// 0.999 — which is the animation caught a frame before it finished.
    ///
    /// That is worth waiting out rather than absorbing into a tolerance. The
    /// exactness of the coordinate agreement is the property this suite exists
    /// to prove; a tolerance wide enough to swallow a half-finished animation
    /// would also swallow a genuine coordinate-space bug.
    ///
    /// Warping repeatedly rather than sleeping a fixed time, because the report
    /// only updates when the pointer moves: a settled window that was last
    /// measured mid-animation would keep reporting the stale figure forever.
    async fn settle(&self) {
        for _ in 0..50 {
            let (window, _) = self.move_to_offset(SETTLE_OFFSET).await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Some(report) = self.app.read()
                && report.bool("pointer_in_window")
                && report.f64("pointer_x") == f64::from(SETTLE_OFFSET.0)
                && report.f64("pointer_y") == f64::from(SETTLE_OFFSET.1)
            {
                return;
            }
            let _ = window;
        }
        panic!(
            "input-test's window never stopped moving under the pointer — it reports a \
             position that does not match where the pointer was put, which is what an \
             unfinished window-open animation looks like"
        );
    }

    /// Where `input-test`'s window is *now*.
    ///
    /// Re-read before every move rather than captured once at setup, and that
    /// is not defensiveness — a rect captured at setup was measured to be wrong
    /// by 117 pixels in y by the time the first warp happened, because the
    /// window is still being placed when it first appears in `ListWindows` with
    /// a non-zero size. The symptom was a warp that succeeded, a pointer that
    /// genuinely went where it was told, and an application that never saw it,
    /// which reads like a broken feature rather than a stale coordinate.
    async fn window(&self) -> WindowRecord {
        let windows: Vec<WindowRecordDict> =
            harness::windows(&self.connection, &self.bus_name, "ListWindows", &())
                .await
                .expect("ListWindows should succeed");
        windows
            .into_iter()
            .map(WindowRecordDict::into)
            .find(|w: &WindowRecord| w.title == "wgaf input-test" && w.width > 0)
            .expect("input-test's window disappeared mid-test")
    }

    /// Moves the pointer to a window-relative offset, returning the window it
    /// aimed at and what the daemon says it reached.
    async fn move_to_offset(&self, offset: (i32, i32)) -> (WindowRecord, (i32, i32)) {
        let window = self.window().await;
        let target = (window.x + offset.0, window.y + offset.1);
        let reached = harness::input::<(i32, i32), _>(
            &self.connection,
            &self.bus_name,
            "MouseMoveAbsolute",
            &target,
        )
        .await
        .unwrap_or_else(|e| panic!("MouseMoveAbsolute{target:?} failed: {e}"));
        (window, reached)
    }
}

/// Waits until `input-test`'s window is listed with a real size.
async fn wait_for_mapped_window(connection: &zbus::Connection, bus_name: &str) -> WindowRecord {
    for _ in 0..100 {
        let windows: Vec<WindowRecordDict> =
            harness::windows(connection, bus_name, "ListWindows", &())
                .await
                .expect("ListWindows should succeed once the extension is present");
        let found = windows
            .into_iter()
            .map(WindowRecordDict::into)
            .find(|w: &WindowRecord| w.title == "wgaf input-test" && w.width > 0 && w.height > 0);
        if let Some(window) = found {
            return window;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("input-test's window was never listed with a non-zero size");
}

/// The whole point of the feature: the pointer arrives where it was aimed, and
/// the application agrees that it did.
#[tokio::test]
#[ignore = "needs a live GNOME Wayland session with the wgaf extension installed"]
async fn the_pointer_arrives_where_the_application_can_see_it() {
    let fixture = Fixture::setup().await;

    for &offset in OFFSETS {
        let (window, reached) = fixture.move_to_offset(offset).await;
        let target = (window.x + offset.0, window.y + offset.1);
        assert_eq!(
            reached, target,
            "the daemon reported landing somewhere other than the requested position"
        );

        // The description carries the numbers, because a bare "the pointer did
        // not arrive" leaves the reader unable to tell a wrong warp from a
        // wrong window rectangle — the two failures look identical in the
        // report and have completely different causes.
        let described = format!(
            "the pointer to arrive at offset {offset:?} of the window at \
             ({}, {}) {}x{}, i.e. global {target:?}",
            window.x, window.y, window.width, window.height
        );
        let report = fixture
            .app
            .wait_for(&described, |r| {
                r.bool("pointer_in_window")
                    && r.f64("pointer_x") == f64::from(offset.0)
                    && r.f64("pointer_y") == f64::from(offset.1)
            })
            .await;

        // Re-asserted rather than left to the predicate, so a timeout failure
        // says which coordinate was wrong instead of only that one was.
        //
        // Compared exactly rather than within a tolerance: the spike measured a
        // delta of exactly (0.0, 0.0) across seven positions, so anything else
        // is a change worth understanding rather than absorbing.
        assert_eq!(
            (report.f64("pointer_x"), report.f64("pointer_y")),
            (f64::from(offset.0), f64::from(offset.1)),
            "input-test reported a different position than wgaf was asked for"
        );
    }
}

/// A single warp is a teleport: the application is told once, not given a path.
///
/// This is a documented contract in `docs/cli-reference.md`, so it is worth a
/// test — an implementation that interpolated would still pass the arrival test
/// above while breaking what the documentation promises.
#[tokio::test]
#[ignore = "needs a live GNOME Wayland session with the wgaf extension installed"]
async fn one_move_delivers_one_motion_event() {
    let fixture = Fixture::setup().await;

    // Settle at a known position first, so the count below covers exactly one
    // move rather than also catching the pointer's arrival into the window.
    fixture.move_to_offset((320, 240)).await;
    fixture
        .app
        .wait_for("the pointer to settle inside the window", |r| {
            r.bool("pointer_in_window") && r.f64("pointer_x") == 320.0
        })
        .await;

    let before = fixture.app.read().expect("a report").u64("motion_count");

    // A long hop across the window. A path would deliver many events; a
    // teleport delivers one.
    fixture.move_to_offset((60, 420)).await;
    let after = fixture
        .app
        .wait_for("the pointer to arrive at the far corner", |r| {
            r.f64("pointer_x") == 60.0 && r.f64("pointer_y") == 420.0
        })
        .await;

    assert_eq!(
        after.u64("motion_count") - before,
        1,
        "a single absolute move must deliver exactly one motion event — more than \
         one means the pointer travelled rather than teleporting, which contradicts \
         the contract documented for `wgaf mouse move-to`"
    );
}

/// `GetPointerPosition` must agree with where the pointer was just put.
///
/// Worth its own test because the two are different mechanisms in the
/// extension: one confirms its own warp, the other queries the compositor
/// fresh. The spike found the warp is asynchronous, so a naive implementation
/// of either would disagree with the other.
#[tokio::test]
#[ignore = "needs a live GNOME Wayland session with the wgaf extension installed"]
async fn reading_the_position_back_agrees_with_the_move() {
    let fixture = Fixture::setup().await;

    for &offset in OFFSETS {
        let (_window, reached) = fixture.move_to_offset(offset).await;
        let read_back: (i32, i32) = harness::input(
            &fixture.connection,
            &fixture.bus_name,
            "GetPointerPosition",
            &(),
        )
        .await
        .expect("GetPointerPosition should succeed");

        assert_eq!(
            read_back, reached,
            "GetPointerPosition disagrees with the position MouseMoveAbsolute reported \
             reaching — the warp is asynchronous, so this is what a missing confirmation \
             looks like"
        );
    }
}

/// An off-screen coordinate is refused, and the pointer does not move.
///
/// The refusal is the visible half; that nothing moved is the half that matters.
/// Mutter clamps silently when asked to warp off-screen, so an implementation
/// that warped first and validated afterwards would return the right error
/// having already put the pointer somewhere the caller never chose.
#[tokio::test]
#[ignore = "needs a live GNOME Wayland session with the wgaf extension installed"]
async fn an_off_screen_position_is_refused_without_moving_the_pointer() {
    let fixture = Fixture::setup().await;

    fixture.move_to_offset((320, 240)).await;
    let before: (i32, i32) = harness::input(
        &fixture.connection,
        &fixture.bus_name,
        "GetPointerPosition",
        &(),
    )
    .await
    .expect("GetPointerPosition should succeed");

    // Far outside any plausible desktop, so this does not depend on the
    // machine's monitor layout the way a gap-between-monitors case would.
    let err = harness::input::<(i32, i32), _>(
        &fixture.connection,
        &fixture.bus_name,
        "MouseMoveAbsolute",
        &(1_000_000i32, 1_000_000i32),
    )
    .await
    .expect_err("a coordinate far off every monitor must be refused");

    match err {
        zbus::Error::MethodError(name, description, _) => {
            assert_eq!(
                name.as_str(),
                wgaf_common::INPUT_ERROR_OUT_OF_BOUNDS,
                "expected OutOfBounds, got {name}: {description:?}"
            );
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }

    let after: (i32, i32) = harness::input(
        &fixture.connection,
        &fixture.bus_name,
        "GetPointerPosition",
        &(),
    )
    .await
    .expect("GetPointerPosition should succeed");

    assert_eq!(
        after, before,
        "a refused move must leave the pointer exactly where it was"
    );
}
