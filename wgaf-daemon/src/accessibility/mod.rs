//! Daemon-side accessibility automation: a client of the AT-SPI
//! accessibility bus (`org.a11y.Bus`, distinct from the session bus), via the
//! `atspi` crate (Odilia project, pure-Rust, `zbus`-based — matches this
//! workspace's own `zbus`/`tokio` stack exactly). Enumerates registered
//! accessible applications, walks/filters their accessible object trees, and
//! invokes actions (click/press/activate, set text, focus) on found
//! elements. Exposed to the CLI via the daemon's own `org.wgaf.Accessibility1`
//! interface, see `crate::dbus::accessibility_api`.
//!
//! Like `input/` and unlike `windows/`, there is no GNOME Shell Extension
//! hop here — GJS/Mutter plays no role in AT-SPI at all; applications
//! register their own accessible trees directly with the separate a11y
//! bus, independent of the compositor.
//!
//! **Element references.** AT-SPI's own object-reference scheme — a
//! `(bus_name, object_path)` pair — is already stable and unique for the
//! lifetime of the owning application process, so it's exposed directly as
//! [`wgaf_common::ElementRef`] rather than wrapped in an invented id scheme
//! (contrast `windows/`'s window ids, which had to substitute Mutter's
//! stable sequence number for X11's unsafe/unstable XIDs — there is no
//! equivalent problem here).
//!
//! **Audit logging, not policy:** every *mutating* action (`invoke_action`,
//! `set_text`, `focus_element`) is logged via `tracing` on the
//! `wgaf_daemon::accessibility::audit` target before it executes, mirroring
//! `input::AUDIT_TARGET`'s "accountability trail, not an allow/deny gate"
//! design (the permissions module owns real policy). Read-only queries
//! (`list_apps`, `find_elements`, `get_tree`, `get_element_info`) do *not* log here, same
//! reasoning as `input`'s module docs: nothing is blocked by this, so
//! logging only the state-changing calls keeps the audit trail meaningful
//! rather than noisy.

mod actions;
mod connection;
mod query;
mod tree;

use thiserror::Error;
use tokio::sync::OnceCell;
use wgaf_common::{AppRecord, ElementRecord, ElementRef, TreeNode};

/// `tracing` target for the accessibility-automation audit trail (see module
/// docs).
const AUDIT_TARGET: &str = "wgaf_daemon::accessibility::audit";

/// `GetTree`'s depth when the caller passes `max_depth <= 0`.
const DEFAULT_TREE_DEPTH: u32 = 10;
/// Hard clamp on a caller-supplied `max_depth` — guards against a
/// pathologically large request rather than reflecting any real AT-SPI
/// limit.
const MAX_TREE_DEPTH: u32 = 64;
/// Hard cap on the number of nodes any single tree walk (`GetTree` or the
/// whole-tree search `FindElements` does internally) will visit — some
/// accessible trees (a browser's DOM, an Electron app's) can be enormous;
/// this bounds the work a single call can do, mirroring
/// `input::DEFAULT_MAX_TYPE_TEXT_CHARS`'s "sane hard cap, not a policy
/// decision" role.
const MAX_TREE_NODES: usize = 5000;
/// `FindElements`'s result count when the caller passes `max_results <= 0`.
const DEFAULT_FIND_RESULTS: usize = 100;
/// Hard clamp on a caller-supplied `max_results`.
const MAX_FIND_RESULTS: usize = 1000;

/// Errors surfaced by the daemon's accessibility-automation layer.
#[derive(Debug, Error)]
pub enum AccessibilityError {
    /// Could not connect to the AT-SPI accessibility bus at all — usually an
    /// environment problem rather than a code bug. Not cached as permanent:
    /// the next call retries (mirrors
    /// `input::InputError::DeviceUnavailable`/`windows::WindowsError::ExtensionUnavailable`'s
    /// "recover without a daemon restart" precedent).
    ///
    /// **`reason` carries the whole explanation, including the remedy.** This
    /// variant deliberately appends no hint of its own: it is raised for
    /// several distinct causes — no accessibility bus at all, a stale address
    /// left behind by one that exited, an unreachable session bus — and a
    /// single trailing guess is wrong for most of them. `connection::diagnose`
    /// works out which case applies and words it accordingly.
    #[error("AT-SPI accessibility bus unavailable: {reason}")]
    BusUnavailable { reason: String },

