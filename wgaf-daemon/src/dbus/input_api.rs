//! The daemon's own public input-automation D-Bus API (`org.wgaf.Input1`).
//! Thin delegation to [`crate::input::InputBackend`] — this module's only
//! job is D-Bus marshaling and translating [`crate::input::InputError`]
//! into a stable, named D-Bus error (`InputApiError`), following the exact
//! pattern `windows_api.rs` established for `org.wgaf.Windows1`.

use std::sync::Arc;

use zbus::DBusError;
use zbus::interface;
use zbus::message::Header;

use wgaf_common::WindowRecord;

use crate::input::{InputBackend, InputError};
use crate::permissions::{Capability, Outcome, PermissionError, PermissionGate, VerifiedTarget};
use crate::verification::VerificationLevel;
use crate::windows::{DEFAULT_FOCUS_CONFIRM_TIMEOUT, FocusOutcome, WindowManager, WindowsError};

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
    /// `MouseMoveAbsolute` was given a coordinate that is not on any monitor.
    ///
    /// Named rather than folded into [`Self::ZBus`] because it is an ordinary
    /// outcome a script should branch on, not a fault: a layout with two
    /// monitors of different heights has coordinates inside its bounding box
    /// that are on no screen, and a caller computing a position can land on one
    /// without doing anything wrong. The description names the monitors, so the
    /// gap is visible rather than merely asserted.
    OutOfBounds(String),
    /// The monitor layout could not be read, so no coordinate can be checked
    /// and no absolute move can be made safely.
    ///
    /// Separate from [`Self::DeviceUnavailable`]: `uinput` is fine, the
    /// compositor's display configuration is what is missing, and telling a
    /// user to check their `uinput` permissions would send them the wrong way.
    MonitorLayoutUnavailable(String),
    /// A targeted method (`TypeTextAt`/`KeyPressAt`/`KeyReleaseAt`/
    /// `HotkeyAt`) named a window id that does not correspond to any
    /// currently open window — see [`InputApi::verify_target`].
    ///
    /// Named rather than folded into [`Self::ZBus`] for the same reason
    /// `WindowsApiError::WindowNotFound` is on `org.wgaf.Windows1`: a caller
    /// resolving its own target (e.g. from a prior `ListWindows`) should be
    /// able to branch on "that window is gone" without string-matching a
    /// description.
    WindowNotFound(String),
    /// A targeted method's window could not be confirmed focused before the
    /// timeout, after the daemon attempted to correct it — see
    /// [`InputApi::verify_target`] and
    /// `crate::windows::WindowManager::ensure_focused`'s
    /// `FocusOutcome::TimedOut`.
    ///
    /// **Not [`Self::PermissionDenied`]**: no policy was consulted for this
    /// outcome, and nothing was refused. **Not [`Self::ZBus`]** either:
    /// nothing malfunctioned — most often this is the compositor's own
    /// focus-stealing prevention declining the request. Per ADR-0007 this is
    /// a third outcome, a verification failure: permitted and attempted, and
    /// the world was not what the caller assumed.
    VerificationFailed(String),
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

/// Absolute pointer positioning is served by [`WindowManager`], so its errors
/// have to reach `org.wgaf.Input1`'s callers too — see the note on
/// [`InputApi`]'s two handles. [`InputApi::verify_target`] is the other
/// consumer: it resolves a window id through [`WindowManager`] as well, which
/// is what makes [`WindowsError::WindowNotFound`] reachable here now — it
/// used to be impossible, since no pointer method took a window id.
impl From<WindowsError> for InputApiError {
    fn from(err: WindowsError) -> Self {
        match err {
            WindowsError::OutOfBounds { .. } => Self::OutOfBounds(err.to_string()),
            WindowsError::DisplayConfig(_) => Self::MonitorLayoutUnavailable(err.to_string()),
            // The pointer lives in the Shell extension, so "the extension is
            // missing" is exactly as fatal to an absolute move as a missing
            // `uinput` device is to a relative one, and it reads the same way
            // to a caller: the mechanism this call needs is not there.
            WindowsError::ExtensionUnavailable { .. } => Self::DeviceUnavailable(err.to_string()),
            WindowsError::DBus(e) => Self::ZBus(e),
            // Reachable via `verify_target` (an id absent from `ListWindows`,
            // or one that closed between its initial lookup and
            // `ensure_focused`'s confirmation re-fetch) — mirrors
            // `WindowsApiError`'s own `WindowNotFound` message verbatim.
            WindowsError::WindowNotFound(id) => {
                Self::WindowNotFound(format!("window {id} not found"))
            }
        }
    }
}

/// How many characters [`InputApi::type_text_at`] types before re-verifying
/// `window`'s focus, when `verification_level` requires re-checking mid-burst
/// at all.
///
/// **An engineering estimate, not a measured one** — the same honesty
/// `WindowManager::DEFAULT_FOCUS_CONFIRM_TIMEOUT`'s doc comment already
/// applies to the other half of this mechanism. Chosen for two competing
/// reasons:
///
/// - **Small enough that a focus change is still caught quickly.** If focus
///   moves away mid-burst, at most this many characters land in the wrong
///   window before the next check stops the rest — "a stray keystroke or
///   two" stretched to a stray chunk, not the whole paste.
/// - **Large enough that a maximum-length call doesn't flood the extension.**
///   Each re-check is a real D-Bus round trip (`verify_target` calls
///   `ListWindows`), not a free comparison. At this size, a call at
///   `DEFAULT_MAX_TYPE_TEXT_CHARS` (4,096 characters) re-verifies on the
///   order of a hundred times, not thousands — turning every character into
///   its own round trip would be its own kind of flood.
const TYPE_TEXT_AT_CHUNK_CHARS: usize = 32;

