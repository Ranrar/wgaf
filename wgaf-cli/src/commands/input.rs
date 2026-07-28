//! `wgaf type`/`wgaf key`/`wgaf mouse ...` subcommands: a thin D-Bus client
//! of the daemon's own `org.wgaf.Input1` interface (see
//! `wgaf-daemon/src/dbus/input_api.rs` and `wgaf-daemon/src/input/`). No
//! business logic here, same convention as `commands/window.rs`: parse args
//! (done by `clap` in `main.rs`), call the daemon, format the reply.
//!
//! Every one of these calls is fire-and-forget from the CLI's perspective
//! (the daemon's `org.wgaf.Input1` methods all return `()` on success) —
//! `--json` therefore just wraps a `{"ok": true, "message": ...}` status,
//! emitted by the shared `crate::output::print_ok`.

use zbus::Connection;

use super::describe_dbus_error;

async fn connect() -> zbus::Result<Connection> {
    Connection::session().await
}

/// Turns a `zbus::Error` into a boxed error, substituting the daemon's named
/// `org.wgaf.Input1` error (device-unavailable / unknown-key / invalid-button)
/// for a short human-readable message where recognized.
fn map_err(err: zbus::Error) -> Box<dyn std::error::Error> {
    match describe_dbus_error(&err) {
        Some(msg) => msg.into(),
        None => Box::new(err),
    }
}

pub async fn type_text(
    bus_name: &str,
    text: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "TypeText",
            &(text,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(
        json,
        &format!("typed {} character(s)", text.chars().count()),
    );
    Ok(())
}

pub async fn key_press(
    bus_name: &str,
    key: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "KeyPress",
            &(key,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("pressed key `{key}`"));
    Ok(())
}

pub async fn key_release(
    bus_name: &str,
    key: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "KeyRelease",
            &(key,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("released key `{key}`"));
    Ok(())
}

pub async fn mouse_move(
    bus_name: &str,
    dx: i32,
    dy: i32,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "MouseMove",
            &(dx, dy),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("moved mouse by ({dx}, {dy})"));
    Ok(())
}

pub async fn mouse_click(
    bus_name: &str,
    button: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "MouseClick",
            &(button,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("clicked {button} mouse button"));
    Ok(())
}

pub async fn mouse_scroll(
    bus_name: &str,
    dx: i32,
    dy: i32,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "MouseScroll",
            &(dx, dy),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("scrolled by ({dx}, {dy})"));
    Ok(())
}
