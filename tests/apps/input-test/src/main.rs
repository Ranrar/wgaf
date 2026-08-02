//! A GTK4 application that reports the keyboard and mouse input it receives,
//! for testing that wgaf's synthesized input actually reaches an application.
//!
//! It draws one window containing a text entry and a button, and nothing else.
//! Every key, click, pointer motion and scroll it observes rewrites its report
//! file. A test drives wgaf, then reads that file — never asking wgaf what
//! happened, which is the whole point (see the `report` crate's documentation).
//!
//! # The gap this closes
//!
//! `tests/input.rs` confirms that synthesized events reach the **kernel**: it
//! writes to `/dev/uinput` and reads the events back from the evdev device.
//! Nothing confirms that anything on the far side received them. Between the
//! kernel and an application sit libinput, the compositor, the xkb keymap and
//! the toolkit's input method, and a fault in any of them is invisible to a
//! test that stops at the kernel. This application is the far side.
//!
//! # Why the report carries both `typed` and `keys`
//!
//! They answer different questions, and a test that had only one of them would
//! misdiagnose half its failures.
//!
//! `typed` is the entry's text — the honest end-to-end answer, and what a user
//! would actually see. But it depends on the session's keyboard layout: wgaf
//! synthesizes US-QWERTY key *positions*, so on a non-US layout the same
//! keystroke produces a different character, and the mismatch is the desktop's
//! doing rather than wgaf's.
//!
//! `keys` is the raw key events with their modifier state, which arrive whatever
//! the layout is. So an empty `keys` means the input never arrived at all, while
//! a populated `keys` with an unexpected `typed` means it arrived and the layout
//! translated it differently. Those are entirely different bugs and only one of
//! them is wgaf's.
//!
//! The shift path is the specific thing this makes checkable. `wgaf type`
//! synthesizes `KEY_LEFTSHIFT` around capitals and shifted symbols, and until
//! now nothing verified the result; a `Shift_L` entry in `keys` followed by an
//! event carrying `shift: true` is that verification.
//!
//! # What a test must not assume
//!
//! - **The window must be focused before input is sent.** Synthesized input goes
//!   wherever the compositor is pointing it, not to a particular process, so an
//!   unfocused window receives nothing at all. `window_focused` is reported so a
//!   harness can treat that as a failed *precondition* rather than a failed
//!   assertion. Focus this window with `wgaf window focus` first.
//! - **Pointer motion is not comparable to the distance requested.** libinput
//!   applies pointer acceleration to relative motion, so `wgaf mouse move 50 0`
//!   does not move the pointer 50 logical pixels. The reported coordinates prove
//!   motion *arrived* and in which direction; they are not a measurement of the
//!   command's argument, and `docs/cli-reference.md` says the same thing.
//! - **Clicks can only be aimed at the window, not at a widget.** wgaf has no
//!   absolute pointer positioning yet, so a test cannot put the pointer on the
//!   button. That is why clicks are captured window-wide — see [`build_ui`].
//! - **`seq` advances per event, so typing produces a burst.** Take `seq` as a
//!   baseline once the application is up, then wait for it to exceed that.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use gtk4::gdk::{Key, ModifierType};
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, EventControllerKey,
    EventControllerMotion, EventControllerScroll, EventControllerScrollFlags, GestureClick,
    HeaderBar, Label, Orientation, PropagationPhase,
};
use report::Reporter;
use serde::Serialize;

/// The application id, which is also what Mutter reports as a window's
/// `app_id`. Tests filter on it, so it is part of this application's contract
/// with them.
const APP_ID: &str = "dev.wgaf.InputTest";

/// Fixed so a test can find the window by name without a search. Changing it
/// breaks tests by design — it is the contract.
const TITLE: &str = "wgaf input-test";

/// Fixed starting geometry. Deliberately roomy: a click can only be aimed at
/// the window as a whole, so a bigger target is a more reliable one.
const SIZE: (i32, i32) = (640, 480);

/// How many key and click events the report keeps.
///
/// The events are a rolling window rather than a full history because a single
/// `wgaf type` can produce thousands — the character cap is in the thousands,
/// and each character is a press and a release. Keeping all of them would make
/// the report unreadable by eye during a failure, which is most of the reason
/// this channel is a file at all.
///
/// Nothing is lost that a test needs: `key_event_count` and `click_count` are
/// totals over the whole run, so an assertion about *how many* events arrived
/// still holds, and an assertion about *which* events arrived is about the last
/// few in practice.
const EVENT_LOG_LIMIT: usize = 64;

