//! Consolidated audit logging for every permission-gated (mutating) D-Bus
//! call, superseding the earlier separate per-module `tracing` targets
//! (`input::AUDIT_TARGET`, `accessibility::AUDIT_TARGET`) with one shared
//! target: `wgaf_daemon::permissions::audit`. Those modules' own docs
//! called their logging "an accountability trail, not an allow/deny gate,
//! ahead of the real permission engine" — this module is that engine's
//! logging half.
//!
//! Unlike the earlier per-module logging (which only recorded *that* an
//! action happened), every gated call now logs **twice**: once when the
//! check is requested (before the policy lookup/prompt), and once with the
//! resolved outcome (`allowed` / `denied` / `prompted-allow` /
//! `prompted-deny`) — so the audit trail shows both the attempt and its
//! result, not just "action happened".
//!
//! **Source identification.** Every entry records the caller's D-Bus unique
//! connection name, and — resolved via `org.freedesktop.DBus`'s
//! `GetConnectionUnixProcessID` (the same `zbus::fdo::DBusProxy` pattern
//! `windows::WindowManager::check_extension_version` already uses for
//! extension-availability discovery) — the calling process's PID and, where
//! `/proc/<pid>/comm` is readable, its process name. Resolution failures
//! (the caller's process having already exited, `/proc` being unreadable,
//! etc.) degrade to `None` fields rather than failing the underlying D-Bus
//! call — audit logging must never be a reason a permitted action fails.

use zbus::message::Header;

use super::policy::Capability;

/// `tracing` target every permission-check audit entry is logged on.
pub(crate) const AUDIT_TARGET: &str = "wgaf_daemon::permissions::audit";

/// The calling process's identity, resolved on a best-effort basis (see
/// module docs) once per permission check.
#[derive(Debug, Clone, Default)]
pub struct CallerInfo {
    /// The caller's D-Bus unique connection name (e.g. `:1.87`) taken
    /// straight from the method call's message header — always present for
    /// a real D-Bus method call (`None` only if the header is missing a
    /// sender entirely, which would itself be unusual).
    pub unique_name: Option<String>,
    /// The caller's process id, via `GetConnectionUnixProcessID`.
    pub pid: Option<u32>,
    /// The caller's process name, via `/proc/<pid>/comm`.
    pub process_name: Option<String>,
}

impl CallerInfo {
    /// Resolves the caller of the method call carrying `header`, using
    /// `connection` (the same connection the daemon's D-Bus API is served
    /// on) to query `org.freedesktop.DBus` for the sender's PID.
    pub async fn resolve(connection: &zbus::Connection, header: &Header<'_>) -> Self {
        let Some(sender) = header.sender() else {
            return Self::default();
        };
        let unique_name = sender.to_string();

        let pid = match zbus::fdo::DBusProxy::new(connection).await {
            Ok(dbus_proxy) => {
                let bus_name: zbus::names::BusName<'_> = sender.to_owned().into();
                dbus_proxy
                    .get_connection_unix_process_id(bus_name)
                    .await
                    .ok()
            }
            Err(_) => None,
        };
        let process_name = pid.and_then(read_process_name);

        Self {
            unique_name: Some(unique_name),
            pid,
            process_name,
        }
    }
}

/// Reads `/proc/<pid>/comm` for a short process name — best-effort only
/// (see module docs: any failure here is `None`, never propagated).
fn read_process_name(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty())
}

/// The resolved outcome of one permission check, for the "after" log entry
/// (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Denied,
    PromptedAllow,
    PromptedDeny,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
            Outcome::PromptedAllow => "prompted-allow",
            Outcome::PromptedDeny => "prompted-deny",
        }
    }
}

/// Logs the "attempt" half of a permission check, before the policy lookup
/// or any prompt happens.
pub(crate) fn log_attempt(capability: Capability, caller: &CallerInfo) {
    tracing::info!(
        target: AUDIT_TARGET,
        capability = capability.as_str(),
        sender = caller.unique_name.as_deref().unwrap_or("<unknown>"),
        pid = caller.pid,
        process = caller.process_name.as_deref().unwrap_or("<unknown>"),
        "permission check requested"
    );
}

/// Logs the "outcome" half of a permission check, once the policy decision
/// (including any `Prompt` resolution) is known.
pub(crate) fn log_outcome(capability: Capability, caller: &CallerInfo, outcome: Outcome) {
    tracing::info!(
        target: AUDIT_TARGET,
        capability = capability.as_str(),
        sender = caller.unique_name.as_deref().unwrap_or("<unknown>"),
        pid = caller.pid,
        process = caller.process_name.as_deref().unwrap_or("<unknown>"),
        outcome = outcome.as_str(),
        "permission check outcome"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_as_str_matches_documented_values() {
        assert_eq!(Outcome::Allowed.as_str(), "allowed");
        assert_eq!(Outcome::Denied.as_str(), "denied");
        assert_eq!(Outcome::PromptedAllow.as_str(), "prompted-allow");
        assert_eq!(Outcome::PromptedDeny.as_str(), "prompted-deny");
    }

    #[test]
    fn read_process_name_returns_none_for_nonexistent_pid() {
        // PID 0 never has a /proc entry of its own from userspace.
        assert_eq!(read_process_name(0), None);
    }
}
