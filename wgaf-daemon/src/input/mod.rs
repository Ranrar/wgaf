//! Daemon-side input automation: keyboard and mouse synthesis via a
//! `/dev/uinput` virtual device. Unlike `windows/`, this module has no
//! GNOME Shell Extension dependency — GJS has no `uinput` access, so input
//! synthesis has to live in the Rust daemon itself and talk to the kernel
//! directly. Exposed to the CLI via the daemon's own `org.wgaf.Input1`
//! interface, see `crate::dbus::input_api`.
//!
//! **Audit logging, not policy:** every synthesized action is logged via
//! `tracing` on the `wgaf_daemon::input::audit` target before it executes.
//! This is deliberately *not* an allow/deny gate — nothing here blocks an
//! action — it exists purely so there's some accountability trail for input
//! synthesis ahead of the real permission/policy engine (see
//! `crate::permissions`). Do not add allow/deny logic here; that's
//! explicitly out of scope for this module.

mod codes;
mod device;
mod keyboard;
mod mouse;

use std::sync::{Arc, Mutex};

use thiserror::Error;
use tokio::sync::OnceCell;

use device::UinputDevice;

/// `tracing` target for the input-synthesis audit trail (see module docs).
const AUDIT_TARGET: &str = "wgaf_daemon::input::audit";

/// Default name this daemon's virtual `uinput` device reports to the
/// kernel. Overridable via `Config::input_device_name` purely for test
/// isolation: `/proc/bus/input/devices` is a machine-global namespace with
/// no notion of "which daemon process created this device", so
/// `wgaf-daemon/tests/input.rs` gives each spawned test daemon its own
/// unique device name to tell its device apart from a concurrently-running
/// test's (or a real production daemon's).
pub(crate) const DEFAULT_DEVICE_NAME: &str = "wgaf virtual input device";

/// Safety cap on `TypeText`'s input length — not a policy decision (the
/// permission module owns those, see `crate::permissions`), just a sane
/// default guarding against a runaway/mistaken caller asking the daemon to
/// synthesize an unbounded number of key events in one call. This only
/// partially addresses rate limiting — a proper token-bucket-style rate
/// limiter across calls is deferred, not implemented here.
const MAX_TYPE_TEXT_LEN: usize = 4096;

/// Errors surfaced by the daemon's input-automation layer.
#[derive(Debug, Error)]
pub enum InputError {
    /// The `uinput` virtual device could not be opened or set up —
    /// typically `/dev/uinput` missing the expected permissions. Mirrors
    /// `WindowsError::ExtensionUnavailable`'s "clear, actionable error"
    /// style: never a raw ioctl errno dump, never silently falling back to
    /// anything, and never suggesting `sudo`/root as the fix.
    #[error(
        "uinput device unavailable at `{path}`: {reason} — this is normally a permissions \
         problem, not a code bug: ensure a udev rule grants access (e.g. \
         `KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\"` in a file under \
         /etc/udev/rules.d/) and that this user is a member of the `input` group \
         (`sudo usermod -aG input $USER`, then log out and back in — group membership \
         changes don't apply to already-running sessions)."
    )]
    DeviceUnavailable { path: String, reason: String },

    /// `KeyPress`/`KeyRelease`/`TypeText` referenced a key name or character
    /// this daemon doesn't have a mapping for.
    #[error("unknown key `{0}`")]
    UnknownKey(String),

    /// `MouseClick` was given something other than `left`/`right`/`middle`.
    #[error("invalid mouse button `{0}` — expected `left`, `right`, or `middle`")]
    InvalidButton(String),

    /// `TypeText` was given more text than [`MAX_TYPE_TEXT_LEN`] allows.
    #[error("text too long ({len} chars, max {MAX_TYPE_TEXT_LEN})")]
    TextTooLong { len: usize },

    /// Any other I/O failure writing to the `uinput` device after it was
    /// successfully created (e.g. it disappeared underneath us).
    #[error("I/O error writing to the uinput device: {0}")]
    Io(#[from] std::io::Error),
}