    /// `FindElements`/`GetTree`'s `app` filter matched no currently
    /// registered accessible application.
    #[error("no registered accessible application matching `{0}` — see `wgaf a11y list-apps`")]
    AppNotFound(String),

    /// An [`ElementRef`] no longer corresponds to a live accessible object
    /// (the application exited, or the widget was destroyed).
    #[error("accessible element not found (likely gone/destroyed): {0}")]
    ElementNotFound(String),

    /// An [`ElementRef`] is not a well-formed D-Bus `(bus name, object path)`
    /// pair, so no lookup was ever attempted. A caller-input mistake, not a
    /// timing problem — kept distinct from [`Self::ElementNotFound`] because
    /// the remedy differs: fix the reference, rather than re-run the query
    /// that produced it.
    #[error(
        "invalid element reference `{element}`: {reason} — expected \
         `<bus-name>#<object-path>`, e.g. `:1.42#/org/a11y/atspi/accessible/1`, \
         as printed by `wgaf a11y find`"
    )]
    InvalidElementRef { element: String, reason: String },

    /// `InvokeAction`/`SetText`/`FocusElement` was attempted on an element
    /// that doesn't implement the AT-SPI interface the operation needs, has
    /// no matching action, or reported failure performing it.
    #[error("action not supported: {0}")]
    ActionNotSupported(String),

    /// Any other `atspi`-level failure (e.g. establishing the initial
    /// connection) not already covered by [`Self::BusUnavailable`].
    #[error("AT-SPI error: {0}")]
    Atspi(#[from] atspi::AtspiError),

    /// Any other D-Bus-level failure talking to an accessible application.
    #[error("D-Bus error talking to the accessibility bus: {0}")]
    DBus(#[from] zbus::Error),
}

/// Owns the daemon's lazily-established AT-SPI bus connection and exposes
/// the enumerate/walk/find/act operations `org.wgaf.Accessibility1`
/// delegates to. Analogous in spirit to `windows::WindowManager` and
/// `input::InputBackend`: one instance created at daemon startup, the actual
/// resource (here, the AT-SPI bus connection) is only established lazily on
/// first real use via `tokio::sync::OnceCell`, cached only on success — so a
/// daemon started before accessibility is enabled for the session recovers
/// automatically on the next call, without a daemon restart.
pub struct AccessibilityBackend {
    connection: OnceCell<atspi::AccessibilityConnection>,
}

impl Default for AccessibilityBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessibilityBackend {
    /// Does not touch the AT-SPI bus — safe to call unconditionally at
    /// daemon startup. The connection is only actually established on first
    /// use, via [`Self::connection`].
    pub fn new() -> Self {
        Self {
            connection: OnceCell::new(),
        }
    }

    async fn connection(&self) -> Result<&atspi::AccessibilityConnection, AccessibilityError> {
        self.connection.get_or_try_init(connection::connect).await
    }

    /// Freshly checks whether the AT-SPI bus is reachable, for
    /// `org.wgaf.Daemon1.Status`.
    ///
    /// Opens its own throwaway connection rather than going through
    /// [`Self::connection`], for the same reason `WindowManager::probe_available`
    /// bypasses its cache: the `OnceCell` holds a successful connection for
    /// the daemon's lifetime, so consulting it would report "available" long
    /// after the a11y stack had gone away. The throwaway connection is
    /// dropped immediately.
    pub async fn probe_bus(&self) -> Result<(), AccessibilityError> {
        connection::connect().await.map(|_| ())
    }

    /// Whether an AT-SPI connection is currently held. Activity, not health —
    /// `false` merely means no accessibility call has been made yet this run.
    /// Reads the `OnceCell` without initializing it.
    pub fn is_connected(&self) -> bool {
        self.connection.initialized()
    }

    /// Enumerates every currently-registered accessible application (the AT-SPI
    /// registry's root object's children).
    pub async fn list_apps(&self) -> Result<Vec<AppRecord>, AccessibilityError> {
        let conn = self.connection().await?;
        let registry_root = conn.root_accessible_on_registry().await?;
        let children = registry_root
            .get_children()
            .await
            .map_err(translate_element_error)?;

        let mut apps = Vec::with_capacity(children.len());
        for child in children {
            let Some(element) = tree::element_ref_from_object_ref(&child) else {
                continue;
            };
            let proxy = accessible_proxy_for(conn.connection(), &element).await?;
            let name = proxy.name().await.map_err(translate_element_error)?;
            apps.push(AppRecord { element, name });
        }
        Ok(apps)
    }

    /// Finds elements within the application matching `app_name` (see
    /// `query::find_matching_app`) whose role/name/description satisfy the
    /// given filters (empty = match anything, see `query::matches`).
    /// Searches the application's whole accessible tree (bounded by
    /// [`MAX_TREE_NODES`]), not just its immediate children.
    pub async fn find_elements(
        &self,
        app_name: &str,
        role: &str,
        name_filter: &str,
        description_filter: &str,
        max_results: i32,
    ) -> Result<Vec<ElementRecord>, AccessibilityError> {
        let apps = self.list_apps().await?;
        let app = query::find_matching_app(&apps, app_name)
            .ok_or_else(|| AccessibilityError::AppNotFound(app_name.to_string()))?
            .clone();

        let conn = self.connection().await?;
        let nodes = tree::walk(
            conn.connection(),
            app.element,
            MAX_TREE_DEPTH,
            MAX_TREE_NODES,
        )
        .await?;

        let cap = clamp_max_results(max_results);
        Ok(nodes
            .into_iter()
            .filter(|n| {
                query::matches(
                    &n.role,
                    &n.name,
                    &n.description,
                    role,
                    name_filter,
                    description_filter,
                )
            })
            .take(cap)
            .map(tree::to_element_record)
            .collect())
    }

    /// Walks the accessible tree rooted at the application matching
    /// `app_name`, up to `max_depth` levels deep (`<= 0` uses
    /// [`DEFAULT_TREE_DEPTH`], clamped to [`MAX_TREE_DEPTH`]), returned as a
    /// flat, depth-first-ordered list (see `tree::walk`'s docs for why).
    pub async fn get_tree(
        &self,
        app_name: &str,
        max_depth: i32,
    ) -> Result<Vec<TreeNode>, AccessibilityError> {
        let apps = self.list_apps().await?;
        let app = query::find_matching_app(&apps, app_name)
            .ok_or_else(|| AccessibilityError::AppNotFound(app_name.to_string()))?
            .clone();

        let conn = self.connection().await?;
        tree::walk(
            conn.connection(),
            app.element,
            clamp_depth(max_depth),
            MAX_TREE_NODES,
        )
        .await
    }

    /// Re-reads a single element's info directly from its [`ElementRef`],
    /// without needing to re-run a `find`/`tree` search first.
    pub async fn get_element_info(
        &self,
        element: &ElementRef,
    ) -> Result<ElementRecord, AccessibilityError> {
        let conn = self.connection().await?;
        let proxy = accessible_proxy_for(conn.connection(), element).await?;
        tree::element_record(&proxy, element.clone()).await
    }

    /// Invokes an accessible action on `element` — see
    /// `actions::invoke_action`'s docs for `action_name`'s semantics.
    pub async fn invoke_action(
        &self,
        element: &ElementRef,
        action_name: &str,
    ) -> Result<(), AccessibilityError> {
        tracing::info!(
            target: AUDIT_TARGET,
            action = "invoke_action",
            element = %element,
            action_name = %action_name,
            "invoking accessible action"
        );
        let conn = self.connection().await?;
        actions::invoke_action(conn.connection(), element, action_name).await
    }

    /// Replaces `element`'s text content, if it implements `EditableText`.
    pub async fn set_text(
        &self,
        element: &ElementRef,
        text: &str,
    ) -> Result<(), AccessibilityError> {
        tracing::info!(
            target: AUDIT_TARGET,
            action = "set_text",
            element = %element,
            len = text.len(),
            "setting accessible element text"
        );
        let conn = self.connection().await?;
        actions::set_text(conn.connection(), element, text).await
    }

    /// Reads `element`'s text, if it implements `Text`.
    ///
    /// **No audit line, deliberately.** The `permissions::audit` trail records
    /// what wgaf *did to* the desktop; this changes nothing, exactly like
    /// `find_elements` and `get_tree`, neither of which logs either. Logging
    /// every read would also put the widget's contents — which may be whatever
    /// the user is typing — into a file, which is the opposite of what the
    /// trail is for.
    pub async fn get_text(&self, element: &ElementRef) -> Result<String, AccessibilityError> {
        let conn = self.connection().await?;
        actions::get_text(conn.connection(), element).await
    }

    /// Requests keyboard focus for `element`, if it implements `Component`.
    pub async fn focus_element(&self, element: &ElementRef) -> Result<(), AccessibilityError> {
        tracing::info!(
            target: AUDIT_TARGET,
            action = "focus",
            element = %element,
            "focusing accessible element"
        );
        let conn = self.connection().await?;
        actions::focus(conn.connection(), element).await
    }

    /// Scrolls `element` into view, if it implements `Component`.
    ///
    /// Audited like the other mutating operations: it moves what the user is
    /// looking at, so the trail should say wgaf did it.
    pub async fn scroll_element(&self, element: &ElementRef) -> Result<(), AccessibilityError> {
        tracing::info!(
            target: AUDIT_TARGET,
            action = "scroll",
            element = %element,
            "scrolling accessible element into view"
        );
        let conn = self.connection().await?;
        actions::scroll_to(conn.connection(), element).await
    }
}

/// Builds an `AccessibleProxy` for `element` on `conn`, with property
/// caching disabled — matches `atspi`'s own `root_accessible_on_registry`
/// precedent (see `atspi-connection`'s docs: some accessible objects, like
/// the registry root, have an incomplete `DBus` property-caching
/// implementation, and the elements this daemon looks up are arbitrary,
/// short-lived query results rather than long-held handles, so caching
/// properties across calls would offer little benefit for the added risk of
/// serving stale data).
async fn accessible_proxy_for<'a>(
    conn: &'a zbus::Connection,
    element: &ElementRef,
) -> Result<atspi::proxy::accessible::AccessibleProxy<'a>, AccessibilityError> {
    // Owned `String`s (not borrowed `&str`s) so the resulting `BusName`/
    // `ObjectPath` don't tie the proxy's lifetime to `element`'s — matches
    // `windows::proxy::ShellExtensionProxy`'s existing
    // `.destination(bus_name.to_string())` convention.
    // Validate the two halves separately, before the builder, so a malformed
    // reference is reported as the caller-input mistake it is
    // (`InvalidElementRef`, a named D-Bus error) rather than escaping as a
    // bare `zbus::Error` that the API layer would flatten into a generic
    // failure — which is what used to reach the CLI as an unreadable
    // `MethodError(...)` debug dump.
    let invalid = |reason: String| AccessibilityError::InvalidElementRef {
        element: element.to_string(),
        reason,
    };
    let destination = zbus::names::BusName::try_from(element.bus_name.clone())
        .map_err(|e| invalid(e.to_string()))?;
    let path = zbus::zvariant::ObjectPath::try_from(element.object_path.clone())
        .map_err(|e| invalid(e.to_string()))?;

    Ok(atspi::proxy::accessible::AccessibleProxy::builder(conn)
        .destination(destination)?
        .path(path)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?)
}

