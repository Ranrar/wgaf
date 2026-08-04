//! A GTK4 application with a fixed, fully specified accessible tree, for
//! testing wgaf's AT-SPI automation against something that reports its own
//! state.
//!
//! It draws one window whose accessible names and descriptions are set
//! deliberately, and rewrites its report file whenever something changes it. A
//! test drives `wgaf a11y`, then reads that file — never asking wgaf what the
//! application contains, which is the whole point (see the `report` crate's
//! documentation).
//!
//! # What this replaces, and why it had to be replaced
//!
//! `wgaf-daemon/tests/accessibility.rs` used to drive `gtk4-demo`. Its
//! expectations were facts discovered by hand-scanning roughly ninety-five
//! nodes of an application this project does not control — that a particular
//! entry advertises zero actions, that a particular view is read-only — and a
//! GTK upgrade could invalidate any of them silently. It also could not run
//! where `gtk4-demo` is not installed, which is how it broke CI.
//!
//! Worse, driving somebody else's application meant every assertion had to be
//! read back through AT-SPI: the only way to know whether a click had worked
//! was to ask AT-SPI again. That is exactly the shape of test that lets one bug
//! produce a false pass. This application reports what it observed through a
//! channel wgaf is not part of, so `wgaf a11y click` is verified by the
//! activation counter going up, not by asking what the button looks like now.
//!
//! # The tree is deliberate, element by element
//!
//! Each of these exists to make one thing checkable, and removing one silently
//! removes a test's ability to check it:
//!
//! - **[`ACTIVATE_NAME`]** — a button that implements `Action`. `wgaf a11y
//!   click` on it must raise `activate_count`.
//! - **[`INERT_NAME`]** — a plain container that implements neither `Action`
//!   nor `EditableText`, so both "this element cannot do that" paths have an
//!   unambiguous target. See the measurements below for why this is a
//!   container and not, as one would expect, a label.
//! - **[`ENTRY_NAME`]** — an editable entry: `wgaf a11y set-text` must replace
//!   `entry_text`.
//! - **[`READONLY_NAME`]** — an entry that is *not* editable. It still offers
//!   `EditableText`, and declines by returning failure, which is a different
//!   branch of the daemon's error handling from [`INERT_NAME`]'s missing
//!   interface. Both halves are asserted: the error, and `readonly_text` being
//!   unchanged — a rejection that changed the text anyway would be a worse bug
//!   than either alone.
//! - **[`PLAIN_LABEL_NAME`]** — a label, so a find has a target with a role
//!   that is neither a button nor a text box.
//! - **[`REMOVE_NAME`]** and **[`DISPOSABLE_NAME`]** — activating the first
//!   destroys the second. That is how a test gets a genuinely stale element
//!   reference without killing the application and losing the report with it.
//! - **[`FOCUS_TARGET_NAME`]** — a focusable button that does not start
//!   focused, for `wgaf a11y focus`. `focused_widget` says which widget
//!   actually holds the focus, so a focus grab is verified here rather than by
//!   asking AT-SPI whether it thinks it worked.
//! - **The deep chain** — [`DEEP_NESTING`] nested boxes ending in
//!   [`DEEP_LEAF_NAME`], so `GetTree`'s depth clamping meets a tree that is
//!   genuinely deeper than its default rather than a synthetic one.
//! - **The wide list** — [`WIDE_ITEM_COUNT`] labels sharing a name prefix, so
//!   `FindElements`'s result cap has more matches available than it will
//!   return.
//!
//! # Measured against GTK 4.22.4, because guessing was wrong
//!
//! Each of these was established by running this application and driving it
//! with `wgaf a11y`, after a first draft that assumed otherwise. They decide
//! which element can stand for which case, so they are worth re-measuring
//! rather than trusting if this ever stops behaving:
//!
//! - **A `GtkLabel` implements `Action`, and clicking one succeeds.** The
//!   obvious choice for "an element that cannot be clicked" is therefore the
//!   wrong one. A plain `GtkBox` ([`INERT_NAME`]) genuinely offers no `Action`,
//!   which is why the inert element here is a container.
//! - **A `GtkCheckButton` does *not* implement `Action`**, so a check box
//!   cannot be toggled through AT-SPI at all. An earlier draft had one, whose
//!   only contribution would have been an affordance that does not work; it was
//!   removed rather than kept as a decoration.
//! - **A `GtkEntry` implements `Action`** as well as `EditableText`.
//! - **A button's accessible name comes from its visible label, not from the
//!   accessible name set on it.** `update_property` is honoured for the
//!   *description*, and for the name on labels, entries and containers — but on
//!   a button the visible label wins. So every button here is labelled with the
//!   exact string that is its contract, and a test filtering by name gets what
//!   this file says it will.
//! - **A button therefore contains a child label with the same accessible
//!   name.** A find by name alone matches both; filter by role as well.
//! - **The window's own accessible object has a working default action**, and
//!   invoking it closes the window and ends the application. Nothing here needs
//!   that, but a test sweeping every element and clicking it will lose the
//!   application partway through and misread everything after as a wgaf
//!   failure. It cost a probe run to find that out.
//!
//! # What a test must not assume
//!
//! - **Accessibility must be enabled in the session.** With
//!   `toolkit-accessibility` off, or no AT-SPI bus running, this application
//!   still draws its window and still reports — it simply never registers, and
//!   `ListApps` never sees it. That is an environment failure, not a wgaf one,
//!   and a suite should check for the bus before blaming the daemon.
//! - **Registration is not instant.** The window is mapped before the
//!   accessible tree is exported. Poll `ListApps` rather than assuming the
//!   application is there because it wrote a report.
//! - **`seq` advances per observed change**, so take it as a baseline and wait
//!   for it to exceed that, never for a particular value.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use gtk4::prelude::*;
use gtk4::{
    AccessibleRole, Align, Application, ApplicationWindow, Box as GtkBox, Button, Entry, HeaderBar,
    Label, Orientation, PolicyType, ScrolledWindow, Widget, accessible::Property,
};
use report::Reporter;
use serde::Serialize;