/// The [`InputApiError::VerificationFailed`] wording for a mid-burst
/// `TypeTextAt` re-check that failed after `typed` of `total` characters had
/// already been typed.
///
/// **Deliberately distinct wording from [`InputApi::verify_target`]'s own
/// default message**, which says "nothing was synthesized" — true for the
/// first, pre-typing check this method also performs, and simply false here:
/// something was typed, and this says how much. Per ADR-0007,
/// a verification failure is neither a malfunction nor a refusal, so this
/// states the fact — how far typing got before the target could no longer be
/// confirmed focused — without implying either.
fn partial_progress_message(window: u32, app_id: &str, typed: usize, total: usize) -> String {
    format!(
        "window {window} (`{app_id}`) could not be reconfirmed focused partway through \
         `TypeTextAt` — {typed} of {total} characters were already typed; the remaining {} \
         were not",
        total - typed,
    )
}

/// Decides what [`InputApi::type_text_at`]'s chunk loop reports when a
/// mid-burst re-check of [`InputApi::verify_target`] fails with `err`.
///
/// **Only a genuine [`InputApiError::VerificationFailed`] gets its message
/// rewritten** to [`partial_progress_message`]'s wording. Anything else —
/// [`InputApiError::WindowNotFound`] (the window closed between chunks, a
/// real possibility during a long paste), [`InputApiError::PermissionDenied`]
/// (`FocusWindow` policy changed mid-run), or a raw
/// [`InputApiError::ZBus`] — is passed through unchanged.
///
/// This is not a cosmetic choice. Per ADR-0007
/// and W16's error/denial/verification-failure classification
/// (`plan-first-release.md` §16, which names `WindowNotFound` a plain
/// error), a window that has actually closed is not "the world moved and a
/// re-check couldn't confirm it" — it is gone, an ordinary error, and
/// relabeling it as a verification failure would both say something false
/// ("could not be reconfirmed focused" implies a focus problem, not a
/// missing window) and — once W16 wires up exit codes by matching on this
/// enum — misclassify the outcome the caller sees.
///
/// Pulled out as its own pure function (no backend, no device, no D-Bus) so
/// this distinction is asserted directly — see this module's tests — without
/// needing to drive `type_text_at`'s real chunk loop with more than a single
/// synthetic error.
fn chunk_verification_outcome(
    err: InputApiError,
    window: u32,
    app_id: &str,
    typed: usize,
    total: usize,
) -> InputApiError {
    match err {
        InputApiError::VerificationFailed(_) => InputApiError::VerificationFailed(
            partial_progress_message(window, app_id, typed, total),
        ),
        other => other,
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

/// Formats a window id as the `target=` value [`InputApi::verify_target`]'s
/// audit line carries — see plan-first-release.md's "Action verification"
/// W17.4 subsection for the exact `window:{id}` shape this must match.
fn window_target(id: u32) -> String {
    format!("window:{id}")
}

/// Maps [`WindowManager::ensure_focused`]'s outcome to the `focused`/
/// [`Outcome`] pair [`InputApi::verify_target`]'s audit line carries for its
/// post-correction check.
///
/// A pure mapping, pulled out of `verify_target` for direct unit testing
/// (see this module's tests) — driving `verify_target` far enough to reach
/// this branch needs a live stub extension connection, same as every other
/// test in this module that exercises it.
fn verification_outcome_for_focus(outcome: FocusOutcome) -> (bool, Outcome) {
    match outcome {
        FocusOutcome::AlreadyFocused | FocusOutcome::Corrected => (true, Outcome::Allowed),
        FocusOutcome::TimedOut => (false, Outcome::VerificationFailed),
    }
}

/// Serves `org.wgaf.Input1`.
///
/// **Two backends, deliberately.** Every other interface in this daemon
/// delegates to exactly one subsystem, and this one does not: relative input
/// (`TypeText`, `KeyPress`, `MouseMove`, …) goes to [`InputBackend`] and its
/// `uinput` device, while absolute pointer positioning
/// (`MouseMoveAbsolute`, `GetPointerPosition`) goes to [`WindowManager`] and
/// through it to the GNOME Shell extension.
///
/// The split is real and not an accident of layering. `uinput` can only express
/// *relative* motion, which libinput then accelerates, so it cannot put the
/// pointer on a chosen pixel at all; absolute positioning is a geometry problem
/// and the compositor is the only thing that knows the geometry. Grouping them
/// here is a decision about the **user's** model — "moving the mouse" is one
/// idea and belongs on one interface — taken knowing it costs this struct its
/// single-subsystem tidiness.
///
/// The practical consequence to remember: these two paths fail differently.
/// A relative move fails when `/dev/uinput` is unavailable; an absolute one
/// fails when the extension or Mutter's display configuration is.
///
/// `windows` has a second, independent reason to live here beyond absolute
/// pointer positioning: [`Self::verify_target`], the pre-condition check
/// behind `TypeTextAt`/`KeyPressAt`/`KeyReleaseAt`/`HotkeyAt` below, needs it
/// to resolve and (if necessary) correct a target window's focus — and needs
/// `permissions` alongside it, since a focus correction is gated by its own
/// capability. Neither `WindowManager` nor `InputBackend` gains permission
/// awareness for this; see `crate::permissions`'s module doc for why that
/// boundary stays in this D-Bus layer instead.
pub struct InputApi {
    backend: Arc<InputBackend>,
    windows: Arc<WindowManager>,
    permissions: Arc<PermissionGate>,
    /// How much pre-condition checking the targeted `*At` methods perform
    /// before running — see `crate::verification::VerificationLevel` and
    /// [`Self::verify_target`]. Read from `Config::verification_level` at
    /// daemon startup; fixed for the daemon's lifetime like every other
    /// config-derived value on this struct.
    verification_level: VerificationLevel,
}

impl InputApi {
    pub fn new(
        backend: Arc<InputBackend>,
        windows: Arc<WindowManager>,
        permissions: Arc<PermissionGate>,
        verification_level: VerificationLevel,
    ) -> Self {
        Self {
            backend,
            windows,
            permissions,
            verification_level,
        }
    }

    /// Confirms `window` exists and currently has keyboard focus before a
    /// targeted input call is allowed to proceed, correcting focus first
    /// (rather than merely refusing) if it does not — the pre-condition half
    /// of action verification, and the mechanism that closes the general case
    /// of synthesized input landing in whatever window happens to have focus.
    ///
    /// # Cheap in the common case, deliberately
    ///
    /// An already-focused target returns immediately: no permission check, no
    /// call into [`WindowManager`]'s focus-correction path. This matters
    /// because every targeted call pays this check when `verification_level`
    /// is not `none`, and the overwhelmingly common case is a caller acting on
    /// a window that is already in front.
    ///
    /// # Why a focus correction needs its own permission check
    ///
    /// The capability the *calling* method already checked (`TypeText`,
    /// `KeyPress`, ...) answers "may this caller do this kind of thing at
    /// all" — it says nothing about moving keyboard focus. A focus correction
    /// is a genuine use of [`Capability::FocusWindow`] and is checked as such,
    /// here, only when a correction is actually needed. Denying it must read
    /// as `FocusWindow` being refused, not the calling capability: the message
    /// [`PermissionError`] produces already names the capability passed to
    /// [`PermissionGate::check`], so passing `Capability::FocusWindow` here —
    /// rather than whatever capability the outer method checked — is what
    /// keeps that message honest.
    ///
    /// # `VerificationFailed`, not an error and not a denial
    ///
    /// A window that does not gain focus within
    /// [`DEFAULT_FOCUS_CONFIRM_TIMEOUT`] is reported as
    /// [`InputApiError::VerificationFailed`], per ADR-0007: nothing refused
    /// the request and nothing malfunctioned, the desktop simply did not end
    /// up the way the caller assumed — most often the compositor's own
    /// focus-stealing prevention declining the correction.
    ///
    /// # Every invocation is logged, individually
    ///
    /// Every return path above logs one outcome line via
    /// [`PermissionGate::log_verification_outcome`], attributed to
    /// `capability` — the *calling* method's own capability, passed in by
    /// each of `type_text_at`/`key_press_at`/`key_release_at`/`hotkey_at`,
    /// never [`Capability::FocusWindow`] even on the path that checks it
    /// internally (see above). This method is called once per
    /// `KeyPressAt`/`KeyReleaseAt`/`HotkeyAt` invocation, but potentially
    /// many times per `TypeTextAt` call — see its chunking doc comment — so
    /// a paste re-checked on the order of a hundred times produces on the
    /// order of a hundred audit lines, one per check. **That is intentional,
    /// not a bug to fix**: collapsing a chunked call's re-checks into one
    /// summary line would mean threading chunk-loop state (`typed`/`total`)
    /// into a method that otherwise has no reason to know chunking exists,
    /// for the three callers that never chunk anything. Each line is cheap
    /// and accurate; `permissions::audit`'s module doc accepts the same
    /// tradeoff for `PermissionGate::check`'s own audit line.
    async fn verify_target(
        &self,
        capability: Capability,
        window: u32,
        connection: &zbus::Connection,
        header: &Header<'_>,
    ) -> Result<WindowRecord, InputApiError> {
        let windows = self.windows.list_windows().await?;
        let record = windows
            .into_iter()
            .find(|w| w.id == window)
            .ok_or(WindowsError::WindowNotFound(window))?;

        if record.focused {
            self.permissions
                .log_verification_outcome(
                    capability,
                    connection,
                    header,
                    VerifiedTarget {
                        target: &window_target(window),
                        app_id: &record.app_id,
                        focused: true,
                        outcome: Outcome::Allowed,
                    },
                )
                .await;
            return Ok(record);
        }

        if let Err(err) = self
            .permissions
            .check(Capability::FocusWindow, connection, header)
            .await
        {
            self.permissions
                .log_verification_outcome(
                    capability,
                    connection,
                    header,
                    VerifiedTarget {
                        target: &window_target(window),
                        app_id: &record.app_id,
                        focused: false,
                        outcome: Outcome::Denied,
                    },
                )
                .await;
            return Err(err.into());
        }

        let (record, focus_outcome) = self
            .windows
            .ensure_focused(window, DEFAULT_FOCUS_CONFIRM_TIMEOUT)
            .await?;

        let (focused, outcome) = verification_outcome_for_focus(focus_outcome);
        self.permissions
            .log_verification_outcome(
                capability,
                connection,
                header,
                VerifiedTarget {
                    target: &window_target(window),
                    app_id: &record.app_id,
                    focused,
                    outcome,
                },
            )
            .await;

        match focus_outcome {
            FocusOutcome::AlreadyFocused | FocusOutcome::Corrected => Ok(record),
            FocusOutcome::TimedOut => Err(InputApiError::VerificationFailed(format!(
                "window {window} (`{}`) could not be confirmed focused before the timeout — \
                 nothing was synthesized",
                record.app_id
            ))),
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

    /// Like [`Self::type_text`], but first verifies `window` has keyboard
    /// focus — correcting it first if it does not — before typing, and then
    /// re-verifies it periodically during the typing itself.
    ///
    /// # Why one check is not enough for this method specifically
    ///
    /// A maximum-length call is on the order of 4,000 characters emitted in
    /// one burst. Checking focus only before the first keystroke — as
    /// [`Self::key_press_at`]/[`Self::key_release_at`]/[`Self::hotkey_at`]
    /// do, correctly, since each of those is a single event — would still
    /// let the rest of an oversized paste land in whatever window stole
    /// focus partway through: the user alt-tabs, a notification steals
    /// focus, an application grabs it on startup. See `issues.md`'s S1 for
    /// the general hazard this closes one more layer of.
    ///
    /// So with `verification_level` other than `none`, the text is planned
    /// in full upfront (preserving [`crate::input::InputBackend::type_text`]'s
    /// "the whole string is resolved before anything is typed" atomicity —
    /// an untypeable character or an oversized paste still fails the whole
    /// call, before [`Self::verify_target`] even runs), then typed in chunks
    /// of [`TYPE_TEXT_AT_CHUNK_CHARS`] characters, re-verifying `window`
    /// before every chunk after the first. A re-check that fails stops
    /// typing immediately — nothing from that chunk or any later one is
    /// synthesized — and reports how far it got via
    /// [`InputApiError::VerificationFailed`].
    ///
    /// Chunk boundaries always fall on whole characters, never inside one: a
    /// composed (dead-key) character's two keystrokes are never separated by
    /// a re-check, which can itself take up to a couple of seconds if it has
    /// to correct focus. See `crate::input::keyboard::TypePlan`'s doc
    /// comment for why that would be a real hazard rather than a cosmetic
    /// one.
    ///
    /// # `verification_level = "none"`
    ///
    /// This behaves exactly like [`Self::type_text`]: `window` is accepted
    /// but never consulted, the text is never planned ahead of time, and
    /// there is no chunking overhead at all — a caller can disable the guard
    /// entirely without losing access to the method, and without paying
    /// anything extra for keeping it off.
    async fn type_text_at(
        &self,
        text: &str,
        window: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::TypeText, connection, &header)
            .await?;
        // `!= None` rather than `== Basic`, deliberately. `Strict` can never
        // occur in a live config — `Config::validate` rejects it at load
        // time — but if that invariant were ever broken by a future bug,
        // `!= None` fails toward *performing* this check rather than
        // skipping it, which is the safer direction for a security-relevant
        // guard to fail in. Don't "simplify" this to `== Basic`.
        if self.verification_level == VerificationLevel::None {
            return Ok(self.backend.type_text(text).await?);
        }

        // Validated and planned in full before `verify_target` is even
        // called, exactly like plain `type_text` — so `TextTooLong`,
        // `CharacterNotTypeable`, `KeyboardLayoutUnavailable`, and `Stopped`
        // still surface here precisely as they do for an untargeted call,
        // before any focus check and before anything is typed.
        let mut prepared = self.backend.plan_type_text(text).await?;
        let total = prepared.char_len();

        let record = self
            .verify_target(Capability::TypeText, window, connection, &header)
            .await?;

        let mut typed = 0usize;
        while let Some(chars) = self
            .backend
            .type_next_chunk(&mut prepared, TYPE_TEXT_AT_CHUNK_CHARS)
            .await?
        {
            typed += chars;
            if typed == total {
                // Nothing remains to protect — the last chunk needs no
                // re-check after it runs.
                break;
            }
            // Re-verified before the *next* chunk, not after this one, so a
            // focus change noticed here stops before another character
            // lands anywhere.
            if let Err(err) = self
                .verify_target(Capability::TypeText, window, connection, &header)
                .await
            {
                return Err(chunk_verification_outcome(
                    err,
                    window,
                    &record.app_id,
                    typed,
                    total,
                ));
            }
        }
        Ok(())
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

    /// Like [`Self::key_press`], but first verifies `window` has keyboard
    /// focus — see [`Self::type_text_at`]'s doc comment, which applies here
    /// unchanged.
    async fn key_press_at(
        &self,
        key: &str,
        window: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyPress, connection, &header)
            .await?;
        // See `type_text_at`'s comment on `!= None` vs `== Basic`.
        if self.verification_level != VerificationLevel::None {
            self.verify_target(Capability::KeyPress, window, connection, &header)
                .await?;
        }
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

    /// Like [`Self::hotkey`], but first verifies `window` has keyboard focus
    /// — see [`Self::type_text_at`]'s doc comment, which applies here
    /// unchanged. Gated by `KeyPress`, exactly like [`Self::hotkey`]: there is
    /// no separate `Hotkey` capability, targeted or not.
    async fn hotkey_at(
        &self,
        keys: Vec<String>,
        window: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyPress, connection, &header)
            .await?;
        // See `type_text_at`'s comment on `!= None` vs `== Basic`.
        if self.verification_level != VerificationLevel::None {
            self.verify_target(Capability::KeyPress, window, connection, &header)
                .await?;
        }
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

    /// Like [`Self::key_release`], but first verifies `window` has keyboard
    /// focus — see [`Self::type_text_at`]'s doc comment, which applies here
    /// unchanged.
    async fn key_release_at(
        &self,
        key: &str,
        window: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(), InputApiError> {
        self.permissions
            .check(Capability::KeyRelease, connection, &header)
            .await?;
        // See `type_text_at`'s comment on `!= None` vs `== Basic`.
        if self.verification_level != VerificationLevel::None {
            self.verify_target(Capability::KeyRelease, window, connection, &header)
                .await?;
        }
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

    /// Moves the pointer to an absolute position in global logical pixels,
    /// returning where it landed.
    ///
    /// Coordinates may be negative: a monitor placed left of or above the
    /// primary has them, and rejecting them would make those layouts
    /// unreachable.
    ///
    /// A coordinate that is not on any monitor fails with `OutOfBounds` and
    /// moves nothing. It is **not** clamped to the nearest valid point, which
    /// would put the pointer somewhere the caller did not choose and then let
    /// them click there — the compositor does exactly that if asked, which is
    /// why the check happens here first.
    ///
    /// The reply means the pointer has arrived, not merely that the request was
    /// sent: the extension waits for the warp to take effect. Without that,
    /// moving and then clicking would be a race against the compositor.
    async fn mouse_move_absolute(
        &self,
        x: i32,
        y: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<(i32, i32), InputApiError> {
        self.permissions
            .check(Capability::MouseMoveAbsolute, connection, &header)
            .await?;
        Ok(self.windows.warp_pointer(x, y).await?)
    }

    /// The pointer's current position in global logical pixels.
    ///
    /// Ungated, like every other read-only method on the daemon's interfaces:
    /// it observes the desktop rather than changing it.
    async fn get_pointer_position(&self) -> Result<(i32, i32), InputApiError> {
        Ok(self.windows.get_pointer().await?)
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
        let character_not_typeable =
            InputApiError::CharacterNotTypeable("not typeable".to_string());
        let keyboard_layout_unavailable =
            InputApiError::KeyboardLayoutUnavailable("unavailable".to_string());
        let stopped = InputApiError::Stopped("stopped".to_string());
        let window_not_found = InputApiError::WindowNotFound("window 1 not found".to_string());
        let verification_failed = InputApiError::VerificationFailed("timed out".to_string());
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
        assert_eq!(
            character_not_typeable.name().as_str(),
            wgaf_common::INPUT_ERROR_CHARACTER_NOT_TYPEABLE
        );
        assert_eq!(
            keyboard_layout_unavailable.name().as_str(),
            wgaf_common::INPUT_ERROR_KEYBOARD_LAYOUT_UNAVAILABLE
        );
        assert_eq!(stopped.name().as_str(), wgaf_common::INPUT_ERROR_STOPPED);
        assert_eq!(
            window_not_found.name().as_str(),
            wgaf_common::INPUT_ERROR_WINDOW_NOT_FOUND
        );
        assert_eq!(
            verification_failed.name().as_str(),
            wgaf_common::INPUT_ERROR_VERIFICATION_FAILED
        );
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

    // -----------------------------------------------------------------
    // `verify_target` and the four targeted `*At` methods.
    //
    // `verify_target` never touches `InputBackend`, so it can be tested
    // thoroughly without any risk of synthesizing real input — see
    // `issues.md`'s S1 for why that risk is taken seriously in this crate's
    // test suites. Exactly one test below (`verification_level_none_...`)
    // does reach the real backend, and it is deliberately the smallest,
    // least visible action this daemon can synthesize; see its own doc
    // comment.
    // -----------------------------------------------------------------

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use zbus::object_server::SignalEmitter;

    use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};

    use crate::input::{DEFAULT_MAX_TYPE_TEXT_CHARS, InputLimits};
    use crate::permissions::PolicyMap;

    // `window_target`/`verification_outcome_for_focus` are the pure pieces
    // of `verify_target`'s W17.4 audit logging — see this module's report on
    // why the rest of that logging (the actual `tracing::info!` fields) is
    // exercised only indirectly, through the async tests below, rather than
    // asserted on directly: no tracing-output-capture convention exists
    // anywhere in this codebase yet, and this phase does not introduce one
    // just for this.

    #[test]
    fn window_target_uses_the_window_colon_id_shape() {
        assert_eq!(window_target(227), "window:227");
        assert_eq!(window_target(0), "window:0");
    }

    #[test]
    fn verification_outcome_for_focus_maps_already_focused_and_corrected_to_allowed() {
        assert_eq!(
            verification_outcome_for_focus(FocusOutcome::AlreadyFocused),
            (true, Outcome::Allowed)
        );
        assert_eq!(
            verification_outcome_for_focus(FocusOutcome::Corrected),
            (true, Outcome::Allowed)
        );
    }

    #[test]
    fn verification_outcome_for_focus_maps_timed_out_to_verification_failed() {
        assert_eq!(
            verification_outcome_for_focus(FocusOutcome::TimedOut),
            (false, Outcome::VerificationFailed)
        );
    }

    /// The id `StubExtension`'s one canned window answers to.
    const STUB_WINDOW_ID: u32 = 1;

    /// A stand-in extension sized for exactly what `verify_target` needs —
    /// adapted from `windows::tests::StubExtension` (phase 2 of this same
    /// feature), not reused directly, since that one is private to
    /// `windows/mod.rs`.
    ///
    /// It still has to implement every member `ensure_extension_available`'s
    /// introspection check requires, or `WindowManager` rejects it as an
    /// outdated extension before any test gets to exercise `verify_target` at
    /// all — everything below `list_windows`/`focus_window` is a stub only to
    /// satisfy that check.
    struct StubExtension {
        /// Whether the canned window currently has focus, read back through
        /// `ListWindows`.
        focused: AtomicBool,
        /// If set, a `FocusWindow` call whose `id` equals this value marks the
        /// canned window focused and emits `WindowFocusChanged` for it,
        /// synchronously, inside the same method call. `None` models
        /// focus-stealing prevention silently declining the request:
        /// `FocusWindow` succeeds and nothing else happens.
        focus_succeeds_and_emits_for: Option<u32>,
    }

    #[zbus::interface(name = "org.gnome.Shell.Extensions.Wgaf.V1")]
    impl StubExtension {
        fn list_windows(&self) -> Vec<WindowRecordDict> {
            vec![
                WindowRecord {
                    id: STUB_WINDOW_ID,
                    title: "Stub".to_string(),
                    app_id: "org.wgaf.Stub".to_string(),
                    workspace: 0,
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                    focused: self.focused.load(Ordering::SeqCst),
                    maximized: false,
                }
                .into(),
            ]
        }

        async fn focus_window(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
            let Some(emit_for) = self.focus_succeeds_and_emits_for else {
                return;
            };
            if emit_for == id {
                self.focused.store(true, Ordering::SeqCst);
            }
            if let Err(e) = Self::window_focus_changed(&emitter, emit_for).await {
                eprintln!("stub failed to emit WindowFocusChanged: {e}");
            }
        }

        fn move_window(&self, _id: u32, _x: i32, _y: i32) {}
        fn resize_window(&self, _id: u32, _width: i32, _height: i32) {}
        fn close_window(&self, _id: u32) {}
        fn get_workspaces(&self) -> Vec<WorkspaceRecordDict> {
            vec![]
        }
        fn warp_pointer(&self, x: i32, y: i32) -> (i32, i32) {
            (x, y)
        }
        fn get_pointer(&self) -> (i32, i32) {
            (0, 0)
        }

        #[zbus(signal)]
        async fn window_created(
            emitter: &SignalEmitter<'_>,
            window: WindowRecordDict,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn window_closed(emitter: &SignalEmitter<'_>, id: u32) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn window_focus_changed(emitter: &SignalEmitter<'_>, id: u32) -> zbus::Result<()>;
    }

    /// Starts a `StubExtension` on a private, unique bus name (leaked for the
    /// test process's lifetime, same rationale as `windows::tests`'s
    /// equivalent) and returns a `WindowManager` freshly connected to it.
    async fn manager_against_stub(
        tag: &str,
        focused: bool,
        focus_succeeds_and_emits_for: Option<u32>,
    ) -> WindowManager {
        let bus_name = format!("org.wgaf.Test.VerifyTarget{tag}{}", std::process::id());
        let stub_connection = zbus::connection::Builder::session()
            .expect("session bus builder")
            .name(bus_name.as_str())
            .expect("valid bus name")
            .serve_at(
                wgaf_common::EXTENSION_OBJECT_PATH,
                StubExtension {
                    focused: AtomicBool::new(focused),
                    focus_succeeds_and_emits_for,
                },
            )
            .expect("serve stub extension")
            .build()
            .await
            .expect("stub extension registers on the session bus");
        std::mem::forget(stub_connection);

        let client_connection = zbus::Connection::session()
            .await
            .expect("connect to session bus");
        WindowManager::connect_to(
            client_connection,
            &bus_name,
            wgaf_common::EXTENSION_OBJECT_PATH,
            wgaf_common::EXTENSION_INTERFACE_NAME,
            // Never read by `verify_target`; a real Mutter or another stub is
            // not needed for these tests to be meaningful.
            "org.gnome.Mutter.DisplayConfig",
        )
        .await
        .expect("connect to the stub extension")
    }

    fn test_backend() -> InputBackend {
        InputBackend::new(
            format!("wgaf test device input-api {}", std::process::id()),
            InputLimits {
                max_events_per_second: 0,
                max_type_text_chars: DEFAULT_MAX_TYPE_TEXT_CHARS,
                device_settle: Duration::ZERO,
            },
            "us-ascii",
        )
    }

    fn make_api(
        windows: WindowManager,
        policy_toml: &str,
        verification_level: VerificationLevel,
    ) -> InputApi {
        let policy: PolicyMap = toml::from_str(policy_toml).expect("valid policy toml");
        InputApi::new(
            Arc::new(test_backend()),
            Arc::new(windows),
            Arc::new(PermissionGate::new(policy)),
            verification_level,
        )
    }

    /// A locally built D-Bus method-call message, used only for its
    /// [`Header`] — building one needs no live connection at all. It carries
    /// no `sender`, so `CallerInfo::resolve` degrades to its "unknown caller"
    /// default rather than touching the bus, exactly as it would for a real
    /// message that happened to arrive without one.
    fn synthetic_message() -> zbus::Message {
        zbus::Message::method_call("/org/wgaf/Input", "Test")
            .expect("valid method-call path/member")
            .build(&())
            .expect("build a message with an empty body")
    }

    #[tokio::test]
    async fn already_focused_target_is_ok_without_consulting_focus_window_policy() {
        let manager = manager_against_stub("AlreadyFocused", true, None).await;
        // `FocusWindow` is denied outright — if `verify_target` consulted it
        // for an already-focused target, this call would fail.
        let api = make_api(
            manager,
            "[capabilities]\nFocusWindow = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let record = api
            .verify_target(
                Capability::TypeText,
                STUB_WINDOW_ID,
                &connection,
                &msg.header(),
            )
            .await
            .expect("an already-focused target must succeed without a FocusWindow check");
        assert_eq!(record.id, STUB_WINDOW_ID);
        assert!(record.focused);
    }

    #[tokio::test]
    async fn unfocused_target_is_corrected_when_focus_window_is_allowed_and_confirmed() {
        let manager = manager_against_stub("Corrected", false, Some(STUB_WINDOW_ID)).await;
        let api = make_api(manager, "[capabilities]\n", VerificationLevel::Basic);
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let record = api
            .verify_target(
                Capability::TypeText,
                STUB_WINDOW_ID,
                &connection,
                &msg.header(),
            )
            .await
            .expect("focus should be corrected and confirmed");
        assert_eq!(record.id, STUB_WINDOW_ID);
        assert!(
            record.focused,
            "the record returned alongside a correction must reflect the new state"
        );
    }

    /// `verify_target` calls `ensure_focused` with the real, hardcoded
    /// `DEFAULT_FOCUS_CONFIRM_TIMEOUT` rather than taking a timeout
    /// parameter of its own, so this genuinely waits the full two seconds
    /// rather than a shortened one.
    #[tokio::test]
    async fn unfocused_target_that_never_confirms_is_a_verification_failure() {
        let manager = manager_against_stub("TimedOut", false, None).await;
        let api = make_api(manager, "[capabilities]\n", VerificationLevel::Basic);
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let err = api
            .verify_target(
                Capability::TypeText,
                STUB_WINDOW_ID,
                &connection,
                &msg.header(),
            )
            .await
            .expect_err("a focus request the stub never confirms must not report success");
        assert!(
            matches!(err, InputApiError::VerificationFailed(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn unfocused_target_denied_focus_window_names_focus_window_not_the_caller_capability() {
        let manager = manager_against_stub("Denied", false, Some(STUB_WINDOW_ID)).await;
        let api = make_api(
            manager,
            "[capabilities]\nFocusWindow = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let err = api
            .verify_target(
                Capability::TypeText,
                STUB_WINDOW_ID,
                &connection,
                &msg.header(),
            )
            .await
            .expect_err("FocusWindow denied by policy must refuse the correction");
        match err {
            InputApiError::PermissionDenied(message) => assert!(
                message.contains("FocusWindow"),
                "the denial must name FocusWindow, not the calling method's own capability; \
                 got: {message}"
            ),
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_window_id_is_reported_as_window_not_found() {
        let manager = manager_against_stub("NotFound", false, None).await;
        let api = make_api(manager, "[capabilities]\n", VerificationLevel::Basic);
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();
        let unknown_id = STUB_WINDOW_ID + 999;

        let err = api
            .verify_target(Capability::TypeText, unknown_id, &connection, &msg.header())
            .await
            .expect_err("an id absent from ListWindows must not be treated as focusable");
        assert!(
            matches!(err, InputApiError::WindowNotFound(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn type_text_at_is_refused_by_a_type_text_denial_before_verify_target_runs() {
        let manager = manager_against_stub("TypeTextDenied", false, None).await;
        let api = make_api(
            manager,
            "[capabilities]\nTypeText = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        // Window `0` never exists in the stub's `ListWindows` — reaching
        // `verify_target` would fail with `WindowNotFound`, not
        // `PermissionDenied`, so getting `PermissionDenied` here proves
        // `TypeText` is checked before the target is ever resolved.
        let err = api
            .type_text_at("hello", 0, msg.header(), &connection)
            .await
            .expect_err("TypeText denied by policy must refuse TypeTextAt too");
        match err {
            InputApiError::PermissionDenied(message) => {
                assert!(message.contains("TypeText"), "got: {message}")
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn key_press_at_is_refused_by_a_key_press_denial_before_verify_target_runs() {
        let manager = manager_against_stub("KeyPressDenied", false, None).await;
        let api = make_api(
            manager,
            "[capabilities]\nKeyPress = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let err = api
            .key_press_at("a", 0, msg.header(), &connection)
            .await
            .expect_err("KeyPress denied by policy must refuse KeyPressAt too");
        match err {
            InputApiError::PermissionDenied(message) => {
                assert!(message.contains("KeyPress"), "got: {message}")
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn key_release_at_is_refused_by_a_key_release_denial_before_verify_target_runs() {
        let manager = manager_against_stub("KeyReleaseDenied", false, None).await;
        let api = make_api(
            manager,
            "[capabilities]\nKeyRelease = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let err = api
            .key_release_at("a", 0, msg.header(), &connection)
            .await
            .expect_err("KeyRelease denied by policy must refuse KeyReleaseAt too");
        match err {
            InputApiError::PermissionDenied(message) => {
                assert!(message.contains("KeyRelease"), "got: {message}")
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn hotkey_at_is_refused_by_a_key_press_denial_before_verify_target_runs() {
        // `hotkey`/`hotkey_at` are gated by `KeyPress`, not a capability of
        // their own — see `hotkey`'s doc comment.
        let manager = manager_against_stub("HotkeyDenied", false, None).await;
        let api = make_api(
            manager,
            "[capabilities]\nKeyPress = \"Deny\"\n",
            VerificationLevel::Basic,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        let err = api
            .hotkey_at(
                vec!["ctrl".to_string(), "t".to_string()],
                0,
                msg.header(),
                &connection,
            )
            .await
            .expect_err("KeyPress denied by policy must refuse HotkeyAt too");
        match err {
            InputApiError::PermissionDenied(message) => {
                assert!(message.contains("KeyPress"), "got: {message}")
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    /// The one test in this module that reaches the real `InputBackend` —
    /// see `issues.md`'s S1 for why that is normally avoided here.
    ///
    /// **It synthesizes a key *release*, deliberately.** Releasing a modifier
    /// that was never pressed is a genuine no-op wherever it lands, so this is
    /// safe on its own terms rather than safe only because
    /// `device_settle: Duration::ZERO` (via `test_backend`) makes delivery
    /// unlikely. That distinction matters: the S1 records the settle-time
    /// mitigation losing its race twice, so a test that needs it is a test
    /// that will eventually escape. An earlier version of this pressed
    /// `leftctrl` and never released it, which — had it ever been delivered —
    /// would have left the session with a modifier logically stuck down, the
    /// exact failure `docs/cli-reference.md` warns about under `wgaf key`.
    /// `tests/kill_switch.rs` reaches the same conclusion by a different
    /// route: its `synthesize()` is a zero-delta `MouseMove`, chosen so the
    /// event cannot affect anything even when it does arrive.
    ///
    /// (A few tests below this one also reach the real `InputBackend` — the
    /// `TypeTextAt` chunking tests — but none of them get far enough to
    /// create a device at all, let alone type anything; see each one's own
    /// doc comment.)
    ///
    /// **This test needs no `/dev/uinput`, deliberately** — the same property
    /// `tests/input.rs`'s rate-limit and length-cap tests rely on, reached a
    /// different way. What is under test is that `verify_target` was *not*
    /// consulted, which is proven the moment the call gets past it; whether
    /// the backend then finds a device is a fact about the machine, not about
    /// this logic. So `DeviceUnavailable` is accepted as readily as `Ok`:
    /// both mean the call reached the backend. Asserting plain success
    /// instead made this fail in CI's unit-test step, which runs in a
    /// container with no `/dev/uinput` (see `.github/workflows/ci.yml` — only
    /// the third test step maps the device in).
    #[tokio::test]
    async fn verification_level_none_skips_verify_target_and_reaches_the_backend() {
        let manager = manager_against_stub("NoneSkip", false, None).await;
        // Both conditions that would make `verify_target` fail if it ran: an
        // id absent from `ListWindows`, and `FocusWindow` denied outright.
        let api = make_api(
            manager,
            "[capabilities]\nFocusWindow = \"Deny\"\n",
            VerificationLevel::None,
        );
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();
        let nonexistent_window = STUB_WINDOW_ID + 999;

        match api
            .key_release_at("leftctrl", nonexistent_window, msg.header(), &connection)
            .await
        {
            // Reached the backend, which is the whole assertion — see the
            // doc comment on why the device's absence is not this test's
            // business.
            Ok(()) => {}
            Err(InputApiError::DeviceUnavailable(_)) => {}
            // `WindowNotFound` or `PermissionDenied` here would mean
            // `verify_target` ran despite `verification_level = none`.
            Err(other) => panic!(
                "verification_level = none must skip verify_target entirely and reach the \
                 backend; got {other:?}"
            ),
        }
    }

    // -----------------------------------------------------------------
    // `TypeTextAt`'s chunked, mid-burst-re-verifying path (W17.1).
    //
    // Per this module's S1 note above: no test here types more than zero
    // chunks of real text through a real, non-stopped backend. Proving the
    // abort-with-partial-progress path end-to-end would require at least one
    // chunk (`TYPE_TEXT_AT_CHUNK_CHARS` = dozens of real characters) to
    // succeed before a second check could fail it — that is exactly the
    // "actually typing real text on whatever machine runs `cargo test`"
    // hazard `issues.md`'s S1 exists to name, so it is deliberately not
    // covered here. `partial_progress_message` and `chunk_verification_
    // outcome` below are unit-tested directly instead, which pins the exact
    // wording and the error-classification decision without driving the
    // loop at all, and the loop's chunk-boundary/grouping arithmetic is
    // already covered, fully, by `input::tests`
    // (`chunking_never_splits_a_composed_characters_keystrokes` and its
    // neighbours) with zero device involvement. Full end-to-end proof of the
    // abort path itself is left to the `#[ignore]`d desktop-test suite, same
    // as phase 3 left the corresponding gap for `verify_target`'s own
    // correction path.
    // -----------------------------------------------------------------

    #[test]
    fn partial_progress_message_states_typed_and_remaining_counts() {
        let message = partial_progress_message(7, "org.example.App", 40, 100);
        assert!(
            message.contains("40 of 100"),
            "must state how many were typed and the total: {message}"
        );
        assert!(
            message.contains('7') && message.contains("org.example.App"),
            "must identify the window: {message}"
        );
        assert!(
            message.contains("60"),
            "must state how many characters remain untyped: {message}"
        );
        assert!(
            !message.contains("nothing was synthesized"),
            "must not reuse `verify_target`'s own wording, which would be false here: {message}"
        );
    }

    /// A genuine verification failure (e.g. `verify_target`'s own
    /// focus-confirmation timeout) is the one case that gets its message
    /// rewritten with the partial-progress wording — proving the mid-burst
    /// count actually makes it into what the caller sees.
    #[test]
    fn chunk_verification_outcome_enriches_a_genuine_verification_failure() {
        let original = InputApiError::VerificationFailed(
            "window 227 (`org.example.App`) could not be confirmed focused before the timeout — \
             nothing was synthesized"
                .to_string(),
        );

        let outcome = chunk_verification_outcome(original, 227, "org.example.App", 40, 100);

        match outcome {
            InputApiError::VerificationFailed(message) => {
                assert!(message.contains("40 of 100"), "got: {message}");
                assert!(
                    !message.contains("nothing was synthesized"),
                    "must not keep verify_target's own wording, which is false mid-burst: \
                     {message}"
                );
            }
            other => panic!("expected VerificationFailed, got {other:?}"),
        }
    }

    /// A window that closed between chunks is `WindowNotFound` — a plain
    /// error, per W16's classification — and must reach the caller as
    /// `WindowNotFound`, not be relabeled `VerificationFailed`. Reporting
    /// "could not be reconfirmed focused" for a window that no longer exists
    /// would be false, and would misclassify the outcome once W16 wires up
    /// exit codes by matching on this enum.
    #[test]
    fn chunk_verification_outcome_preserves_window_not_found() {
        let original = InputApiError::WindowNotFound("window 227 not found".to_string());

        let outcome = chunk_verification_outcome(original, 227, "org.example.App", 40, 100);

        assert!(
            matches!(outcome, InputApiError::WindowNotFound(_)),
            "a closed window must stay WindowNotFound, not become VerificationFailed: {outcome:?}"
        );
    }

    /// Likewise for a `FocusWindow` correction denied by policy mid-run — a
    /// denial, not a verification failure, and the two must stay
    /// distinguishable exactly as ADR-0007 requires everywhere else.
    #[test]
    fn chunk_verification_outcome_preserves_permission_denied() {
        let original = InputApiError::PermissionDenied("FocusWindow denied by policy".to_string());

        let outcome = chunk_verification_outcome(original, 227, "org.example.App", 40, 100);

        assert!(
            matches!(outcome, InputApiError::PermissionDenied(_)),
            "a policy denial must stay PermissionDenied, not become VerificationFailed: \
             {outcome:?}"
        );
    }

    /// Oversized text must still be refused before `verify_target` runs, on
    /// the chunked path exactly as on the untargeted `type_text` — proven by
    /// aiming at a window id that does not exist: reaching `verify_target`
    /// at all would fail with `WindowNotFound`, not `TextTooLong`, so getting
    /// `TextTooLong` here proves planning happens first. Fails before
    /// `InputBackend` ever resolves a layout or touches a device.
    #[tokio::test]
    async fn type_text_at_reports_text_too_long_before_verify_target_runs() {
        let manager = manager_against_stub("TooLong", false, None).await;
        let api = make_api(manager, "[capabilities]\n", VerificationLevel::Basic);
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();
        let text: String = std::iter::repeat_n('a', DEFAULT_MAX_TYPE_TEXT_CHARS + 1).collect();
        let nonexistent_window = STUB_WINDOW_ID + 999;

        let err = api
            .type_text_at(&text, nonexistent_window, msg.header(), &connection)
            .await
            .expect_err("oversized text must be refused before any focus check");
        assert!(matches!(err, InputApiError::TextTooLong(_)), "got {err:?}");
    }

    /// An empty string has nothing to type: `verify_target` still runs once
    /// (matching `type_text_at`'s behaviour before this phase, which checked
    /// unconditionally regardless of text length), but the chunk loop then
    /// ends on its very first iteration without ever reaching
    /// `InputBackend::run` — so this exercises the whole chunked path
    /// end-to-end, including a real (not stopped) backend, without creating
    /// a device or synthesizing anything at all.
    #[tokio::test]
    async fn type_text_at_with_empty_text_completes_without_typing_a_chunk() {
        let manager = manager_against_stub("EmptyText", true, None).await;
        let api = make_api(manager, "[capabilities]\n", VerificationLevel::Basic);
        let connection = zbus::Connection::session().await.expect("session bus");
        let msg = synthetic_message();

        api.type_text_at("", STUB_WINDOW_ID, msg.header(), &connection)
            .await
            .expect("empty text has nothing to type and nothing left to verify");
    }
}
