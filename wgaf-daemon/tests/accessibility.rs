//! Integration tests for `org.wgaf.Accessibility1` against the deterministic
//! `accessibility-test` application, the real AT-SPI bus, and the real daemon
//! binary.
//!
//! # What changed, and why it matters
//!
//! This suite used to drive `gtk4-demo`. Two problems came with that, and both
//! are gone:
//!
//! - **Its expectations were discovered rather than declared.** That a
//!   particular entry advertised zero actions, that a particular view was
//!   read-only — facts established by hand-scanning roughly ninety-five nodes
//!   of an application this project does not control, which a GTK upgrade could
//!   invalidate without anything here changing. `tests/apps/accessibility-test`
//!   states its tree in its own source, so a failure here means wgaf changed.
//! - **Every assertion travelled back through AT-SPI.** The only way to know
//!   whether a click had worked was to ask AT-SPI what the widget looked like
//!   afterwards, so one bug in the layer under test could produce a pass. The
//!   application now writes what it observed to a report file, and the
//!   mutating tests below assert on that file — a path wgaf is not part of.
//!   That is this project's first rule, and this suite is where it was hardest
//!   to obey.
//!
//! # Two limitations of the toolkit, measured rather than assumed
//!
//! Both were established on GNOME 50 / GTK 4.22.4 by driving this application,
//! and both are recorded in `issues.md`. Tests below assert what actually
//! happens and say so, rather than asserting what should happen and being
//! marked as failures nobody acts on:
//!
//! - **`FocusElement` cannot succeed against any GTK4 application.** The
//!   toolkit's AT-SPI bridge answers `Component.GrabFocus` with
//!   `org.freedesktop.DBus.Error.NotSupported`. The same was true of
//!   `gtk4-demo` under the previous suite, so this is a property of GTK rather
//!   than of the application or of wgaf.
//! - **A destroyed widget does not produce `ElementNotFound`.** It answers
//!   `org.freedesktop.DBus.Error.UnknownMethod`, which the daemon does not
//!   currently classify as a stale element. An exited *application* does
//!   produce `ElementNotFound` correctly, and both cases have a test.
//!
//! # Why this suite is `#[ignore]`d
//!
//! It opens real windows on a real GNOME session and needs a real accessibility
//! bus, so a plain `cargo test` must not start it. Run it deliberately:
//!
//! ```text
//! make test-desktop
//! ```
//!
//! It does **not** synthesize input, so it is safe alongside a session in use
//! in a way the keyboard suites are not.

mod harness;

use harness::{TestApp, accessibility, dbus_error_name, spawn_daemon, wait_for_daemon};
use std::time::Duration;
use wgaf_common::{AppRecord, ElementRecord, ElementRef, TreeNode};
use zbus::Connection;

/// The AT-SPI application name, which is the binary's name.
const APP: &str = "accessibility-test";

// The accessible names `accessibility-test` fixes. These are its contract with
// this suite — see its source, which explains what each element is for and why
// the buttons carry their contract name as a visible label.
const ACTIVATE: &str = "wgaf activate";
const INERT: &str = "wgaf inert";
const ENTRY: &str = "wgaf editable entry";
const READONLY: &str = "wgaf read-only entry";
const REMOVE: &str = "wgaf remove";
const DISPOSABLE: &str = "wgaf disposable";
const FOCUS_TARGET: &str = "wgaf focus target";
const DEEP_LEAF: &str = "wgaf deep leaf";
const WIDE_ITEM_PREFIX: &str = "wgaf wide item";

/// The text the deep leaf label *displays*, as distinct from [`DEEP_LEAF`],
/// which is its accessible *name*. The fixture sets the two differently on
/// purpose — see `accessibility-test`'s source.
const DEEP_LEAF_TEXT: &str = "Deep leaf";

/// What the application's read-only entry contains, and must go on containing.
const READONLY_TEXT: &str = "read-only";

/// The description set on the activate button, and on nothing else.
const ACTIVATE_DESCRIPTION: &str = "Increments the activation counter";

