//! Permissions & security hardening. Formalizes the permission/consent
//! model across `org.wgaf.Windows1`/`Input1`/`Accessibility1`'s mutating
//! methods — a central `permissions` module owning policy config and audit
//! logging.
//!
//! **Scope: audit + explicit allow/deny/prompt policy, not sandboxing.**
//! This module does not change *what* any operation can do (it's still the
//! same `WindowManager`/`InputBackend`/`AccessibilityBackend` calls) — it
//! decides *whether* a mutating call is allowed to happen at all, and
//! records that decision. Read-only methods (`ListWindows`,
//! `GetWorkspaces`, `ListApps`, `FindElements`, `GetTree`,
//! `GetElementInfo`) have no [`Capability`] variant and are never checked —
//! see `policy`'s module docs for why.
//!
//! **Default-allow.** See `policy`'s module docs for the full rationale:
//! wgaf is a dev tool, and nothing that already worked should stop working
//! just because this module exists. `permissions.toml` is an opt-in
//! *restriction*, and its absence is not an error.
//!
//! **Architecture.**
//! ```text
//! windows_api.rs / input_api.rs / accessibility_api.rs
//!       |  PermissionGate::check(capability, connection, header)
//!       v
//! permissions::PermissionGate
//!       |                              |
//!       v (policy lookup)              v (Prompt only)
//! policy::PolicyMap               notify::prompt_user
//!       |                              | org.freedesktop.Notifications
//!       v                              v (ActionInvoked/NotificationClosed)
//!    (Allow/Deny/Prompt)          user's Allow/Deny choice, cached
//!       |
//!       v (both branches)
//! audit::log_attempt / audit::log_outcome
//! ```
//!
//! **A second, narrower audit entry point.** [`PermissionGate::log_verification_outcome`]
//! sits alongside [`PermissionGate::check`] for a different situation: a
//! caller (currently only `dbus::input_api::InputApi::verify_target`) that
//! has already decided allowed/denied/verification-failed by some other
//! means — a focus or pointer precondition, not a policy lookup — and just needs that
//! decision recorded under the same caller-identity fields `check` itself
//! logs. It never consults [`policy::PolicyMap`] and cannot deny anything on
//! its own; see `audit`'s `log_verification_outcome` for the full rationale.
//!
//! **Prompt caching.** Per the roadmap's own language ("persist the
//! decision... rather than prompting on every call"), a `Prompt` capability
//! is only ever shown to the user once per daemon run: the first resolution
//! (`true`/`false`) is cached in [`PermissionGate`] for the remainder of the
//! process's lifetime. This is deliberately in-memory only, not persisted
//! across daemon restarts — judged sufficient for now rather than
//! over-engineered.
//!
//! **Error-namespace choice.** A denial surfaces as a `PermissionDenied`
//! variant added to each of `WindowsApiError`/`InputApiError`/
//! `AccessibilityApiError` (i.e. `org.wgaf.Windows1.Error.PermissionDenied`,
//! `org.wgaf.Input1.Error.PermissionDenied`,
//! `org.wgaf.Accessibility1.Error.PermissionDenied`) rather than a fourth,
//! cross-cutting D-Bus error namespace — consistent with this codebase's
//! existing convention that each interface owns its own named error space
//! (see `dbus/windows_api.rs`'s `WindowsApiError`, `input_api.rs`'s
//! `InputApiError`, `accessibility_api.rs`'s `AccessibilityApiError`). This
//! [`PermissionError`] (defined here) is an internal type the three
//! `dbus::*_api` modules convert *from*, not itself a D-Bus-visible error
//! type.

mod audit;
mod notify;
pub mod policy;

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;
use zbus::message::Header;

pub use audit::{CallerInfo, Outcome, Precondition, VerifiedTarget};
pub use policy::{Capability, PolicyMap, PolicyValue};

