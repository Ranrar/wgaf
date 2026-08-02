//! Monitor layout, read from Mutter's own `org.gnome.Mutter.DisplayConfig`.
//!
//! This is the daemon's authority on "is this coordinate on a screen", which
//! absolute pointer positioning needs and nothing else in wgaf previously had.
//!
//! # Why not an extension method
//!
//! An earlier design put `getMonitors` on the wgaf extension alongside
//! `warpPointer`. It does not belong there. `DisplayConfig` is a native GNOME
//! API, which this project's decision rules rank above anything we would write
//! ourselves; reading it directly keeps a third method off the extension's
//! surface (and therefore out of the availability check and the drift test);
//! and it keeps working when the wgaf extension is not installed at all, so the
//! daemon can report a display layout regardless.
//!
//! The cost is honest: `DisplayConfig` is a *configuration* interface — the one
//! `gnome-control-center` drives — not a query API meant for automation. Its
//! reply carries every mode of every output when all we want is the logical
//! layout, and it is Mutter's own interface rather than a contract offered to
//! third parties. Verified working from an ordinary session process with no
//! special permission on GNOME 50.1 / Mutter 18.
//!
//! # Why the pointer path needs this at all
//!
//! Mutter **silently clamps** a warp to a coordinate that is not on any
//! monitor: no error, no signal, the pointer simply ends up somewhere else and
//! the caller is told the move succeeded. Measured 2026-08-02. So the check has
//! to happen here, before the warp is issued, because afterwards the
//! information that the coordinate was invalid no longer exists.
//!
//! # The bounding box is not the layout
//!
//! Monitors are checked individually and never merged into one rectangle. On
//! the maintainer's real layout — a rotated 1080x1920 panel at (0,0) with a
//! 2560x1440 primary at (1080,0) — the desktop is L-shaped, and the region
//! below y=1440 on the right is inside the bounding box while being on no
//! monitor at all. A bounding-box check accepts `(2000, 1700)` and passes it to
//! a compositor that will quietly clamp it.

use std::collections::HashMap;

use thiserror::Error;
use zbus::zvariant::OwnedValue;

/// Mutter's D-Bus name for the display configuration service.
///
/// Duplicated in the `#[zbus::proxy]` attribute below, which requires string
/// literals. This copy exists because the owner check needs the name as a
/// value, and because it is `Config`'s default; a test pins them together.
pub const DEFAULT_BUS_NAME: &str = "org.gnome.Mutter.DisplayConfig";

/// Object path the display configuration lives at. Not configurable — only the
/// bus name is, so a stub can be reached; a stub serves the same path.
const OBJECT_PATH: &str = "/org/gnome/Mutter/DisplayConfig";

/// One monitor's modes, as `GetCurrentState` reports them.
///
/// `((ssss)a(siiddada{sv})a{sv})`: the connector/vendor/product/serial tuple,
/// the available modes, and the monitor's properties.
type MonitorInfo = (
    (String, String, String, String),
    Vec<ModeInfo>,
    HashMap<String, OwnedValue>,
);

/// One display mode: id, width, height, refresh rate, preferred scale, the
/// supported scales, and properties (which carry `is-current`).
type ModeInfo = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);

/// One logical monitor: position, scale, transform, primary flag, the physical
/// monitors composing it, and properties.
type LogicalMonitorInfo = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, String, String)>,
    HashMap<String, OwnedValue>,
);

/// `GetCurrentState`'s full reply: serial, monitors, logical monitors, and
/// global properties.
type CurrentState = (
    u32,
    Vec<MonitorInfo>,
    Vec<LogicalMonitorInfo>,
    HashMap<String, OwnedValue>,
);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig"
)]
trait DisplayConfig {
    /// The current display configuration: which monitors exist, their modes,
    /// and how they are arranged into logical monitors.
    fn get_current_state(&self) -> zbus::Result<CurrentState>;
}