// Roles as `GetRoleName` reports them for this toolkit — measured, not guessed.
// The daemon deliberately reports `GetRoleName`'s string rather than the fixed
// numeric-enum name (see `accessibility/tree.rs`), so these are the friendly
// names an AT tool would show: a `GtkButton` is `button`, not `push button`.
const ROLE_BUTTON: &str = "button";
const ROLE_TEXT_BOX: &str = "text box";
const ROLE_GROUP: &str = "group";
const ROLE_LABEL: &str = "label";

/// The daemon's default `GetTree` depth, `DEFAULT_TREE_DEPTH`.
///
/// Written here rather than imported because it is private to the daemon's
/// accessibility module. The application's own `deep_nesting` report is what
/// keeps this honest: it says how deep the tree actually goes, and the test
/// below asserts the relationship between the two rather than either number
/// alone.
const DAEMON_DEFAULT_TREE_DEPTH: u32 = 10;

/// The daemon's default `FindElements` result cap, `DEFAULT_FIND_RESULTS`.
const DAEMON_DEFAULT_FIND_RESULTS: usize = 100;

/// How long to wait for the application to register with AT-SPI.
///
/// Registration happens after the window maps, so the harness returning from
/// `TestApp::spawn` is not enough. Generous on purpose: this bounds a failure,
/// not a success.
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(15);

/// Everything a test needs: a running daemon, a running application, and a
/// connection to talk to the daemon over.
struct Fixture {
    connection: Connection,
    bus_name: String,
    app: TestApp,
    // Dropped last, killing the daemon. Named rather than `_daemon` because a
    // field that is never read is exactly what the underscore is for, and this
    // one genuinely is not read.
    _daemon: harness::DaemonGuard,
}

impl Fixture {
    /// Starts the daemon and the application, and waits until AT-SPI has the
    /// application's tree.
    ///
    /// `tag` distinguishes this test's daemon bus name from every other's, so
    /// the suite does not depend on being run single-threaded even though
    /// `make test-desktop` does run it that way.
    async fn start(tag: &str) -> Self {
        harness::require_wayland_session();
        harness::require_a11y_bus().await;

        let bus_name = format!("org.wgaf.Test.A11y.{tag}{}", std::process::id());
        let daemon = spawn_daemon("a11y", &bus_name, "");
        let connection = wait_for_daemon(&bus_name).await;
        let app = TestApp::spawn(APP).await;

        let fixture = Self {
            connection,
            bus_name,
            app,
            _daemon: daemon,
        };
        fixture.wait_until_registered().await;
        fixture
    }

