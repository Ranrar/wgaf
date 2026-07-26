//! D-Bus naming shared between `wgaf-daemon` and `wgaf-cli`.

/// Well-known session bus name the daemon registers on startup.
pub const BUS_NAME: &str = "org.wgaf.Daemon";

/// Object path the daemon's root interface is served at.
pub const OBJECT_PATH: &str = "/org/wgaf/Daemon";

/// Versioned interface name, following the `org.freedesktop.*1` convention
/// so a future `Daemon2` can be introduced without breaking existing clients.
pub const INTERFACE_NAME: &str = "org.wgaf.Daemon1";
