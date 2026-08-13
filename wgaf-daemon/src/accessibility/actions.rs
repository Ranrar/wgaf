//! Action invocation on a found accessible element: `InvokeAction` (via the
//! `org.a11y.atspi.Action` interface), `SetText` (via `EditableText`),
//! `GetElementText` (via `Text`), `FocusElement` (via `Component.GrabFocus`),
//! and `ScrollElement` (via `Component.ScrollTo`).
//!
//! Every function here first reads the element's `GetInterfaces()` and
//! checks the specific interface it needs is present, returning
//! [`AccessibilityError::ActionNotSupported`] with a specific, actionable
//! message if not — rather than letting the D-Bus call fail with an opaque
//! `UnknownInterface`/`UnknownMethod` error from a toolkit that simply
//! doesn't implement that interface on that widget (e.g. calling `SetText`
//! on a read-only label).

use atspi::Interface;
use wgaf_common::ElementRef;

use super::{
    AccessibilityError, accessible_proxy_for, translate_element_error, translate_toolkit_refusal,
};

/// Performs one accessible action on `element`.
///
/// `action_name` selects which of the element's actions to invoke, matched
/// case-insensitively against each action's machine-readable name (AT-SPI's
/// `Action.GetName`) — e.g. `"click"`, `"press"`, `"activate"`, whichever the
/// toolkit exposes for this widget. An empty `action_name` invokes the
/// *default* action (index 0) — AT-SPI's own convention (see
/// `atspi::proxy::action::ActionProxy::get_actions`'s docs: "if there is more
/// than one action available, the first one is considered the 'default'
/// action of the object"), which is what `wgaf a11y click` uses.
pub(crate) async fn invoke_action(
    conn: &zbus::Connection,
    element: &ElementRef,
    action_name: &str,
) -> Result<(), AccessibilityError> {
    let accessible = accessible_proxy_for(conn, element).await?;
    let interfaces = accessible
        .get_interfaces()
        .await
        .map_err(translate_element_error)?;
    if !interfaces.contains(Interface::Action) {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` does not implement org.a11y.atspi.Action — click/press/activate is not \
             available on this element"
        )));
    }

    let action = atspi::proxy::action::ActionProxy::builder(conn)
        .destination(element.bus_name.as_str())?
        .path(element.object_path.as_str())?
        .build()
        .await?;

    let count = action.n_actions().await.map_err(translate_element_error)?;
    if count <= 0 {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` implements Action but exposes zero actions"
        )));
    }

    let index = if action_name.is_empty() {
        0
    } else {
        let mut resolved = None;
        for i in 0..count {
            let name = action.get_name(i).await.map_err(translate_element_error)?;
            if name.eq_ignore_ascii_case(action_name) {
                resolved = Some(i);
                break;
            }
        }
        resolved.ok_or_else(|| {
            AccessibilityError::ActionNotSupported(format!(
                "`{element}` has no action named `{action_name}`"
            ))
        })?
    };

    let performed = action
        .do_action(index)
        .await
        .map_err(translate_element_error)?;
    if !performed {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}`'s action at index {index} reported failure (DoAction returned false)"
        )));
    }
    Ok(())
}

/// Replaces `element`'s text content via `EditableText.SetTextContents`.
pub(crate) async fn set_text(
    conn: &zbus::Connection,
    element: &ElementRef,
    text: &str,
) -> Result<(), AccessibilityError> {
    let accessible = accessible_proxy_for(conn, element).await?;
    let interfaces = accessible
        .get_interfaces()
        .await
        .map_err(translate_element_error)?;
    if !interfaces.contains(Interface::EditableText) {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` does not implement org.a11y.atspi.EditableText — it is not a text \
             field this daemon can set text on"
        )));
    }

    let editable = atspi::proxy::editable_text::EditableTextProxy::builder(conn)
        .destination(element.bus_name.as_str())?
        .path(element.object_path.as_str())?
        .build()
        .await?;

    let ok = editable
        .set_text_contents(text)
        .await
        .map_err(translate_element_error)?;
    if !ok {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}`'s SetTextContents reported failure"
        )));
    }
    Ok(())
}

/// Reads `element`'s text through `org.a11y.atspi.Text`.
///
/// # Why this is the counterpart of [`set_text`] and not its mirror
///
/// `set_text` needs `EditableText`; this needs only `Text`, and the difference
/// is the point. **Measured against GTK 4.22 on 2026-08-11:** a static label
/// implements `Text` and not `EditableText`, so this reads widgets nothing can
/// write — which is most of what is on a screen. Requiring `EditableText` here
/// would have made it useful only for the widgets a script had just filled in.
///
/// # Ungated, per the read-only rule
///
/// It observes rather than changes, like `FindElements` and `GetTree`. See
/// `permissions::policy`'s module docs for why read-only methods have no
/// `Capability` variant at all rather than one defaulting to `Allow`.
///
/// # `CharacterCount` first, then `GetText`
///
/// The length is a property and the text is a method taking `(start, end)`.
/// AT-SPI documents `-1` as "to the end", and this asks for the count instead
/// and passes it — **the sentinel is deliberately not relied on**, because it
/// was the one part of the interface that could not be exercised while probing
/// (see W18.5's measurement table), and this project has been caught by an
/// unverified sentinel before: `get_layout_columns()` returns `-1` and means
/// something else entirely.
///
/// The two calls are not atomic, so a widget edited between them yields the
/// prefix of the new text rather than the old text — a torn read, not a wrong
/// one. Left alone deliberately: AT-SPI offers no atomic "read it all", and the
/// caller that cares is reading a widget nothing else is typing into.
pub(crate) async fn get_text(
    conn: &zbus::Connection,
    element: &ElementRef,
) -> Result<String, AccessibilityError> {
    let accessible = accessible_proxy_for(conn, element).await?;
    let interfaces = accessible
        .get_interfaces()
        .await
        .map_err(translate_element_error)?;
    if !interfaces.contains(Interface::Text) {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` does not implement org.a11y.atspi.Text — it is not an element this \
             daemon can read text from. `wgaf a11y info` reports an element's name, which is \
             what most widgets carry instead"
        )));
    }

    let text = atspi::proxy::text::TextProxy::builder(conn)
        .destination(element.bus_name.as_str())?
        .path(element.object_path.as_str())?
        .build()
        .await?;

    let count = text
        .character_count()
        .await
        .map_err(translate_element_error)?;
    if count <= 0 {
        // An empty widget is an ordinary answer, and asking for `(0, 0)` would
        // be a round trip to be told so.
        return Ok(String::new());
    }

    text.get_text(0, count)
        .await
        .map_err(translate_element_error)
}