#[derive(Parser)]
#[command(
    about = "A GTK4 window that reports the keyboard and mouse input it receives, for testing wgaf.",
    long_about = "Draws a text entry and a button, and writes a JSON report describing every key, \
                  click, pointer motion and scroll it receives.\n\nThe window must be focused \
                  before input is sent to it — synthesized input goes wherever the compositor is \
                  pointing it, not to a particular process."
)]
struct Args {
    /// Where to write the JSON report. The directory must already exist.
    #[arg(long, value_name = "PATH")]
    report: PathBuf,
}

/// One key event as the application received it.
#[derive(Serialize)]
struct KeyEvent {
    /// The key's name as the toolkit resolved it after applying the keymap —
    /// `a`, `A`, `Shift_L`, `Return`. Note this is a **GDK** name and not the
    /// evdev name wgaf was given: `wgaf key press enter` shows up here as
    /// `Return`. `null` for a keyval with no name, which should not happen for
    /// anything wgaf can synthesize.
    name: Option<String>,
    /// `true` for a press, `false` for a release. Both are reported: the shift
    /// path is only fully visible if the release is seen too, since a shift that
    /// is pressed and never released would leave the session broken in a way
    /// that no press-only log would show.
    pressed: bool,
    /// The hardware keycode, which is the evdev code plus 8 by X11 convention
    /// that Wayland kept. A test comparing against the code wgaf synthesized
    /// must account for the offset.
    keycode: u32,
    /// Modifier state at the time of the event. `shift` is the one that matters:
    /// it is how `wgaf type "A"` is told apart from `wgaf type "a"` at the
    /// event level, independently of what the layout produced.
    shift: bool,
    ctrl: bool,
    alt: bool,
    /// Caps Lock. Reported because a session with it stuck on would invert every
    /// letter and otherwise look like wgaf's shift handling being wrong.
    lock: bool,
}

/// One mouse click as the application received it.
#[derive(Serialize)]
struct ClickEvent {
    /// GDK button number: 1 left, 2 middle, 3 right. **This is not wgaf's
    /// ordering** — `wgaf mouse click middle` arrives here as `2` and `right` as
    /// `3`, so a test must map rather than compare names to numbers.
    button: u32,
    /// `true` for the press, `false` for the release. `wgaf mouse click` sends
    /// both, so a complete click is two entries.
    pressed: bool,
    /// Where in the window the click landed, in logical pixels from its
    /// top-left. Useful for telling "the click landed somewhere" from "the click
    /// landed on the button"; not a screen coordinate.
    x: f64,
    y: f64,
}

