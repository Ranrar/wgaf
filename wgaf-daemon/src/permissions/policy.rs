//! The capability catalog and the TOML-deserializable policy map loaded
//! from `permissions.toml`.
//!
//! **Default-allow, not default-deny.** wgaf is a dev/automation tool: every
//! mutating capability that already worked (`wgaf window focus`, `wgaf
//! type`, `wgaf a11y click`, ...) must keep working the moment this module
//! ships, with no config file required. Permissions here are an
//! opt-in *restriction* an operator configures (deny/prompt specific
//! capabilities), never an opt-in *unlock* the operator must grant before
//! anything works. Concretely: [`PolicyMap::get`] returns [`PolicyValue::Allow`]
//! for any capability not explicitly listed in the loaded file, and
//! [`PolicyMap::load`] returns an all-default (all-`Allow`) map, not an
//! error, when `permissions.toml` doesn't exist at all.
//!
//! **Same format as `config.toml`, not a new one.** Uses plain TOML (via
//! `toml::from_str`, the same crate/entry-point `crate::config::Config::load`
//! already uses) so the daemon has one configuration format, not two. The
//! policy map lives under its own `[capabilities]` table (`TypeText =
//! "Deny"`-style entries) so it reads as clearly distinct from
//! `config.toml`'s flat top-level fields, while still being ordinary TOML —
//! see the parsing tests below for the exact accepted syntax.
//! `permissions.toml` is always a *sibling file* of `config.toml` (same
//! directory), never the same file.
//!
//! **Read-only methods are never gated.** `ListWindows`, `GetWorkspaces`,
//! `ListApps`, `FindElements`, `GetTree`, `GetElementInfo` have no
//! [`Capability`] variant at all and are never checked — only the thirteen
//! mutating methods across `org.wgaf.Windows1`/`Input1`/`Accessibility1`
//! listed below are gated.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// One gated, mutating D-Bus method across the daemon's three existing
/// interfaces. Variant names match the D-Bus method names verbatim (not an
/// invented shorthand), so a `permissions.toml` entry like
/// `FocusWindow = "Deny"` reads as exactly what it gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Capability {
    // org.wgaf.Windows1
    FocusWindow,
    MoveWindow,
    ResizeWindow,
    CloseWindow,
    // org.wgaf.Input1
    TypeText,
    KeyPress,
    KeyRelease,
    MouseMove,
    MouseClick,
    MouseScroll,
    // org.wgaf.Accessibility1
    InvokeAction,
    SetText,
    FocusElement,
}

impl Capability {
    /// The capability's name exactly as it appears in `permissions.toml`
    /// and in audit-log entries — always identical to the gated D-Bus
    /// method's own name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Capability::FocusWindow => "FocusWindow",
            Capability::MoveWindow => "MoveWindow",
            Capability::ResizeWindow => "ResizeWindow",
            Capability::CloseWindow => "CloseWindow",
            Capability::TypeText => "TypeText",
            Capability::KeyPress => "KeyPress",
            Capability::KeyRelease => "KeyRelease",
            Capability::MouseMove => "MouseMove",
            Capability::MouseClick => "MouseClick",
            Capability::MouseScroll => "MouseScroll",
            Capability::InvokeAction => "InvokeAction",
            Capability::SetText => "SetText",
            Capability::FocusElement => "FocusElement",
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The three policy outcomes a capability can be configured to. `Prompt` is
/// a real, interactive implementation (see `crate::permissions::notify`),
/// not just a documented placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PolicyValue {
    Allow,
    Deny,
    Prompt,
}

/// The parsed `permissions.toml` policy map: capability -> policy value,
/// for whichever capabilities the file's `[capabilities]` table mentions.
/// Any capability not present defaults to [`PolicyValue::Allow`] (see
/// module docs).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyMap {
    /// The `[capabilities]` table. `#[serde(default)]` so a file with no
    /// `[capabilities]` section at all (or no file — see [`Self::load`])
    /// still parses, as an empty map.
    #[serde(default)]
    capabilities: HashMap<Capability, PolicyValue>,
}

impl PolicyMap {
    /// The effective policy for `capability` — [`PolicyValue::Allow`] if it
    /// isn't mentioned in the loaded file at all.
    pub fn get(&self, capability: Capability) -> PolicyValue {
        self.capabilities
            .get(&capability)
            .copied()
            .unwrap_or(PolicyValue::Allow)
    }