    /// Polls `ListApps` until the application's accessible tree is exported.
    ///
    /// **Not the same thing as the application being up.** It writes its first
    /// report as soon as its window is presented, which is what `TestApp::spawn`
    /// waits for; AT-SPI registration follows some time after that. A test that
    /// starts querying at the earlier moment fails with `AppNotFound` and looks
    /// like a daemon bug.
    async fn wait_until_registered(&self) -> AppRecord {
        let deadline = tokio::time::Instant::now() + REGISTRATION_TIMEOUT;
        loop {
            let apps: Vec<AppRecord> = self
                .call("ListApps", &())
                .await
                .expect("ListApps should succeed against a healthy accessibility bus");
            if let Some(app) = apps.into_iter().find(|app| app.name == APP) {
                return app;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "`{APP}` never registered with AT-SPI within {REGISTRATION_TIMEOUT:?}. It is \
                 running and reporting, so this is an accessibility-bus problem rather than an \
                 application one."
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn call<R, A>(&self, method: &str, args: &A) -> zbus::Result<R>
    where
        R: serde::de::DeserializeOwned + zbus::zvariant::Type,
        A: serde::Serialize + zbus::zvariant::Type,
    {
        accessibility(&self.connection, &self.bus_name, method, args).await
    }

    /// `FindElements` with the role and name filters this suite uses.
    async fn find(&self, role: &str, name: &str) -> Vec<ElementRecord> {
        self.call("FindElements", &(APP, role, name, "", 0i32))
            .await
            .expect("FindElements should succeed")
    }

    /// The one element matching `role` and `name`, failing if there is not
    /// exactly one.
    ///
    /// The count is asserted rather than the first match taken, because
    /// `FindElements` matches names by substring: a second element whose name
    /// contained this one would silently make every later assertion about a
    /// different widget. **A button's own child label carries the same
    /// accessible name**, which is precisely why the role filter is not
    /// optional here.
    async fn element(&self, role: &str, name: &str) -> ElementRef {
        let found = self.find(role, name).await;
        assert_eq!(
            found.len(),
            1,
            "expected exactly one `{role}` named `{name}`, got {}: {:?}",
            found.len(),
            found.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        found[0].element.clone()
    }
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// The read-only surface in one fixture: the application registers, its
/// elements are findable by each filter the daemon offers, and a reference can
/// be re-read afterwards.
///
/// Grouped deliberately. These share a fixture and assert nothing that can
/// affect each other, and a separate daemon and GTK4 window per assertion would
/// cost seconds apiece to tell four failures apart that a single failure
/// message already distinguishes.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn finds_the_applications_elements_by_role_name_and_description() {
    let fixture = Fixture::start("Find").await;

    // By role and name together.
    let activate = fixture.element(ROLE_BUTTON, ACTIVATE).await;

    // By description, which only the activate button carries.
    let by_description: Vec<ElementRecord> = fixture
        .call("FindElements", &(APP, "", "", ACTIVATE_DESCRIPTION, 0i32))
        .await
        .expect("FindElements by description should succeed");
    assert_eq!(
        by_description.len(),
        1,
        "only the activate button carries that description, got: {:?}",
        by_description.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(by_description[0].element, activate);

    // By role alone, for the roles this application deliberately contains. The
    // assertion is "at least one", not an exact count: the window's own header
    // bar contributes buttons and labels of its own, and pinning a total would
    // be pinning GNOME's titlebar rather than anything wgaf does.
    for role in [ROLE_BUTTON, ROLE_TEXT_BOX, ROLE_GROUP, ROLE_LABEL] {
        let found = fixture.find(role, "").await;
        assert!(!found.is_empty(), "no element with role `{role}` was found");
        assert!(
            found.iter().all(|element| element.role == role),
            "a filter for `{role}` returned something else"
        );
    }

    // And a found reference can be re-read without re-running the search.
    let info: ElementRecord = fixture
        .call("GetElementInfo", &(activate.clone(),))
        .await
        .expect("GetElementInfo should succeed against a live element");
    assert_eq!(info.element, activate);
    assert_eq!(info.name, ACTIVATE);
    assert_eq!(info.role, ROLE_BUTTON);
    assert_eq!(info.description, ACTIVATE_DESCRIPTION);
}

/// `FindElements` returns the daemon's default number of results when asked for
/// none, and fewer when asked for fewer.
///
/// The application's wide list exists so that there are genuinely more matches
/// than the cap. Its own `wide_item_count` report is what proves that — a cap
/// tested against a list shorter than the cap would pass while testing nothing,
/// and this asserts the premise rather than assuming it.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn find_elements_caps_results_at_the_daemons_default_and_honours_a_smaller_cap() {
    let fixture = Fixture::start("Cap").await;

    let available = fixture
        .app
        .read()
        .expect("the application reports")
        .u64("wide_item_count") as usize;
    assert!(
        available > DAEMON_DEFAULT_FIND_RESULTS,
        "the application offers {available} matching elements, which is not more than the \
         daemon's default cap of {DAEMON_DEFAULT_FIND_RESULTS} — this test would pass without \
         the cap being applied at all"
    );

    let capped: Vec<ElementRecord> = fixture
        .call("FindElements", &(APP, "", WIDE_ITEM_PREFIX, "", 0i32))
        .await
        .expect("FindElements should succeed");
    assert_eq!(
        capped.len(),
        DAEMON_DEFAULT_FIND_RESULTS,
        "asking for no particular number must give the daemon's default"
    );

    let five: Vec<ElementRecord> = fixture
        .call("FindElements", &(APP, "", WIDE_ITEM_PREFIX, "", 5i32))
        .await
        .expect("FindElements should succeed");
    assert_eq!(five.len(), 5);

    // A request above the number available returns everything, which is what
    // separates "the cap applied" from "the search stopped early".
    let all: Vec<ElementRecord> = fixture
        .call("FindElements", &(APP, "", WIDE_ITEM_PREFIX, "", 1000i32))
        .await
        .expect("FindElements should succeed");
    assert_eq!(all.len(), available);
}

/// `GetTree` descends to the daemon's default depth when asked for none, and
/// further when asked for more.
///
/// As with the result cap, the application reports how deep it actually is, so
/// the premise — that its tree is deeper than a default walk goes — is
/// asserted rather than assumed.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn get_tree_stops_at_the_default_depth_and_a_deeper_request_reaches_the_leaf() {
    let fixture = Fixture::start("Tree").await;

    let nesting = fixture
        .app
        .read()
        .expect("the application reports")
        .u64("deep_nesting") as u32;
    assert!(
        nesting > DAEMON_DEFAULT_TREE_DEPTH,
        "the application nests {nesting} deep, which does not exceed the daemon's default walk \
         depth of {DAEMON_DEFAULT_TREE_DEPTH} — this test would pass without any clamping"
    );

    let default: Vec<TreeNode> = fixture
        .call("GetTree", &(APP, 0i32))
        .await
        .expect("GetTree should succeed");
    let deepest = default.iter().map(|node| node.depth).max().unwrap_or(0);
    assert_eq!(
        deepest, DAEMON_DEFAULT_TREE_DEPTH,
        "a default walk must stop at the daemon's default depth"
    );
    assert!(
        !default.iter().any(|node| node.name == DEEP_LEAF),
        "the deep leaf is below the default depth and must not be reached by a default walk"
    );

    // The root is the application object itself, which is what every element
    // reference in the tree hangs off.
    assert_eq!(default[0].depth, 0);
    assert_eq!(default[0].role, "application");
    assert_eq!(default[0].name, APP);

    let deep: Vec<TreeNode> = fixture
        .call("GetTree", &(APP, 30i32))
        .await
        .expect("GetTree should succeed");
    let leaf = deep
        .iter()
        .find(|node| node.name == DEEP_LEAF)
        .unwrap_or_else(|| panic!("a walk 30 deep must reach `{DEEP_LEAF}`"));
    assert!(
        leaf.depth > DAEMON_DEFAULT_TREE_DEPTH,
        "the leaf is at depth {} — it must be below the default depth for this test to mean \
         anything",
        leaf.depth
    );
}

#[tokio::test]
#[ignore = "spawns a daemon; run via `make test-desktop`"]
async fn an_unknown_application_reports_app_not_found() {
    // Deliberately no application and no accessibility precondition: this
    // asserts what the daemon says about a name nothing answers to, which is
    // true whether or not anything is registered.
    harness::require_wayland_session();
    let bus_name = format!("org.wgaf.Test.A11y.NoApp{}", std::process::id());
    let _daemon = spawn_daemon("a11y", &bus_name, "");
    let connection = wait_for_daemon(&bus_name).await;

    let err = accessibility::<Vec<ElementRecord>, _>(
        &connection,
        &bus_name,
        "FindElements",
        &("no-such-application-xyz", "", "", "", 0i32),
    )
    .await
    .expect_err("FindElements against an unknown application must fail");

    assert_eq!(
        dbus_error_name(&err),
        Some(wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND)
    );
}

// ---------------------------------------------------------------------------
// Actions — every assertion here terminates in the application's report
// ---------------------------------------------------------------------------

/// `InvokeAction` activates the button, and **the application says so**.
///
/// This is the test the previous suite could not write. Verifying a click by
/// asking AT-SPI what the widget looks like afterwards lets one bug in the
/// layer under test produce a pass; the activation counter is written by the
/// application to a file wgaf never touches.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn invoking_the_default_action_activates_the_button_and_the_application_reports_it() {
    let fixture = Fixture::start("Click").await;
    let activate = fixture.element(ROLE_BUTTON, ACTIVATE).await;

    assert_eq!(
        fixture
            .app
            .read()
            .expect("the application reports")
            .u64("activate_count"),
        0,
        "nothing has activated it yet"
    );

    // An empty action name means the element's default action, AT-SPI's own
    // convention and what `wgaf a11y click` sends.
    fixture
        .call::<(), _>("InvokeAction", &(activate, ""))
        .await
        .expect("InvokeAction should succeed against a button");

    let report = fixture
        .app
        .wait_for("the activation to reach the button", |report| {
            report.u64("activate_count") == 1
        })
        .await;
    assert_eq!(report.u64("activate_count"), 1);
}

/// `SetText` replaces the entry's contents, and **the application says so**.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn setting_text_replaces_the_entry_contents_and_the_application_reports_it() {
    let fixture = Fixture::start("SetText").await;
    let entry = fixture.element(ROLE_TEXT_BOX, ENTRY).await;

    const TEXT: &str = "set through the accessibility bus";
    fixture
        .call::<(), _>("SetText", &(entry, TEXT))
        .await
        .expect("SetText should succeed against an editable entry");

    fixture
        .app
        .wait_for("the entry to hold the text that was set", |report| {
            report.str("entry_text") == TEXT
        })
        .await;
}

/// `GetElementText` reads back exactly what `SetText` put in, and reads a
/// static label too.
///
/// # This closes the verification hole, so it is the assertion that matters
///
/// wgaf could set text and never confirm it arrived, which meant no automation
/// could check its own work against an application this project did not write.
/// The round trip below is that check, performed the way a script would: set,
/// read, compare.
///
/// **The label half is not padding.** `Text` and `EditableText` are different
/// interfaces, and a label implements only the first — so this pins that
/// reading is available for what an application *displays*, not merely for what
/// a script has just typed. A version of this method requiring `EditableText`
/// would pass the first assertion and fail the second.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn reading_an_element_returns_the_text_that_was_set_and_a_labels_own_text() {
    let fixture = Fixture::start("GetText").await;