/// Maps a D-Bus error indicating the destination object/service no longer
/// exists (`org.freedesktop.DBus.Error.UnknownObject`/`.ServiceUnknown` — what
/// a stale [`ElementRef`] looks like once the widget's destroyed or the
/// application's exited) to [`AccessibilityError::ElementNotFound`]. Mirrors
/// `windows::translate_window_error`'s "translate the specific known case,
/// pass everything else through" pattern.
///
/// **Both `MethodError` and `FDO` are matched, and missing the second one was a
/// real bug.** zbus reports the same bus error under either variant depending
/// on how the call was made: a *method* call gives `MethodError`, while reading
/// a *property* gives `FDO`. `GetElementInfo` begins by reading `Accessible`'s
/// `Name` property, so an element whose application had exited came back as a
/// generic D-Bus failure while the identical fault on `InvokeAction` — which
/// calls a method first — was reported correctly as `ElementNotFound`. Found by
/// `tests/accessibility.rs` killing the application it had just queried; the
/// two paths had never been compared before, because nothing tested the second.
fn translate_element_error(err: zbus::Error) -> AccessibilityError {
    let stale_detail = match &err {
        zbus::Error::MethodError(name, description, _)
            if is_stale_object_error_name(name.as_str()) =>
        {
            Some(description.clone().unwrap_or_else(|| err.to_string()))
        }
        zbus::Error::FDO(fdo)
            if is_stale_object_error_name(zbus::DBusError::name(&**fdo).as_str()) =>
        {
            Some(fdo.to_string())
        }
        _ => None,
    };

    match stale_detail {
        Some(detail) => AccessibilityError::ElementNotFound(detail),
        None => AccessibilityError::DBus(err),
    }
}

