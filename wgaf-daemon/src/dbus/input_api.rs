//! The daemon's own public input-automation D-Bus API (`org.wgaf.Input1`).
//! Thin delegation to [`crate::input::InputBackend`] — this module's only
//! job is D-Bus marshaling and translating [`crate::input::InputError`]
//! into a stable, named D-Bus error (`InputApiError`), following the exact
//! pattern `windows_api.rs` established for `org.wgaf.Windows1`.

use std::sync::Arc;

use zbus::DBusError;
use zbus::interface;
use zbus::message::Header;

use crate::input::{InputBackend, InputError};
use crate::permissions::{Capability, PermissionError, PermissionGate};

/// D-Bus error names for `org.wgaf.Input1`, matching
/// `wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE`/`INPUT_ERROR_UNKNOWN_KEY`/
/// `INPUT_ERROR_INVALID_BUTTON`/`INPUT_ERROR_PERMISSION_DENIED` (asserted in
/// this module's tests).
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.wgaf.Input1.Error")]
enum InputApiError {
    /// Catch-all for D-Bus-level failures not otherwise translated below
    /// (includes `InputError::Io`, which doesn't warrant its own named D-Bus
    /// error — callers see it as a generic failure description, same as any
    /// other `zbus::Error::Failure`).
    #[zbus(error)]
    ZBus(zbus::Error),
    DeviceUnavailable(String),
    UnknownKey(String),
    InvalidButton(String),
    /// `TypeText` exceeded `config.toml`'s `input_max_type_text_chars`.
    ///
    /// Named rather than folded into [`Self::ZBus`] because the limit is
    /// configurable: a user who lowers it meets this as an ordinary outcome,
    /// not an exceptional one, and a script should be able to branch on it
    /// without string-matching the description.
    TextTooLong(String),
    /// A runaway caller flooded synthetic input past the point where
    /// throttling it was still the right answer — see
    /// `crate::input::rate_limit`. Merely exceeding the budget produces no
    /// error at all; the call is slowed instead.
    RateLimited(String),
    /// The call was refused by `permissions.toml`'s policy (or the
    /// caller declined an interactive `Prompt`) — see `crate::permissions`.
    PermissionDenied(String),
    /// `TypeText` was given a character the active keyboard layout has no key
    /// sequence for.
    ///
    /// Named rather than folded into [`Self::UnknownKey`] because it is a
    /// different question: `UnknownKey` means "there is no such key", this
    /// means "this layout cannot produce that character". A script that
    /// branches on it can substitute or skip; one that cannot tell them apart
    /// has to guess.
    CharacterNotTypeable(String),
    /// The kill switch is engaged — see `org.wgaf.Daemon1.Stop`.
    ///
    /// Named rather than folded into [`Self::PermissionDenied`] because the two
    /// call for different responses: a denial is permanent policy, this is a
    /// live emergency stop somebody can lift with `wgaf release`. A script that
    /// cannot tell them apart cannot know whether retrying later is sensible.
    Stopped(String),
    /// The session's keyboard layout could not be determined, so `TypeText`
    /// does not know what its keystrokes would produce. Environmental — no
    /// Wayland session, or no keyboard on any seat.
    KeyboardLayoutUnavailable(String),
}

impl From<InputError> for InputApiError {
    fn from(err: InputError) -> Self {
        match err {
            InputError::DeviceUnavailable { .. } => Self::DeviceUnavailable(err.to_string()),
            InputError::UnknownKey(_) => Self::UnknownKey(err.to_string()),
            InputError::InvalidButton(_) => Self::InvalidButton(err.to_string()),
            // An empty combination is a malformed request, same shape of
            // mistake as an unknown key name.
            InputError::EmptyHotkey => Self::UnknownKey(err.to_string()),
            InputError::RateLimited { .. } => Self::RateLimited(err.to_string()),
            InputError::TextTooLong { .. } => Self::TextTooLong(err.to_string()),
            InputError::CharacterNotTypeable { .. } => Self::CharacterNotTypeable(err.to_string()),
            InputError::Stopped => Self::Stopped(err.to_string()),
            // A misconfigured layout reaches the caller the same way an absent
            // one does: either way `wgaf type` cannot run, and the message
            // already says which it is.
            InputError::KeyboardLayoutUnavailable(_) | InputError::KeyboardLayoutInvalid(_) => {
                Self::KeyboardLayoutUnavailable(err.to_string())
            }
            InputError::Io(_) => Self::ZBus(zbus::Error::Failure(err.to_string())),
        }
    }
}

impl From<PermissionError> for InputApiError {
    fn from(err: PermissionError) -> Self {
        match err {
            PermissionError::Denied { .. } | PermissionError::DeniedByPrompt { .. } => {
                Self::PermissionDenied(err.to_string())
            }
            PermissionError::DBus(e) => Self::ZBus(e),
        }
    }
}