    /// Every capability whose configured value is not the `Allow` default,
    /// sorted by capability name so the output is stable between calls.
    ///
    /// Exists for `org.wgaf.Daemon1.Status`, which reports what the daemon is
    /// actually enforcing. Returning only the non-default entries keeps the
    /// report short and makes the common case unmistakable: an empty list
    /// means nothing is restricted. Listing all 13 capabilities with mostly
    /// `Allow` would bury the one or two that matter.
    pub fn restrictions(&self) -> Vec<(Capability, PolicyValue)> {
        let mut restricted: Vec<(Capability, PolicyValue)> = self
            .capabilities
            .iter()
            .filter(|(_, value)| **value != PolicyValue::Allow)
            .map(|(capability, value)| (*capability, *value))
            .collect();
        restricted.sort_by_key(|(capability, _)| capability.as_str());
        restricted
    }

    /// Loads the policy map from `path` if given and it exists. **A missing
    /// path (or no path given at all) is not an error** — it returns
    /// [`PolicyMap::default`], an empty map under which every capability
    /// resolves to `Allow` via [`Self::get`]. This mirrors
    /// `crate::config::Config::load`'s existing "absent file -> defaults"
    /// convention (indeed, this is now the exact same `toml::from_str`
    /// entry point that uses), except here "defaults" specifically means
    /// "no restrictions configured" rather than a struct's `Default` impl.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_path_defaults_every_capability_to_allow() {
        let map = PolicyMap::load(None).expect("no path is not an error");
        assert_eq!(map.get(Capability::TypeText), PolicyValue::Allow);
        assert_eq!(map.get(Capability::FocusWindow), PolicyValue::Allow);
        assert_eq!(map.get(Capability::InvokeAction), PolicyValue::Allow);
    }

    #[test]
    fn nonexistent_file_path_defaults_every_capability_to_allow() {
        let path = std::path::Path::new("/nonexistent/wgaf-permissions-test-does-not-exist.toml");
        assert!(!path.exists());
        let map = PolicyMap::load(Some(path)).expect("nonexistent path is not an error");
        assert_eq!(map.get(Capability::MouseClick), PolicyValue::Allow);
    }

    #[test]
    fn empty_file_defaults_every_capability_to_allow() {
        let map: PolicyMap = toml::from_str("").expect("empty TOML file parses");
        assert_eq!(map.get(Capability::TypeText), PolicyValue::Allow);
    }

    #[test]
    fn missing_capabilities_table_defaults_every_capability_to_allow() {
        // No `[capabilities]` section at all (as opposed to an empty one) —
        // `#[serde(default)]` on the field must cover this too.
        let map: PolicyMap = toml::from_str("# just a comment, no [capabilities] table\n")
            .expect("TOML without a [capabilities] table parses");
        assert_eq!(map.get(Capability::TypeText), PolicyValue::Allow);
    }

    #[test]
    fn partial_file_honors_specified_capabilities_and_defaults_the_rest() {
        let map: PolicyMap = toml::from_str(
            r#"
            [capabilities]
            TypeText = "Deny"
            MouseClick = "Prompt"
            "#,
        )
        .expect("valid TOML policy map");

        assert_eq!(map.get(Capability::TypeText), PolicyValue::Deny);
        assert_eq!(map.get(Capability::MouseClick), PolicyValue::Prompt);
        // Not mentioned -> default-allow.
        assert_eq!(map.get(Capability::FocusWindow), PolicyValue::Allow);
        assert_eq!(map.get(Capability::KeyPress), PolicyValue::Allow);
        assert_eq!(map.get(Capability::InvokeAction), PolicyValue::Allow);
    }

    #[test]
    fn load_reads_a_real_file_from_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("wgaf-permissions-test-{}.toml", std::process::id()));
        std::fs::write(&path, "[capabilities]\nCloseWindow = \"Deny\"\n")
            .expect("write test permissions.toml");

        let map = PolicyMap::load(Some(&path)).expect("load should succeed");
        assert_eq!(map.get(Capability::CloseWindow), PolicyValue::Deny);
        assert_eq!(map.get(Capability::FocusWindow), PolicyValue::Allow);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capability_display_matches_as_str() {
        assert_eq!(Capability::FocusWindow.to_string(), "FocusWindow");
        assert_eq!(Capability::SetText.to_string(), "SetText");
    }
}