/// The application id, which is also what Mutter reports as a window's
/// `app_id`. Tests filter on it, so it is part of this application's contract
/// with them.
const APP_ID: &str = "dev.wgaf.AccessibilityTest";

/// Fixed so a test can find the window without a search.
const TITLE: &str = "wgaf accessibility-test";

/// Fixed starting geometry. The wide list lives in a scrolled window so that
/// [`WIDE_ITEM_COUNT`] items do not decide how big this has to be.
const SIZE: (i32, i32) = (640, 480);

// ---------------------------------------------------------------------------
// The accessible contract
//
// These are the names a test filters on, so they are part of this
// application's contract with it — changing one breaks tests by design.
//
// Buttons carry their contract name as their *visible* label, because that is
// where GTK takes a button's accessible name from regardless of what is set on
// it explicitly (see the module docs). Everything else has its accessible name
// set explicitly and shows something shorter.
// ---------------------------------------------------------------------------

/// The button `wgaf a11y click` activates.
const ACTIVATE_NAME: &str = "wgaf activate";
/// Set as a description so that `FindElements`'s description filter has
/// something to match that no other element in this application shares.
const ACTIVATE_DESCRIPTION: &str = "Increments the activation counter";

/// A container that implements neither `Action` nor `EditableText`.
const INERT_NAME: &str = "wgaf inert";

/// The editable entry `wgaf a11y set-text` writes to.
const ENTRY_NAME: &str = "wgaf editable entry";

/// The entry that refuses to be written to. See the module docs.
const READONLY_NAME: &str = "wgaf read-only entry";
/// What [`READONLY_NAME`]'s entry contains, and must still contain after a
/// rejected `set-text`.
const READONLY_TEXT: &str = "read-only";

/// A label, for a find whose target is neither a button nor a text box.
const PLAIN_LABEL_NAME: &str = "wgaf plain label";

/// The button that destroys [`DISPOSABLE_NAME`]'s label when activated.
const REMOVE_NAME: &str = "wgaf remove";

/// The label that goes away, leaving a stale element reference behind.
const DISPOSABLE_NAME: &str = "wgaf disposable";

/// A focusable button that does not start focused.
const FOCUS_TARGET_NAME: &str = "wgaf focus target";

/// The label at the bottom of the deep chain.
const DEEP_LEAF_NAME: &str = "wgaf deep leaf";

