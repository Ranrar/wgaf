//! A GTK4 application with a fixed, known set of windows, for testing wgaf's
//! window management against something that reports its own state.
//!
//! It draws three windows and nothing else. Every time one of them changes size,
//! gains or loses focus, is maximized, or is shown or hidden, the application
//! rewrites its report file. A test drives wgaf, then reads that file — never
//! asking wgaf what happened, which is the whole point (see the `report`
//! crate's documentation).
//!
//! # Why three windows
//!
//! One window would be easier and would test less. Window enumeration
//! realistically breaks on applications that have a main window plus something
//! else, so the fixed set is deliberately:
//!
//! - **`main`** — an ordinary window with a header bar, so it is client-side
//!   decorated like most GTK4 applications;
//! - **`secondary`** — a second independent top-level of the same application,
//!   which is what catches an enumeration that assumes one window per process;
//! - **`dialog`** — transient for `main`, which is the case a compositor treats
//!   differently from an independent top-level.
//!
//! All three exist from startup and stay up, so a test never has to arrange for
//! one to appear.
//!
//! # What this application can and cannot verify
//!
//! It reports **size** but not **position**, and that is a property of Wayland
//! rather than an omission. A Wayland client is never told where the compositor
//! placed it — there is no position getter in GTK4 and no request in the
//! protocol — so `wgaf window move` cannot be checked against the application's
//! own account of itself. The only available source for a window's position is
//! the GNOME Shell Extension's own reply, and asserting on that would mean
//! verifying wgaf with wgaf, which is exactly what these applications exist to
//! avoid.
//!
//! So: `wgaf window list`, `focus`, `resize`, and `close` are verifiable here.
//! `move` is not, and no amount of work on this application changes that.
//!
//! Two further things a test must not assume, both established by running this:
//!
//! - **Which window is focused at startup varies between runs.** Assert the
//!   change `wgaf window focus` causes, never the state it starts in.
//! - **Mapping produces a burst of reports**, so `seq` is already well past 1
//!   by the time all three windows are up. Take `seq` as a baseline once the
//!   application is up, then wait for it to exceed that; never wait for a
//!   particular value.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use gtk4::prelude::*;
use gtk4::{Align, Application, ApplicationWindow, HeaderBar, Label, Window};
use report::Reporter;
use serde::Serialize;

/// The application id, which is also what Mutter reports as a window's
/// `app_id`. Tests filter on it, so it is part of this application's contract
/// with them.
const APP_ID: &str = "dev.wgaf.WindowTest";

/// Titles are fixed so a test can find a window by name without a search.
/// Changing one of these breaks tests by design — they are the contract.
const MAIN_TITLE: &str = "wgaf window-test";
const SECONDARY_TITLE: &str = "wgaf window-test — secondary";
const DIALOG_TITLE: &str = "wgaf window-test — dialog";

/// Fixed starting geometry, so a resize is measurable as a change from a known
/// value rather than from whatever the compositor felt like.
const MAIN_SIZE: (i32, i32) = (640, 480);
const SECONDARY_SIZE: (i32, i32) = (400, 300);
const DIALOG_SIZE: (i32, i32) = (320, 200);

#[derive(Parser)]
#[command(
    about = "A fixed set of GTK4 windows that reports its own state, for testing wgaf.",
    long_about = "Draws three windows — a main window, an independent secondary window, and a \
                  dialog transient for the main one — and writes a JSON report describing all \
                  three every time any of them changes.\n\nSize is reported; position is not, \
                  because a Wayland client is never told where it was placed."
)]
struct Args {
    /// Where to write the JSON report. The directory must already exist.
    #[arg(long, value_name = "PATH")]
    report: PathBuf,
}

/// One window's state, as the application itself sees it.
///
/// Deliberately **not** shaped like `wgaf_common::WindowRecord`: this is the
/// application's own account of itself, and making the two structurally
/// identical would invite a test to compare them field by field as though they
/// measured the same thing. They do not — notably, there is no `x`/`y` here.
#[derive(Serialize)]
struct WindowState {
    /// `main`, `secondary`, or `dialog` — how a test addresses a window without
    /// depending on the order they happen to be reported in.
    role: &'static str,
    title: String,
    /// The window's current logical size in pixels, as the toolkit sees it.
    /// Note this is the client's own size and need not equal the frame
    /// rectangle Mutter reports for the same window.
    width: i32,
    height: i32,
    /// Whether this window currently has keyboard focus — what verifies
    /// `wgaf window focus`.
    focused: bool,
    maximized: bool,
    fullscreen: bool,
    visible: bool,
}