/// Requests keyboard focus for `element` via `Component.GrabFocus`.
pub(crate) async fn focus(
    conn: &zbus::Connection,
    element: &ElementRef,
) -> Result<(), AccessibilityError> {
    let accessible = accessible_proxy_for(conn, element).await?;
    let interfaces = accessible
        .get_interfaces()
        .await
        .map_err(translate_element_error)?;
    if !interfaces.contains(Interface::Component) {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` does not implement org.a11y.atspi.Component — it cannot be given \
             keyboard focus directly"
        )));
    }

    let component = atspi::proxy::component::ComponentProxy::builder(conn)
        .destination(element.bus_name.as_str())?
        .path(element.object_path.as_str())?
        .build()
        .await?;

    let ok = component.grab_focus().await.map_err(|err| {
        translate_toolkit_refusal(
            err,
            format!(
                "`{element}` implements org.a11y.atspi.Component but its toolkit refused \
                 GrabFocus — GTK4 does not implement focus grabbing over AT-SPI for any widget, \
                 so this cannot succeed against a GTK application. Use `wgaf a11y click`, which \
                 activates the element directly and does work"
            ),
        )
    })?;
    if !ok {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}`'s GrabFocus reported failure"
        )));
    }
    Ok(())
}

/// Scrolls `element` into view via `Component.ScrollTo`.
///
/// # It succeeds on fewer toolkits than it looks like it should
///
/// **Measured on 2026-08-12 against GTK 4.22.4 and Firefox.** GTK4 advertises
/// `Component` on the widget, lists `ScrollTo` in its introspection XML, and
/// then refuses every call with `NotSupported` — the same way it refuses
/// [`focus`], and for the same reason. Firefox implements it and it works. The
/// interface check below therefore cannot predict the outcome; it only rules
/// out the elements that could never have worked, and
/// [`translate_toolkit_refusal`] handles the rest.
///
/// # Why an off-screen element is still worth reaching, given `a11y click` works anyway
///
/// The same measurement found that a widget scrolled far out of view answers
/// `Action.DoAction` and `Text.GetText` perfectly well — AT-SPI dispatches to
/// the widget, not to a pixel, so **wgaf never needed to scroll in order to
/// click**. What scrolling is for is the cases where something outside AT-SPI
/// has to see the element: a human watching the automation, a screenshot, or
/// pointer input from `wgaf mouse`, which does address pixels.
///
/// # `ScrollType::Anywhere`, and no choice of it
///
/// AT-SPI's `Anywhere` means "scroll as little as needed to put this on
/// screen", which is the whole intent here. The other six variants pin the
/// element to a named edge or corner, which is a layout preference no caller
/// has asked for; exposing the enum would widen the command's surface for no
/// use that exists. All seven were measured — GTK4 refuses every one
/// identically, so nothing is being hidden by the omission.
pub(crate) async fn scroll_to(
    conn: &zbus::Connection,
    element: &ElementRef,
) -> Result<(), AccessibilityError> {
    let accessible = accessible_proxy_for(conn, element).await?;
    let interfaces = accessible
        .get_interfaces()
        .await
        .map_err(translate_element_error)?;
    if !interfaces.contains(Interface::Component) {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}` does not implement org.a11y.atspi.Component — it has no position on \
             screen and cannot be scrolled into view"
        )));
    }

    let component = atspi::proxy::component::ComponentProxy::builder(conn)
        .destination(element.bus_name.as_str())?
        .path(element.object_path.as_str())?
        .build()
        .await?;

    // `atspi-proxies` types `ScrollTo` as returning `bool`, which matches
    // Firefox's introspection. **GTK4 declares it returning nothing at all** —
    // measured, and a real divergence in the interface rather than a quirk of
    // one build. It is latent rather than live only because GTK4 fails the call
    // before a reply body would be decoded; a toolkit that implements it as
    // void *and succeeds* would fail here with a signature mismatch rather than
    // a refusal. Worth knowing before trusting this against a third toolkit.
    let ok = component
        .scroll_to(atspi::ScrollType::Anywhere)
        .await
        .map_err(|err| {
            translate_toolkit_refusal(
                err,
                format!(
                    "`{element}`'s toolkit refused ScrollTo — GTK4 does not implement scrolling \
                     over AT-SPI, so this cannot succeed against a GTK application. Note that an \
                     off-screen element can still be read and activated: `wgaf a11y click` and \
                     `wgaf a11y text` do not need it to be visible"
                ),
            )
        })?;
    if !ok {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}`'s ScrollTo reported failure"
        )));
    }
    Ok(())
}
