//! `wgaf window ...` subcommands: a thin D-Bus client of the daemon's own
//! `org.wgaf.Windows1` interface (itself a thin wrapper around the GNOME
//! Shell Extension bridge — see `wgaf-daemon/src/windows/mod.rs` and
//! `wgaf-daemon/src/dbus/windows_api.rs`). No business logic here: parse
//! args (done by `clap` in `main.rs`), call the daemon, format the reply.

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};

use super::{CliResult, connect, map_err};

pub async fn list(bus_name: &str, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "ListWindows",
            &(),
        )
        .await
        .map_err(map_err)?;
    let dicts: Vec<WindowRecordDict> = reply.body().deserialize()?;
    let windows: Vec<WindowRecord> = dicts.into_iter().map(Into::into).collect();

    if json {
        crate::output::print_json(&windows)?;
    } else if windows.is_empty() {
        println!("No windows.");
    } else {
        for w in &windows {
            let flags = match (w.focused, w.maximized) {
                (true, true) => " [focused, maximized]",
                (true, false) => " [focused]",
                (false, true) => " [maximized]",
                (false, false) => "",
            };
            println!(
                "{:>4}  ws={:<3} {:>5},{:<5} {:>4}x{:<4}  {:<20}  {}{}",
                w.id, w.workspace, w.x, w.y, w.width, w.height, w.app_id, w.title, flags
            );
        }
    }
    Ok(())
}

/// Streams window events until interrupted.
///
/// # Asking permission before subscribing
///
/// `WatchWindows` is called first and its failure is reported rather than
/// swallowed. Without it a denied caller would sit on a silent stream, which on
/// an idle desktop is indistinguishable from a working one — the user would
/// conclude wgaf was broken rather than that their own policy said no. The
/// daemon's error names `permissions.toml`, so nothing extra is added here.
///
/// The same call also surfaces a missing GNOME Shell extension up front, for the
/// same reason: silence and "not installed" look identical from here.
///
/// # No replay
///
/// D-Bus signals are fire-and-forget. Whatever happened before this command
/// started is gone and cannot be asked for. `wgaf window list` is the snapshot;
/// this is the feed.
pub async fn watch(bus_name: &str, json: bool) -> CliResult<()> {
    use futures_util::StreamExt;

    let connection = connect().await?;

    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "WatchWindows",
            &(),
        )
        .await
        .map_err(map_err)?;

    // Built before the first event can be consumed, but note this is not a
    // guarantee of catching everything: the daemon has been emitting since it
    // started, and anything before this match rule reached the bus was never
    // ours to see. That is the no-replay contract, not a gap to close.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(bus_name)?
        .path(wgaf_common::WINDOWS_OBJECT_PATH)?
        .interface(wgaf_common::WINDOWS_INTERFACE_NAME)?
        .build();
    let mut stream = zbus::MessageStream::for_match_rule(rule, &connection, None).await?;

    if !json {
        eprintln!("Watching for window events. Press Ctrl-C to stop.");
    }

    loop {
        tokio::select! {
            // Ctrl-C is a normal way to end this command, not a failure: it is
            // the documented way to stop, so it exits zero and prints nothing
            // alarming. Biased so a pending interrupt wins over a busy stream
            // rather than waiting for a lull on an active desktop.
            biased;

            _ = tokio::signal::ctrl_c() => {
                if !json {
                    eprintln!("Stopped.");
                }
                return Ok(());
            }

            message = stream.next() => {
                let Some(message) = message else {
                    // The daemon exited or dropped the connection. Worth an
                    // error rather than a silent zero exit: a script that
                    // depends on this feed needs to know it ended early.
                    return Err("the connection to wgaf-daemon ended".into());
                };

                let message = message?;
                let Some(member) = message.header().member().map(|m| m.to_string()) else {
                    continue;
                };

                // Every one of these signals carries a bare window id. The
                // creation record the extension sends is blank at emission time
                // and the daemon discards it, so `wgaf window list` is where
                // detail comes from.
                let id: u32 = match message.body().deserialize() {
                    Ok(id) => id,
                    // A signal whose body does not match is skipped rather than
                    // ending the watch. A future daemon adding a differently
                    // shaped signal on this interface must not stop an older
                    // client dead.
                    Err(_) => continue,
                };

                let event = match member.as_str() {
                    "WindowCreated" => "created",
                    "WindowClosed" => "closed",
                    "WindowFocusChanged" => "focus-changed",
                    _ => continue,
                };

                if json {
                    crate::output::print_json_line(&serde_json::json!({
                        "event": event,
                        "id": id,
                    }))?;
                } else {
                    println!("{event:<13} {id}");
                }
            }
        }
    }
}

pub async fn focus(bus_name: &str, id: u32, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "FocusWindow",
            &(id,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("focused window {id}"));
    Ok(())
}

pub async fn move_window(bus_name: &str, id: u32, x: i32, y: i32, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "MoveWindow",
            &(id, x, y),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("moved window {id} to ({x}, {y})"));
    Ok(())
}

pub async fn resize(bus_name: &str, id: u32, width: i32, height: i32, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "ResizeWindow",
            &(id, width, height),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("resized window {id} to {width}x{height}"));
    Ok(())
}

pub async fn close(bus_name: &str, id: u32, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "CloseWindow",
            &(id,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("closed window {id}"));
    Ok(())
}

pub async fn workspaces(bus_name: &str, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "GetWorkspaces",
            &(),
        )
        .await
        .map_err(map_err)?;
    let dicts: Vec<WorkspaceRecordDict> = reply.body().deserialize()?;
    let workspaces: Vec<WorkspaceRecord> = dicts.into_iter().map(Into::into).collect();

    if json {
        crate::output::print_json(&workspaces)?;
    } else if workspaces.is_empty() {
        println!("No workspaces.");
    } else {
        for w in &workspaces {
            println!(
                "{:>3}  windows={:<3}{}",
                w.index,
                w.n_windows,
                if w.active { "  [active]" } else { "" }
            );
        }
    }
    Ok(())
}