/// The whole report: everything observed so far, every time.
///
/// Borrows the event logs rather than owning them, so that building a report
/// cannot drift from what was recorded — there is no field-by-field copy to
/// forget to update when an event grows a field.
#[derive(Serialize)]
struct State<'a> {
    /// So a test can match this window to the process it started, and tell it
    /// apart from a stale instance left by an earlier run.
    pid: u32,
    /// Whether the window currently has keyboard focus. **A harness should check
    /// this before sending input**, not after: input sent to an unfocused window
    /// goes somewhere else entirely, and the resulting empty report looks like a
    /// synthesis failure.
    window_focused: bool,
    /// The window's current logical size in pixels, as the toolkit sees it.
    ///
    /// Here so that `wgaf window resize` can be verified from this application
    /// rather than from wgaf's own reply — the same reason `window-test` reports
    /// it. A test that combines resizing with input needs both halves from one
    /// application, and reading the size back through `wgaf window list` would
    /// be verifying wgaf with wgaf.
    ///
    /// Note this is the *client's* size and need not equal the frame rectangle
    /// Mutter reports. For the CSD windows this application draws they have
    /// matched exactly in every measurement so far, but that is an observation
    /// about GTK4 client-side decorations, not a guarantee to rely on.
    width: i32,
    height: i32,
    /// Whether the text entry holds the focus within the window. It is focused
    /// at startup and nothing here moves it, so a `false` means something
    /// outside this application took it — most likely a click that landed on
    /// another widget.
    entry_focused: bool,
    /// The entry's full text: what `wgaf type` actually produced, after the
    /// keymap and the input method had their say.
    typed: String,
    /// Every key event, capped at [`EVENT_LOG_LIMIT`]; oldest dropped first.
    keys: &'a [KeyEvent],
    /// Total key events over the whole run, uncapped.
    key_event_count: u64,
    /// Every click, capped at [`EVENT_LOG_LIMIT`]; oldest dropped first.
    clicks: &'a [ClickEvent],
    /// Total click events over the whole run, uncapped. A `wgaf mouse click`
    /// contributes two — a press and a release.
    click_count: u64,
    /// How many times the button was activated in GTK's own sense — a full
    /// press-and-release on the button widget. Distinct from `clicks`, which
    /// records raw button events anywhere in the window: a click that lands on
    /// empty space appears in `clicks` and not here.
    button_activations: u64,
    /// Where the button is, in logical pixels from the window's top-left, or
    /// `null` before the window has been laid out.
    ///
    /// **This is what makes clicking the button testable at all.** wgaf can now
    /// place the pointer on an exact coordinate, but a test that hardcoded one
    /// would be asserting against this application's current layout rather than
    /// against wgaf, and would break silently the first time a margin changed —
    /// landing on empty space, reporting a click, and activating nothing.
    /// Reporting the real bounds keeps the aiming honest: the test asks the
    /// application where to click and then checks that clicking there worked.
    ///
    /// Prefer aiming at the centre (`button_x + button_width / 2`) rather than
    /// the origin, which is the corner and is a pixel away from being outside.
    button_x: Option<f64>,
    button_y: Option<f64>,
    button_width: Option<f64>,
    button_height: Option<f64>,
    /// How many pointer motion events arrived. This is the honest measure of
    /// `wgaf mouse move`; the distance is not, because of pointer acceleration.
    motion_count: u64,
    /// The last pointer position seen, in logical pixels from the window's
    /// top-left, or `null` if the pointer has never been over the window.
    pointer_x: Option<f64>,
    pointer_y: Option<f64>,
    /// Whether the pointer is currently over the window. A test that needs the
    /// pointer here should move it and wait for this rather than assume.
    pointer_in_window: bool,
    /// How many scroll events arrived.
    scroll_count: u64,
    /// Accumulated scroll deltas. Sign follows GDK, where positive `dy` is
    /// scrolling **down** — the opposite of `wgaf mouse scroll`, whose positive
    /// `dy` is up because that is the kernel's `REL_WHEEL` convention. A test
    /// must expect the sign to flip.
    scroll_dx: f64,
    scroll_dy: f64,
}

/// Everything the application has observed, plus the widgets it reads state
/// from at report time.
///
/// One structure rather than several `Rc<RefCell<_>>`s so that a report is
/// always internally consistent: every field is written from the same borrow.
struct Observed {
    entry: Entry,
    window: ApplicationWindow,
    button: Button,
    keys: Vec<KeyEvent>,
    key_event_count: u64,
    clicks: Vec<ClickEvent>,
    click_count: u64,
    button_activations: u64,
    motion_count: u64,
    pointer: Option<(f64, f64)>,
    pointer_in_window: bool,
    scroll_count: u64,
    scroll_dx: f64,
    scroll_dy: f64,
}

impl Observed {
    fn state(&self) -> State<'_> {
        let button_bounds = self.button.compute_bounds(&self.window);
        State {
            pid: std::process::id(),
            window_focused: self.window.is_active(),
            // `default_width`/`default_height` track the current size of a
            // mapped GTK4 window, which is why they are the documented way to
            // persist one. `width()`/`height()` would report the widget's
            // allocated size, which is 0 until the window has been mapped and
            // laid out — the same trap `window-test` documents.
            width: self.window.default_width(),
            height: self.window.default_height(),
            entry_focused: self.entry.has_focus(),
            typed: self.entry.text().into(),
            keys: &self.keys,
            key_event_count: self.key_event_count,
            clicks: &self.clicks,
            click_count: self.click_count,
            button_activations: self.button_activations,
            // `compute_bounds` answers relative to the widget passed in, so
            // this is window-relative — the same space `pointer_x`/`pointer_y`
            // use, which is what lets a test add them to a window origin and
            // aim. It returns `None` until the window has been laid out.
            button_x: button_bounds.map(|b| f64::from(b.x())),
            button_y: button_bounds.map(|b| f64::from(b.y())),
            button_width: button_bounds.map(|b| f64::from(b.width())),
            button_height: button_bounds.map(|b| f64::from(b.height())),
            motion_count: self.motion_count,
            pointer_x: self.pointer.map(|(x, _)| x),
            pointer_y: self.pointer.map(|(_, y)| y),
            pointer_in_window: self.pointer_in_window,
            scroll_count: self.scroll_count,
            scroll_dx: self.scroll_dx,
            scroll_dy: self.scroll_dy,
        }
    }
}

