//! Action invocation on a found accessible element: `InvokeAction` (via the
//! `org.a11y.atspi.Action` interface), `SetText` (via `EditableText`), and
//! `FocusElement` (via `Component.GrabFocus`).
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

use super::{AccessibilityError, accessible_proxy_for, translate_element_error};

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

    let ok = component
        .grab_focus()
        .await
        .map_err(translate_element_error)?;
    if !ok {
        return Err(AccessibilityError::ActionNotSupported(format!(
            "`{element}`'s GrabFocus reported failure"
        )));
    }
    Ok(())
}
