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
mod keymap;
mod mouse;
mod rate_limit;
mod wayland;
mod xkb;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use tokio::sync::OnceCell;
use tokio::time::Instant;

use device::UinputDevice;
use rate_limit::{Acquired, TokenBucket};

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

/// How long to wait after creating the virtual device before writing the first
/// event to it, when `config.toml` does not say otherwise. Overridable via
/// `Config::input_device_settle_ms`.
///
/// **Creating a `uinput` device does not make it usable.** `UI_DEV_CREATE`
/// returns as soon as the kernel has the device, but nothing is listening to it
/// yet: udev has to process the new node and re-broadcast it, and only then does
/// the compositor open it through libinput. Events written before that reach
/// nobody. The kernel accepts every one of them, so the daemon has no way to
/// notice — it reports success for input that was silently discarded.
///
/// That is not a theoretical race. Without this wait the **first** input command
/// after the daemon starts was reliably lost, every time: `wgaf type` reported
/// typing 34 characters and `tests/apps/input-test` received zero events, while
/// the very next command worked. It went unnoticed for seven phases because
/// `tests/input.rs` reads the events back from the kernel, which is the one
/// place they do arrive.
///
/// The delay was measured rather than guessed. On the development machine
/// `udevadm monitor` puts udev's processing of the new event node **36 ms**
/// after the kernel's own `add`, and the empirical threshold agrees: a 0 ms wait
/// loses all input, 50 ms delivers all of it. The default is an order of
/// magnitude above that, because being slow once per daemon lifetime is cheap
/// and being wrong is silent.
///
/// **This widens the race rather than closing it, and it is a stopgap.** A
/// duration is the wrong shape for this: nothing verifies the margin held, so
/// if it is ever too short the original bug returns silently.
///
/// It was chosen on the belief that the last hop — the compositor opening the
/// device — could not be observed from this process. **That belief was wrong**,
/// and was disproven the same day: the compositor holds the device's own event
/// node open, under the same user this daemon runs as, so `/proc/<pid>/fd`
/// makes "someone has picked up our device" a directly checkable condition.
/// Waiting for that instead of for a clock is the actual fix, and it is filed
/// in `issues.md` rather than done here. Until then, raise this value if a
/// loaded or slow machine still drops the first command.
///
/// The cost is paid **once per daemon lifetime**, not per call: the device lives
/// in a `OnceCell` (see [`InputBackend::device`]) and is created at most once.
pub(crate) const DEFAULT_DEVICE_SETTLE_MS: u64 = 300;

/// Cap on how many characters one `TypeText` call may synthesize, when
/// `config.toml` does not say otherwise. Overridable via
/// `Config::input_max_type_text_chars`.
///
/// **This is the only bound on burst depth**, which is why it survives
/// alongside [`rate_limit`] rather than being subsumed by it. The limiter
/// charges a call's whole cost up front, sleeps once, and then emits every
/// event as fast as the kernel accepts them — it paces *between* calls, not
/// *within* one. Without this cap, a single enormous `TypeText` would wait and
/// then fire uninterruptibly.
///
/// **Not a security control**, and lowering it does not make one. A caller
/// that can issue one `TypeText` can issue a hundred, and any process running
/// as this user can open `/dev/uinput` directly and bypass wgaf entirely —
/// see `rate_limit`'s module docs for the full version of that argument. What
/// it does buy a cautious user is that an oversized paste **fails loudly
/// instead of executing**, which is worth having against accidents and
/// opportunistic pastes even though a determined attacker routes around it.
pub(crate) const DEFAULT_MAX_TYPE_TEXT_CHARS: usize = 4096;

/// Which keyboard layout `TypeText` resolves characters against, when
/// `config.toml` does not say otherwise. Overridable via
/// `Config::input_keyboard_layout`.
///
/// `"auto"` takes the session keymap's first layout. See [`keymap`] for why
/// that is a decision rather than a detection: the *active* layout index only
/// arrives with keyboard focus, which a headless daemon never has.
pub(crate) const DEFAULT_KEYBOARD_LAYOUT: &str = keymap::AUTO;

/// The configured value selecting the historical US-QWERTY table instead of the
/// session's keymap.
///
/// Kept reachable because scripts written against key *positions* rather than
/// characters exist, and because it is the behaviour every wgaf release before
/// this one had.
pub(crate) const US_ASCII_LAYOUT: &str = "us-ascii";