/// Everything a signal handler needs: what has been observed, and where to
/// write it.
struct App {
    observed: RefCell<Observed>,
    reporter: RefCell<Reporter>,
}

impl App {
    /// Writes the current state to the report file.
    ///
    /// Called after every observed event, so that a test can wait for `seq` to
    /// advance rather than sleeping.
    fn report(&self) {
        // The borrow is held across the write because `State` borrows the event
        // logs. Safe: writing the report is plain file I/O and does not run the
        // GTK main loop, so no handler can re-enter and ask for a mutable
        // borrow while this one is outstanding.
        let observed = self.observed.borrow();
        let state = observed.state();

        if let Err(err) = self.reporter.borrow_mut().report(&state) {
            // Loud and immediate. A silent reporting failure would leave a test
            // waiting on a `seq` that is never going to change, and blaming
            // wgaf for input that in fact arrived.
            eprintln!("input-test: failed to write report: {err}");
        }
    }
}

/// Appends `item` to `log`, dropping the oldest entry once `limit` is reached.
///
/// Split out because it is the only non-trivial logic in this application and
/// getting it wrong would be silent: a log that grew without bound would only
/// show up as an enormous report file late in a long run.
fn push_capped<T>(log: &mut Vec<T>, item: T, limit: usize) {
    if limit == 0 {
        return;
    }
    while log.len() >= limit {
        log.remove(0);
    }
    log.push(item);
}

/// Records a key press or release.
fn record_key(app: &Rc<App>, key: Key, keycode: u32, state: ModifierType, pressed: bool) {
    {
        let mut observed = app.observed.borrow_mut();
        observed.key_event_count += 1;
        push_capped(
            &mut observed.keys,
            KeyEvent {
                name: key.name().map(Into::into),
                pressed,
                keycode,
                shift: state.contains(ModifierType::SHIFT_MASK),
                ctrl: state.contains(ModifierType::CONTROL_MASK),
                alt: state.contains(ModifierType::ALT_MASK),
                lock: state.contains(ModifierType::LOCK_MASK),
            },
            EVENT_LOG_LIMIT,
        );
    }
    app.report();
}

/// Records a mouse button press or release.
fn record_click(app: &Rc<App>, button: u32, x: f64, y: f64, pressed: bool) {
    {
        let mut observed = app.observed.borrow_mut();
        observed.click_count += 1;
        push_capped(
            &mut observed.clicks,
            ClickEvent {
                button,
                pressed,
                x,
                y,
            },
            EVENT_LOG_LIMIT,
        );
    }
    app.report();
}

fn main() -> gtk4::glib::ExitCode {
    let args = Args::parse();

    let reporter = match Reporter::new("input-test", &args.report) {
        Ok(reporter) => reporter,
        Err(err) => {
            // Fail before showing the window: a running application that is not
            // reporting is worse than one that never started, because a test
            // would wait out its whole timeout before finding out.
            eprintln!("input-test: {err}");
            return gtk4::glib::ExitCode::FAILURE;
        }
    };
    // The `Application::connect_activate` closure is `Fn`, so it may run more
    // than once and cannot consume what it captures. The `Option` is what lets
    // the first activation take the reporter and any later one find it gone.
    let reporter = Rc::new(RefCell::new(Some(reporter)));

    let application = Application::builder().application_id(APP_ID).build();

    application.connect_activate(move |application| {
        let Some(reporter) = reporter.borrow_mut().take() else {
            // A second activation would otherwise open a second window sharing
            // one report file, and the two would overwrite each other.
            eprintln!("input-test: ignoring a repeat activation");
            return;
        };
        build_ui(application, reporter);
    });

    // The arguments were parsed by clap above; GTK must not try to parse them
    // again and reject `--report`.
    application.run_with_args::<&str>(&[])
}