    const TEXT: &str = "read back through the accessibility bus";
    let entry = fixture.element(ROLE_TEXT_BOX, ENTRY).await;
    fixture
        .call::<(), _>("SetText", &(entry.clone(), TEXT))
        .await
        .expect("SetText should succeed against an editable entry");

    let read: String = fixture
        .call("GetElementText", &(entry,))
        .await
        .expect("GetElementText should succeed against an entry implementing Text");
    assert_eq!(
        read, TEXT,
        "the entry did not read back what was written into it — the round trip that lets a \
         script verify its own typing is what this method exists for"
    );

    // The read-only entry is the one element whose contents this suite already
    // treats as fixed, so it is the safest thing to assert an exact value on.
    let readonly = fixture.element(ROLE_TEXT_BOX, READONLY).await;
    let read: String = fixture
        .call("GetElementText", &(readonly,))
        .await
        .expect("a read-only entry still implements Text");
    assert_eq!(read, READONLY_TEXT);

    // A label: `Text` without `EditableText`.
    //
    // **Its displayed text is not its accessible name**, and this test found
    // that out the hard way — a first version asserted the two matched and got
    // `"Deep leaf"` where it expected `"wgaf deep leaf"`. The fixture sets them
    // separately on purpose (`Label::builder().label("Deep leaf")` plus a fixed
    // accessible name), and that is the ordinary case rather than a quirk: an
    // accessible name is written for a screen reader, the label is written for
    // the screen.
    //
    // So the two are asserted to **differ**, not to match. `wgaf a11y find
    // --name` searches the first and `wgaf a11y text` returns the second, and a
    // script that assumes they are interchangeable will be wrong on most real
    // applications.
    let label = fixture.element(ROLE_LABEL, DEEP_LEAF).await;
    let read: String = fixture
        .call("GetElementText", &(label,))
        .await
        .expect("a label implements Text even though nothing can write to it");
    assert_eq!(
        read, DEEP_LEAF_TEXT,
        "a label should read back the text it displays"
    );
    assert_ne!(
        read, DEEP_LEAF,
        "displayed text and accessible name are different things, and this fixture sets them \
         to different values so that nothing here can quietly conflate them"
    );
}

