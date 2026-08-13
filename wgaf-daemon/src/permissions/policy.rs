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
//! for any capability not explicitly listed in the loaded file.
//!
//! **But the file itself is required.** Default-allow applies *within* a
//! policy file, not to its absence. [`PolicyMap::load_required`] — what the
//! daemon actually uses — fails if `permissions.toml` is missing, owned by
//! another account, or group/world-writable, and the daemon refuses to start.
//! Treating a missing file as "allow everything" made the whole mechanism
//! fail-open: deleting the file silently removed every restriction the user
//! had configured, and a file lost to a bad sync or a stray `rm` was
//! indistinguishable from a deliberate decision. Allowing everything is still
//! available — it just has to be stated, with an empty `[capabilities]`
//! table, or by passing `--permissions-optional`. [`PolicyMap::load`] keeps
//! the old lenient behaviour for that flag and for tests.
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
//! `GetWorkspaceLayout`, `GetMonitors`, `ListApps`, `FindElements`, `GetTree`,
//! `GetElementInfo` have no [`Capability`] variant at all and are never
//! checked — only the mutating methods across
//! `org.wgaf.Windows1`/`Input1`/`Accessibility1` listed below are gated, plus
//! [`Capability::WatchWindows`], which gates a subscription rather than a
//! change. `Capability::ALL` (test-only) is the catalog the tests below check
//! the enum against; it cannot fall behind, for the reason its doc comment
//! gives.

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
    /// Subscribing to the window event stream — `wgaf window watch`.
    ///
    /// **The first capability that gates observing rather than acting**, which
    /// is why it is worth a variant of its own rather than being left ungated
    /// like the other read-only calls. `ListWindows` is a snapshot the caller
    /// asked for; a subscription is an open feed of every window title that
    /// appears on the desktop for as long as it stays open, and an ungated one
    /// would leave no trace in the audit trail. Decided in
    /// [ADR-0003](../../../.vscode/Documentation/adr/adr-0003-window-signal-gating.md).
    ///
    /// Defaults to `Allow`, so nobody who does not go looking sees any
    /// difference from it having been ungated. What it buys is that
    /// `WatchWindows = "deny"` is a sentence a user can write, and that
    /// subscriptions appear in `permissions::audit`.
    WatchWindows,
    /// Making a different workspace active.
    ///
    /// Its own variant rather than being folded into the three below, because
    /// it is the only reversible one: switching moves the user's view and they
    /// can switch back, where adding, removing and reordering change the shape
    /// of the session. An operator who wants automation to navigate their
    /// desktop but not rearrange it needs exactly that line.
    SwitchWorkspace,
    AddWorkspace,
    /// Removing a workspace.
    ///
    /// Kept separate from [`Capability::AddWorkspace`] on the W5 precedent: a
    /// caller granted "add a workspace" has not thereby granted "remove one",
    /// and only removal disturbs windows that are already open — Mutter moves
    /// them to a neighbouring workspace.
    RemoveWorkspace,
    ReorderWorkspace,
    /// Sending a window to a different workspace.
    ///
    /// Gated as a *window* operation, not a workspace one, and separate from
    /// both: it changes neither the set of workspaces nor which is active, it
    /// moves someone's window out of sight. An operator who allows
    /// `SwitchWorkspace` so automation can navigate has not thereby agreed to
    /// have their windows rearranged behind them.
    MoveWindowToWorkspace,
    /// Minimizing a window, or restoring one.
    ///
    /// One capability for both directions, like the five below, because the
    /// pair is one decision: an operator who is willing to have automation
    /// minimize their windows has no separate interest in whether it may put
    /// them back. Splitting them would produce the state nobody wants — windows
    /// that can be hidden and not restored.
    SetWindowMinimized,
    SetWindowMaximized,
    SetWindowFullscreen,
    /// Keeping a window above the others, or stopping.
    ///
    /// Worth its own line rather than sharing with the two above: an
    /// always-on-top window covers whatever the user is looking at and stays
    /// there, which is a different thing to consent to than a window changing
    /// size.
    SetWindowAbove,
    SetWindowOnAllWorkspaces,
    /// Raising a window to the top of its stack layer, or lowering it.
    ///
    /// Separate from [`Capability::FocusWindow`] because the two are not the
    /// same act: focusing raises *and* redirects the keyboard, raising only
    /// changes what is visible. A caller allowed to rearrange what is on top
    /// has not thereby been allowed to move keyboard focus.
    RestackWindow,
    // org.wgaf.Input1
    TypeText,
    KeyPress,
    KeyRelease,
    MouseMove,
    /// Deliberately **not** folded into [`Capability::MouseMove`]. Relative
    /// motion nudges the pointer from wherever it happens to be; absolute
    /// positioning puts it on a chosen pixel. A caller granted "nudge the
    /// pointer" has not thereby granted "put the pointer on the Confirm
    /// button", and only the second one composes with `MouseClick` into
    /// clicking a specific thing.
    MouseMoveAbsolute,
    MouseClick,
    MouseScroll,
    // org.wgaf.Accessibility1
    InvokeAction,
    SetText,
    FocusElement,
    /// Scrolling an element into view — `wgaf a11y scroll-to`.
    ///
    /// Gated rather than left ungated with the reads, because it changes what
    /// the user is looking at: a script that scrolls someone's document while
    /// they are reading it has altered their session, however mildly.
    ///
    /// Kept separate from [`Capability::FocusElement`] even though the two are
    /// the same AT-SPI interface and are refused by the same toolkits. The
    /// interface an operation happens to arrive through is not what a
    /// permission describes — moving the keyboard focus decides where the next
    /// keystroke lands, which is the S1 hazard's whole subject matter, while
    /// scrolling moves a viewport. An operator willing to allow the second has
    /// not thereby allowed the first.
    ScrollElement,
}