/// How `TypeText` turns characters into keystrokes.
pub(crate) enum Typing {
    /// Resolved from the session's own keymap — correct on whatever layout the
    /// desktop is actually using, including characters behind AltGr and dead
    /// keys.
    Keymap(Arc<xkb::LayoutMap>),
    /// The historical flat US-QWERTY table. Correct on exactly one layout, and
    /// unable to express a third-level modifier at all.
    UsAscii,
}

/// Sustained synthetic-input budget, in kernel events per second, when
/// `config.toml` does not say otherwise.
///
/// Deliberately generous — far above any real automation need, far below
/// "the desktop is unusable". For scale, a fast human types perhaps 10
/// characters per second (40 events); this allows 3,000. It exists to stop a
/// loop bug from taking the session away from its user, not to pace
/// legitimate work.
///
/// **Tune this against a real session rather than trusting the number.** It
/// was chosen by reasoning about orders of magnitude, not by measuring the
/// point at which GNOME Shell actually stops keeping up.
pub(crate) const DEFAULT_MAX_EVENTS_PER_SECOND: u32 = 3000;

/// The point at which a backlog stops being a slow script and starts being a
/// runaway: past this much accumulated delay, a call is refused with
/// [`InputError::RateLimited`] instead of throttled.
///
/// Deliberately *not* configurable, unlike the rate and the character cap:
/// those are statements about a particular machine and a particular user's
/// appetite for risk, whereas "30 seconds of queued synthetic input means
/// something is stuck in a loop" is true everywhere.
const MAX_THROTTLE_DELAY: Duration = Duration::from_secs(30);

/// The input backend's tunables, resolved from `config.toml` at startup.
///
/// Grouped rather than passed as loose arguments so that adding one does not
/// grow [`InputBackend::new`]'s signature by another anonymous number.
///
/// Most of these are safety ceilings; [`Self::device_settle`] is not one, and
/// is here because it shares their single source and lifetime rather than
/// because it shares their purpose.
#[derive(Debug, Clone, Copy)]
pub struct InputLimits {
    /// Sustained events per second; `0` disables the rate limiter.
    pub max_events_per_second: u32,
    /// Characters one `TypeText` may synthesize.
    ///
    /// **`0` means no characters may be typed, not "no limit"** — the
    /// opposite of `max_events_per_second`'s `0`. The inconsistency is
    /// deliberate: this cap exists so that an oversized paste fails rather
    /// than executes, and reading `0` as "unlimited" would hand exactly that
    /// outcome to a user who typed it expecting to switch the guard off. The
    /// fail-safe reading is the one that cannot surprise anybody dangerously.
    pub max_type_text_chars: usize,
    /// How long to wait after creating the `uinput` device before writing to
    /// it, so the first command is not silently discarded. Paid once per daemon
    /// lifetime. See [`DEFAULT_DEVICE_SETTLE_MS`]; `0` disables the wait, which
    /// is only useful for a test that never expects delivery.
    pub device_settle: Duration,
}

