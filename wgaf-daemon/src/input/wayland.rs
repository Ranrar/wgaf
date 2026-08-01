//! Reading the session's keyboard keymap from the Wayland compositor.
//!
//! # One connection, once, at startup
//!
//! The daemon connects, binds a seat, takes the keymap the compositor hands
//! every keyboard client, and closes. It holds nothing afterwards and listens
//! for nothing.
//!
//! That is a consequence of a decision made elsewhere: the layout is resolved
//! once at startup and deliberately not tracked (see [`super::keymap`]), so the
//! keymap is needed exactly once. Keeping a connection alive would mean running
//! `wayland-client`'s event loop alongside the tokio runtime — real integration
//! work, with its own failure modes — to observe changes that are then ignored
//! by design. If live keymap updates are ever wanted, that is the moment to
//! answer the event-loop question, not before.
//!
//! # Why Wayland rather than something easier
//!
//! The compositor's own keymap is the authority on what a keystroke will
//! produce, and it arrives on bind rather than on focus, so a surfaceless
//! client can read it. Two alternatives were rejected: routing it through the
//! GNOME Shell extension would couple *input correctness* to the extension
//! being installed, when input works without it today; and reading
//! `org.gnome.desktop.input-sources` is GNOME-only and ignores `xkb-options`.
//!
//! **What this cannot get is which layout is currently active.** That index
//! arrives in `wl_keyboard.modifiers`, which Mutter sends only to a client with
//! keyboard focus — measured, not assumed. A headless daemon never receives it,
//! and taking focus to find out is exactly the hazard this project avoids. The
//! choice of layout is therefore configuration, handled in [`super::keymap`].
//!
//! # Deliberately thin
//!
//! Nothing here can run in CI, since it needs a live Wayland session. So it
//! contains no logic worth testing: it fetches a keymap or reports why it
//! could not. Everything that decides anything lives in [`super::keymap`] and
//! [`super::xkb`], which are tested against keymaps compiled from names.

use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use xkbcommon::xkb;

/// How long to wait for the compositor to deliver a keymap before giving up.
///
/// The keymap normally arrives within a couple of round trips. This is a
/// backstop so a compositor that advertises a seat and then says nothing cannot
/// hang daemon startup indefinitely.
const KEYMAP_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the session's keymap could not be read.
///
/// Every variant is recoverable from the daemon's point of view: window and
/// accessibility commands need no keymap, so this degrades to `wgaf type`
/// failing with a reason rather than the daemon refusing to start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KeymapReadError {
    #[error(
        "not running under Wayland, or the compositor is unreachable ({0}). \
         `wgaf type` needs the session's keyboard layout; other commands are \
         unaffected."
    )]
    NoConnection(String),

    #[error("the Wayland compositor reported no keyboard on any seat")]
    NoKeyboard,

    #[error("the compositor sent no keymap within {}s", KEYMAP_TIMEOUT.as_secs())]
    TimedOut,

    #[error(
        "the compositor sent a keyboard keymap in a format wgaf does not \
         understand, so the layout could not be read"
    )]
    UnsupportedFormat,

    #[error("the session's keymap could not be compiled: {0}")]
    Invalid(String),

    #[error("Wayland protocol error while reading the keymap: {0}")]
    Protocol(String),
}

/// What the dispatch handlers accumulate while the queue is pumped.
#[derive(Default)]
struct Collector {
    seat: Option<wl_seat::WlSeat>,
    /// Whether the seat has told us what it can do yet. Binding a seat and
    /// hearing back are separate round trips, so "no keyboard" cannot be
    /// concluded from the seat existing.
    capabilities_seen: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keymap: Option<xkb::Keymap>,
    /// Set when the compositor sent a keymap we could not use. Kept separate
    /// from `keymap` so the caller can tell "nothing arrived" from "something
    /// arrived and was unusable".
    failure: Option<KeymapReadError>,
}