/// The whole report: every window, every time.
///
/// Reporting all three on every change costs nothing at this size and means a
/// test reads one document rather than correlating three.
#[derive(Serialize)]
struct State {
    /// So a test can match these windows to the process it started, and tell
    /// them apart from a stale instance left by an earlier run.
    pid: u32,
    /// How many windows this application **tracks**, which is three for its
    /// whole life — not how many are still open.
    ///
    /// A closed window stays in the array and is reported as `visible: false`,
    /// so this number never changes. **It is therefore useless as a readiness
    /// or liveness signal**, and it read as one: the first version of the
    /// window-management tests waited for `window_count == 3` before driving
    /// wgaf, which the very first report already satisfies, and so they ran
    /// while the compositor had mapped one of the three windows. Wait for the
    /// compositor's own view instead, and read `visible` to see what closed.
    window_count: usize,
    windows: Vec<WindowState>,
}

/// A window plus the role it plays, kept together so the reporter can describe
/// the whole set from one list.
struct RoledWindow {
    role: &'static str,
    window: Window,
}

impl RoledWindow {
    fn state(&self) -> WindowState {
        WindowState {
            role: self.role,
            title: self.window.title().map(Into::into).unwrap_or_default(),
            // `default_width`/`default_height` track the current size of a
            // mapped GTK4 window, which is why they are the documented way to
            // persist a window's size. `width()`/`height()` would report the
            // allocated size of the widget, which is 0 until the window has
            // been mapped and laid out.
            width: self.window.default_width(),
            height: self.window.default_height(),
            focused: self.window.is_active(),
            maximized: self.window.is_maximized(),
            fullscreen: self.window.is_fullscreen(),
            visible: self.window.is_visible(),
        }
    }
}

fn main() -> gtk4::glib::ExitCode {
    let args = Args::parse();

    let reporter = match Reporter::new("window-test", &args.report) {
        Ok(reporter) => Rc::new(RefCell::new(reporter)),
        Err(err) => {
            // Fail before showing any window: a running application that is not
            // reporting is worse than one that never started, because a test
            // would wait out its whole timeout before finding out.
            eprintln!("window-test: {err}");
            return gtk4::glib::ExitCode::FAILURE;
        }
    };

    let application = Application::builder().application_id(APP_ID).build();

    application.connect_activate(move |application| {
        build_windows(application, Rc::clone(&reporter));
    });

    // The arguments were parsed by clap above; GTK must not try to parse them
    // again and reject `--report`.
    application.run_with_args::<&str>(&[])
}

fn build_windows(application: &Application, reporter: Rc<RefCell<Reporter>>) {
    let main = ApplicationWindow::builder()
        .application(application)
        .title(MAIN_TITLE)
        .default_width(MAIN_SIZE.0)
        .default_height(MAIN_SIZE.1)
        .build();
    // An explicit header bar, so the window is client-side decorated the way a
    // typical GTK4 application is.
    main.set_titlebar(Some(&HeaderBar::builder().build()));
    main.set_child(Some(&body(
        "wgaf window-test\n\nThis window exists to be listed, focused, resized and closed.",
    )));

    let secondary = ApplicationWindow::builder()
        .application(application)
        .title(SECONDARY_TITLE)
        .default_width(SECONDARY_SIZE.0)
        .default_height(SECONDARY_SIZE.1)
        .build();
    secondary.set_titlebar(Some(&HeaderBar::builder().build()));
    secondary.set_child(Some(&body(
        "Secondary window\n\nA second top-level of the same application.",
    )));

    let dialog = Window::builder()
        .title(DIALOG_TITLE)
        .default_width(DIALOG_SIZE.0)
        .default_height(DIALOG_SIZE.1)
        .transient_for(&main)
        .modal(false)
        .build();
    dialog.set_child(Some(&body(
        "Dialog\n\nTransient for the main window, and deliberately not modal so\n\
         a test can still drive the others.",
    )));

    let windows = Rc::new(vec![
        RoledWindow {
            role: "main",
            window: main.clone().upcast(),
        },
        RoledWindow {
            role: "secondary",
            window: secondary.clone().upcast(),
        },
        RoledWindow {
            role: "dialog",
            window: dialog.clone().upcast(),
        },
    ]);

    for roled in windows.iter() {
        watch(&roled.window, &windows, &reporter);
    }

    // **Which window ends up focused at startup is not deterministic**, and no
    // presentation order fixes it: three identical runs of this application
    // gave focus to `main`, then `dialog`, then `secondary`. The compositor
    // decides, and the transient-for relationship competes with the order.
    //
    // So no test may assume an initial focus. Assert the *transition* instead —
    // read whichever window the report says is focused, ask wgaf to focus a
    // different one, and check the report changed. That is a stronger assertion
    // about `wgaf window focus` than any starting state would have been.
    dialog.present();
    secondary.present();
    main.present();

    // The first report says "up, nothing has happened yet" — the state a test
    // waits for before it starts driving anything.
    write_report(&windows, &reporter);
}

