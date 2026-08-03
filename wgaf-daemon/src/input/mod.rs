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
use tokio::sync::{OnceCell, watch};
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

    /// The kill switch is engaged: someone stopped wgaf, and nothing will be
    /// synthesized until they say otherwise.
    ///
    /// **The message names `wgaf release`** because the state outlives the
    /// emergency that caused it. Someone who stopped wgaf during a runaway
    /// script an hour ago, and now finds `wgaf type` doing nothing, needs the
    /// failure itself to point at the way back — nothing else on screen will.
    #[error(
        "input is stopped — wgaf's kill switch is engaged and no input will be synthesized. \
         Run `wgaf release` to allow input again."
    )]
    Stopped,

    /// Any other I/O failure writing to the `uinput` device after it was
    /// successfully created (e.g. it disappeared underneath us).
    #[error("I/O error writing to the uinput device: {0}")]
    Io(#[from] std::io::Error),
}

/// Returns [`InputError::Stopped`] if the kill switch is engaged.
///
/// Free function rather than a method so the emit loops in [`keyboard`] can
/// call it: they run on a blocking-pool thread with nothing but the flag,
/// deliberately, since holding a `&InputBackend` there would mean holding the
/// backend across the whole synthesis.
pub(crate) fn check_not_stopped(stopped: &AtomicBool) -> Result<(), InputError> {
    match stopped.load(Ordering::SeqCst) {
        true => Err(InputError::Stopped),
        false => Ok(()),
    }
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
    /// `None` until the first synthesis creates the device, and `None` again
    /// after [`Self::stop`] drops it. An initialization failure (permissions)
    /// leaves it `None`, so the next call retries rather than requiring a
    /// daemon restart — the property `windows::WindowManager`'s
    /// `verified: OnceCell<()>` gets from not caching failures.
    ///
    /// **Not a `OnceCell`, and that is the kill switch's doing.** The device
    /// has to be *droppable*: stopping wgaf must leave no virtual keyboard or
    /// pointer registered with the kernel, and a cell that can only be filled
    /// once cannot express that. Creation therefore happens inside the lock,
    /// on the blocking-pool thread that is about to use it.
    device: Arc<Mutex<Option<UinputDevice>>>,
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
    /// The kill switch. `true` refuses every synthesis until [`Self::release`].
    ///
    /// An `Arc` because it is read from inside the blocking closures that emit
    /// events, per keystroke — a stop that only took effect between D-Bus calls
    /// would let a 16,000-event `TypeText` run to completion after the user hit
    /// the panic key, which is the exact case this exists for.
    ///
    /// In memory only, never persisted: it represents a live emergency, and one
    /// that survived a reboot would turn a five-second panic into a mystery
    /// next week.
    stopped: Arc<AtomicBool>,
    /// Resolved from `config.toml` at startup; see [`InputLimits`].
    limits: InputLimits,
    /// Broadcasts whether a virtual device exists, so the GNOME Shell Extension
    /// can hold its emergency-key grab **only while wgaf can actually type**.
    ///
    /// The extension used to register that shortcut for the whole session,
    /// which took `Escape` away from every application on the desktop even
    /// while the daemon was idle or absent. Arming it from this signal is what
    /// makes bare `Escape` an acceptable default instead of a desktop-wide tax.
    ///
    /// A `watch` and not a callback: it holds the *current* value as well as
    /// notifying on change, so a subscriber that connects late still learns the
    /// truth rather than waiting for the next edge.
    /// `Arc` because the "device now exists" edge is published from inside the
    /// blocking closure that creates it, and `watch::Sender` is not `Clone`.
    device_present: Arc<watch::Sender<bool>>,
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
            device: Arc::new(Mutex::new(None)),
            device_name: device_name.into(),
            layout_spec: keyboard_layout.into(),
            layout: OnceCell::new(),
            limiter: Mutex::new(TokenBucket::new(
                limits.max_events_per_second,
                MAX_THROTTLE_DELAY,
                Instant::now(),
            )),
            throttle_reported: AtomicBool::new(false),
            stopped: Arc::new(AtomicBool::new(false)),
            limits,
            // Starts `false` and correctly so: `new` deliberately does not
            // touch `/dev/uinput`, so no device exists yet.
            device_present: Arc::new(watch::channel(false).0),
        }
    }

    /// Subscribes to "does a virtual device exist right now".
    ///
    /// The receiver carries the current value immediately, so a subscriber
    /// never has to assume a starting state — see [`Self::device_present`].
    pub fn device_present_watch(&self) -> watch::Receiver<bool> {
        self.device_present.subscribe()
    }

    /// Republishes the device's existence after it may have changed.
    ///
    /// Reads the real state rather than tracking it, so the signal cannot drift
    /// from the slot it describes — a creation that failed halfway, or a device
    /// dropped by `stop` while a synthesis was queued, both report honestly.
    /// `send_if_modified` keeps this silent on the overwhelmingly common path
    /// where nothing changed, so the hot synthesis path costs one comparison.
    fn republish_device_presence(&self) {
        let present = self.device_created();
        self.device_present.send_if_modified(|current| {
            if *current == present {
                false
            } else {
                *current = present;
                true
            }
        });
    }

    /// Engages the kill switch: refuses every further synthesis and takes the
    /// virtual device away from the kernel.
    ///
    /// **One-way.** Calling it twice stops harder than once in no respect at
    /// all, and nothing here ever clears the flag — coming back is always an
    /// explicit [`Self::release`]. During a failure the user may well hit the
    /// panic key repeatedly, and a second press that restarted the automation
    /// they were trying to kill would be the worst possible reading of it.
    ///
    /// The order matters. The flag is set *first*, so an in-flight `TypeText`
    /// sees it on its next keystroke and returns [`InputError::Stopped`] rather
    /// than discovering a device that vanished mid-string; only then is the
    /// device dropped, which issues `UI_DEV_DESTROY` and leaves no virtual
    /// keyboard or pointer registered.
    pub async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        tracing::warn!(
            target: AUDIT_TARGET,
            action = "stop",
            "kill switch engaged: refusing all input synthesis until `wgaf release`"
        );

        // On the blocking pool because the lock is held for the whole of any
        // synthesis currently running through the device. That synthesis is
        // already unwinding — it checks the flag set above per keystroke — but
        // "already unwinding" is not "instant", and the async executor must not
        // be the thing that waits for it.
        let device = Arc::clone(&self.device);
        tokio::task::spawn_blocking(move || {
            let mut slot = device
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // `UinputDevice::drop` issues UI_DEV_DESTROY.
            *slot = None;
        })
        .await
        .expect("input stop task panicked");

        // After the device is gone, never before: the extension drops its
        // emergency-key grab on this signal, and announcing the drop early
        // would open a window where wgaf could still type and the panic key
        // no longer worked.
        self.republish_device_presence();
    }

    /// Releases the kill switch, allowing synthesis again.
    ///
    /// **Releasing is not resuming**, and the name is the difference: nothing
    /// that was interrupted is continued or replayed. The daemon keeps no
    /// record of what it was told to type, and the caller that was flooding
    /// already received [`InputError::Stopped`] and gave up. This lifts the
    /// brake; driving again is the caller's business.
    ///
    /// The virtual device is recreated lazily by the next synthesis, exactly as
    /// it was on the first one.
    pub fn release(&self) {
        self.stopped.store(false, Ordering::SeqCst);
        tracing::info!(
            target: AUDIT_TARGET,
            action = "release",
            "kill switch released: input synthesis is allowed again"
        );
    }

    /// Whether the kill switch is currently engaged. Reported by
    /// `org.wgaf.Daemon1.Status`.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
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

    /// Whether the daemon holds a live virtual device right now.
    ///
    /// An *activity* signal, not a health one — `false` means either that
    /// nothing has synthesized input yet, or that [`Self::stop`] took the
    /// device away. Never creates one as a side effect of asking.
    ///
    /// `try_lock` rather than `lock`: the lock is held for the duration of a
    /// synthesis, and `wgaf status` must not block behind a 4,000-character
    /// `TypeText` to answer a yes/no question. Held *is* the answer anyway —
    /// only a synthesis holds it, and a synthesis has a device.
    pub fn device_created(&self) -> bool {
        match self.device.try_lock() {
            Ok(slot) => slot.is_some(),
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner().is_some(),
        }
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
        // Every synthesis method is checked against the kill switch by
        // [`Self::run`], the funnel they all pass through — so there is exactly
        // one place to forget, and a method that forgets it is a method that
        // does not synthesize. This one is checked here as well because it is
        // the only one that does real work *before* reaching the funnel:
        // resolving the layout can open a Wayland connection, and a stopped
        // daemon should say it is stopped rather than report whatever that
        // turns up.
        check_not_stopped(&self.stopped)?;

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
        // The flag travels into the emit loop, not just into `run`: this is the
        // one call that can be tens of thousands of events long, and a stop
        // checked only on the way in would let the whole flood finish.
        let stopped = Arc::clone(&self.stopped);
        self.run(cost, move |device| {
            keyboard::type_planned(device, &plan, &stopped)
        })
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
        let stopped = Arc::clone(&self.stopped);
        self.run(cost, move |device| {
            keyboard::hotkey(device, &codes, &stopped)
        })
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
        // Twice, deliberately. The first check refuses a call that arrives
        // while stopped without spending rate-limiter budget on it; the second
        // catches a stop that landed during the throttle sleep, which can be up
        // to thirty seconds long.
        check_not_stopped(&self.stopped)?;
        self.throttle(cost).await?;
        check_not_stopped(&self.stopped)?;

        let device = Arc::clone(&self.device);
        let stopped = Arc::clone(&self.stopped);
        let name = self.device_name.clone();
        let settle = self.limits.device_settle;
        let device_present = Arc::clone(&self.device_present);
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
            let mut slot = device
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            // The last check before anything is created or emitted, and the
            // only one that cannot be raced: `stop` sets the flag and then
            // takes this same lock to drop the device, so a stop that arrives
            // while this call is queued is visible here — and no device is
            // created afterwards to leave registered.
            check_not_stopped(&stopped)?;

            if slot.is_none() {
                *slot = Some(UinputDevice::create(&name, settle)?);
                // Announced here rather than after this call returns, and the
                // ordering is a safety property: the extension arms the
                // emergency key on this edge, and `UinputDevice::create` has
                // just spent its settle wait letting the compositor notice the
                // device. Publishing after the first keystroke instead would
                // leave the opening events of a runaway unprotected — exactly
                // the ones a panic press needs to catch.
                device_present.send_replace(true);
            }
            let device = slot
                .as_mut()
                .expect("the device was just created if it was absent");

            // Creating the device waits for the compositor to notice it, which
            // is a third of a second the check above cannot see past. A stop
            // that arrived during it must not be answered with one last
            // keystroke — the emit loops re-check per keystroke, but a single
            // `KeyPress` or `MouseClick` has no loop to catch it.
            check_not_stopped(&stopped)?;
            f(device)
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

    /// A stopped backend refuses every kind of synthesis, and registers no
    /// virtual device while doing so — a panic stop that left a keyboard behind
    /// would leave the user with the thing they were trying to be rid of.
    #[tokio::test]
    async fn a_stopped_backend_refuses_synthesis_and_creates_no_device() {
        let backend = backend(US_ASCII_LAYOUT);
        backend.stop().await;
        assert!(backend.is_stopped());

        for result in [
            backend.type_text("hello").await,
            backend.key_press("a").await,
            backend.key_release("a").await,
            backend.hotkey(&["ctrl".into(), "t".into()]).await,
            backend.mouse_move(1, 1).await,
            backend.mouse_click("left").await,
            backend.mouse_scroll(0, 1).await,
        ] {
            let err = result.expect_err("a stopped backend synthesizes nothing");
            assert!(matches!(err, InputError::Stopped), "got {err:?}");
        }

        assert!(
            !backend.device_created(),
            "refusing a call must not create the device it refused to use"
        );
    }

    /// Stopping twice is not a toggle. Someone in a panic hits the shortcut
    /// repeatedly, and the second press must not restart the automation the
    /// first one stopped.
    #[tokio::test]
    async fn stopping_twice_stays_stopped() {
        let backend = backend(US_ASCII_LAYOUT);
        backend.stop().await;
        backend.stop().await;
        assert!(backend.is_stopped());
        assert!(matches!(
            backend.mouse_move(1, 1).await,
            Err(InputError::Stopped)
        ));
    }

    /// `release` lifts the refusal.
    ///
    /// Asserted on the flag rather than on a successful call: proving synthesis
    /// works again means creating a real `uinput` device, which a unit test has
    /// no business registering with the kernel. `tests/kill_switch.rs` makes
    /// that assertion against a real daemon.
    #[tokio::test]
    async fn release_lifts_the_refusal() {
        let backend = backend(US_ASCII_LAYOUT);
        backend.stop().await;
        backend.release();
        assert!(!backend.is_stopped());
    }
}
