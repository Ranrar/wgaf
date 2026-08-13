//! `wgaf a11y ...` subcommands: a thin D-Bus client of the daemon's own
//! `org.wgaf.Accessibility1` interface (see
//! `wgaf-daemon/src/dbus/accessibility_api.rs` and
//! `wgaf-daemon/src/accessibility/`). No business logic here, same
//! convention as `commands/window.rs`/`commands/input.rs`: parse args (done
//! by `clap` in `main.rs`), call the daemon, format the reply.

use wgaf_common::{AppRecord, ElementRecord, ElementRef, TreeNode};
use zbus::Connection;

use super::{CliResult, connect, map_err};

async fn call<R, A>(
    connection: &Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::ACCESSIBILITY_OBJECT_PATH,
            Some(wgaf_common::ACCESSIBILITY_INTERFACE_NAME),
            method,
            args,
        )
        .await?;
    reply.body().deserialize()
}

pub async fn list_apps(bus_name: &str, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let apps: Vec<AppRecord> = call(&connection, bus_name, "ListApps", &())
        .await
        .map_err(map_err)?;

    if json {
        crate::output::print_json(&apps)?;
    } else if apps.is_empty() {
        println!("No accessible applications registered.");
    } else {
        for app in &apps {
            println!("{:<30}  {}", app.name, app.element);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn find(
    bus_name: &str,
    app: &str,
    role: &str,
    name: &str,
    description: &str,
    max_results: i32,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let elements: Vec<ElementRecord> = call(
        &connection,
        bus_name,
        "FindElements",
        &(app, role, name, description, max_results),
    )
    .await
    .map_err(map_err)?;

    if json {
        crate::output::print_json(&elements)?;
    } else if elements.is_empty() {
        println!("No matching elements found.");
    } else {
        for e in &elements {
            println!(
                "{:<20}  {:<24}  {}  {}",
                e.role, e.name, e.element, e.description
            );
        }
    }
    Ok(())
}

pub async fn tree(bus_name: &str, app: &str, max_depth: i32, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let nodes: Vec<TreeNode> = call(&connection, bus_name, "GetTree", &(app, max_depth))
        .await
        .map_err(map_err)?;

    if json {
        crate::output::print_json(&nodes)?;
    } else if nodes.is_empty() {
        println!("No elements found.");
    } else {
        for n in &nodes {
            let indent = "  ".repeat(n.depth as usize);
            println!("{indent}{} \"{}\"  {}", n.role, n.name, n.element);
        }
    }
    Ok(())
}

pub async fn get_element_info(bus_name: &str, element: &ElementRef, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let info: ElementRecord = call(&connection, bus_name, "GetElementInfo", &(element.clone(),))
        .await
        .map_err(map_err)?;

    if json {
        crate::output::print_json(&info)?;
    } else {
        println!("name:        {}", info.name);
        println!("role:        {}", info.role);
        println!("description: {}", info.description);
        println!("child_count: {}", info.child_count);
        println!("states:      {}", info.states.join(", "));
        println!("element:     {}", info.element);
    }
    Ok(())
}

pub async fn click(
    bus_name: &str,
    element: &ElementRef,
    action: &str,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    call::<(), _>(
        &connection,
        bus_name,
        "InvokeAction",
        &(element.clone(), action),
    )
    .await
    .map_err(map_err)?;
    crate::output::print_ok(json, &format!("invoked action on {element}"));
    Ok(())
}

pub async fn set_text(
    bus_name: &str,
    element: &ElementRef,
    text: &str,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    call::<(), _>(&connection, bus_name, "SetText", &(element.clone(), text))
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("set text on {element}"));
    Ok(())
}

/// Prints the element's text verbatim on success.
///
/// **Not `print_ok`**, which would wrap it in a sentence: the point of this
/// command is that its output *is* the value, so `$(wgaf a11y text "$ref")`
/// captures the text and nothing else. `--json` puts it in the `message` field
/// every other command uses, so a JSON consumer needs no special case either.
pub async fn text(bus_name: &str, element: &ElementRef, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let text: String = call(&connection, bus_name, "GetElementText", &(element.clone(),))
        .await
        .map_err(map_err)?;
    if json {
        crate::output::print_ok(json, &text);
    } else {
        // No trailing newline: the text is the output, and a newline this
        // command invented would end up inside the captured value.
        print!("{text}");
    }
    Ok(())
}

pub async fn focus(bus_name: &str, element: &ElementRef, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    call::<(), _>(&connection, bus_name, "FocusElement", &(element.clone(),))
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("focused {element}"));
    Ok(())
}

pub async fn scroll_to(bus_name: &str, element: &ElementRef, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    call::<(), _>(&connection, bus_name, "ScrollElement", &(element.clone(),))
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("scrolled {element} into view"));
    Ok(())
}