/// How many boxes the deep chain nests before reaching [`DEEP_LEAF_NAME`].
///
/// **Must stay above the daemon's default `GetTree` depth**, which is 10
/// (`DEFAULT_TREE_DEPTH` in `wgaf-daemon/src/accessibility/mod.rs`). The whole
/// point is that a default-depth walk stops short of the leaf and a deeper
/// request reaches it; drop this below that default and the clamping test
/// keeps passing while testing nothing. Pinned by a unit test below.
const DEEP_NESTING: u32 = 14;

/// How many labels the wide list holds.
///
/// **Must stay above the daemon's default `FindElements` result cap**, which
/// is 100 (`DEFAULT_FIND_RESULTS`), for the same reason [`DEEP_NESTING`] must
/// stay above the depth default: a cap that is never reached is a cap that is
/// never tested. Pinned by a unit test below.
const WIDE_ITEM_COUNT: u32 = 120;

/// The name prefix every wide-list item shares, so a test can select exactly
/// that set with a name filter and nothing else.
const WIDE_ITEM_PREFIX: &str = "wgaf wide item";

/// The widget-name slugs `focused_widget` reports.
///
/// Separate from the accessible names above, and deliberately short: this is
/// what a test compares against, and `"focus-target"` is easier to read in a
/// failure message than the full accessible name. Set with
/// [`gtk4::prelude::WidgetExt::set_widget_name`], which carries no accessible
/// meaning at all — it cannot be confused with the name AT-SPI reports.
const SLUG_ENTRY: &str = "entry";
const SLUG_READONLY: &str = "readonly-entry";
const SLUG_ACTIVATE: &str = "activate";
const SLUG_REMOVE: &str = "remove";
const SLUG_FOCUS_TARGET: &str = "focus-target";

/// Every slug this application assigns, for the ancestor search in
/// [`focused_slug`] and for the unit test that keeps them distinct.
const SLUGS: [&str; 5] = [
    SLUG_ENTRY,
    SLUG_READONLY,
    SLUG_ACTIVATE,
    SLUG_REMOVE,
    SLUG_FOCUS_TARGET,
];

#[derive(Parser)]
#[command(
    about = "A GTK4 window with a fixed, fully specified accessible tree, for testing wgaf.",
    long_about = "Draws a window whose accessible names and descriptions are fixed, and writes a \
                  JSON report describing every change made to it through the accessibility \
                  bus.\n\nThe session must have accessibility enabled (`toolkit-accessibility`) \
                  or the window is drawn but never registers with AT-SPI."
)]
struct Args {
    /// Where to write the JSON report. The directory must already exist.
    #[arg(long, value_name = "PATH")]
    report: PathBuf,
}

/// The whole report: everything observed so far, every time.
#[derive(Serialize)]
struct State {
    /// So a test can match this window to the process it started, and tell it
    /// apart from a stale instance left by an earlier run.
    pid: u32,
    /// Whether the window currently has keyboard focus.
    ///
    /// Unlike the input suites, an accessibility test does **not** need this to
    /// be true — AT-SPI reaches an application whether or not the user is
    /// looking at it, which is one of the reasons it is a better automation
    /// route than synthesized input. Reported anyway, because a focus grab that
    /// raised the window would be worth being able to see.
    window_focused: bool,
    /// How many times [`ACTIVATE_NAME`]'s button has been activated, by any
    /// route. **This is the end-to-end answer for `wgaf a11y click`**: it rises
    /// only if the action actually reached the widget.
    activate_count: u64,
    /// [`ENTRY_NAME`]'s current text — what `wgaf a11y set-text` produced, read
    /// back through GTK rather than through AT-SPI.
    entry_text: String,
    /// [`READONLY_NAME`]'s current text, which must stay [`READONLY_TEXT`]. A
    /// `set-text` that reported failure and changed the text anyway would pass
    /// a test that only checked the error.
    readonly_text: String,
    /// The slug of whatever holds the focus inside this window, or `null` if
    /// nothing this application named does. **This is the end-to-end answer for
    /// `wgaf a11y focus`** — see the `SLUG_*` constants for the values it can
    /// take, and [`focused_slug`] for why it is not simply the focused widget's
    /// own name.
    focused_widget: Option<String>,
    /// Whether [`DISPOSABLE_NAME`]'s label is still in the widget tree. Once
    /// this is `false`, any element reference a test took for it earlier is
    /// stale.
    disposable_present: bool,
    /// How deep the chain of boxes ending in [`DEEP_LEAF_NAME`] is, as this
    /// application built it.
    ///
    /// Reported rather than left to the test to hardcode: the assertion is
    /// "wgaf's default walk does not reach as deep as this application goes",
    /// and the application is the honest source for the second half of that.
    deep_nesting: u32,
    /// How many wide-list items exist, for the same reason.
    wide_item_count: u32,
}

