//! The daemon's own public input-automation D-Bus API (`org.wgaf.Input1`).
//! Thin delegation to [`crate::input::InputBackend`] — this module's only
//! job is D-Bus marshaling and translating [`crate::input::InputError`]
//! into a stable, named D-Bus error (`InputApiError`), following the exact
//! pattern `windows_api.rs` established for `org.wgaf.Windows1`.

use zbus::DBusError;
use zbus::interface;

use crate::input::{InputBackend, InputError};

/// D-Bus error names for `org.wgaf.Input1`, matching
/// `wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE`/`INPUT_ERROR_UNKNOWN_KEY`/
/// `INPUT_ERROR_INVALID_BUTTON` (asserted in this module's tests).
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.wgaf.Input1.Error")]
enum InputApiError {
    /// Catch-all for D-Bus-level failures not otherwise translated below
    /// (includes `InputError::TextTooLong`/`InputError::Io`, which don't
    /// warrant their own named D-Bus error — callers see them as a generic
    /// failure description, same as any other `zbus::Error::Failure`).
    #[zbus(error)]
    ZBus(zbus::Error),
    DeviceUnavailable(String),
    UnknownKey(String),
    InvalidButton(String),
}

impl From<InputError> for InputApiError {
    fn from(err: InputError) -> Self {
        match err {
            InputError::DeviceUnavailable { .. } => Self::DeviceUnavailable(err.to_string()),
            InputError::UnknownKey(_) => Self::UnknownKey(err.to_string()),
            InputError::InvalidButton(_) => Self::InvalidButton(err.to_string()),
            InputError::TextTooLong { .. } | InputError::Io(_) => {
                Self::ZBus(zbus::Error::Failure(err.to_string()))
            }
        }
    }
}

pub struct InputApi {
    backend: InputBackend,
}

impl InputApi {
    pub fn new(backend: InputBackend) -> Self {
        Self { backend }
    }
}

// Interface name must match `wgaf_common::INPUT_INTERFACE_NAME` (zbus
// requires a string literal here, so it can't reference the constant
// directly) — see the existing convention in `dbus/mod.rs`/`windows_api.rs`.
#[interface(name = "org.wgaf.Input1")]
impl InputApi {
    /// Types `text`, character by character, using a US-QWERTY ASCII
    /// mapping (see `crate::input::codes`). Non-ASCII characters fail the
    /// whole call with `UnknownKey`.
    async fn type_text(&self, text: &str) -> Result<(), InputApiError> {
        Ok(self.backend.type_text(text).await?)
    }

    /// Presses (holds down) one key, by evdev key name (`a`, `KEY_A`,
    /// `enter`, `leftshift`, ...) — see `crate::input::codes::key_name_to_code`.
    /// No ASCII/shift awareness: callers wanting a capital letter or a
    /// shifted symbol press/release `leftshift` themselves around the key.
    async fn key_press(&self, key: &str) -> Result<(), InputApiError> {
        Ok(self.backend.key_press(key).await?)
    }

    /// Releases a key previously pressed via `KeyPress`.
    async fn key_release(&self, key: &str) -> Result<(), InputApiError> {
        Ok(self.backend.key_release(key).await?)
    }

    /// Moves the pointer by `(dx, dy)` relative to its current position.
    /// There is no absolute-move method — see `crate::input::mouse`'s
    /// module docs for why.
    async fn mouse_move(&self, dx: i32, dy: i32) -> Result<(), InputApiError> {
        Ok(self.backend.mouse_move(dx, dy).await?)
    }

    /// Clicks (press then release) `button`, which must be `left`, `right`,
    /// or `middle`.
    async fn mouse_click(&self, button: &str) -> Result<(), InputApiError> {
        Ok(self.backend.mouse_click(button).await?)
    }

    /// Scrolls: `dx` horizontal (`REL_HWHEEL`, positive = right), `dy`
    /// vertical (`REL_WHEEL`, positive = up).
    async fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<(), InputApiError> {
        Ok(self.backend.mouse_scroll(dx, dy).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_prefix_matches_wgaf_common_constants() {
        let device_unavailable = InputApiError::DeviceUnavailable("unavailable".to_string());
        let unknown_key = InputApiError::UnknownKey("unknown".to_string());
        let invalid_button = InputApiError::InvalidButton("invalid".to_string());
        assert_eq!(
            device_unavailable.name().as_str(),
            wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE
        );
        assert_eq!(
            unknown_key.name().as_str(),
            wgaf_common::INPUT_ERROR_UNKNOWN_KEY
        );
        assert_eq!(
            invalid_button.name().as_str(),
            wgaf_common::INPUT_ERROR_INVALID_BUTTON
        );
    }
}