pub struct InputApi {
    backend: Arc<InputBackend>,
    permissions: Arc<PermissionGate>,
}

impl InputApi {
    pub fn new(backend: Arc<InputBackend>, permissions: Arc<PermissionGate>) -> Self {
        Self {
            backend,
            permissions,
        }
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
    async fn type_text(
        &self,
        text: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::TypeText, connection, &header)
            .await?;
        Ok(self.backend.type_text(text).await?)
    }

    /// Presses (holds down) one key, by evdev key name (`a`, `KEY_A`,
    /// `enter`, `leftshift`, ...) — see `crate::input::codes::key_name_to_code`.
    /// No ASCII/shift awareness: callers wanting a capital letter or a
    /// shifted symbol press/release `leftshift` themselves around the key.
    async fn key_press(
        &self,
        key: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyPress, connection, &header)
            .await?;
        Ok(self.backend.key_press(key).await?)
    }

    /// Presses a key combination: every key held down in order, then released
    /// in reverse.
    ///
    /// Gated by `KeyPress` rather than a capability of its own — it presses
    /// keys, and a caller allowed to press `ctrl` then `t` separately can
    /// already do everything this does. A new capability would only mean a
    /// policy that denied `Hotkey` while allowing `KeyPress` looked like it
    /// prevented something.
    async fn hotkey(
        &self,
        keys: Vec<String>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyPress, connection, &header)
            .await?;
        Ok(self.backend.hotkey(&keys).await?)
    }

    /// Releases a key previously pressed via `KeyPress`.
    async fn key_release(
        &self,
        key: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyRelease, connection, &header)
            .await?;
        Ok(self.backend.key_release(key).await?)
    }

    /// Moves the pointer by `(dx, dy)` relative to its current position.
    /// There is no absolute-move method — see `crate::input::mouse`'s
    /// module docs for why.
    async fn mouse_move(
        &self,
        dx: i32,
        dy: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::MouseMove, connection, &header)
            .await?;
        Ok(self.backend.mouse_move(dx, dy).await?)
    }

    /// Clicks (press then release) `button`, which must be `left`, `right`,
    /// or `middle`.
    async fn mouse_click(
        &self,
        button: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::MouseClick, connection, &header)
            .await?;
        Ok(self.backend.mouse_click(button).await?)
    }

    /// Scrolls: `dx` horizontal (`REL_HWHEEL`, positive = right), `dy`
    /// vertical (`REL_WHEEL`, positive = up).
    async fn mouse_scroll(
        &self,
        dx: i32,
        dy: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::MouseScroll, connection, &header)
            .await?;
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
        let permission_denied = InputApiError::PermissionDenied("denied".to_string());
        let rate_limited = InputApiError::RateLimited("limited".to_string());
        let text_too_long = InputApiError::TextTooLong("too long".to_string());
        let stopped = InputApiError::Stopped("stopped".to_string());
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
        assert_eq!(
            permission_denied.name().as_str(),
            wgaf_common::INPUT_ERROR_PERMISSION_DENIED
        );
        assert_eq!(
            rate_limited.name().as_str(),
            wgaf_common::INPUT_ERROR_RATE_LIMITED
        );
        assert_eq!(
            text_too_long.name().as_str(),
            wgaf_common::INPUT_ERROR_TEXT_TOO_LONG
        );
        assert_eq!(stopped.name().as_str(), wgaf_common::INPUT_ERROR_STOPPED);
    }

    /// Every `InputError` that is not a plain I/O failure must map to a
    /// **named** D-Bus error, not the `ZBus` catch-all.
    ///
    /// Guards the direction the per-variant assertions above cannot: adding an
    /// `InputError` variant and folding it into `ZBus` out of momentum. That
    /// is exactly how `TextTooLong` spent five phases as a generic failure
    /// while its siblings were named, and how `RateLimited` could have.
    #[test]
    fn every_non_io_input_error_maps_to_a_named_dbus_error() {
        let cases = [
            InputError::DeviceUnavailable {
                path: "/dev/uinput".to_string(),
                reason: "denied".to_string(),
            },
            InputError::UnknownKey("nope".to_string()),
            InputError::InvalidButton("nope".to_string()),
            InputError::TextTooLong { len: 10, max: 5 },
            InputError::RateLimited { seconds: 42.0 },
            InputError::Stopped,
        ];

        for err in cases {
            let rendered = err.to_string();
            let api_error = InputApiError::from(err);
            assert!(
                !matches!(api_error, InputApiError::ZBus(_)),
                "`{rendered}` fell through to the ZBus catch-all instead of \
                 getting a named D-Bus error"
            );
        }
    }
}