/// Errors returned by [`PermissionGate::check`]. Each of
/// `dbus::windows_api`/`dbus::input_api`/`dbus::accessibility_api` converts
/// this into its own interface's `PermissionDenied` error variant (see
/// module docs) rather than serving it directly.
#[derive(Debug, Error)]
pub enum PermissionError {
    /// The configured policy (`permissions.toml`) denies this capability
    /// outright.
    ///
    /// # Worded as a decision, not a fault
    ///
    /// A denial is the permission system working, not failing: the policy said
    /// no and wgaf obeyed it. So the message states what happened and where the
    /// rule lives, and stops there. It used to continue "an administrator can
    /// change this capability's policy value to `Allow` or `Prompt` if this
    /// restriction was unintentional", which reads as an apology for a
    /// malfunction and guesses that the reader did not mean it — on a tool whose
    /// policy file is almost always edited by the person now running the
    /// command.
    ///
    /// **The file is still named**, and deliberately: ADR-0003 requires a denied
    /// watch to say so *and* name `permissions.toml`, because the alternative is
    /// a user who cannot find the rule that stopped them. What was dropped is
    /// the advice, not the location — the audit trail carries the rest.
    #[error("`{capability}` denied by permission policy (permissions.toml)")]
    Denied { capability: &'static str },

    /// The capability is configured as `Prompt`, and the user declined (or
    /// didn't respond to, within the timeout) the resulting notification.
    ///
    /// The three cases — clicked Deny, dismissed, timed out — are deliberately
    /// one outcome here, because the gate is fail-closed and treats them
    /// identically. The message says so rather than claiming the user chose,
    /// since on a timeout nobody did.
    #[error("`{capability}` denied: the prompt was declined or went unanswered")]
    DeniedByPrompt { capability: &'static str },

    /// A D-Bus-level failure while resolving the caller's identity (for
    /// audit logging) or while showing/awaiting a `Prompt` notification.
    /// Deliberately fails the call rather than silently treating it as
    /// allowed or denied — a broken notification service (say) should be a
    /// visible error, not a silent policy decision.
    #[error("D-Bus error while checking permissions: {0}")]
    DBus(#[from] zbus::Error),
}

/// Owns the daemon-wide permission policy and the in-memory cache of
/// `Prompt` resolutions made so far this run. One instance is created at
/// daemon startup (`main.rs`) and shared, via `Arc`, across `WindowsApi`/
/// `InputApi`/`AccessibilityApi` — unlike `WindowManager`/`InputBackend`/
/// `AccessibilityBackend` (one instance per interface), this one is
/// genuinely shared across all three, since policy and the prompt cache are
/// cross-cutting by design.
pub struct PermissionGate {
    policy: PolicyMap,
    /// `Prompt`-capability resolutions already made this daemon run (see
    /// module docs on caching). A `std::sync::Mutex` is fine here: every
    /// critical section is a single non-blocking `HashMap` op, never held
    /// across an `.await`.
    prompt_cache: Mutex<HashMap<Capability, bool>>,
}

impl PermissionGate {
    pub fn new(policy: PolicyMap) -> Self {
        Self {
            policy,
            prompt_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Capabilities restricted away from the `Allow` default, for
    /// `org.wgaf.Daemon1.Status` — see [`PolicyMap::restrictions`].
    ///
    /// Surfacing this closes a real gap in shipped behaviour: until now a
    /// user whose call was refused by `permissions.toml` had no way to see
    /// *which* file that came from or *what else* was restricted, since the
    /// path may be an XDG default they never chose explicitly.
    pub fn restrictions(&self) -> Vec<(Capability, PolicyValue)> {
        self.policy.restrictions()
    }

    /// Interactive `Prompt` decisions resolved so far this run, sorted by
    /// capability name. In-memory only and lost on restart, which is itself
    /// worth being able to see.
    pub fn prompt_decisions(&self) -> Vec<(Capability, bool)> {
        let cache = self
            .prompt_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut decisions: Vec<(Capability, bool)> = cache
            .iter()
            .map(|(cap, allowed)| (*cap, *allowed))
            .collect();
        drop(cache);
        decisions.sort_by_key(|(capability, _)| capability.as_str());
        decisions
    }

    /// Checks whether `capability` is currently permitted for the caller
    /// identified by `header`, on `connection` (the same connection the
    /// calling D-Bus method arrived on — used both to resolve the caller's
    /// identity for audit logging, and, for a `Prompt` capability, to reach
    /// the session's notification service).
    ///
    /// Logs the attempt and its outcome via `audit` (see module docs) in
    /// every case, including denial.
    pub async fn check(
        &self,
        capability: Capability,
        connection: &zbus::Connection,
        header: &Header<'_>,
    ) -> Result<(), PermissionError> {
        let caller = CallerInfo::resolve(connection, header).await;
        audit::log_attempt(capability, &caller);

        let policy = self.policy.get(capability);
        let result = match policy {
            PolicyValue::Allow => Ok(()),
            PolicyValue::Deny => Err(PermissionError::Denied {
                capability: capability.as_str(),
            }),
            PolicyValue::Prompt => {
                if self.resolve_prompt(capability, connection).await? {
                    Ok(())
                } else {
                    Err(PermissionError::DeniedByPrompt {
                        capability: capability.as_str(),
                    })
                }
            }
        };

        let outcome = match (policy, &result) {
            (_, Ok(())) if policy == PolicyValue::Prompt => audit::Outcome::PromptedAllow,
            (_, Ok(())) => audit::Outcome::Allowed,
            (PolicyValue::Prompt, Err(_)) => audit::Outcome::PromptedDeny,
            (_, Err(_)) => audit::Outcome::Denied,
        };
        audit::log_outcome(capability, &caller, outcome);

        result
    }

    /// Logs the outcome of a *targeted* action's own precondition check —
    /// the audit counterpart of `dbus::input_api::InputApi::verify_target`,
    /// not of [`Self::check`] itself (see this module's doc comment on the
    /// two entry points, and `audit::log_verification_outcome`'s own doc
    /// comment for the full rationale).
    ///
    /// `capability` must be the *calling* method's own capability
    /// (`TypeText`, `KeyPress`, ...), never [`Capability::FocusWindow`],
    /// even when a `FocusWindow` check ran internally to decide
    /// `verified.outcome` — that check already gets its own,
    /// separately-attributed, line via [`Self::check`].
    ///
    /// Resolves [`CallerInfo`] itself, the same way [`Self::check`] does,
    /// rather than accepting an already-resolved one — this is a
    /// self-contained audit event, not a continuation of a specific
    /// `check()` call.
    pub async fn log_verification_outcome(
        &self,
        capability: Capability,
        connection: &zbus::Connection,
        header: &Header<'_>,
        verified: VerifiedTarget<'_>,
    ) {
        let caller = CallerInfo::resolve(connection, header).await;
        audit::log_verification_outcome(capability, &caller, verified);
    }

    /// Resolves a `Prompt`-policy capability: returns the cached decision if
    /// this capability has already been prompted for this run, otherwise
    /// shows a real notification and caches the result.
    async fn resolve_prompt(
        &self,
        capability: Capability,
        connection: &zbus::Connection,
    ) -> Result<bool, PermissionError> {
        // FIXED: recover from poisoning instead of panicking. The guarded
        // state is just a `HashMap<Capability, bool>` of already-resolved
        // `Prompt` decisions; every critical section here is a single
        // `get`/`insert` that either completes or doesn't (no multi-step
        // invariant across the map that a panic partway through could leave
        // torn), and per the struct docs, this lock is never held across an
        // `.await`, so the only realistic way to poison it is a panic from
        // deep inside `HashMap`'s own internals — not something recovering
        // makes worse. Treating poisoning as fatal here would mean one
        // panicking caller permanently disables the *entire* permission gate
        // (every capability check, for all three D-Bus interfaces) for the
        // rest of the daemon's life, which is a far worse failure mode than
        // proceeding with a possibly-incomplete cache and simply re-prompting
        // if an entry didn't make it in.
        if let Some(cached) = self
            .prompt_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&capability)
            .copied()
        {
            return Ok(cached);
        }

        let allowed = notify::prompt_user(connection, capability).await?;

        self.prompt_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(capability, allowed);
        Ok(allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_error_messages_name_the_capability() {
        let denied = PermissionError::Denied {
            capability: Capability::TypeText.as_str(),
        };
        assert!(denied.to_string().contains("TypeText"));

        let prompt_denied = PermissionError::DeniedByPrompt {
            capability: Capability::MouseClick.as_str(),
        };
        assert!(prompt_denied.to_string().contains("MouseClick"));
    }

    #[test]
    fn gate_allows_by_default_with_empty_policy() {
        let gate = PermissionGate::new(PolicyMap::default());
        assert_eq!(gate.policy.get(Capability::FocusWindow), PolicyValue::Allow);
    }

    #[test]
    fn gate_reads_deny_from_configured_policy() {
        let policy: PolicyMap = toml::from_str("[capabilities]\nCloseWindow = \"Deny\"\n")
            .expect("valid TOML policy map");
        let gate = PermissionGate::new(policy);
        assert_eq!(gate.policy.get(Capability::CloseWindow), PolicyValue::Deny);
        // Unmentioned capability still defaults to Allow.
        assert_eq!(gate.policy.get(Capability::FocusWindow), PolicyValue::Allow);
    }
}
