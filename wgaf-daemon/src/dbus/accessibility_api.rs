//! The daemon's own public accessibility-automation D-Bus API
//! (`org.wgaf.Accessibility1`). Thin delegation to
//! [`crate::accessibility::AccessibilityBackend`] — this module's only job
//! is D-Bus marshaling and translating
//! [`crate::accessibility::AccessibilityError`] into a stable, named D-Bus
//! error (`AccessibilityApiError`), following the exact pattern
//! `windows_api.rs`/`input_api.rs` established.

use std::sync::Arc;

use wgaf_common::{AppRecord, ElementRecord, ElementRef, TreeNode};
use zbus::DBusError;
use zbus::interface;
use zbus::message::Header;

use crate::accessibility::{AccessibilityBackend, AccessibilityError};
use crate::permissions::{Capability, PermissionError, PermissionGate};

/// D-Bus error names for `org.wgaf.Accessibility1`, matching
/// `wgaf_common::ACCESSIBILITY_ERROR_*` (asserted in this module's tests).
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.wgaf.Accessibility1.Error")]
enum AccessibilityApiError {
    /// Catch-all for D-Bus-level failures not otherwise translated below
    /// (includes `AccessibilityError::Atspi`/`AccessibilityError::DBus`,
    /// which don't warrant their own named D-Bus error — callers see them
    /// as a generic failure description, same as `InputApiError`'s catch-all
    /// handles `TextTooLong`/`Io`).
    #[zbus(error)]
    ZBus(zbus::Error),
    BusUnavailable(String),
    AppNotFound(String),
    ElementNotFound(String),
    InvalidElementRef(String),
    ActionNotSupported(String),
    /// The call was refused by `permissions.toml`'s policy (or the
    /// caller declined an interactive `Prompt`) — see `crate::permissions`.
    PermissionDenied(String),
}

impl From<AccessibilityError> for AccessibilityApiError {
    fn from(err: AccessibilityError) -> Self {
        match err {
            AccessibilityError::BusUnavailable { .. } => Self::BusUnavailable(err.to_string()),
            AccessibilityError::AppNotFound(_) => Self::AppNotFound(err.to_string()),
            AccessibilityError::ElementNotFound(_) => Self::ElementNotFound(err.to_string()),
            AccessibilityError::InvalidElementRef { .. } => {
                Self::InvalidElementRef(err.to_string())
            }
            AccessibilityError::ActionNotSupported(_) => Self::ActionNotSupported(err.to_string()),
            AccessibilityError::Atspi(_) | AccessibilityError::DBus(_) => {
                Self::ZBus(zbus::Error::Failure(err.to_string()))
            }
        }
    }
}

impl From<PermissionError> for AccessibilityApiError {
    fn from(err: PermissionError) -> Self {
        match err {
            PermissionError::Denied { .. } | PermissionError::DeniedByPrompt { .. } => {
                Self::PermissionDenied(err.to_string())
            }
            PermissionError::DBus(e) => Self::ZBus(e),
        }
    }
}

pub struct AccessibilityApi {
    backend: Arc<AccessibilityBackend>,
    permissions: Arc<PermissionGate>,
}

impl AccessibilityApi {
    pub fn new(backend: Arc<AccessibilityBackend>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            backend,
            permissions,
        }
    }
}

// Interface name must match `wgaf_common::ACCESSIBILITY_INTERFACE_NAME` (zbus
// requires a string literal here, so it can't reference the constant
// directly) — see the existing convention in `dbus/mod.rs`/`windows_api.rs`/
// `input_api.rs`.
#[interface(name = "org.wgaf.Accessibility1")]
impl AccessibilityApi {
    /// Enumerates every currently-registered accessible application.
    async fn list_apps(&self) -> Result<Vec<AppRecord>, AccessibilityApiError> {
        Ok(self.backend.list_apps().await?)
    }

    /// Finds elements within application `app` (matched by name — see
    /// `crate::accessibility::query::find_matching_app`) whose role/name/
    /// description satisfy the given filters. Any of `role`/`name`/
    /// `description` may be empty to mean "match anything". `max_results
    /// <= 0` uses a built-in default (100); values are hard-capped at 1000
    /// regardless.
    async fn find_elements(
        &self,
        app: &str,
        role: &str,
        name: &str,
        description: &str,
        max_results: i32,
    ) -> Result<Vec<ElementRecord>, AccessibilityApiError> {
        Ok(self
            .backend
            .find_elements(app, role, name, description, max_results)
            .await?)
    }

