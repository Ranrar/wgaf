//! Lazy connection to the AT-SPI accessibility bus (`org.a11y.Bus`) — a
//! distinct D-Bus bus from the session bus, with its address discovered via
//! `org.a11y.Bus.GetAddress` on the session bus. That indirection is not
//! hand-rewritten here: `atspi::AccessibilityConnection::new()` (from the
//! `atspi-connection` crate, re-exported as `atspi::connection`/
//! `atspi::AccessibilityConnection`) already performs it.

use super::AccessibilityError;

/// Connects to the AT-SPI bus, translating a failure into
/// [`AccessibilityError::BusUnavailable`] with a clear, actionable message —
/// never a raw `AtspiError` debug dump. Called only from
/// `AccessibilityBackend::connection`'s `OnceCell::get_or_try_init`, so this
/// only actually runs on first use, and again on every subsequent call if it
/// failed (nothing here is cached on failure — see `mod.rs`).
pub(crate) async fn connect() -> Result<atspi::AccessibilityConnection, AccessibilityError> {
    atspi::AccessibilityConnection::new()
        .await
        .map_err(|e| AccessibilityError::BusUnavailable {
            reason: e.to_string(),
        })
}