/// Failures reading the monitor layout.
#[derive(Debug, Error)]
pub enum DisplayConfigError {
    /// Mutter's display-configuration service is not on the bus. On a GNOME
    /// session it always is, so this means the daemon is running somewhere it
    /// cannot do absolute positioning — a different compositor, or no session.
    #[error(
        "Mutter's display configuration service is unavailable (`{bus_name}`) — absolute \
         pointer positioning needs the monitor layout, which only the compositor knows. Is this \
         a GNOME session?"
    )]
    Unavailable { bus_name: String },

    /// The service answered but the reply could not be understood, or
    /// described no usable monitor.
    #[error("could not read the monitor layout from Mutter: {0}")]
    Unreadable(String),

    /// Any other D-Bus-level failure.
    #[error("D-Bus error reading the monitor layout: {0}")]
    DBus(#[from] zbus::Error),
}

/// One logical monitor's position and size, in the same global logical pixel
/// space as `Meta.Window.get_frame_rect()` and `warp_pointer`.
///
/// That the three agree is measured, not assumed: seven warps to known offsets
/// inside a window produced a client-reported position equal to
/// `target - frame_rect origin` exactly, on 2026-08-02. Measured on scale-1.0
/// monitors only.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorRect {
    /// Connector name, e.g. `DP-3`. Only used for diagnostics.
    pub connector: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    /// Mutter's transform enum. Odd values are the 90°/270° rotations, for
    /// which the logical size has width and height swapped relative to the
    /// mode's.
    pub transform: u32,
    pub primary: bool,
}

impl MonitorRect {
    /// Whether this monitor covers the given point.
    ///
    /// Half-open on the far edges, as screen rectangles conventionally are: a
    /// 2560-wide monitor at x=1080 covers x=1080..=3639, not 3640. The spike
    /// confirmed `(3639, 1439)` is warpable on this layout and that x=3640
    /// would be the neighbouring pixel column, which does not exist.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// The set of logical monitors currently making up the desktop.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorLayout {
    monitors: Vec<MonitorRect>,
}

impl MonitorLayout {
    pub fn new(monitors: Vec<MonitorRect>) -> Self {
        Self { monitors }
    }

    pub fn monitors(&self) -> &[MonitorRect] {
        &self.monitors
    }

