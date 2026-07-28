use std::path::{Path, PathBuf};

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
    ///
    /// Retained for tests and for `--config-optional`; the daemon itself uses
    /// [`Self::load_required`].
    pub fn load(path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        match path {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path)?;
                Ok(toml::from_str(&text)?)
            }
            _ => Ok(Self::default()),
        }
    }

    /// Loads config, **requiring** the file to exist and to be trustworthy —
    /// see [`crate::secure_file`] for the rules and the reasoning.
    ///
    /// An empty file is perfectly valid and yields every default; the point is
    /// that "use the defaults" becomes something you can see on disk rather
    /// than a silent fallback, and that a file only this user can write is the
    /// one deciding which bus names the daemon talks to.
    pub fn load_required(path: &Path) -> Result<Self, crate::secure_file::SecureFileError> {
        let text = crate::secure_file::read_trusted(
            path,
            "configuration",
            "config.toml",
            format!(
                ": > {}\n    chmod 600 {}\n\n\
                 An empty file gives every built-in default. Or pass --config-optional \
                 to run without one.",
                path.display(),
                path.display()
            ),
        )?;
        toml::from_str(&text).map_err(|source| crate::secure_file::SecureFileError::Malformed {
            kind: "configuration",
            path: path.display().to_string(),
            reason: source.to_string(),
        })
    }
}

/// Pure computation of the default `--config` path per the XDG Base
/// Directory spec: `$XDG_CONFIG_HOME/wgaf/config.toml`, falling back to
/// `$HOME/.config/wgaf/config.toml` if `$XDG_CONFIG_HOME` is unset or empty.
/// Returns `None` if neither variable yields a usable base directory (e.g.
/// `$HOME` also unset) — in that case there simply is no default, exactly
/// like before this default existed at all.
///
/// Takes the two env values as plain `Option<&str>` rather than reading
/// `std::env` itself, so the resolution logic can be unit-tested without
/// touching real process environment state (mutating `std::env::set_var`
/// in tests is both `unsafe` since Rust 2024 and process-global, which would
/// race with any other test in this binary reading the same variables).
/// [`default_config_path`] is the thin wrapper that supplies the real
/// environment.
fn resolve_default_config_path(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("wgaf").join("config.toml"))
}

/// The default `--config` path, used when `--config` isn't passed
/// explicitly: `$XDG_CONFIG_HOME/wgaf/config.toml`, falling back to
/// `$HOME/.config/wgaf/config.toml` per the XDG Base Directory spec. `None`
/// if neither env var is usable (no default in that case, same as before
/// this existed).
///
/// A `dirs`-style crate was deliberately not added for this: the fallback
/// is a handful of lines with no other XDG directories (data/cache/state)
/// needed elsewhere in this daemon, so a whole new dependency for one
/// lookup wasn't justified.
pub fn default_config_path() -> Option<PathBuf> {
    resolve_default_config_path(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_config_home_takes_priority_when_set() {
        let path = resolve_default_config_path(Some("/custom/xdg"), Some("/home/someone"));
        assert_eq!(path, Some(PathBuf::from("/custom/xdg/wgaf/config.toml")));
    }

    #[test]
    fn falls_back_to_home_dot_config_when_xdg_unset() {
        let path = resolve_default_config_path(None, Some("/home/someone"));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/someone/.config/wgaf/config.toml"))
        );
    }

    #[test]
    fn falls_back_to_home_dot_config_when_xdg_is_empty_string() {
        // A set-but-empty XDG_CONFIG_HOME is treated the same as unset, per
        // the XDG Base Directory spec ("All paths set in these environment
        // variables must be absolute... If an implementation... finds a
        // relative path, it should [ignore it]" — empty is the degenerate
        // relative-path case).
        let path = resolve_default_config_path(Some(""), Some("/home/someone"));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/someone/.config/wgaf/config.toml"))
        );
    }

    #[test]
    fn no_default_when_neither_var_is_usable() {
        assert_eq!(resolve_default_config_path(None, None), None);
        assert_eq!(resolve_default_config_path(Some(""), Some("")), None);
    }

    #[test]
    fn load_still_falls_back_to_defaults_when_default_path_does_not_exist() {
        // The resolved default path pointing at a nonexistent file must not
        // be an error — same "absent file -> Config::default()" contract
        // `Config::load` already had for an explicit `--config`.
        let path = PathBuf::from("/nonexistent/wgaf-config-test-does-not-exist/config.toml");
        assert!(!path.exists());
        let config = Config::load(Some(&path)).expect("nonexistent default path is not an error");
        assert_eq!(config.bus_name, wgaf_common::BUS_NAME);
    }
}
