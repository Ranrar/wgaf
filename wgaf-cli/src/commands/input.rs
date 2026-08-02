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

use super::{connect, map_err};

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

pub async fn hotkey(
    bus_name: &str,
    keys: &[String],
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "Hotkey",
            &(keys,),
        )
        .await
        .map_err(map_err)?;
    crate::output::print_ok(json, &format!("pressed `{}`", keys.join("+")));
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

/// Moves the pointer to an absolute screen position.
///
/// The daemon replies with the position actually reached, and that is what gets
/// reported rather than the position that was requested. They are the same in
/// every ordinary case; printing the reply means that if they ever differ, the
/// user sees where the pointer really is instead of being told what they
/// already typed.
pub async fn mouse_move_to(
    bus_name: &str,
    x: i32,
    y: i32,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "MouseMoveAbsolute",
            &(x, y),
        )
        .await
        .map_err(map_err)?;
    let (actual_x, actual_y): (i32, i32) = reply.body().deserialize()?;

    if json {
        crate::output::print_json(&PointerPosition {
            x: actual_x,
            y: actual_y,
        })?;
    } else {
        crate::output::print_ok(json, &format!("moved pointer to ({actual_x}, {actual_y})"));
    }
    Ok(())
}

/// Prints the pointer's current screen position.
pub async fn mouse_position(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            "GetPointerPosition",
            &(),
        )
        .await
        .map_err(map_err)?;
    let (x, y): (i32, i32) = reply.body().deserialize()?;

    if json {
        crate::output::print_json(&PointerPosition { x, y })?;
    } else {
        println!("{x} {y}");
    }
    Ok(())
}

/// `--json` shape for the two pointer-position commands.
///
/// Bare `x`/`y` rather than wrapped in an `ok` envelope, because these return a
/// *record* like `window list` does, not a bare success like `mouse click`.
#[derive(serde::Serialize)]
struct PointerPosition {
    x: i32,
    y: i32,
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
