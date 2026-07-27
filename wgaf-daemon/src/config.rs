use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bus_name: String,
    pub log_level: String,
    /// Bus name the daemon looks for the GNOME Shell Extension bridge on.
    /// Defaults to `wgaf_common::EXTENSION_BUS_NAME`; overridable so tests
    /// can point the daemon at a stub extension service on a private,
    /// unique bus name instead of the real one (see
    /// `wgaf-daemon/tests/windows_stub.rs`), without needing to run a real
    /// GNOME Shell session.
    pub extension_bus_name: String,
    /// Name the daemon's virtual `uinput` device reports to the kernel.
    /// Defaults to `crate::input::DEFAULT_DEVICE_NAME`; overridable so
    /// tests can give each spawned daemon a unique device name — see that
    /// constant's doc comment for why (`/proc/bus/input/devices` has no
    /// concept of "which process created this").
    pub input_device_name: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bus_name: wgaf_common::BUS_NAME.to_string(),
            log_level: "info".to_string(),
            extension_bus_name: wgaf_common::EXTENSION_BUS_NAME.to_string(),
            input_device_name: crate::input::DEFAULT_DEVICE_NAME.to_string(),
        }
    }
}

impl Config {
    /// Loads config from `path` if given and it exists, falling back to defaults otherwise.
    pub fn load(path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        match path {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path)?;
                Ok(toml::from_str(&text)?)
            }
            _ => Ok(Self::default()),
        }
    }
}