/// Builds the single window and subscribes to every input event worth
/// reporting.
///
/// **The controllers are attached to the window itself in the capture phase,
/// not to a child widget.** Three reasons, all learned by running this:
///
/// - **A key event only reaches widgets on the path to the focused one.** An
///   earlier version put the key controller on the content box, which observed
///   nothing whenever focus sat outside that box — including the case where no
///   widget was focused at all, which is what a freshly mapped window gives you
///   if you do not arrange otherwise. The window is the root of every
///   propagation chain, so it is the one place that cannot be bypassed.
/// - **A child widget consumes what it handles.** The entry claims key events
///   and the button claims clicks that land on it, so a bubble-phase controller
///   would silently miss exactly the events a test cares most about. The capture
///   phase runs before any child gets its turn.
/// - **wgaf cannot aim.** Absolute pointer positioning does not exist yet, so a
///   test has no way to put the pointer over the button before clicking. A click
///   target the size of the window is the only one it can reliably hit.
///
/// Every handler returns `Proceed`, so observing an event never stops it
/// reaching the widget it was aimed at — the entry must still receive the text
/// it is being typed into.
fn build_ui(application: &Application, reporter: Reporter) {
    let window = ApplicationWindow::builder()
        .application(application)
        .title(TITLE)
        .default_width(SIZE.0)
        .default_height(SIZE.1)
        .build();
    // An explicit header bar, so the window is client-side decorated the way a
    // typical GTK4 application is.
    window.set_titlebar(Some(&HeaderBar::builder().build()));

    let entry = Entry::builder()
        .placeholder_text("typed text appears here")
        .hexpand(true)
        .build();

    let button = Button::builder().label("Click target").build();

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        // So the box fills the window and a click anywhere lands on it.
        .vexpand(true)
        .hexpand(true)
        .build();
    content.append(&caption(
        "wgaf input-test\n\nType into the entry, click anywhere.\nEverything received is written \
         to the report file.",
    ));
    content.append(&entry);
    content.append(&button);
    window.set_child(Some(&content));

    let app = Rc::new(App {
        observed: RefCell::new(Observed {
            entry: entry.clone(),
            window: window.clone(),
            button: button.clone(),
            keys: Vec::new(),
            key_event_count: 0,
            clicks: Vec::new(),
            click_count: 0,
            button_activations: 0,
            motion_count: 0,
            pointer: None,
            pointer_in_window: false,
            scroll_count: 0,
            scroll_dx: 0.0,
            scroll_dy: 0.0,
        }),
        reporter: RefCell::new(reporter),
    });

    let keys = EventControllerKey::new();
    keys.set_propagation_phase(PropagationPhase::Capture);
    keys.connect_key_pressed({
        let app = Rc::clone(&app);
        move |_, key, keycode, state| {
            record_key(&app, key, keycode, state, true);
            gtk4::glib::Propagation::Proceed
        }
    });
    keys.connect_key_released({
        let app = Rc::clone(&app);
        move |_, key, keycode, state| record_key(&app, key, keycode, state, false)
    });
    window.add_controller(keys);

    let clicks = GestureClick::new();
    // Button 0 means every button, so `wgaf mouse click right` and `middle` are
    // observed too. The default is button 1 only, which would have made
    // two-thirds of `wgaf mouse click` unverifiable.
    clicks.set_button(0);
    clicks.set_propagation_phase(PropagationPhase::Capture);
    clicks.connect_pressed({
        let app = Rc::clone(&app);
        move |gesture, _, x, y| record_click(&app, gesture.current_button(), x, y, true)
    });
    clicks.connect_released({
        let app = Rc::clone(&app);
        move |gesture, _, x, y| record_click(&app, gesture.current_button(), x, y, false)
    });
    window.add_controller(clicks);

    button.connect_clicked({
        let app = Rc::clone(&app);
        move |_| {
            app.observed.borrow_mut().button_activations += 1;
            app.report();
        }
    });

    let motion = EventControllerMotion::new();
    motion.set_propagation_phase(PropagationPhase::Capture);
    motion.connect_motion({
        let app = Rc::clone(&app);
        move |_, x, y| {
            {
                let mut observed = app.observed.borrow_mut();
                observed.motion_count += 1;
                observed.pointer = Some((x, y));
                observed.pointer_in_window = true;
            }
            app.report();
        }
    });
    motion.connect_leave({
        let app = Rc::clone(&app);
        move |_| {
            app.observed.borrow_mut().pointer_in_window = false;
            app.report();
        }
    });
    window.add_controller(motion);

    // `BOTH_AXES` rather than `VERTICAL`, so a horizontal `wgaf mouse scroll` is
    // observed as well as a vertical one.
    let scroll = EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
    scroll.set_propagation_phase(PropagationPhase::Capture);
    scroll.connect_scroll({
        let app = Rc::clone(&app);
        move |_, dx, dy| {
            {
                let mut observed = app.observed.borrow_mut();
                observed.scroll_count += 1;
                observed.scroll_dx += dx;
                observed.scroll_dy += dy;
            }
            app.report();
            gtk4::glib::Propagation::Proceed
        }
    });
    window.add_controller(scroll);

    // Report whenever focus changes, so a harness can wait for the window to be
    // focused before it sends anything — the precondition that decides whether
    // the input arrives at all.
    window.connect_is_active_notify({
        let app = Rc::clone(&app);
        move |_| app.report()
    });

    // Report whenever the window is resized, so `wgaf window resize` can be
    // verified from this side. Both dimensions are watched: a resize that
    // changes only one would otherwise go unreported, and a test waiting on it
    // would time out against a window that had in fact done as it was told.
    window.connect_default_width_notify({
        let app = Rc::clone(&app);
        move |_| app.report()
    });
    window.connect_default_height_notify({
        let app = Rc::clone(&app);
        move |_| app.report()
    });

    // Which widget holds the focus inside our own window is ours to decide,
    // unlike focus *between* windows — so a test never has to arrange for the
    // entry to be ready to receive text.
    //
    // `set_focus` rather than `grab_focus`, and **before** `present` rather than
    // after: `grab_focus` acts on the widget hierarchy as it stands, and on
    // Wayland the window is not mapped yet at this point, so the call is
    // silently dropped and the window comes up with nothing focused. That was
    // not a cosmetic difference — with no focused widget, key events never
    // reached the entry and the application reported that it had received
    // nothing at all while wgaf reported success.
    //
    // Spelled out in full because `GtkWindowExt` and `RootExt` both provide a
    // `set_focus` and neither is more obviously right at the call site.
    GtkWindowExt::set_focus(&window, Some(&entry));
    window.present();

    // The first report says "up, nothing has happened yet" — the state a test
    // waits for before it starts driving anything.
    app.report();
}