impl Capability {
    /// Every capability, in the order they are declared above.
    ///
    /// Rust cannot enumerate an enum's variants, so this is a hand-written
    /// list — the kind that rots quietly, exactly like
    /// `REQUIRED_EXTENSION_METHODS` in `windows/mod.rs`. [`Self::ordinal`] is
    /// what keeps it honest.
    ///
    /// **Test-only, because the daemon has no reason to enumerate the
    /// catalog** — every check is against one named capability, and
    /// [`PolicyMap::restrictions`] deliberately reports only what is
    /// configured. Its job is to give the tests below something to be
    /// exhaustive *about*, so that "the docs list every capability" and "no two
    /// share a name" are assertions rather than review habits. Making it
    /// non-test to avoid a dead-code warning would be inventing a caller.
    #[cfg(test)]
    pub const ALL: &'static [Capability] = &[
        Capability::FocusWindow,
        Capability::MoveWindow,
        Capability::ResizeWindow,
        Capability::CloseWindow,
        Capability::WatchWindows,
        Capability::SwitchWorkspace,
        Capability::AddWorkspace,
        Capability::RemoveWorkspace,
        Capability::ReorderWorkspace,
        Capability::MoveWindowToWorkspace,
        Capability::SetWindowMinimized,
        Capability::SetWindowMaximized,
        Capability::SetWindowFullscreen,
        Capability::SetWindowAbove,
        Capability::SetWindowOnAllWorkspaces,
        Capability::RestackWindow,
        Capability::TypeText,
        Capability::KeyPress,
        Capability::KeyRelease,
        Capability::MouseMove,
        Capability::MouseMoveAbsolute,
        Capability::MouseClick,
        Capability::MouseScroll,
        Capability::InvokeAction,
        Capability::SetText,
        Capability::FocusElement,
        Capability::ScrollElement,
    ];

    /// This capability's position in [`Self::ALL`].
    ///
    /// **The only reason this exists is to make [`Self::ALL`] uncheatable.**
    /// The match below is exhaustive, so a new variant is a compile error here;
    /// the only way to fix it is to give the variant an index, and
    /// `every_capability_is_in_all` then fails unless that index is where the
    /// variant actually sits in `ALL`. A list that cannot silently fall behind
    /// is worth the twenty lines.
    ///
    /// Test-only for the same reason `ALL` is — it exists to police that list,
    /// not to be called. The compile error therefore lands on `cargo test`
    /// rather than `cargo build`, which is where every other drift check in
    /// this tree lands too.
    #[cfg(test)]
    const fn ordinal(self) -> usize {
        match self {
            Capability::FocusWindow => 0,
            Capability::MoveWindow => 1,
            Capability::ResizeWindow => 2,
            Capability::CloseWindow => 3,
            Capability::WatchWindows => 4,
            Capability::SwitchWorkspace => 5,
            Capability::AddWorkspace => 6,
            Capability::RemoveWorkspace => 7,
            Capability::ReorderWorkspace => 8,
            Capability::MoveWindowToWorkspace => 9,
            Capability::SetWindowMinimized => 10,
            Capability::SetWindowMaximized => 11,
            Capability::SetWindowFullscreen => 12,
            Capability::SetWindowAbove => 13,
            Capability::SetWindowOnAllWorkspaces => 14,
            Capability::RestackWindow => 15,
            Capability::TypeText => 16,
            Capability::KeyPress => 17,
            Capability::KeyRelease => 18,
            Capability::MouseMove => 19,
            Capability::MouseMoveAbsolute => 20,
            Capability::MouseClick => 21,
            Capability::MouseScroll => 22,
            Capability::InvokeAction => 23,
            Capability::SetText => 24,
            Capability::FocusElement => 25,
            Capability::ScrollElement => 26,
        }
    }

    /// The capability's name exactly as it appears in `permissions.toml`
    /// and in audit-log entries — always identical to the gated D-Bus
    /// method's own name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Capability::FocusWindow => "FocusWindow",
            Capability::MoveWindow => "MoveWindow",
            Capability::ResizeWindow => "ResizeWindow",
            Capability::CloseWindow => "CloseWindow",
            Capability::WatchWindows => "WatchWindows",
            Capability::SwitchWorkspace => "SwitchWorkspace",
            Capability::AddWorkspace => "AddWorkspace",
            Capability::RemoveWorkspace => "RemoveWorkspace",
            Capability::ReorderWorkspace => "ReorderWorkspace",
            Capability::MoveWindowToWorkspace => "MoveWindowToWorkspace",
            Capability::SetWindowMinimized => "SetWindowMinimized",
            Capability::SetWindowMaximized => "SetWindowMaximized",
            Capability::SetWindowFullscreen => "SetWindowFullscreen",
            Capability::SetWindowAbove => "SetWindowAbove",
            Capability::SetWindowOnAllWorkspaces => "SetWindowOnAllWorkspaces",
            Capability::RestackWindow => "RestackWindow",
            Capability::TypeText => "TypeText",
            Capability::KeyPress => "KeyPress",
            Capability::KeyRelease => "KeyRelease",
            Capability::MouseMove => "MouseMove",
            Capability::MouseMoveAbsolute => "MouseMoveAbsolute",
            Capability::MouseClick => "MouseClick",
            Capability::MouseScroll => "MouseScroll",
            Capability::InvokeAction => "InvokeAction",
            Capability::SetText => "SetText",
            Capability::FocusElement => "FocusElement",
            Capability::ScrollElement => "ScrollElement",
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