/// Connects to the compositor, reads the keyboard keymap, and disconnects.
pub(crate) fn read_session_keymap() -> Result<xkb::Keymap, KeymapReadError> {
    let conn =
        Connection::connect_to_env().map_err(|e| KeymapReadError::NoConnection(e.to_string()))?;

    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut collector = Collector::default();
    let deadline = Instant::now() + KEYMAP_TIMEOUT;

    // Each round trip advances one step: globals, then seat capabilities, then
    // the keymap. Looping rather than counting round trips, because how many it
    // takes is the compositor's business, not ours.
    while collector.keymap.is_none() && collector.failure.is_none() {
        if Instant::now() >= deadline {
            return Err(if collector.keyboard.is_some() {
                KeymapReadError::TimedOut
            } else {
                KeymapReadError::NoKeyboard
            });
        }

        queue
            .roundtrip(&mut collector)
            .map_err(|e| KeymapReadError::Protocol(e.to_string()))?;

        // The seat has told us what it can do, and a keyboard is not among it:
        // no amount of further waiting will produce one. Checked on
        // `capabilities_seen` rather than on the seat existing, because binding
        // the seat and hearing its capabilities are different round trips.
        if collector.capabilities_seen && collector.keyboard.is_none() {
            return Err(KeymapReadError::NoKeyboard);
        }
    }

    if let Some(err) = collector.failure {
        return Err(err);
    }

    // Release the keyboard explicitly rather than leaving it to connection
    // teardown, so the compositor stops considering us a keyboard client the
    // moment we have what we came for.
    if let Some(keyboard) = &collector.keyboard {
        keyboard.release();
    }
    let _ = queue.roundtrip(&mut collector);

    collector.keymap.ok_or(KeymapReadError::TimedOut)
}

impl Dispatch<wl_registry::WlRegistry, ()> for Collector {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };

        // One seat is enough — the first with a keyboard answers the question.
        if interface == wl_seat::WlSeat::interface().name && state.seat.is_none() {
            // Version 7 is where `wl_keyboard` requires the keymap be mapped
            // MAP_PRIVATE, which is what the keymap reader does.
            let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ());
            state.seat = Some(seat);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Collector {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };

        state.capabilities_seen = true;

        if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Collector {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `Keymap` is the only event of interest. `Enter`, `Key` and
        // `Modifiers` never arrive for a surfaceless client anyway, and are
        // ignored rather than handled.
        let wl_keyboard::Event::Keymap { format, fd, size } = event else {
            return;
        };

        if state.keymap.is_some() || state.failure.is_some() {
            return;
        }

        if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
            state.failure = Some(KeymapReadError::UnsupportedFormat);
            return;
        }

        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        // SAFETY: `fd` and `size` come straight from the compositor's keymap
        // event and describe a region it has just published for us to map. The
        // fd is owned here, so nothing else closes it; `new_from_fd` maps it
        // copy-on-write and does not retain it past the call.
        let compiled = unsafe {
            xkb::Keymap::new_from_fd(
                &ctx,
                fd,
                size as usize,
                xkb::KEYMAP_FORMAT_TEXT_V1,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
        };

        match compiled {
            Ok(Some(keymap)) => state.keymap = Some(keymap),
            Ok(None) => state.failure = Some(KeymapReadError::Invalid("compile failed".into())),
            Err(e) => state.failure = Some(KeymapReadError::Invalid(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real session keymap end to end.
    ///
    /// `#[ignore]`d: it needs a live Wayland session, so an ordinary
    /// `cargo test` must not depend on one. Run with
    /// `cargo test --bins -- --ignored reads_the_real_session_keymap`.
    #[test]
    #[ignore = "needs a live Wayland session"]
    fn reads_the_real_session_keymap() {
        let keymap = read_session_keymap().expect("should read the session keymap");

        let layouts = crate::input::keymap::available_layouts(&keymap);
        assert!(!layouts.is_empty(), "a keymap with no layouts");

        let index = crate::input::keymap::resolve(&keymap, crate::input::keymap::AUTO)
            .expect("auto should always resolve");

        let map = crate::input::xkb::LayoutMap::build(
            &keymap,
            index,
            crate::input::codes::registered_codes(),
        );

        println!("layouts: {layouts:?}");
        println!("auto -> [{index}] {}", map.layout_name());
        println!("characters typeable: {}", map.len());

        let missing: Vec<char> = (0x20u8..0x7f)
            .map(char::from)
            .filter(|c| map.strokes(*c).is_none())
            .collect();
        println!("printable ASCII missing: {missing:?}");

        for c in ['@', '$', '{', '~', '|', '\\'] {
            println!("  {c:?} -> {:?}", map.strokes(c));
        }

        assert!(
            missing.is_empty(),
            "this session's layout cannot type {missing:?}"
        );
    }
}
