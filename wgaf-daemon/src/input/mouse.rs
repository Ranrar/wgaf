//! Mouse primitives: relative pointer movement, button click, and scroll.
//!
//! Absolute positioning is deliberately not offered — Wayland has no
//! protocol giving a client (or this daemon, via `uinput`) a reliable
//! global authority for "screen coordinates" the way X11's root window did;
//! a compositor may have multiple outputs at arbitrary offsets/scales with
//! no stable shared origin. Relative motion (`REL_X`/`REL_Y`) is the
//! mechanism `uinput` actually supports meaningfully here, so it's the only
//! one implemented, per this session's design decision and the roadmap's
//! own note on the same limitation.

use crate::input::InputError;
use crate::input::codes::{self, REL_HWHEEL, REL_WHEEL};
use crate::input::device::UinputDevice;

/// Moves the pointer by `(dx, dy)` relative to its current position.
pub(crate) fn move_relative(device: &mut UinputDevice, dx: i32, dy: i32) -> Result<(), InputError> {
    device.rel_move(dx, dy)
}

/// Resolves a button name (`left`/`right`/`middle`) via
/// [`codes::button_name_to_code`], or [`InputError::InvalidButton`] naming
/// the input verbatim.
pub(crate) fn resolve_button(name: &str) -> Result<u16, InputError> {
    codes::button_name_to_code(name).ok_or_else(|| InputError::InvalidButton(name.to_string()))
}

/// Clicks (press then release) the given button code.
pub(crate) fn click(device: &mut UinputDevice, button_code: u16) -> Result<(), InputError> {
    device.key_event(button_code, 1)?;
    device.key_event(button_code, 0)
}

/// Scrolls: `dy` is vertical (`REL_WHEEL`, positive = up, matching the
/// kernel's own convention), `dx` is horizontal (`REL_HWHEEL`, positive =
/// right). Either may be `0`; both are always emitted, for a simple,
/// predictable contract rather than skipping zero axes.
///
/// The two axes go out as two separately-`SYN_REPORT`-terminated events, not
/// one batch — see the implementation note below for why that's acceptable
/// here (unlike [`move_relative`], where X and Y must land atomically).
pub(crate) fn scroll(device: &mut UinputDevice, dx: i32, dy: i32) -> Result<(), InputError> {
    // Two separate emit calls that need to land in the same SYN batch: reuse
    // the raw `rel_event`/sync pattern via two calls plus a shared final
    // sync would double-sync, so build it the same way `rel_move` does but
    // for the wheel axes. `UinputDevice` only exposes single-axis
    // `rel_event` (auto-syncing) and the X/Y-specific `rel_move`, so for two
    // wheel axes we accept two small, separately-synced writes rather than
    // adding a third bespoke "batch of two arbitrary axes" method — scroll
    // events aren't required to be atomic the way a single pointer move is.
    device.rel_event(REL_HWHEEL, dx)?;
    device.rel_event(REL_WHEEL, dy)
}