/// An element with no text at all is refused by name, not answered with an
/// empty string.
///
/// The distinction is the whole point: an empty answer must mean "this field is
/// empty", so "this element cannot hold text" has to be a different outcome. A
/// script that could not tell them apart would read a button as a blank field.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn an_element_without_text_is_refused_rather_than_read_as_empty() {
    let fixture = Fixture::start("GetTextUnsupported").await;
    let button = fixture.element(ROLE_BUTTON, ACTIVATE).await;

    let err = fixture
        .call::<String, _>("GetElementText", &(button,))
        .await
        .expect_err("a button implements no Text interface and must say so");
    assert_eq!(
        dbus_error_name(&err),
        Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED),
        "got {err:?}"
    );
}

/// An element offering no `Action` and no `EditableText` refuses both, with the
/// daemon's own named error rather than a raw D-Bus fault.
///
/// The target is a **container**, not a label. A `GtkLabel` implements `Action`
/// and can be clicked successfully — measured, after a first version of this
/// suite assumed otherwise — so a label cannot stand for "an element that
/// cannot be clicked".
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn an_inert_element_refuses_both_an_action_and_a_text_change() {
    let fixture = Fixture::start("Inert").await;
    let inert = fixture.element(ROLE_GROUP, INERT).await;

    let action_err = fixture
        .call::<(), _>("InvokeAction", &(inert.clone(), ""))
        .await
        .expect_err("an inert element must refuse an action");
    assert_eq!(
        dbus_error_name(&action_err),
        Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED)
    );

    let text_err = fixture
        .call::<(), _>("SetText", &(inert, "nothing should happen"))
        .await
        .expect_err("an element without EditableText must refuse a text change");
    assert_eq!(
        dbus_error_name(&text_err),
        Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED)
    );
}