/// The window contents.
///
/// **The wrapping is load-bearing, not cosmetic.** A GTK window can never be
/// smaller than its contents' minimum size, so a long unwrapped label silently
/// sets a floor on the window's width: the dialog asked for 320 and got 451
/// before this wrapped. That breaks two things at once — the declared starting
/// geometry stops being what the application actually shows, and a
/// `wgaf window resize` to anything narrower is clamped by GTK while the test
/// blames wgaf for ignoring it. Wrapping at a small character count keeps the
/// minimum well below every size these tests use.
fn body(text: &str) -> Label {
    Label::builder()
        .label(text)
        .wrap(true)
        .max_width_chars(24)
        .halign(Align::Center)
        .valign(Align::Center)
        .justify(gtk4::Justification::Center)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build()
}

/// Subscribes to every property whose change is worth a report.
///
/// These are property notifications rather than a timer: a test should be able
/// to wait for `seq` to advance, which only works if a report follows the event
/// promptly and does not appear when nothing happened.
fn watch(window: &Window, windows: &Rc<Vec<RoledWindow>>, reporter: &Rc<RefCell<Reporter>>) {
    macro_rules! on_change {
        ($connect:ident) => {{
            let windows = Rc::clone(windows);
            let reporter = Rc::clone(reporter);
            window.$connect(move |_| write_report(&windows, &reporter));
        }};
    }

    on_change!(connect_default_width_notify);
    on_change!(connect_default_height_notify);
    on_change!(connect_is_active_notify);
    on_change!(connect_maximized_notify);
    on_change!(connect_fullscreened_notify);
    on_change!(connect_visible_notify);

    // Report the closure itself, so a test can distinguish "the window closed"
    // from "the application died". The default handler still runs afterwards.
    {
        let windows = Rc::clone(windows);
        let reporter = Rc::clone(reporter);
        window.connect_close_request(move |_| {
            write_report(&windows, &reporter);
            gtk4::glib::Propagation::Proceed
        });
    }

    // **The signal that actually makes a close observable.** The two above are
    // not enough, and believing they were cost an afternoon of blaming wgaf.
    //
    // `wgaf window close` made the window vanish from the compositor's list
    // while this application went on reporting it as visible, roughly half the
    // time and with nothing to distinguish the runs. The close was real —
    // Mutter only drops a window once the client has destroyed the surface —
    // and neither watcher above fired for it:
    //
    // - `close-request` is emitted for a *request*, and the path a compositor
    //   close takes does not always go through one.
    // - `notify::visible` is a **property notification during dispose**, which
    //   GObject is entitled to swallow while the widget is being torn down.
    //   Whether it arrives is a race, which is exactly what the intermittency
    //   looked like.
    //
    // `destroy` is emitted unconditionally, so a report always follows a close.
    // Reading the window's state here is safe: this application holds a
    // reference to every window for its whole life, so the object is still
    // alive and `is_visible()` correctly answers `false`.
    let windows = Rc::clone(windows);
    let reporter = Rc::clone(reporter);
    window.connect_destroy(move |_| write_report(&windows, &reporter));
}

fn write_report(windows: &Rc<Vec<RoledWindow>>, reporter: &Rc<RefCell<Reporter>>) {
    let states: Vec<WindowState> = windows.iter().map(RoledWindow::state).collect();
    let state = State {
        pid: std::process::id(),
        window_count: states.len(),
        windows: states,
    };

    if let Err(err) = reporter.borrow_mut().report(&state) {
        // Loud and immediate. A silent reporting failure would leave a test
        // waiting on a `seq` that is never going to change, and blaming wgaf.
        eprintln!("window-test: failed to write report: {err}");
    }
}
