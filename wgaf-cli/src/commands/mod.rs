pub mod accessibility;
pub mod input;
pub mod monitor;
pub mod window;
pub mod workspace;

use zbus::Connection;

// Re-exported so `window`/`input`/`accessibility` can keep writing
// `use super::{connect, map_err, CliResult};` instead of reaching into
// `crate::error` themselves — `commands` is still the module that speaks for
// "how a D-Bus call becomes a command result", even though the classification
// table itself now lives in `crate::error` (see that module's doc comment for
// why it moved).
pub(crate) use crate::error::{CliResult, map_err};

pub async fn ping(bus_name: &str, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await
        // `ping` is usually the first command a user runs, and the most
        // likely thing to go wrong for them is "the daemon isn't running" —
        // so it deserves the same readable rendering as every other command
        // rather than the propagated raw `zbus::Error` it used before.
        .map_err(map_err)?;
    let response: String = reply.body().deserialize()?;
    // FIXED: `--json` is a global flag advertised on every subcommand's
    // `--help` (including this one), but this command silently ignored it
    // and always printed plain text — found during the Phase 7 --help
    // consistency pass. Every other command family already honors it.
    crate::output::print_ok_response(json, &response);
    Ok(())
}

/// `wgaf stop` — the kill switch.
///
/// Ungated by design: no `permissions.toml` policy can refuse it, so this
/// command fails only if the daemon cannot be reached at all.
pub async fn stop(bus_name: &str, json: bool) -> CliResult<()> {
    daemon_call(bus_name, "Stop").await?;
    crate::output::print_ok(
        json,
        "input stopped — no keystrokes, clicks or scrolls will be synthesized. \
         Run `wgaf release` to allow them again.",
    );
    Ok(())
}

/// `wgaf release` — lifts the kill switch.
///
/// The message says what the command does *not* do, because that is the part
/// people expect: releasing a brake is not resuming a journey, and the command
/// that was interrupted has to be run again.
pub async fn release(bus_name: &str, json: bool) -> CliResult<()> {
    daemon_call(bus_name, "Release").await?;
    crate::output::print_ok(
        json,
        "input released — new commands are allowed again. Whatever was stopped \
         is not restarted; run it again if you still want it.",
    );
    Ok(())
}

/// Calls an argument-less, reply-less `org.wgaf.Daemon1` method.
async fn daemon_call(bus_name: &str, method: &str) -> CliResult<()> {
    connect()
        .await?
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            method,
            &(),
        )
        .await
        .map_err(map_err)?;
    Ok(())
}

/// `wgaf status` — the daemon's self-report, rendered for a human (or as
/// JSON).
///
/// Returns `Ok(false)` when some subsystem is unavailable, so `main` can exit
/// non-zero without treating it as a CLI error: an unhealthy subsystem is a
/// successful *report*, not a failed command, and conflating the two would
/// make `wgaf status` print its own error text instead of the daemon's
/// actionable guidance.
pub async fn status(bus_name: &str, json: bool) -> CliResult<bool> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Status",
            &(),
        )
        .await
        .map_err(map_err)?;
    let dict: wgaf_common::dict::DaemonStatusDict = reply.body().deserialize()?;
    let status: wgaf_common::DaemonStatus = dict.into();

    let healthy =
        status.extension_available && status.uinput_accessible && status.accessibility_available;

    if json {
        crate::output::print_json(&status)?;
    } else {
        crate::output::print_status(&status);
    }
    Ok(healthy)
}

/// Opens a session-bus connection. Shared by every command family — see
/// [`crate::error::map_err`] for the other half of this pair.
pub(crate) async fn connect() -> zbus::Result<Connection> {
    Connection::session().await
}