/// Whether a D-Bus error name indicates the destination object/service no
/// longer exists — what a stale [`ElementRef`] looks like once the widget's
/// destroyed or the owning application's exited. Split out as its own pure
/// function (rather than inlined in [`translate_element_error`]) so it's
/// unit-testable without constructing a full `zbus::Error::MethodError`
/// (which needs a real `zbus::Message`).
fn is_stale_object_error_name(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.UnknownObject" | "org.freedesktop.DBus.Error.ServiceUnknown"
    )
}

/// Maps a toolkit's outright refusal of an operation
/// (`org.freedesktop.DBus.Error.NotSupported`) to
/// [`AccessibilityError::ActionNotSupported`] carrying `explanation`, and
/// leaves every other error to [`translate_element_error`].
///
/// # Why this is shared rather than written once inline
///
/// **GTK4 answers `NotSupported` with an empty description**, so the raw error
/// reaches the user as `org.freedesktop.DBus.Error.NotSupported:` — a fault
/// name, a colon, and nothing. Two operations hit this, for one underlying
/// reason (the GTK AT-SPI bridge implements neither), and both were measured
/// against `accessibility-test` on GTK 4.22.4:
///
/// - `Component.GrabFocus` — every widget tried, including a focusable button
///   that a keyboard `Tab` focuses normally.
/// - `Component.ScrollTo` / `ScrollToPoint` — every [`atspi::ScrollType`], on
///   a label and on an entry alike.
///
/// The interface *is* advertised by `GetInterfaces` and the methods *are* in
/// the introspection XML, so the checks in `actions` cannot catch this ahead of
/// time: the only signal is the call failing. Firefox implements both and
/// succeeds, so this is toolkit unevenness rather than a dead interface.
///
/// **`explanation` must name a remedy, not merely restate the failure.** That is
/// the difference this exists to make; each caller knows what to suggest.
///
/// Both `MethodError` and `FDO` are matched for the reason
/// [`translate_element_error`] documents at length — zbus reports the same bus
/// error under either variant depending on whether a method or a property was
/// called. Both operations here are methods, so only the first can fire today;
/// matching one of the two is the bug that was already found once in this file,
/// and it is not worth re-introducing for a line.
fn translate_toolkit_refusal(err: zbus::Error, explanation: String) -> AccessibilityError {
    let refused = match &err {
        zbus::Error::MethodError(name, _, _) => is_unsupported_error_name(name.as_str()),
        zbus::Error::FDO(fdo) => is_unsupported_error_name(zbus::DBusError::name(&**fdo).as_str()),
        _ => false,
    };

    if refused {
        AccessibilityError::ActionNotSupported(explanation)
    } else {
        translate_element_error(err)
    }
}

