//! Audit logging for every permission-gated (mutating) D-Bus call, on the
//! `wgaf_daemon::permissions::audit` target. The `input`/`accessibility`
//! modules' own docs called their logging "an accountability trail, not an
//! allow/deny gate, ahead of the real permission engine" — this module is
//! that engine's logging half.
//!
//! Unlike the per-module logging (which only records *that* an action
//! happened), every gated call logs **twice** here: once when the check is
//! requested (before the policy lookup/prompt), and once with the resolved
//! outcome (`allowed` / `denied` / `prompted-allow` / `prompted-deny`) — so
//! the audit trail shows both the attempt and its result, not just "action
//! happened".
//!
//! **This target does not (yet) supersede the per-module ones.** It was
//! originally introduced as a consolidation that would replace
//! `input::AUDIT_TARGET` (`wgaf_daemon::input::audit`) and
//! `accessibility::AUDIT_TARGET` (`wgaf_daemon::accessibility::audit`) —
//! but those were never removed, and still emit. Three targets coexist, so
//! one gated `TypeText` produces three lines across two targets: this
//! module's attempt and outcome lines, plus `input/mod.rs`'s own
//! `synthesizing text input` line. Retiring the two legacy targets in
//! favour of this one is an open cleanup item; until then, anything
//! consuming the audit trail must know about all three and must not treat a
//! per-call line count as meaningful.
//!
//! **A third line, for targeted precondition checks.** [`log_verification_outcome`]
//! answers a different question from the attempt/outcome pair above — not
//! "is this capability permitted", but "did this capability's target end up
//! in the state its caller assumed" (currently only
//! `dbus::input_api::InputApi::verify_target`'s focus check). See its own
//! doc comment for why it is attributed to the calling capability, never
//! `FocusWindow`, and why one long `TypeTextAt` call can legitimately
//! produce many of these lines.
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
    /// The action was permitted and attempted, but a precondition it depends
    /// on (e.g. a targeted input call's window ending up focused) could not
    /// be confirmed in time. Per ADR-0007 this is neither an allow nor a
    /// denial: no policy was consulted, and nothing malfunctioned — the
    /// desktop simply did not end up the way the caller assumed.
    VerificationFailed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
            Outcome::PromptedAllow => "prompted-allow",
            Outcome::PromptedDeny => "prompted-deny",
            Outcome::VerificationFailed => "verification-failed",
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

/// What a targeted precondition check found, for
/// [`log_verification_outcome`]'s audit line.
///
/// Bundled into one type purely to keep
/// [`super::PermissionGate::log_verification_outcome`]'s argument count
/// under clippy's `too_many_arguments` threshold — these four fields have
/// no meaning apart from each other or from the call they describe.
///
/// `target` is caller-formatted (e.g. `window:227`) rather than a typed id,
/// since this module has no reason to know what kinds of targets exist —
/// see `permissions`'s module doc on the boundary between policy/audit and
/// the subsystems it gates.
#[derive(Debug, Clone, Copy)]
pub struct VerifiedTarget<'a> {
    pub target: &'a str,
    pub app_id: &'a str,
    /// Which precondition was checked, and whether it held.
    ///
    /// **Two different questions, and the line has to say which one it
    /// answered.** Keyboard input needs the target to hold keyboard focus;
    /// a click needs the pointer to be over it — a window can hold focus
    /// while the pointer sits over a different one entirely, so a reader
    /// who cannot tell the two lines apart cannot reconstruct why an action
    /// was refused.
    pub precondition: Precondition,
    pub met: bool,
    pub outcome: Outcome,
}

/// Which precondition a targeted action checked before synthesizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precondition {
    /// The target holds keyboard focus — `TypeTextAt` and the key methods.
    Focus,
    /// The pointer is over the target — `MouseClickAt`, `MouseScrollAt`.
    Pointer,
}

impl Precondition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Pointer => "pointer",
        }
    }
}

/// Logs the outcome of a *targeted* action's own precondition check —
/// currently only `dbus::input_api::InputApi::verify_target`'s focus check
/// — carrying `verified`'s target, `app_id`, and whether it ended up
/// focused, in addition to the caller-identity fields [`log_outcome`]
/// already carries.
///
/// **Distinct question from [`log_outcome`].** That line answers "is
/// `capability` itself permitted", decided once by [`super::PermissionGate::check`]'s
/// policy lookup. This one answers "did `capability`'s target end up in the
/// state its caller assumed" — a precondition check, not a policy decision,
/// so `capability` here must always be the *calling* method's own
/// capability (`TypeText`, `KeyPress`, ...), never `FocusWindow`, even when
/// a `FocusWindow` check ran internally to produce `verified.outcome`: that
/// check already gets its own, separately-attributed, line via
/// [`log_outcome`].
pub(crate) fn log_verification_outcome(
    capability: Capability,
    caller: &CallerInfo,
    verified: VerifiedTarget<'_>,
) {
    tracing::info!(
        target: AUDIT_TARGET,
        capability = capability.as_str(),
        sender = caller.unique_name.as_deref().unwrap_or("<unknown>"),
        pid = caller.pid,
        process = caller.process_name.as_deref().unwrap_or("<unknown>"),
        target = verified.target,
        app_id = verified.app_id,
        precondition = verified.precondition.as_str(),
        met = verified.met,
        outcome = verified.outcome.as_str(),
        "target verification outcome"
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
        assert_eq!(Outcome::VerificationFailed.as_str(), "verification-failed");
    }

    #[test]
    fn read_process_name_returns_none_for_nonexistent_pid() {
        // PID 0 never has a /proc entry of its own from userspace.
        assert_eq!(read_process_name(0), None);
    }
}