/// Owns the daemon's one virtual input device and exposes the
/// keyboard/mouse operations `org.wgaf.Input1` delegates to. Analogous in
/// spirit to `windows::WindowManager`: one instance created at daemon
/// startup and served for the interface's lifetime — but where
/// `WindowManager` defers only its *availability check* to first use (the
/// extension proxy itself is built eagerly, cheaply, without I/O),
/// `InputBackend` defers the device's actual creation too, since opening
/// `/dev/uinput` and running its setup ioctls is the operation that can
/// fail on permissions. This means a daemon started before `/dev/uinput`
/// permissions are fixed (e.g. before a fresh `input` group membership has
/// been picked up by a re-login) will recover on the next call without a
/// daemon restart, exactly like `WindowManager::ensure_extension_available`.
///
/// The device handle is behind a `std::sync::Mutex` (not `tokio::sync::Mutex`)
/// because every actual use of it happens inside a `tokio::task::spawn_blocking`
/// closure (see [`InputBackend::run`]) — that closure runs on a dedicated
/// blocking-pool thread, so a synchronous lock there never blocks the async
/// executor. The `uinput` writes themselves are fixed-size, near-instant
/// char-device writes (not disk/network I/O), but routing them through
/// `spawn_blocking` keeps the async methods honestly non-blocking without
/// forcing every event write in `device.rs`/`keyboard.rs`/`mouse.rs` to
/// itself be async.
pub struct InputBackend {
    /// Cached only on success — an initialization failure (permissions) is
    /// not cached, so the next call retries rather than requiring a daemon
    /// restart. Mirrors `windows::WindowManager`'s `verified: OnceCell<()>`.
    device: OnceCell<Arc<Mutex<UinputDevice>>>,
    device_name: String,
}

impl InputBackend {
    /// Does not touch `/dev/uinput` — safe to call unconditionally at
    /// daemon startup, exactly like `WindowManager::connect_to` doesn't
    /// require the extension to already be running. The device is only
    /// actually opened/created on first real use, via [`Self::device`].
    ///
    /// `device_name` is normally `Config::input_device_name` (which itself
    /// defaults to [`DEFAULT_DEVICE_NAME`]) — see that constant's doc
    /// comment for why it's configurable at all.
    pub fn new(device_name: impl Into<String>) -> Self {
        Self {
            device: OnceCell::new(),
            device_name: device_name.into(),
        }
    }

    async fn device(&self) -> Result<Arc<Mutex<UinputDevice>>, InputError> {
        let device = self
            .device
            .get_or_try_init(|| async {
                let name = self.device_name.clone();
                let device = tokio::task::spawn_blocking(move || UinputDevice::create(&name))
                    .await
                    .expect("uinput device-creation task panicked")?;
                Ok::<_, InputError>(Arc::new(Mutex::new(device)))
            })
            .await?;
        Ok(Arc::clone(device))
    }

    pub async fn type_text(&self, text: &str) -> Result<(), InputError> {
        if text.chars().count() > MAX_TYPE_TEXT_LEN {
            return Err(InputError::TextTooLong {
                len: text.chars().count(),
            });
        }
        tracing::info!(target: AUDIT_TARGET, action = "type_text", len = text.len(), "synthesizing text input");
        let text = text.to_string();
        self.run(move |device| keyboard::type_text(device, &text))
            .await
    }

    pub async fn key_press(&self, key: &str) -> Result<(), InputError> {
        let code = keyboard::resolve_key(key)?;
        tracing::info!(target: AUDIT_TARGET, action = "key_press", key = %key, "synthesizing key press");
        self.run(move |device| keyboard::press(device, code)).await
    }

    pub async fn key_release(&self, key: &str) -> Result<(), InputError> {
        let code = keyboard::resolve_key(key)?;
        tracing::info!(target: AUDIT_TARGET, action = "key_release", key = %key, "synthesizing key release");
        self.run(move |device| keyboard::release(device, code))
            .await
    }

    pub async fn mouse_move(&self, dx: i32, dy: i32) -> Result<(), InputError> {
        tracing::info!(target: AUDIT_TARGET, action = "mouse_move", dx, dy, "synthesizing relative mouse move");
        self.run(move |device| mouse::move_relative(device, dx, dy))
            .await
    }

    pub async fn mouse_click(&self, button: &str) -> Result<(), InputError> {
        let code = mouse::resolve_button(button)?;
        tracing::info!(target: AUDIT_TARGET, action = "mouse_click", button = %button, "synthesizing mouse click");
        self.run(move |device| mouse::click(device, code)).await
    }

    pub async fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<(), InputError> {
        tracing::info!(target: AUDIT_TARGET, action = "mouse_scroll", dx, dy, "synthesizing mouse scroll");
        self.run(move |device| mouse::scroll(device, dx, dy)).await
    }

    /// Resolves the (possibly not-yet-created) device, then runs `f`
    /// against it on a blocking-pool thread — the shared plumbing every
    /// public method above uses (see the struct docs for why
    /// `spawn_blocking` + a synchronous `Mutex` is the right shape here).
    async fn run<F>(&self, f: F) -> Result<(), InputError>
    where
        F: FnOnce(&mut UinputDevice) -> Result<(), InputError> + Send + 'static,
    {
        let device = self.device().await?;
        tokio::task::spawn_blocking(move || {
            let mut device = device.lock().expect("uinput device mutex poisoned");
            f(&mut device)
        })
        .await
        .expect("input synthesis blocking task panicked")
    }
}
