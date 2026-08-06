//! `wgaf workspace ...` subcommands: a thin D-Bus client of the daemon's own
//! `org.wgaf.Windows1` interface, which is itself a wrapper around the GNOME
//! Shell Extension bridge (see `wgaf-daemon/src/windows/mod.rs`). No business
//! logic here: parse args (done by `clap` in `main.rs`), call the daemon,
//! format the reply.
//!
//! Workspaces live on `org.wgaf.Windows1` rather than an interface of their
//! own because Mutter's workspace manager is reached through the same
//! extension bridge as its windows are. The split into a separate CLI noun is
//! about how the commands read — switching a workspace is not an operation on
//! a window.

use wgaf_common::dict::{WorkspaceLayoutDict, WorkspaceRecordDict};
use wgaf_common::{WorkspaceLayout, WorkspaceRecord};

use super::{CliResult, connect, map_err};

/// Calls one method on `org.wgaf.Windows1`, since every command in this module
/// is the same three lines with a different method name.
async fn call<A>(bus_name: &str, method: &str, args: &A) -> CliResult<zbus::Message>
where
    A: serde::Serialize + zbus::zvariant::Type + std::fmt::Debug,
{
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            method,
            args,
        )
        .await
        .map_err(map_err)
}

pub async fn list(bus_name: &str, json: bool) -> CliResult<()> {
    let reply = call(bus_name, "GetWorkspaces", &()).await?;
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

pub async fn layout(bus_name: &str, json: bool) -> CliResult<()> {
    let reply = call(bus_name, "GetWorkspaceLayout", &()).await?;
    let dict: WorkspaceLayoutDict = reply.body().deserialize()?;
    let layout: WorkspaceLayout = dict.into();

    if json {
        crate::output::print_json(&layout)?;
    } else {
        println!("workspaces: {}", layout.n_workspaces);
        println!("active:     {}", layout.active);
        println!(
            "grid:       {} rows x {} columns",
            layout.rows, layout.columns
        );
        // Spelled out rather than printed as a bare `true`/`false`, because
        // the consequence is the point: someone reading this is about to run
        // `add` or `remove` and needs to know whether the result will stay put.
        if layout.dynamic {
            println!(
                "managed by: GNOME (dynamic workspaces — an added workspace is reclaimed once \
                 it is left empty)"
            );
        } else {
            println!("managed by: you (the workspace count is fixed)");
        }
    }
    Ok(())
}

pub async fn switch(bus_name: &str, index: i32, json: bool) -> CliResult<()> {
    call(bus_name, "SwitchWorkspace", &(index,)).await?;
    // "switched to" rather than "asked to switch to": the daemon does not
    // return until the workspace is actually active.
    crate::output::print_ok(json, &format!("switched to workspace {index}"));
    Ok(())
}

pub async fn add(bus_name: &str, json: bool) -> CliResult<()> {
    let reply = call(bus_name, "AddWorkspace", &()).await?;
    let index: i32 = reply.body().deserialize()?;
    // The index goes in the JSON as a number, not only inside the sentence:
    // it is the whole result of this command, and a script that had to parse
    // it back out of prose would break the next time the wording changed.
    crate::output::print_ok_with(
        json,
        &format!("added workspace {index}"),
        serde_json::json!({ "index": index }),
    );
    Ok(())
}

pub async fn remove(bus_name: &str, index: i32, json: bool) -> CliResult<()> {
    call(bus_name, "RemoveWorkspace", &(index,)).await?;
    crate::output::print_ok(json, &format!("removed workspace {index}"));
    Ok(())
}

pub async fn reorder(bus_name: &str, index: i32, new_index: i32, json: bool) -> CliResult<()> {
    call(bus_name, "ReorderWorkspace", &(index, new_index)).await?;
    crate::output::print_ok(
        json,
        &format!("moved workspace {index} to position {new_index}"),
    );
    Ok(())
}