/// The read-only entry refuses a text change **and is unchanged afterwards**.
///
/// Both halves are asserted deliberately. This element implements
/// `EditableText` and declines by returning failure, which is a different
/// branch of the daemon's handling from an element that never offered the
/// interface — and an implementation that reported the error while writing the
/// text anyway would pass a test that checked only the error.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn setting_text_on_the_read_only_entry_is_refused_and_leaves_it_unchanged() {
    let fixture = Fixture::start("ReadOnly").await;
    let readonly = fixture.element(ROLE_TEXT_BOX, READONLY).await;

    let err = fixture
        .call::<(), _>("SetText", &(readonly, "this must not be written"))
        .await
        .expect_err("a read-only entry must refuse a text change");
    assert_eq!(
        dbus_error_name(&err),
        Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED)
    );

    // The application rewrites its report whenever either entry changes, so a
    // write that got through would have produced a new report by now. Reading
    // once is enough: there is nothing to wait for, and waiting for an absence
    // is how a test becomes slow rather than how it becomes correct.
    assert_eq!(
        fixture
            .app
            .read()
            .expect("the application reports")
            .str("readonly_text"),
        READONLY_TEXT,
        "the refused text was written anyway"
    );
}

/// `FocusElement` completes and surfaces a failure against this toolkit.
///
/// **This does not assert that focus was grabbed, because on GTK4 it cannot
/// be.** The toolkit's AT-SPI bridge answers `Component.GrabFocus` with
/// `org.freedesktop.DBus.Error.NotSupported` for every widget — established
/// against `gtk4-demo` under the previous suite and re-measured here on GTK
/// 4.22.4, so it is a property of GTK rather than of this application.
///
/// What is asserted is what a user can still rely on: the call returns rather
/// than hanging, it reports the refusal **as wgaf's own `ActionNotSupported`
/// naming a remedy** rather than as the toolkit's empty-bodied D-Bus error, and
/// the application's focus is left where it was. The application reports
/// `focused_widget` so that the day the bridge does implement this, the
/// assertion to write is already obvious.
///
/// The error assertion is the S3 fix, and it is the half most worth pinning:
/// the refusal used to reach the user as
/// `org.freedesktop.DBus.Error.NotSupported:` — a fault name, a colon, and
/// nothing else.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn focus_element_completes_against_this_gtk4_bridge() {
    let fixture = Fixture::start("Focus").await;
    let target = fixture.element(ROLE_BUTTON, FOCUS_TARGET).await;

    let before = fixture.app.read().expect("the application reports");
    assert_eq!(
        before.json().get("focused_widget").and_then(|v| v.as_str()),
        Some("entry"),
        "the application focuses its entry at startup, so a successful grab elsewhere would be \
         visible as a change"
    );

    let result = fixture.call::<(), _>("FocusElement", &(target,)).await;

    match result {
        Ok(()) => panic!(
            "FocusElement succeeded against a GTK4 widget. That is a better outcome than this \
             test expects — GTK's bridge has answered NotSupported on every version measured — \
             so update this test and the entry in issues.md rather than silencing it."
        ),
        Err(err) => {
            assert_eq!(
                dbus_error_name(&err),
                Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED),
                "the toolkit's refusal must be translated into wgaf's own error, not passed \
                 through as a bare NotSupported: {err}"
            );
            let message = err.to_string();
            assert!(
                message.contains("wgaf a11y click"),
                "the message must name the remedy, since the toolkit supplies no description at \
                 all: {message}"
            );
        }
    }

    assert_eq!(
        fixture
            .app
            .read()
            .expect("the application reports")
            .json()
            .get("focused_widget")
            .and_then(|v| v.as_str()),
        Some("entry"),
        "a refused focus grab must not have moved the focus"
    );
}

