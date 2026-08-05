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

/// Prints a command's failure when `--json` was requested — the failure-path
/// counterpart to [`print_ok`], called from `main()` instead of the plain
/// `error:`/bare-message `eprintln!` it uses otherwise.
///
/// `--json` emits `{"outcome": "<outcome>", "message": "<message>"}` on
/// stdout, mirroring [`print_ok`]'s own `{"ok": true, "message": ...}` shape
/// closely enough that a consumer already handling one recognizes the other.
/// `"outcome"` is one of `"error"`/`"denied"`/`"unverified"` — see
/// `crate::error::Verdict::json_outcome` — part of the JSON contract
/// (`plan-first-release.md` §16, W16.2); do not change those three strings
/// without treating it as a breaking wire-format change.
///
/// Printed to stdout, not stderr, for the same reason `print_ok`'s JSON is:
/// a `--json` consumer should be able to read one stream regardless of
/// whether the command succeeded.
pub fn print_outcome_error(verdict: crate::error::Verdict, message: &str) {
    println!(
        "{}",
        serde_json::json!({ "outcome": verdict.json_outcome(), "message": message })
    );
}

/// Marker prefixing each subsystem line in `wgaf status`.
///
/// Plain ASCII rather than coloured output or Unicode symbols: this is the
/// command people paste into bug reports and pipe through `grep`, and it must
/// survive both intact.
fn marker(ok: bool) -> &'static str {
    if ok { "[ ok ]" } else { "[fail]" }
}

/// Renders [`wgaf_common::DaemonStatus`] for a human.
///
/// Ordered so the parts most likely to be *wrong* come first and carry the
/// daemon's own guidance inline — a new user running this is usually asking
/// "why doesn't it work?", and the answer should be on screen without
/// needing a second command. The daemon's `_detail` text is printed verbatim;
/// it is written to be actionable (which udev rule, which extension to
/// enable) and the CLI has nothing to add to it.
///
/// Only the `--json` shape is a stable interface. This layout may change.
pub fn print_status(s: &wgaf_common::DaemonStatus) {
    // First, above everything, and with a blank line under it. Someone runs
    // this command because nothing is happening, and when the kill switch is
    // engaged that *is* the answer — one field among twenty further down would
    // be read after the subsystem list had already suggested other theories.
    if s.input_stopped {
        println!("!! INPUT STOPPED — the kill switch is engaged.");
        println!("   No keystrokes, clicks or scrolls will be synthesized.");
        println!("   Run `wgaf release` to allow input again.");
        println!();
    }

    println!(
        "wgaf {} — pid {}, up {}s, on {}",
        s.daemon_version, s.daemon_pid, s.daemon_uptime_seconds, s.daemon_bus_name
    );
    if !s.config_path.is_empty() {
        // An absent file is normal, not a fault — but naming the location is
        // exactly what someone asking "where does config go?" needs, so the
        // path is always shown and its presence spelled out.
        let note = if s.config_present {
            ""
        } else {
            " (not present — built-in defaults apply)"
        };
        println!("config: {}{note}", s.config_path);
    }
    println!();

    println!(
        "{} GNOME Shell Extension  ({})",
        marker(s.extension_available),
        s.extension_bus_name
    );
    if !s.extension_detail.is_empty() {
        println!("       {}", s.extension_detail);
    }

    let device = if s.input_device_created {
        format!("{}, device active", s.input_device_name)
    } else {
        format!("{}, no device created yet", s.input_device_name)
    };
    println!(
        "{} Input (/dev/uinput)    ({device})",
        marker(s.uinput_accessible)
    );
    if !s.uinput_detail.is_empty() {
        println!("       {}", s.uinput_detail);
    }

    // Both halves, always. The configured value alone does not say which
    // layout `auto` picked, and the resolved name alone does not say whether
    // it was chosen or detected — and after changing the desktop's layout, the
    // gap between the two is the answer to "why did my text come out wrong".
    let layout = if s.input_keyboard_layout_resolved.is_empty() {
        format!("{}, not resolved yet", s.input_keyboard_layout_configured)
    } else if s.input_keyboard_layout_configured == s.input_keyboard_layout_resolved {
        s.input_keyboard_layout_resolved.clone()
    } else {
        format!(
            "{} -> {}",
            s.input_keyboard_layout_configured, s.input_keyboard_layout_resolved
        )
    };
    println!("       keyboard layout: {layout}");

    let a11y = if s.accessibility_connected {
        "connected"
    } else {
        "not connected yet"
    };
    println!(
        "{} Accessibility (AT-SPI) ({a11y})",
        marker(s.accessibility_available)
    );
    if !s.accessibility_detail.is_empty() {
        println!("       {}", s.accessibility_detail);
    }

    println!();
    if s.permissions_path.is_empty() {
        println!("permissions: no policy file — every capability allowed");
    } else if !s.permissions_present {
        println!(
            "permissions: {} (not present — every capability allowed)",
            s.permissions_path
        );
        println!("       create that file to restrict one; see `man wgaf-status`");
    } else {
        println!("permissions: {}", s.permissions_path);
        if s.permissions_restricted.is_empty() {
            println!("       no capability restricted — every capability allowed");
        } else {
            for entry in &s.permissions_restricted {
                println!("       {entry}");
            }
        }
    }
    if !s.permissions_prompt_decisions.is_empty() {
        println!("prompted this run (not persisted):");
        for entry in &s.permissions_prompt_decisions {
            println!("       {entry}");
        }
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

/// Prints one record of a stream as a single line of JSON, and flushes.
///
/// # Why this is not [`print_json`]
///
/// A streaming command emits **newline-delimited JSON — one object per line,
/// never a JSON array.** An array cannot be produced incrementally: its closing
/// bracket only arrives when the stream ends, and a command like
/// `wgaf window watch` is expected to run until interrupted. A consumer piping
/// it into `jq` would get nothing at all until the process died, and nothing
/// ever if it was killed. One object per line is readable a line at a time,
/// which is the whole point of a stream.
///
/// Compact rather than pretty for the same reason: pretty-printing spreads one
/// record over several lines, and "one line per event" is what makes the output
/// greppable and splittable.
///
/// The explicit flush matters when stdout is a pipe. Rust line-buffers a
/// terminal but block-buffers a pipe, so without it `wgaf window watch --json |
/// jq` would sit silent until 8 KB had accumulated — which on an idle desktop
/// is indistinguishable from a watch that is not working.
pub fn print_json_line<T>(value: &T) -> Result<(), Box<dyn std::error::Error>>
where
    T: serde::Serialize + ?Sized,
{
    use std::io::Write;

    println!("{}", serde_json::to_string(value)?);
    std::io::stdout().flush()?;
    Ok(())
}