/// Everything the application has observed, plus the widgets it reads state
/// from at report time.
///
/// One structure rather than several `Rc<RefCell<_>>`s so that a report is
/// always internally consistent: every field is written from the same borrow.
struct Observed {
    window: ApplicationWindow,
    entry: Entry,
    readonly: Entry,
    /// The disposable label's parent, kept so the label can be removed from it.
    disposable_parent: GtkBox,
    /// `None` once the label has been removed, which is also what makes the
    /// widget's last reference go away and its accessible object with it.
    disposable: Option<Label>,
    activate_count: u64,
}

impl Observed {
    fn state(&self) -> State {
        State {
            pid: std::process::id(),
            window_focused: self.window.is_active(),
            activate_count: self.activate_count,
            entry_text: self.entry.text().into(),
            readonly_text: self.readonly.text().into(),
            focused_widget: focused_slug(&self.window),
            disposable_present: self.disposable.is_some(),
            deep_nesting: DEEP_NESTING,
            wide_item_count: WIDE_ITEM_COUNT,
        }
    }

    /// Removes the disposable label from the widget tree and drops this
    /// application's last reference to it.
    ///
    /// Both halves are needed. Removing it from its parent unparents the
    /// widget, but a reference held here would keep it — and its exported
    /// accessible object — alive, so a stale element reference would go on
    /// answering and the test would prove nothing.
    fn dispose(&mut self) {
        if let Some(label) = self.disposable.take() {
            self.disposable_parent.remove(&label);
        }
    }
}

/// The slug of the widget holding the focus inside `window`, searching upwards
/// from the focused widget.
///
/// **The ancestor walk is not defensive tidying; without it this reports
/// `"GtkText"`.** A `GtkEntry` is a composite widget, and the focus lands on
/// the internal `GtkText` it wraps rather than on the entry a test named. That
/// inner widget has no name of this application's, so the raw answer says
/// nothing about which of *our* elements is focused. Walking up to the nearest
/// widget this application did name gives the answer the report claims to be
/// giving.
///
/// `None` when the focus is somewhere this application never named — nothing at
/// all, or a widget inside the header bar — which a test should read as "not
/// where I put it" rather than as an error.
fn focused_slug(window: &ApplicationWindow) -> Option<String> {
    // Spelled out in full because `GtkWindowExt` and `WidgetExt` both provide a
    // `focus` and they mean different things — the same ambiguity `input-test`
    // documents at its `set_focus` call.
    let mut widget: Option<Widget> = GtkWindowExt::focus(window);

    while let Some(current) = widget {
        let name = current.widget_name();
        if SLUGS.contains(&name.as_str()) {
            return Some(name.into());
        }
        widget = current.parent();
    }
    None
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
    /// Called after every observed change, so that a test can wait for `seq` to
    /// advance rather than sleeping.
    fn report(&self) {
        let state = self.observed.borrow().state();

        if let Err(err) = self.reporter.borrow_mut().report(&state) {
            // Loud and immediate. A silent reporting failure would leave a test
            // waiting on a `seq` that is never going to change, and blaming
            // wgaf for an action that in fact worked.
            eprintln!("accessibility-test: failed to write report: {err}");
        }
    }
}

/// The accessible name of the wide-list item at `index`.
///
/// Zero-padded so the names sort the way the items are laid out, which matters
/// only when a human is reading a failure message — but that is when it matters
/// most. Split out from the building loop because it is the one piece of this
/// application's logic a test can check without a display.
fn wide_item_name(index: u32) -> String {
    format!("{WIDE_ITEM_PREFIX} {index:03}")
}