/// `ScrollElement` is refused by this toolkit, legibly — and the element is
/// reachable regardless, which is the more important half.
///
/// **The target is genuinely off-screen.** The application's wide list lives in
/// a `ScrolledWindow` capped at 80px, so the last item sits roughly 2,500px
/// below a 480px window. Measured on 2026-08-12, in window coordinates;
/// `CoordType::Screen` is useless here, because GTK4 on Wayland reports
/// `(0, 0, w, h)` for every element regardless of where it is.
///
/// Two assertions, and the second is why `wgaf a11y scroll-to` is a
/// convenience rather than a prerequisite:
///
/// - The scroll is refused, as wgaf's own `ActionNotSupported` naming what
///   still works. GTK 4.22.4 answers every `ScrollType` with `NotSupported` on
///   a label and on an entry alike; Firefox implements it and succeeds, so this
///   is toolkit unevenness rather than a dead interface.
/// - **The off-screen element answers anyway.** Reading its text works with no
///   scrolling at all, because AT-SPI dispatches to the widget rather than to a
///   pixel. If this ever stops being true, the reasoning in W18.5 that made
///   `ScrollTo` optional stops holding, and this is where that shows up.
///
/// Not asserted: that a *successful* scroll moves anything. Nothing in this
/// suite's reach implements it, and verifying against Firefox would pin a test
/// to an application this project does not control — the mistake the old
/// `gtk4-demo` suite existed to demonstrate.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn scrolling_is_refused_by_this_gtk4_bridge_and_the_element_answers_anyway() {
    let fixture = Fixture::start("Scroll").await;

    // The last item in the wide list: the furthest thing from the top of the
    // window that this application contains.
    let last = fixture.app.read().expect("the application reports");
    let count = last
        .json()
        .get("wide_item_count")
        .and_then(|v| v.as_u64())
        .expect("the application reports how many wide items it built");
    let name = format!("{WIDE_ITEM_PREFIX} {:03}", count - 1);
    let offscreen = fixture.element(ROLE_LABEL, &name).await;

    match fixture
        .call::<(), _>("ScrollElement", &(offscreen.clone(),))
        .await
    {
        Ok(()) => panic!(
            "ScrollElement succeeded against a GTK4 widget. That is a better outcome than this \
             test expects — GTK's bridge answered NotSupported for every ScrollType when this was \
             measured — so update this test and W18.5 rather than silencing it."
        ),
        Err(err) => {
            assert_eq!(
                dbus_error_name(&err),
                Some(wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED),
                "the toolkit's refusal must be translated into wgaf's own error, not passed \
                 through as a bare NotSupported: {err}"
            );
            let message = err.to_string();
            assert!(
                message.contains("wgaf a11y click"),
                "the message must say what still works, since the toolkit supplies no description \
                 at all: {message}"
            );
        }
    }

    let text: String = fixture.call("GetElementText", &(offscreen,)).await.expect(
        "an off-screen element must still be readable — this is what makes scrolling \
                 optional rather than a prerequisite",
    );
    assert_eq!(
        text,
        format!("Item {}", count - 1),
        "the off-screen element returned text, but not its own"
    );
}

