//! Centralized CLI output formatting — the one place that decides what
//! `--json` actually emits.
//!
//! Before this module existed, `print_ok` was duplicated verbatim in
//! `commands/window.rs`, `commands/input.rs`, and `commands/accessibility.rs`,
//! with `commands::ping` carrying a fourth, subtly different inline variant.
//! Four copies of a contract is three too many: the JSON shape these functions
//! produce is a machine-readable interface other programs parse, so it needs a
//! single definition that can be changed deliberately rather than in four
//! places that can silently drift apart.
//!
//! Two output shapes exist, and the distinction is deliberate:
//!
//! - **Status replies** ([`print_ok`]) — for the daemon methods that return
//!   `()` on success. There is nothing to report but "it worked", so `--json`
//!   emits a compact `{"ok": true, "message": ...}` object.
//! - **Record replies** ([`print_json`]) — for methods returning real data
//!   (window lists, accessible trees). `--json` emits the records themselves,
//!   pretty-printed, with no `ok` wrapper.
//!
//! The compact/pretty split between the two is inherited from the pre-existing
//! behaviour and preserved exactly; it is not a considered design decision, and
//! is worth revisiting the next time the JSON contract is deliberately
//! versioned.

/// Prints a success status line for a command whose daemon call returns
/// nothing on success.
///
/// `--json` emits `{"ok": true, "message": "<message>"}`; otherwise the bare
/// message. Used by every mutating `window`/`type`/`key`/`mouse`/`a11y`
/// subcommand.
pub fn print_ok(json: bool, message: &str) {
    if json {
        println!("{}", serde_json::json!({ "ok": true, "message": message }));
    } else {
        println!("{message}");
    }
}

/// `wgaf ping`'s status line.
///
/// Identical in spirit to [`print_ok`] but emits the payload under a
/// `response` key rather than `message`, because that is the shape `ping`
/// has always produced and this module's extraction was explicitly not a
/// behaviour change. The divergence is a wart, not a design: two status
/// shapes means a consumer handling wgaf's JSON has to special-case one
/// command. Unifying them is a deliberate JSON-contract change and should be
/// made as one — the natural moment is when `wgaf status` lands and the
/// stable JSON surface is defined properly.
pub fn print_ok_response(json: bool, response: &str) {
    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "response": response })
        );
    } else {
        println!("{response}");
    }
}

/// Pretty-prints `value` as JSON — the `--json` arm of every command that
/// returns real records rather than a bare success.
///
/// Callers keep their own human-readable `else` branch; only the JSON arm
/// routes through here, so that "how does wgaf serialize records" has one
/// answer instead of six identical `to_string_pretty` calls.
pub fn print_json<T>(value: &T) -> Result<(), serde_json::Error>
where
    T: serde::Serialize + ?Sized,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
