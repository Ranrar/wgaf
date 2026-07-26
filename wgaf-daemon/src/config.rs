use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bus_name: String,
    pub log_level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bus_name: wgaf_common::BUS_NAME.to_string(),
            log_level: "info".to_string(),
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