/// Whether `name` is the D-Bus error a toolkit sends to say it does not
/// implement an operation it nonetheless advertises.
fn is_unsupported_error_name(name: &str) -> bool {
    name == "org.freedesktop.DBus.Error.NotSupported"
}

fn clamp_depth(max_depth: i32) -> u32 {
    if max_depth <= 0 {
        DEFAULT_TREE_DEPTH
    } else {
        (max_depth as u32).min(MAX_TREE_DEPTH)
    }
}

fn clamp_max_results(max_results: i32) -> usize {
    if max_results <= 0 {
        DEFAULT_FIND_RESULTS
    } else {
        (max_results as usize).min(MAX_FIND_RESULTS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_depth_uses_default_for_non_positive_input() {
        assert_eq!(clamp_depth(0), DEFAULT_TREE_DEPTH);
        assert_eq!(clamp_depth(-5), DEFAULT_TREE_DEPTH);
    }

    #[test]
    fn clamp_depth_passes_through_within_range() {
        assert_eq!(clamp_depth(3), 3);
    }

    #[test]
    fn clamp_depth_hard_caps_large_input() {
        assert_eq!(clamp_depth(i32::MAX), MAX_TREE_DEPTH);
    }

    #[test]
    fn clamp_max_results_uses_default_for_non_positive_input() {
        assert_eq!(clamp_max_results(0), DEFAULT_FIND_RESULTS);
        assert_eq!(clamp_max_results(-1), DEFAULT_FIND_RESULTS);
    }

    #[test]
    fn clamp_max_results_hard_caps_large_input() {
        assert_eq!(clamp_max_results(i32::MAX), MAX_FIND_RESULTS);
    }

    #[test]
    fn recognizes_stale_object_error_names() {
        assert!(is_stale_object_error_name(
            "org.freedesktop.DBus.Error.UnknownObject"
        ));
        assert!(is_stale_object_error_name(
            "org.freedesktop.DBus.Error.ServiceUnknown"
        ));
        assert!(!is_stale_object_error_name(
            "org.freedesktop.DBus.Error.AccessDenied"
        ));
    }

    /// The regression test for the bug `tests/accessibility.rs` found: reading
    /// a *property* off a gone application surfaces the bus error as
    /// [`zbus::Error::FDO`], not [`zbus::Error::MethodError`], and only the
    /// latter used to be translated. `GetElementInfo` starts with a property
    /// read, so it reported a generic D-Bus failure where `InvokeAction`
    /// correctly reported `ElementNotFound` for the identical fault.
    ///
    /// Worth having here rather than only in that suite: the suite needs a live
    /// GNOME session and can never run in CI, and this variant is exactly the
    /// sort of thing a `zbus` upgrade could change underneath us.
    #[test]
    fn a_property_read_against_a_gone_application_is_element_not_found() {
        let err = zbus::Error::FDO(Box::new(zbus::fdo::Error::ServiceUnknown(
            "The name :1.42 was not provided by any .service files".to_string(),
        )));

        match translate_element_error(err) {
            AccessibilityError::ElementNotFound(detail) => {
                assert!(
                    detail.contains(":1.42"),
                    "the detail must name what is gone"
                );
            }
            other => panic!("expected ElementNotFound, got {other:?}"),
        }
    }

    /// The other half of the same guard: an error that is *not* about a missing
    /// destination must still pass through as a D-Bus failure, so widening the
    /// match above cannot quietly start reporting unrelated faults as stale
    /// elements.
    #[test]
    fn an_unrelated_fdo_error_is_not_reported_as_a_missing_element() {
        let err = zbus::Error::FDO(Box::new(zbus::fdo::Error::AccessDenied(
            "not permitted".to_string(),
        )));

        assert!(matches!(
            translate_element_error(err),
            AccessibilityError::DBus(_)
        ));
    }

    /// The S3 fix, at the level that can be tested without a desktop.
    ///
    /// GTK4 refuses `GrabFocus` and `ScrollTo` with a `NotSupported` carrying
    /// **no description at all**, which used to reach the user as
    /// `org.freedesktop.DBus.Error.NotSupported:` — a fault name and a colon.
    /// The whole point of the translation is that the explanation comes from
    /// wgaf, so an empty description is the case worth pinning.
    #[test]
    fn a_description_less_not_supported_becomes_the_explanation_wgaf_supplies() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.NotSupported")
                .expect("a valid error name"),
            None,
            zbus::Message::method_call("/", "Whatever")
                .expect("a valid method call builder")
                .build(&())
                .expect("a valid method call"),
        );

        match translate_toolkit_refusal(err, "try `wgaf a11y click` instead".to_string()) {
            AccessibilityError::ActionNotSupported(explanation) => {
                assert_eq!(explanation, "try `wgaf a11y click` instead");
            }
            other => panic!("expected ActionNotSupported, got {other:?}"),
        }
    }

    /// The same guard the stale-element translator has: widening the match
    /// must not start relabelling unrelated faults as "this toolkit cannot do
    /// that". An `AccessDenied` is a real failure and must stay one.
    #[test]
    fn an_error_other_than_not_supported_is_not_reported_as_a_refusal() {
        let err = zbus::Error::FDO(Box::new(zbus::fdo::Error::AccessDenied(
            "not permitted".to_string(),
        )));

        assert!(matches!(
            translate_toolkit_refusal(err, "should not be used".to_string()),
            AccessibilityError::DBus(_)
        ));
    }

    /// A refusal must not swallow the stale-element case. If the application
    /// exited mid-call, "this toolkit does not support scrolling" would send
    /// the user looking for a toolkit bug that is not there.
    #[test]
    fn a_gone_application_is_still_element_not_found_when_a_refusal_was_possible() {
        let err = zbus::Error::FDO(Box::new(zbus::fdo::Error::ServiceUnknown(
            "The name :1.42 was not provided by any .service files".to_string(),
        )));

        assert!(matches!(
            translate_toolkit_refusal(err, "should not be used".to_string()),
            AccessibilityError::ElementNotFound(_)
        ));
    }

    #[test]
    fn recognizes_the_unsupported_error_name() {
        assert!(is_unsupported_error_name(
            "org.freedesktop.DBus.Error.NotSupported"
        ));
        assert!(!is_unsupported_error_name(
            "org.freedesktop.DBus.Error.UnknownMethod"
        ));
    }
}