use crate::secure_file::SecureFileError;

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
    /// means nothing is restricted. Listing every capability with mostly
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

    /// Loads the policy, **requiring** the file to exist and to be
    /// trustworthy. This is what the daemon actually uses at startup; plain
    /// [`Self::load`] remains for tests and for the explicit
    /// `--permissions-optional` escape hatch.
    ///
    /// Why absence is fatal rather than defaulting to `Allow`: a security
    /// control whose absence is permissive is fail-open. Under the old
    /// behaviour, `rm ~/.config/wgaf/permissions.toml` silently removed every
    /// restriction the user had deliberately configured — no error, no
    /// warning, and `wgaf type` quietly working again after having been
    /// denied. A lost file (bad sync, restored home directory, stray `rm`)
    /// must never be indistinguishable from a considered decision to allow
    /// everything. Saying "allow everything" is still perfectly possible; it
    /// just has to be *said*, with a file containing an empty
    /// `[capabilities]` table.
    ///
    /// Ownership and mode are checked for the same reason the policy is
    /// required at all: a file another account can rewrite is not a policy,
    /// it is a suggestion. `ssh` refuses to use a private key on the same
    /// grounds.
    pub fn load_required(path: &Path) -> Result<Self, SecureFileError> {
        let text = crate::secure_file::read_trusted(
            path,
            "permission policy",
            "permissions.toml",
            format!(
                "printf '[capabilities]\\n' > {}\n    chmod 600 {}\n\n\
                 An empty [capabilities] table allows every capability. Or pass \
                 --permissions-optional to run without this file.",
                path.display(),
                path.display()
            ),
        )?;
        toml::from_str(&text).map_err(|source| SecureFileError::Malformed {
            kind: "permission policy",
            path: path.display().to_string(),
            reason: source.to_string(),
        })
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

    /// `Capability::ALL` really is all of them, and in the order it claims.
    ///
    /// Together with `ordinal`'s exhaustive match — which a new variant breaks
    /// at compile time — this is what stops the catalog drifting from the enum.
    /// The failure it prevents is quiet: an omitted capability is still gated
    /// perfectly well, but disappears from anything that enumerates the list.
    #[test]
    fn every_capability_is_in_all() {
        for (index, capability) in Capability::ALL.iter().enumerate() {
            assert_eq!(
                capability.ordinal(),
                index,
                "`{capability}` is at index {index} in ALL but claims ordinal {}",
                capability.ordinal()
            );
        }
    }

    /// Every capability is named for the D-Bus method it gates, so no two can
    /// share a name — and `permissions.toml` keys them by that name, so a
    /// duplicate would make one of them unconfigurable.
    #[test]
    fn capability_names_are_unique() {
        let mut names: Vec<&str> = Capability::ALL.iter().map(|c| c.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two capabilities share a name");
    }

    /// Every capability parses from its own name, which is the contract
    /// `permissions.toml` rests on: the variant name, the `as_str` name and the
    /// TOML key are all the same string.
    #[test]
    fn every_capability_parses_from_its_own_name() {
        for capability in Capability::ALL {
            let toml = format!("[capabilities]\n{} = \"Deny\"\n", capability.as_str());
            let map: PolicyMap = toml::from_str(&toml)
                .unwrap_or_else(|e| panic!("`{capability}` should parse from its own name: {e}"));
            assert_eq!(
                map.get(*capability),
                PolicyValue::Deny,
                "`{capability}` did not round-trip through permissions.toml"
            );
        }
    }

    /// The policy file shipped by `make install`, read at compile time. Same
    /// approach — and the same reason — as `windows/proxy.rs`'s `include_str!`
    /// of the extension source: the file is part of this repository and ships
    /// with the daemon, so a drift between them is a `cargo test` failure
    /// rather than something a user discovers.
    const SHIPPED_POLICY: &str = include_str!("../../../packaging/permissions.toml");

    /// `packaging/permissions.toml` must mention **every** capability, and
    /// nothing that is not one.
    ///
    /// This file lists each capability explicitly rather than leaving the table
    /// empty, which makes it a catalog a user reads to learn what can be
    /// restricted — and a catalog that silently omits a capability is worse
    /// than no catalog, because its completeness is the whole reason to trust
    /// it. It had fallen behind twice by the time this test was written: the
    /// four workspace capabilities were absent, exactly as
    /// `docs/configuration.md` was still claiming thirteen of them.
    ///
    /// It parses the shipped file rather than grepping it, so a capability
    /// mentioned only in a comment does not count as listed.
    #[test]
    fn the_shipped_policy_file_lists_every_capability() {
        let shipped: PolicyMap = toml::from_str(SHIPPED_POLICY)
            .expect("packaging/permissions.toml must be valid TOML naming only real capabilities");

        for capability in Capability::ALL {
            assert!(
                shipped.capabilities.contains_key(capability),
                "`{capability}` is missing from packaging/permissions.toml — a user reading that \
                 file would not learn the capability exists"
            );
        }

        // The reverse direction is covered by the parse above: an entry naming
        // something that is not a capability fails to deserialize. What this
        // adds is that the file has not grown a *duplicate* set beyond the
        // catalog's size, which a map would otherwise silently collapse.
        assert_eq!(
            shipped.capabilities.len(),
            Capability::ALL.len(),
            "packaging/permissions.toml and the capability catalog disagree in size"
        );
    }

    /// Every entry in the shipped file is `Allow`.
    ///
    /// The file is a starting point, not a policy: wgaf is default-allow, and a
    /// fresh install must behave exactly as it did before permissions existed.
    /// A `Deny` or `Prompt` slipping in here would silently break a working
    /// command for everyone who installs after it, and the failure would look
    /// like a wgaf bug rather than a policy decision.
    #[test]
    fn the_shipped_policy_restricts_nothing() {
        let shipped: PolicyMap =
            toml::from_str(SHIPPED_POLICY).expect("packaging/permissions.toml parses");
        assert!(
            shipped.restrictions().is_empty(),
            "the shipped policy must restrict nothing, but restricts: {:?}",
            shipped.restrictions()
        );
    }
}