fn main() -> gtk4::glib::ExitCode {
    let args = Args::parse();

    let reporter = match Reporter::new("accessibility-test", &args.report) {
        Ok(reporter) => reporter,
        Err(err) => {
            // Fail before showing the window: a running application that is not
            // reporting is worse than one that never started, because a test
            // would wait out its whole timeout before finding out.
            eprintln!("accessibility-test: {err}");
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
            eprintln!("accessibility-test: ignoring a repeat activation");
            return;
        };
        build_ui(application, reporter);
    });

    // The arguments were parsed by clap above; GTK must not try to parse them
    // again and reject `--report`.
    application.run_with_args::<&str>(&[])
}

/// Builds the one window and everything in it.
fn build_ui(application: &Application, reporter: Reporter) {
    let window = ApplicationWindow::builder()
        .application(application)
        .title(TITLE)
        .default_width(SIZE.0)
        .default_height(SIZE.1)
        .build();
    window.set_titlebar(Some(&HeaderBar::builder().build()));

    // The buttons' visible labels *are* their accessible names — see the module
    // docs. Setting the accessible name as well would be dead configuration
    // that reads as though it were doing the work.
    let activate = Button::builder().label(ACTIVATE_NAME).build();
    activate.set_widget_name(SLUG_ACTIVATE);
    activate.update_property(&[Property::Description(ACTIVATE_DESCRIPTION)]);

    let remove = Button::builder().label(REMOVE_NAME).build();
    remove.set_widget_name(SLUG_REMOVE);

    let focus_target = Button::builder().label(FOCUS_TARGET_NAME).build();
    focus_target.set_widget_name(SLUG_FOCUS_TARGET);

    let entry = Entry::builder().text("").build();
    entry.set_widget_name(SLUG_ENTRY);
    entry.update_property(&[Property::Label(ENTRY_NAME)]);

    let readonly = Entry::builder().text(READONLY_TEXT).build();
    readonly.set_editable(false);
    readonly.set_widget_name(SLUG_READONLY);
    readonly.update_property(&[Property::Label(READONLY_NAME)]);

    let plain = Label::builder().label("Plain label").build();
    plain.update_property(&[Property::Label(PLAIN_LABEL_NAME)]);

    // A container, not a label: a GtkLabel implements `Action` and can be
    // clicked, so it cannot stand for "an element that cannot be clicked". See
    // the module docs.
    //
    // **The explicit role is what makes it findable at all.** A box defaults to
    // the `generic` role, and GTK discards an accessible name set on a generic
    // element — measured, after a version of this that set the name and could
    // not be found by it. A group may be named, and is equally inert.
    let inert = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .accessible_role(AccessibleRole::Group)
        .build();
    inert.update_property(&[Property::Label(INERT_NAME)]);
    inert.append(&Label::builder().label("Inert container").build());

    let disposable = Label::builder().label("Disposable").build();
    disposable.update_property(&[Property::Label(DISPOSABLE_NAME)]);
    let disposable_parent = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .build();
    disposable_parent.append(&disposable);

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&caption(
        "wgaf accessibility-test\n\nEvery element here has a fixed accessible name.\nChanges made \
         through the accessibility bus are written to the report file.",
    ));
    content.append(&activate);
    content.append(&entry);
    content.append(&readonly);
    content.append(&plain);
    content.append(&inert);
    content.append(&remove);
    content.append(&disposable_parent);
    content.append(&focus_target);
    content.append(&build_deep_chain());
    content.append(&build_wide_list());
    window.set_child(Some(&content));

    let app = Rc::new(App {
        observed: RefCell::new(Observed {
            window: window.clone(),
            entry: entry.clone(),
            readonly: readonly.clone(),
            disposable_parent,
            disposable: Some(disposable),
            activate_count: 0,
        }),
        reporter: RefCell::new(reporter),
    });

    activate.connect_clicked({
        let app = Rc::clone(&app);
        move |_| {
            app.observed.borrow_mut().activate_count += 1;
            app.report();
        }
    });

    remove.connect_clicked({
        let app = Rc::clone(&app);
        move |_| {
            app.observed.borrow_mut().dispose();
            app.report();
        }
    });

    // Both entries report on change. The read-only one is watched too,
    // deliberately: if a rejected `set-text` ever changed it anyway, nothing
    // else here would notice.
    entry.connect_changed({
        let app = Rc::clone(&app);
        move |_| app.report()
    });
    readonly.connect_changed({
        let app = Rc::clone(&app);
        move |_| app.report()
    });

    // The focus report is what makes `wgaf a11y focus` verifiable from this
    // side, so it must fire whenever the focus moves between widgets.
    window.connect_focus_widget_notify({
        let app = Rc::clone(&app);
        move |_| app.report()
    });
    window.connect_is_active_notify({
        let app = Rc::clone(&app);
        move |_| app.report()
    });

    // The entry starts focused, so `focused_widget` has a known starting value
    // and a focus grab elsewhere is visible as a *change* rather than as a
    // value that was always going to be there.
    //
    // `set_focus` before `present`, not `grab_focus` after: on Wayland the
    // window is not mapped at this point and `grab_focus` is silently dropped,
    // which `input-test` learned the hard way.
    GtkWindowExt::set_focus(&window, Some(&entry));
    window.present();

    // The first report says "up, nothing has happened yet" — the state a test
    // waits for before it starts driving anything.
    app.report();
}