    /// Walks the accessible tree of the application matching `app`, up to
    /// `max_depth` levels deep (`<= 0` uses a built-in default of 10; values
    /// are hard-capped at 64 regardless), returned as a flat, depth-first
    /// list (see [`TreeNode::depth`]).
    async fn get_tree(
        &self,
        app: &str,
        max_depth: i32,
    ) -> Result<Vec<TreeNode>, AccessibilityApiError> {
        Ok(self.backend.get_tree(app, max_depth).await?)
    }

    /// Re-reads a single element's info directly from its [`ElementRef`].
    async fn get_element_info(
        &self,
        element: ElementRef,
    ) -> Result<ElementRecord, AccessibilityApiError> {
        Ok(self.backend.get_element_info(&element).await?)
    }

    /// Invokes an accessible action on `element`. `action_name` selects
    /// which action by its machine-readable name (case-insensitive); empty
    /// invokes the element's default action (index 0) — see
    /// `crate::accessibility::actions::invoke_action`'s docs.
    async fn invoke_action(
        &self,
        element: ElementRef,
        action_name: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), AccessibilityApiError> {
        self.permissions
            .check(Capability::InvokeAction, connection, &header)
            .await?;
        Ok(self.backend.invoke_action(&element, action_name).await?)
    }

    /// Replaces `element`'s text content (requires the AT-SPI
    /// `EditableText` interface).
    async fn set_text(
        &self,
        element: ElementRef,
        text: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), AccessibilityApiError> {
        self.permissions
            .check(Capability::SetText, connection, &header)
            .await?;
        Ok(self.backend.set_text(&element, text).await?)
    }

    /// Reads `element`'s text content (requires the AT-SPI `Text` interface).
    ///
    /// **Ungated**, like every other read-only method on this interface: it
    /// observes the desktop rather than changing it. See `permissions::policy`
    /// for why those have no `Capability` variant at all.
    ///
    /// Works on more than text fields — a static label implements `Text` too,
    /// so this reads what is *displayed*, not only what can be typed into.
    async fn get_element_text(&self, element: ElementRef) -> Result<String, AccessibilityApiError> {
        Ok(self.backend.get_text(&element).await?)
    }

    /// Requests keyboard focus for `element` (requires the AT-SPI
    /// `Component` interface).
    async fn focus_element(
        &self,
        element: ElementRef,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), AccessibilityApiError> {
        self.permissions
            .check(Capability::FocusElement, connection, &header)
            .await?;
        Ok(self.backend.focus_element(&element).await?)
    }

    /// Scrolls `element` into view (requires the AT-SPI `Component`
    /// interface, and a toolkit that implements `ScrollTo` — GTK4 does not).
    ///
    /// Gated: it changes what the user is looking at. See
    /// `Capability::ScrollElement` for why it is not folded into
    /// `FocusElement` despite sharing an interface.
    async fn scroll_element(
        &self,
        element: ElementRef,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), AccessibilityApiError> {
        self.permissions
            .check(Capability::ScrollElement, connection, &header)
            .await?;
        Ok(self.backend.scroll_element(&element).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_prefix_matches_wgaf_common_constants() {
        let bus_unavailable = AccessibilityApiError::BusUnavailable("unavailable".to_string());
        let app_not_found = AccessibilityApiError::AppNotFound("not found".to_string());
        let element_not_found = AccessibilityApiError::ElementNotFound("gone".to_string());
        let invalid_element_ref = AccessibilityApiError::InvalidElementRef("malformed".to_string());
        let action_not_supported =
            AccessibilityApiError::ActionNotSupported("unsupported".to_string());
        let permission_denied = AccessibilityApiError::PermissionDenied("denied".to_string());
        assert_eq!(
            bus_unavailable.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE
        );
        assert_eq!(
            app_not_found.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND
        );
        assert_eq!(
            element_not_found.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND
        );
        assert_eq!(
            invalid_element_ref.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_INVALID_ELEMENT_REF
        );
        assert_eq!(
            action_not_supported.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED
        );
        assert_eq!(
            permission_denied.name().as_str(),
            wgaf_common::ACCESSIBILITY_ERROR_PERMISSION_DENIED
        );
    }
}