    /// Whether the point is on any monitor.
    ///
    /// Deliberately *not* a bounding-box test — see this module's header for
    /// the L-shaped layout that makes the difference observable.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.monitors.iter().any(|m| m.contains(x, y))
    }

    /// A one-line description of the layout, for the `OutOfBounds` error.
    ///
    /// A user who is told only "out of bounds" has to go and find out what the
    /// bounds are; a user told the actual rectangles can see immediately that
    /// they aimed at a gap between two monitors.
    pub fn describe(&self) -> String {
        if self.monitors.is_empty() {
            return "no monitors".to_string();
        }
        self.monitors
            .iter()
            .map(|m| {
                format!(
                    "{} {}x{} at ({},{})",
                    m.connector, m.width, m.height, m.x, m.y
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Reads the monitor layout from Mutter.
pub struct DisplayConfig {
    proxy: DisplayConfigProxy<'static>,
    connection: zbus::Connection,
    bus_name: String,
}

impl DisplayConfig {
    /// Builds a client against a given bus name. Does not contact Mutter — the
    /// first `read` does, so a daemon started before its session is ready is
    /// not fatally broken.
    pub async fn connect(
        connection: zbus::Connection,
        bus_name: &str,
    ) -> Result<Self, DisplayConfigError> {
        let proxy = DisplayConfigProxy::builder(&connection)
            .destination(bus_name.to_string())?
            .path(OBJECT_PATH)?
            .build()
            .await?;
        Ok(Self {
            proxy,
            connection,
            bus_name: bus_name.to_string(),
        })
    }

    /// Fetches the current layout.
    pub async fn read(&self) -> Result<MonitorLayout, DisplayConfigError> {
        // Distinguish "not a GNOME session" from "the call failed", the same
        // way `WindowManager::check_extension_version` separates a missing
        // extension from a broken one. A raw method-call failure would be an
        // unhelpful way to learn the compositor is not Mutter.
        let dbus_proxy = zbus::fdo::DBusProxy::new(&self.connection).await?;
        let name = self.bus_name.as_str().try_into().map_err(|_| {
            DisplayConfigError::Unreadable(format!("`{}` is not a valid bus name", self.bus_name))
        })?;
        if !dbus_proxy
            .name_has_owner(name)
            .await
            .map_err(zbus::Error::from)?
        {
            return Err(DisplayConfigError::Unavailable {
                bus_name: self.bus_name.clone(),
            });
        }

        let (_serial, monitors, logical, _props) = self.proxy.get_current_state().await?;
        let layout = MonitorLayout::new(logical_monitors_to_rects(&monitors, &logical));

        if layout.monitors().is_empty() {
            return Err(DisplayConfigError::Unreadable(
                "Mutter reported no logical monitors".to_string(),
            ));
        }
        Ok(layout)
    }
}

/// The current mode's pixel size for a connector, if it has one.
fn current_mode_size(monitors: &[MonitorInfo], connector: &str) -> Option<(i32, i32)> {
    let monitor = monitors.iter().find(|(id, _, _)| id.0 == connector)?;
    monitor
        .1
        .iter()
        .find(|(_, _, _, _, _, _, props)| {
            props
                .get("is-current")
                .and_then(|v| bool::try_from(v.try_clone().ok()?).ok())
                .unwrap_or(false)
        })
        .map(|(_, width, height, _, _, _, _)| (*width, *height))
}

/// Converts `GetCurrentState`'s two parallel arrays into plain rectangles.
///
/// A logical monitor carries its position, scale and transform but **not its
/// size** — the size has to be recovered from the current mode of the physical
/// monitor(s) composing it, then divided by the scale and rotated. Missing that
/// step yields monitors of size zero, which pass no bounds check at all and
/// would make every warp fail as out of bounds.
fn logical_monitors_to_rects(
    monitors: &[MonitorInfo],
    logical: &[LogicalMonitorInfo],
) -> Vec<MonitorRect> {
    logical
        .iter()
        .filter_map(|(x, y, scale, transform, primary, outputs, _props)| {
            let connector = outputs.first().map(|o| o.0.clone())?;
            let (mode_width, mode_height) = current_mode_size(monitors, &connector)?;
            let (width, height) = logical_size(mode_width, mode_height, *scale, *transform);
            Some(MonitorRect {
                connector,
                x: *x,
                y: *y,
                width,
                height,
                scale: *scale,
                transform: *transform,
                primary: *primary,
            })
        })
        .collect()
}

/// A mode's pixel size expressed in logical pixels: divided by the scale, and
/// with width/height swapped for the quarter-turn transforms.
///
/// Mutter's transform enum is normal/90/180/270 followed by the same four
/// flipped, so the odd values are exactly the quarter turns.
fn logical_size(mode_width: i32, mode_height: i32, scale: f64, transform: u32) -> (i32, i32) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let width = (f64::from(mode_width) / scale).round() as i32;
    let height = (f64::from(mode_height) / scale).round() as i32;
    if transform % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The maintainer's real layout, measured from `GetCurrentState` on
    /// 2026-08-02. Used rather than a tidy invented one because its L shape is
    /// what makes a bounding-box bug visible; a side-by-side pair would not.
    fn real_layout() -> MonitorLayout {
        MonitorLayout::new(vec![
            MonitorRect {
                connector: "HDMI-1".to_string(),
                x: 0,
                y: 0,
                width: 1080,
                height: 1920,
                scale: 1.0,
                transform: 1,
                primary: false,
            },
            MonitorRect {
                connector: "DP-3".to_string(),
                x: 1080,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                transform: 0,
                primary: true,
            },
        ])
    }

    /// The proxy attribute needs a string literal, so the bus name is written
    /// twice. If they ever drift, the owner check would test a different
    /// service than the one the proxy dials — and the symptom would be
    /// "unavailable" on a session where it is running fine.
    #[test]
    fn the_bus_name_constant_matches_the_proxy_attribute() {
        assert_eq!(DEFAULT_BUS_NAME, "org.gnome.Mutter.DisplayConfig");
        assert_eq!(OBJECT_PATH, "/org/gnome/Mutter/DisplayConfig");
    }

    #[test]
    fn a_quarter_turn_swaps_the_logical_size() {
        // HDMI-1: a 1920x1080 panel rotated 90 degrees presents as 1080x1920.
        assert_eq!(logical_size(1920, 1080, 1.0, 1), (1080, 1920));
        assert_eq!(logical_size(1920, 1080, 1.0, 3), (1080, 1920));
        assert_eq!(logical_size(1920, 1080, 1.0, 0), (1920, 1080));
        assert_eq!(logical_size(1920, 1080, 1.0, 2), (1920, 1080));
    }

    #[test]
    fn scale_divides_the_mode_size() {
        assert_eq!(logical_size(3840, 2160, 2.0, 0), (1920, 1080));
        assert_eq!(logical_size(2560, 1440, 1.25, 0), (2048, 1152));
    }

    /// A zero or negative scale would divide the layout into nonsense, and the
    /// resulting monitor would reject every coordinate on it.
    #[test]
    fn a_nonsense_scale_falls_back_to_one_rather_than_producing_a_zero_monitor() {
        assert_eq!(logical_size(1920, 1080, 0.0, 0), (1920, 1080));
    }

    #[test]
    fn points_on_either_monitor_are_in_bounds() {
        let layout = real_layout();
        assert!(layout.contains(0, 0), "top-left of the rotated monitor");
        assert!(layout.contains(500, 1700), "low on the rotated monitor");
        assert!(layout.contains(1500, 700), "middle of the primary");
        assert!(layout.contains(3639, 1439), "bottom-right of the primary");
        assert!(
            layout.contains(1079, 1919),
            "bottom-right of the rotated one"
        );
    }

    /// The case this whole module exists for. `(2000, 1700)` is inside the
    /// layout's bounding box (0,0)-(3640,1920) and on no monitor: the primary
    /// stops at y=1440 and the rotated panel stops at x=1080. A bounding-box
    /// check accepts it and Mutter then silently clamps the pointer to
    /// (2000, 1439).
    #[test]
    fn the_notch_between_monitors_is_out_of_bounds() {
        let layout = real_layout();
        assert!(!layout.contains(2000, 1700), "the notch must be rejected");
        assert!(!layout.contains(1200, 1500));
        assert!(!layout.contains(3639, 1919));
    }

    #[test]
    fn coordinates_outside_every_monitor_are_out_of_bounds() {
        let layout = real_layout();
        assert!(!layout.contains(-1, 500), "negative x");
        assert!(!layout.contains(500, -1), "negative y");
        assert!(!layout.contains(3640, 700), "one past the right edge");
        assert!(!layout.contains(1080, 1920), "one past the bottom edge");
        assert!(!layout.contains(99999, 99999));
    }

    /// Making the right-hand monitor primary in this setup puts the other at a
    /// negative x. Nothing in the bounds logic may assume the origin is the
    /// top-left of the desktop.
    #[test]
    fn a_layout_with_negative_coordinates_works() {
        let layout = MonitorLayout::new(vec![
            MonitorRect {
                connector: "HDMI-1".to_string(),
                x: -1080,
                y: 0,
                width: 1080,
                height: 1920,
                scale: 1.0,
                transform: 1,
                primary: false,
            },
            MonitorRect {
                connector: "DP-3".to_string(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                transform: 0,
                primary: true,
            },
        ]);
        assert!(layout.contains(-1080, 0), "top-left of the left monitor");
        assert!(
            layout.contains(-1, 1919),
            "bottom-right of the left monitor"
        );
        assert!(layout.contains(0, 0));
        assert!(!layout.contains(-1081, 0), "one past the left edge");
        assert!(!layout.contains(-1, 1920));
    }

    #[test]
    fn describe_names_every_monitor_with_its_rectangle() {
        let described = real_layout().describe();
        assert!(
            described.contains("HDMI-1 1080x1920 at (0,0)"),
            "{described}"
        );
        assert!(
            described.contains("DP-3 2560x1440 at (1080,0)"),
            "{described}"
        );
    }

    #[test]
    fn an_empty_layout_describes_itself_rather_than_producing_an_empty_string() {
        assert_eq!(MonitorLayout::new(vec![]).describe(), "no monitors");
        assert!(!MonitorLayout::new(vec![]).contains(0, 0));
    }
}