/// Builds a chain of [`DEEP_NESTING`] nested boxes ending in a label, and
/// returns the outermost box.
///
/// Built from the inside out, which is the only direction that needs no
/// mutable handle on a partially built tree.
fn build_deep_chain() -> GtkBox {
    let leaf = Label::builder().label("Deep leaf").build();
    leaf.update_property(&[Property::Label(DEEP_LEAF_NAME)]);

    let mut inner = GtkBox::builder().orientation(Orientation::Vertical).build();
    inner.append(&leaf);

    // One box is already built above, so this loop adds the rest.
    for _ in 1..DEEP_NESTING {
        let outer = GtkBox::builder().orientation(Orientation::Vertical).build();
        outer.append(&inner);
        inner = outer;
    }
    inner
}

/// Builds the wide list: [`WIDE_ITEM_COUNT`] labels sharing a name prefix,
/// inside a scrolled window so that the list's length does not decide the
/// window's height.
///
/// The height cap is not cosmetic. A GTK window can never be smaller than its
/// contents' minimum size, so an uncontained list of this length would set a
/// floor on the window far above anything a test would ask for — and a resize
/// clamped by that would look like the compositor ignoring the request.
fn build_wide_list() -> ScrolledWindow {
    let list = GtkBox::builder().orientation(Orientation::Vertical).build();
    for index in 0..WIDE_ITEM_COUNT {
        let item = Label::builder().label(format!("Item {index}")).build();
        item.update_property(&[Property::Label(&wide_item_name(index))]);
        list.append(&item);
    }

    ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(PolicyType::Never)
        .min_content_height(80)
        .vexpand(true)
        .build()
}