/// The explanatory text above the entry.
///
/// **The wrapping is load-bearing, not cosmetic.** A GTK window can never be
/// smaller than its contents' minimum size, so a long unwrapped label silently
/// sets a floor on the window's width — which would look like `wgaf window
/// resize` being ignored rather than like GTK clamping it. Wrapping at a small
/// character count keeps the minimum well below any size a test uses.
fn caption(text: &str) -> Label {
    Label::builder()
        .label(text)
        .wrap(true)
        .max_width_chars(32)
        .halign(Align::Center)
        .justify(gtk4::Justification::Center)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that keeps a long `wgaf type` from producing an unreadable
    /// report: the log holds the most recent events, not the first ones.
    #[test]
    fn the_event_log_keeps_the_newest_entries_and_drops_the_oldest() {
        let mut log = Vec::new();
        for i in 0..10 {
            push_capped(&mut log, i, 4);
        }
        assert_eq!(log, vec![6, 7, 8, 9]);
    }

    #[test]
    fn a_log_below_the_limit_keeps_everything_in_order() {
        let mut log = Vec::new();
        for i in 0..3 {
            push_capped(&mut log, i, 4);
        }
        assert_eq!(log, vec![0, 1, 2]);
    }

    /// The boundary: filling the log exactly must not drop anything.
    #[test]
    fn a_log_filled_exactly_to_the_limit_drops_nothing() {
        let mut log = Vec::new();
        for i in 0..4 {
            push_capped(&mut log, i, 4);
        }
        assert_eq!(log, vec![0, 1, 2, 3]);
    }

    /// A zero limit must be a no-op rather than an underflow. Not reachable
    /// through [`EVENT_LOG_LIMIT`], but the guard is what makes that true by
    /// construction rather than by inspection.
    #[test]
    fn a_zero_limit_records_nothing() {
        let mut log = Vec::new();
        push_capped(&mut log, 1, 0);
        assert!(log.is_empty());
    }

    /// The log must shrink to a smaller limit rather than sitting above it,
    /// which is the case a plain "pop one if full" would get wrong.
    #[test]
    fn an_oversized_log_is_trimmed_down_to_the_limit() {
        let mut log = vec![1, 2, 3, 4, 5];
        push_capped(&mut log, 6, 3);
        assert_eq!(log, vec![4, 5, 6]);
    }

    /// The modifier flags are read straight off the event, and `shift` is what
    /// makes `wgaf type "A"` distinguishable from `"a"` at the event level.
    #[test]
    fn modifier_flags_are_read_independently_of_each_other() {
        let state = ModifierType::SHIFT_MASK | ModifierType::CONTROL_MASK;

        assert!(state.contains(ModifierType::SHIFT_MASK));
        assert!(state.contains(ModifierType::CONTROL_MASK));
        assert!(!state.contains(ModifierType::ALT_MASK));
        assert!(!state.contains(ModifierType::LOCK_MASK));
    }
}
