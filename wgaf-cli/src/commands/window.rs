//! `wgaf window ...` subcommands: a thin D-Bus client of the daemon's own
//! `org.wgaf.Windows1` interface (itself a thin wrapper around the GNOME
//! Shell Extension bridge — see `wgaf-daemon/src/windows/mod.rs` and
//! `wgaf-daemon/src/dbus/windows_api.rs`). No business logic here: parse
//! args (done by `clap` in `main.rs`), call the daemon, format the reply.

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};
use wgaf_common::{WindowRecord, WorkspaceRecord};

use super::{connect, map_err};

pub async fn list(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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

pub async fn focus(bus_name: &str, id: u32, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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

pub async fn move_window(
    bus_name: &str,
    id: u32,
    x: i32,
    y: i32,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

pub async fn resize(
    bus_name: &str,
    id: u32,
    width: i32,
    height: i32,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

pub async fn close(bus_name: &str, id: u32, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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

pub async fn workspaces(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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