/// The explanatory text at the top of the window.
///
/// **The wrapping is load-bearing, not cosmetic** — see [`build_wide_list`] for
/// the same reasoning about minimum sizes.
fn caption(text: &str) -> Label {
    Label::builder()
        .label(text)
        .wrap(true)
        .max_width_chars(40)
        .halign(Align::Center)
        .justify(gtk4::Justification::Center)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon's `GetTree` depth when the caller asks for none —
    /// `DEFAULT_TREE_DEPTH` in `wgaf-daemon/src/accessibility/mod.rs`.
    ///
    /// Duplicated here rather than imported: these applications are a separate
    /// Cargo workspace on purpose, so that building wgaf never needs GTK4
    /// development packages, and depending on the daemon to read one integer
    /// would undo that. The duplication is what this test exists to catch — if
    /// the daemon's default ever rises above [`DEEP_NESTING`], this fails and
    /// names the reason.
    const DAEMON_DEFAULT_TREE_DEPTH: u32 = 10;

    /// The daemon's `FindElements` result cap when the caller asks for none —
    /// `DEFAULT_FIND_RESULTS`, duplicated for the same reason.
    const DAEMON_DEFAULT_FIND_RESULTS: u32 = 100;

    /// The tree must be deeper than a default walk goes, or the depth-clamping
    /// test in `wgaf-daemon/tests/accessibility.rs` passes while proving
    /// nothing: a walk that reaches the leaf cannot demonstrate that it stopped
    /// short of it.
    // Both guards below compare two constants, which clippy flags as an
    // assertion that cannot vary. That is exactly what they are for: they fail
    // the moment somebody edits one of the constants, and the message says why
    // the edit broke a test somewhere else.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_deep_chain_is_deeper_than_the_daemons_default_tree_depth() {
        assert!(
            DEEP_NESTING > DAEMON_DEFAULT_TREE_DEPTH,
            "DEEP_NESTING ({DEEP_NESTING}) must exceed the daemon's default GetTree depth \
             ({DAEMON_DEFAULT_TREE_DEPTH}), or the depth-clamping test stops testing the clamp"
        );
    }

    /// The same argument for the result cap: fewer items than the cap and
    /// `FindElements` returns everything, which is indistinguishable from a cap
    /// that was never applied.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_wide_list_is_longer_than_the_daemons_default_result_cap() {
        assert!(
            WIDE_ITEM_COUNT > DAEMON_DEFAULT_FIND_RESULTS,
            "WIDE_ITEM_COUNT ({WIDE_ITEM_COUNT}) must exceed the daemon's default FindElements \
             cap ({DAEMON_DEFAULT_FIND_RESULTS}), or the cap test stops testing the cap"
        );
    }

    #[test]
    fn wide_item_names_share_the_prefix_and_are_zero_padded() {
        assert_eq!(wide_item_name(0), "wgaf wide item 000");
        assert_eq!(wide_item_name(7), "wgaf wide item 007");
        assert_eq!(wide_item_name(119), "wgaf wide item 119");
    }

    /// Every item must be findable by the prefix and each one distinguishable
    /// from the others — a name filter that matched two items as one would
    /// quietly change what the cap test counts.
    #[test]
    fn every_wide_item_name_is_distinct_and_matches_the_prefix() {
        let names: std::collections::HashSet<String> =
            (0..WIDE_ITEM_COUNT).map(wide_item_name).collect();

        assert_eq!(names.len(), WIDE_ITEM_COUNT as usize);
        assert!(names.iter().all(|name| name.starts_with(WIDE_ITEM_PREFIX)));
    }

    /// The accessible names are the contract a test filters on, so two elements
    /// sharing one would make a find ambiguous in a way that shows up as an
    /// off-by-one in a result count rather than as anything obviously wrong.
    #[test]
    fn the_named_elements_all_have_distinct_accessible_names() {
        let distinct: std::collections::HashSet<&str> = NAMED_ELEMENTS.iter().copied().collect();

        assert_eq!(
            distinct.len(),
            NAMED_ELEMENTS.len(),
            "duplicate accessible name"
        );
    }

    /// A fixed element's name must not fall inside another's: `FindElements`
    /// matches names by substring, so `"wgaf inert"` also matching
    /// `"wgaf inert container"` would silently return two elements where a test
    /// asserts one.
    #[test]
    fn no_fixed_element_name_is_a_substring_of_another() {
        for name in NAMED_ELEMENTS {
            for other in NAMED_ELEMENTS {
                assert!(
                    name == other || !other.contains(name),
                    "`{name}` is a substring of `{other}`, so a name filter cannot tell them apart"
                );
            }
        }
    }

    /// The same argument against the wide list, whose items a test counts.
    #[test]
    fn no_fixed_element_name_collides_with_the_wide_item_prefix() {
        for name in NAMED_ELEMENTS {
            assert!(
                !name.contains(WIDE_ITEM_PREFIX),
                "`{name}` would be matched by a search for the wide-item prefix"
            );
        }
    }

    /// The slugs are what `focused_widget` reports, so a duplicate would make a
    /// focus assertion ambiguous.
    #[test]
    fn the_widget_name_slugs_are_distinct() {
        let distinct: std::collections::HashSet<&str> = SLUGS.iter().copied().collect();

        assert_eq!(distinct.len(), SLUGS.len(), "duplicate widget-name slug");
    }

    /// Every accessible name this application fixes, in one place so the tests
    /// above cannot check a stale subset of them.
    const NAMED_ELEMENTS: [&str; 9] = [
        ACTIVATE_NAME,
        INERT_NAME,
        ENTRY_NAME,
        READONLY_NAME,
        PLAIN_LABEL_NAME,
        REMOVE_NAME,
        DISPOSABLE_NAME,
        FOCUS_TARGET_NAME,
        DEEP_LEAF_NAME,
    ];
}