// ---------------------------------------------------------------------------
// Stale references
// ---------------------------------------------------------------------------

/// A reference to a widget the application destroyed stops answering.
///
/// The application destroys it on request — activating its remove button drops
/// the label and the last reference to it — so this is a genuinely stale
/// reference in a **live** application, which is the case a script hits when a
/// dialog closes between finding an element and using it.
///
/// **It is asserted as a failure rather than as `ElementNotFound`**, because
/// the daemon does not currently produce that here: GTK's bridge answers
/// `org.freedesktop.DBus.Error.UnknownMethod`, which
/// `is_stale_object_error_name` does not recognise. Filed in `issues.md`. When
/// that is fixed, tighten this to the named error — the next test shows what
/// that assertion looks like.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn a_reference_to_a_destroyed_widget_stops_answering() {
    let fixture = Fixture::start("Destroyed").await;
    let disposable = fixture.element(ROLE_LABEL, DISPOSABLE).await;
    let remove = fixture.element(ROLE_BUTTON, REMOVE).await;

    // It answers while it exists — otherwise a later failure would prove
    // nothing about the destruction.
    fixture
        .call::<ElementRecord, _>("GetElementInfo", &(disposable.clone(),))
        .await
        .expect("the element must answer before it is destroyed");

    fixture
        .call::<(), _>("InvokeAction", &(remove, ""))
        .await
        .expect("InvokeAction should succeed against the remove button");
    fixture
        .app
        .wait_for("the application to drop the disposable label", |report| {
            !report.bool("disposable_present")
        })
        .await;

    fixture
        .call::<ElementRecord, _>("GetElementInfo", &(disposable,))
        .await
        .expect_err("a destroyed element must not go on answering");
}

/// A reference outliving its application reports `ElementNotFound`.
///
/// This is the path the daemon's named error is actually produced on today: a
/// gone application means a gone bus name, which arrives as
/// `ServiceUnknown` and is classified correctly.
#[tokio::test]
#[ignore = "opens a window on the live session; run via `make test-desktop`"]
async fn a_reference_outliving_its_application_reports_element_not_found() {
    let mut fixture = Fixture::start("Exited").await;
    let entry = fixture.element(ROLE_TEXT_BOX, ENTRY).await;

    fixture.app.stop();

    // The bus name does not disappear the instant the process does, so the
    // first call after the kill can still be answered by a connection that has
    // not been reaped yet. Retry until the daemon reports the element gone, or
    // fail with whatever it said instead.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let result = fixture
            .call::<ElementRecord, _>("GetElementInfo", &(entry.clone(),))
            .await;

        if let Err(err) = &result
            && dbus_error_name(err) == Some(wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND)
        {
            return;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "the daemon never reported the element of an exited application as not found; \
             last answer: {result:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