/// Reads the session keymap and builds the character index for the configured
/// layout. Blocking; callers run it on the blocking pool.
///
/// Splits its failures in two on purpose. A keymap that cannot be read is the
/// *environment* being absent — no Wayland session, no keyboard — and the daemon
/// stays useful for windows and accessibility. A layout that does not resolve is
/// the *configuration* being wrong, which is worth refusing to start over rather
/// than discovering at the first `wgaf type`.
fn resolve_layout(spec: &str) -> Result<xkb::LayoutMap, InputError> {
    let session_keymap = wayland::read_session_keymap()
        .map_err(|e| InputError::KeyboardLayoutUnavailable(e.to_string()))?;

    let index = keymap::resolve(&session_keymap, spec)
        .map_err(|e| InputError::KeyboardLayoutInvalid(e.to_string()))?;

    let map = xkb::LayoutMap::build(&session_keymap, index, codes::registered_codes());

    tracing::info!(
        layout = %map.layout_name(),
        configured = %spec,
        characters = map.len(),
        "resolved keyboard layout"
    );

    Ok(map)
}

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

    /// `Hotkey` was given no keys at all. Named rather than silently succeeding,
    /// since a script building a combination from a variable that turned out
    /// empty should hear about it.
    #[error("no keys given — a hotkey needs at least one key, e.g. `ctrl+shift+t`")]
    EmptyHotkey,

    /// `TypeText` was given more text than `input_max_type_text_chars` allows.
    ///
    /// Names the configured limit rather than a compiled-in one, since the
    /// user may well have chosen it — a message quoting 4096 at someone who
    /// set 256 would send them looking for a bug.
    #[error(
        "text too long ({len} chars, max {max}) — the limit is \
         `input_max_type_text_chars` in config.toml"
    )]
    TextTooLong { len: usize, max: usize },

    /// So much synthetic input is queued that servicing this call would have
    /// meant waiting [`MAX_THROTTLE_DELAY`] or more. Indicates a runaway
    /// caller — a loop bug, typically — rather than a merely busy one, which
    /// the limiter throttles silently instead.
    #[error(
        "input rate limit exceeded: this call would have waited {seconds:.0}s behind queued \
         synthetic input, which indicates a runaway caller rather than a slow one — stop the \
         script that is flooding input. The sustained budget is \
         `input_max_events_per_second` in config.toml."
    )]
    RateLimited { seconds: f64 },

    /// `TypeText` was asked for a character the configured layout has no key
    /// sequence for — an emoji, or a script the layout does not cover.
    ///
    /// **Deliberately an error rather than a fallback.** Reaching for the
    /// clipboard or an accessibility text insertion here would silently change
    /// mechanism mid-string: those replace content rather than typing it, need
    /// a target element wgaf does not have, and would let a caller denied
    /// `SetText` obtain it through `TypeText`. Refusing is the honest answer.
    #[error(
        "`{ch}` cannot be typed on keyboard layout `{layout}` — it has no key \
         sequence there. Nothing was typed."
    )]
    CharacterNotTypeable { ch: char, layout: String },

    /// The session's keymap could not be read, so `TypeText` does not know what
    /// its keystrokes would produce.
    ///
    /// Window and accessibility commands are unaffected; only character-level
    /// typing needs a keymap.
    #[error("the keyboard layout could not be determined: {0}")]
    KeyboardLayoutUnavailable(String),

    /// `input_keyboard_layout` names a layout that does not exist, or one this
    /// session is not configured with.
    ///
    /// Kept separate from [`Self::KeyboardLayoutUnavailable`] because this one
    /// is the user's configuration being wrong rather than the environment
    /// being absent, and only this one is worth refusing to start over.
    #[error("{0}")]
    KeyboardLayoutInvalid(String),

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
    /// The configured `input_keyboard_layout` value, resolved on first use.
    layout_spec: String,
    /// Cached only on success, exactly like [`Self::device`]: a daemon started
    /// before its Wayland session is ready recovers on the next call rather
    /// than needing a restart.
    layout: OnceCell<Arc<xkb::LayoutMap>>,
    /// **One global bucket, deliberately not keyed by caller.** There is one
    /// pointer and one keyboard focus, so the resource being protected is
    /// shared; per-caller buckets would let N processes multiply the ceiling
    /// and defeat the guard exactly when it is most needed.
    ///
    /// A `std::sync::Mutex` because it is only ever held across the bucket's
    /// own arithmetic, never across an `.await` — see [`Self::run`].
    limiter: Mutex<TokenBucket>,
    /// Whether the throttle has already been reported this run. The warning
    /// is worth seeing once; on every throttled call it would itself become
    /// a flood.
    throttle_reported: AtomicBool,
    /// Resolved from `config.toml` at startup; see [`InputLimits`].
    limits: InputLimits,
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
    ///
    /// `limits` normally comes from `config.toml` — see [`InputLimits`], and
    /// note the two `0` values mean opposite things there, deliberately.
    pub fn new(
        device_name: impl Into<String>,
        limits: InputLimits,
        keyboard_layout: impl Into<String>,
    ) -> Self {
        Self {
            device: OnceCell::new(),
            device_name: device_name.into(),
            layout_spec: keyboard_layout.into(),
            layout: OnceCell::new(),
            limiter: Mutex::new(TokenBucket::new(
                limits.max_events_per_second,
                MAX_THROTTLE_DELAY,
                Instant::now(),
            )),
            throttle_reported: AtomicBool::new(false),
            limits,
        }
    }

    async fn device(&self) -> Result<Arc<Mutex<UinputDevice>>, InputError> {
        let device = self
            .device
            .get_or_try_init(|| async {
                let name = self.device_name.clone();
                let settle = self.limits.device_settle;
                let device =
                    tokio::task::spawn_blocking(move || UinputDevice::create(&name, settle))
                        .await
                        .expect("uinput device-creation task panicked")?;
                Ok::<_, InputError>(Arc::new(Mutex::new(device)))
            })
            .await?;
        Ok(Arc::clone(device))
    }

    /// Reports whether `/dev/uinput` is usable right now, **without**
    /// creating the device — see [`device::probe_access`]. Used by
    /// `org.wgaf.Daemon1.Status`.
    ///
    /// Deliberately not routed through [`Self::device`]: that would populate
    /// the `OnceCell` and register a real kernel device, turning a read-only
    /// status query into an action with visible system-wide effects.
    pub async fn probe_device_access(&self) -> Result<(), InputError> {
        tokio::task::spawn_blocking(device::probe_access)
            .await
            .expect("uinput probe task panicked")
    }

    /// Whether the virtual device has actually been created this run.
    ///
    /// An *activity* signal, not a health one — `false` just means nothing
    /// has synthesized input yet. Reads the `OnceCell` without initializing
    /// it, so calling this never creates the device.
    pub fn device_created(&self) -> bool {
        self.device.initialized()
    }

    /// The name this backend's virtual device reports to the kernel.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// How characters are turned into keystrokes, resolved on first use.
    ///
    /// Lazy for the same reason the device is: reading the keymap needs a live
    /// Wayland session, and a daemon started before one is ready should recover
    /// on the next call rather than require a restart. Failures are not cached.
    ///
    /// Costs one Wayland connection, once — see [`wayland`] for why nothing is
    /// held open afterwards.
    pub(crate) async fn typing(&self) -> Result<Typing, InputError> {
        if self
            .layout_spec
            .trim()
            .eq_ignore_ascii_case(US_ASCII_LAYOUT)
        {
            return Ok(Typing::UsAscii);
        }

        let map = self
            .layout
            .get_or_try_init(|| async {
                let spec = self.layout_spec.clone();
                tokio::task::spawn_blocking(move || resolve_layout(&spec))
                    .await
                    .map_err(|e| InputError::KeyboardLayoutUnavailable(e.to_string()))?
                    .map(Arc::new)
            })
            .await?;

        Ok(Typing::Keymap(Arc::clone(map)))
    }

    /// The configured `input_keyboard_layout` value, verbatim.
    pub fn layout_spec(&self) -> &str {
        &self.layout_spec
    }

    /// The name of the layout actually in use, if one has been resolved.
    ///
    /// Reads the `OnceCell` without initializing it, so `wgaf status` can
    /// report this without opening a Wayland connection — the same rule
    /// [`Self::device_created`] follows for the uinput device.
    pub fn resolved_layout_name(&self) -> Option<String> {
        match self
            .layout_spec
            .trim()
            .eq_ignore_ascii_case(US_ASCII_LAYOUT)
        {
            true => Some("US ASCII (layout-independent table)".to_string()),
            false => self.layout.get().map(|map| map.layout_name().to_string()),
        }
    }

    pub async fn type_text(&self, text: &str) -> Result<(), InputError> {
        // Checked before the rate limiter and before the device is resolved,
        // so an oversized paste is refused without waiting, without spending
        // budget, and without registering a uinput device.
        let len = text.chars().count();
        if len > self.limits.max_type_text_chars {
            return Err(InputError::TextTooLong {
                len,
                max: self.limits.max_type_text_chars,
            });
        }

        // Resolved before anything is synthesized, so a string containing one
        // untypeable character types *nothing* rather than stopping partway
        // through and leaving the application holding half of it.
        let typing = self.typing().await?;
        let plan = keyboard::plan_text(text, &typing)?;

        tracing::info!(target: AUDIT_TARGET, action = "type_text", len = text.len(), "synthesizing text input");
        let cost = keyboard::plan_event_cost(&plan);
        self.run(cost, move |device| keyboard::type_planned(device, &plan))
            .await
    }

    pub async fn key_press(&self, key: &str) -> Result<(), InputError> {
        let code = keyboard::resolve_key(key)?;
        tracing::info!(target: AUDIT_TARGET, action = "key_press", key = %key, "synthesizing key press");
        self.run(1, move |device| keyboard::press(device, code))
            .await
    }

    pub async fn key_release(&self, key: &str) -> Result<(), InputError> {
        let code = keyboard::resolve_key(key)?;
        tracing::info!(target: AUDIT_TARGET, action = "key_release", key = %key, "synthesizing key release");
        self.run(1, move |device| keyboard::release(device, code))
            .await
    }

    /// Presses a key combination — every key held, then released in reverse.
    ///
    /// Layout-independent: these are named physical keys, exactly like
    /// [`Self::key_press`], because a shortcut is a position rather than a
    /// character. `Ctrl+Shift+T` is the same three keys on every layout.
    ///
    /// Every key is resolved before any is pressed, so an unknown name in the
    /// middle of a combination presses nothing rather than leaving the first
    /// two held down.
    pub async fn hotkey(&self, keys: &[String]) -> Result<(), InputError> {
        if keys.is_empty() {
            return Err(InputError::EmptyHotkey);
        }

        let codes = keys
            .iter()
            .map(|k| keyboard::resolve_key(k))
            .collect::<Result<Vec<_>, _>>()?;

        tracing::info!(
            target: AUDIT_TARGET,
            action = "hotkey",
            keys = %keys.join("+"),
            "synthesizing key combination"
        );
        // Two events per key: one press on the way in, one release on the way
        // out.
        let cost = 2 * codes.len() as u32;
        self.run(cost, move |device| keyboard::hotkey(device, &codes))
            .await
    }

    pub async fn mouse_move(&self, dx: i32, dy: i32) -> Result<(), InputError> {
        tracing::info!(target: AUDIT_TARGET, action = "mouse_move", dx, dy, "synthesizing relative mouse move");
        // One event: `rel_move` emits X and Y inside a single SYN batch.
        self.run(1, move |device| mouse::move_relative(device, dx, dy))
            .await
    }

    pub async fn mouse_click(&self, button: &str) -> Result<(), InputError> {
        let code = mouse::resolve_button(button)?;
        tracing::info!(target: AUDIT_TARGET, action = "mouse_click", button = %button, "synthesizing mouse click");
        // Two: press and release.
        self.run(2, move |device| mouse::click(device, code)).await
    }

    pub async fn mouse_scroll(&self, dx: i32, dy: i32) -> Result<(), InputError> {
        tracing::info!(target: AUDIT_TARGET, action = "mouse_scroll", dx, dy, "synthesizing mouse scroll");
        // Two: `mouse::scroll` emits REL_HWHEEL and REL_WHEEL as separately
        // SYN_REPORT-terminated events, unlike `move_relative`'s single batch.
        self.run(2, move |device| mouse::scroll(device, dx, dy))
            .await
    }

    /// Charges `cost` kernel events against the rate limiter, sleeping if the
    /// budget is overdrawn and refusing outright if the backlog says runaway.
    ///
    /// Awaits *before* [`Self::run`] reaches `spawn_blocking`, so a throttled
    /// call never occupies a blocking-pool thread while it waits.
    async fn throttle(&self, cost: u32) -> Result<(), InputError> {
        let outcome = {
            // Scoped so the guard is dropped before any `.await` below —
            // this is a `std::sync::Mutex` and must not be held across one.
            let mut limiter = self
                .limiter
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            limiter.acquire(cost, Instant::now())
        };

        match outcome {
            Acquired::Ready => Ok(()),
            Acquired::After(wait) => {
                // Once per run: a legitimately-throttled user should learn
                // why things got slow, but not once per event.
                if !self.throttle_reported.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        target: AUDIT_TARGET,
                        action = "throttle",
                        wait_ms = wait.as_millis() as u64,
                        "synthetic input is exceeding the configured budget and is being slowed \
                         down; if this was not intended, check the script driving wgaf. The \
                         budget is `input_max_events_per_second` in config.toml."
                    );
                }
                tokio::time::sleep(wait).await;
                Ok(())
            }
            Acquired::Runaway { would_wait } => {
                tracing::warn!(
                    target: AUDIT_TARGET,
                    action = "rate_limited",
                    would_wait_s = would_wait.as_secs_f64(),
                    "refusing synthetic input: the queued backlog indicates a runaway caller"
                );
                Err(InputError::RateLimited {
                    seconds: would_wait.as_secs_f64(),
                })
            }
        }
    }

    /// Resolves the (possibly not-yet-created) device, then runs `f`
    /// against it on a blocking-pool thread — the shared plumbing every
    /// public method above uses (see the struct docs for why
    /// `spawn_blocking` + a synchronous `Mutex` is the right shape here).
    ///
    /// Also the single funnel every synthesized event passes through, which
    /// is why the rate limiter is checked here rather than in each public
    /// method: there is exactly one place to forget, and adding a method that
    /// forgets it means not calling `run` at all.
    ///
    /// `cost` is that operation's kernel-event count, not its call count —
    /// see [`keyboard::type_text_event_cost`] for why the distinction is the
    /// whole point.
    async fn run<F>(&self, cost: u32, f: F) -> Result<(), InputError>
    where
        F: FnOnce(&mut UinputDevice) -> Result<(), InputError> + Send + 'static,
    {
        self.throttle(cost).await?;
        let device = self.device().await?;
        tokio::task::spawn_blocking(move || {
            // FIXED: recover from poisoning instead of panicking. The guarded
            // state is just an open `/dev/uinput` file descriptor plus the
            // kernel-side device it created — every operation on it
            // (`emit`/`sync` in `device.rs`) is a sequential, self-contained
            // `write()`/ioctl with no multi-step invariant spanning the
            // `Mutex` that a panic partway through could leave torn. Worst
            // case, a panic mid-sequence (e.g. between a key-press emit and
            // its matching release in `keyboard::type_text`) leaves a key
            // logically "stuck down" at the kernel's evdev layer, which is a
            // synthesized-input correctness issue for that one call, not
            // corruption of the fd itself — the next call's `emit`/`sync`
            // still succeeds against the same still-valid device. Poisoning
            // the whole subsystem over that would make one panicking caller
            // permanently disable input synthesis for every other caller for
            // the rest of the daemon's life, which is worse than proceeding.
            let mut device = device
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            f(&mut device)
        })
        .await
        .expect("input synthesis blocking task panicked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(layout: &str) -> InputBackend {
        InputBackend::new(
            "wgaf test device",
            InputLimits {
                max_events_per_second: 0,
                max_type_text_chars: DEFAULT_MAX_TYPE_TEXT_CHARS,
                device_settle: Duration::ZERO,
            },
            layout,
        )
    }

    /// A combination is resolved in full before any key is pressed.
    ///
    /// Without this, `ctrl shift notakey` would press Ctrl and Shift and then
    /// fail — leaving both **held down**, which makes the session behave as
    /// though the user is leaning on them, with nothing to indicate why.
    /// Asserted through `device_created()`, which is false only if nothing was
    /// ever synthesized.
    #[tokio::test]
    async fn an_unknown_key_in_a_combination_presses_nothing() {
        let backend = backend(US_ASCII_LAYOUT);
        let err = backend
            .hotkey(&["ctrl".into(), "shift".into(), "notakey".into()])
            .await
            .expect_err("`notakey` is not a key");

        assert!(matches!(err, InputError::UnknownKey(_)), "got {err:?}");
        assert!(
            !backend.device_created(),
            "nothing should have been synthesized"
        );
    }

    #[tokio::test]
    async fn an_empty_combination_is_refused() {
        let err = backend(US_ASCII_LAYOUT)
            .hotkey(&[])
            .await
            .expect_err("a hotkey needs keys");
        assert!(matches!(err, InputError::EmptyHotkey), "got {err:?}");
    }

    /// `us-ascii` must not open a Wayland connection — it is the escape hatch
    /// for exactly the environments where reading a keymap is not possible.
    #[tokio::test]
    async fn the_us_ascii_mode_resolves_without_a_session() {
        let backend = backend(US_ASCII_LAYOUT);
        assert!(matches!(
            backend.typing().await.expect("should resolve"),
            Typing::UsAscii
        ));
        assert_eq!(
            backend.resolved_layout_name().as_deref(),
            Some("US ASCII (layout-independent table)")
        );
    }

    /// `wgaf status` must never resolve a layout as a side effect of being
    /// asked — the same rule that keeps it from creating a uinput device.
    #[tokio::test]
    async fn asking_for_the_layout_name_never_resolves_one() {
        let backend = backend("auto");
        assert_eq!(backend.resolved_layout_name(), None);
    }
}
