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
//!
//! `type`/`key press`/`key release`/`key combo` additionally take an
//! optional `--window`, routed here to the daemon's `*At` method variant
//! (`TypeTextAt`/`KeyPressAt`/`KeyReleaseAt`/`HotkeyAt`) instead of the
//! untargeted one — see each function's doc comment. `wgaf mouse ...` does
//! not get this flag: mouse delivery is arbitrated by pointer position, not
//! keyboard focus, and needs a different mechanism (`backlog.md` §2) that
//! was deliberately filed separately and is not built.

use super::{CliResult, connect, map_err};

/// `window` selects between the daemon's untargeted method and its `*At`
/// counterpart (`wgaf-daemon/src/dbus/input_api.rs`). `None` sends byte-for-
/// byte the same method name and argument tuple as before this parameter
/// existed, so a script that never passes `--window` is unaffected by
/// `verification_level` — see that setting's docs in `config.toml`.
pub async fn type_text(
    bus_name: &str,
    text: &str,
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "TypeTextAt",
                    &(text, window),
                )
                .await
                .map_err(map_err)?;
            format!(
                "typed {} character(s) into window {window}",
                text.chars().count()
            )
        }
        None => {
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
            format!("typed {} character(s)", text.chars().count())
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}

/// See [`type_text`]'s doc comment for what `window` does and the
/// `None`-is-unchanged guarantee.
pub async fn key_press(
    bus_name: &str,
    key: &str,
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "KeyPressAt",
                    &(key, window),
                )
                .await
                .map_err(map_err)?;
            format!("pressed key `{key}` into window {window}")
        }
        None => {
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
            format!("pressed key `{key}`")
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}

/// See [`type_text`]'s doc comment for what `window` does and the
/// `None`-is-unchanged guarantee.
pub async fn key_release(
    bus_name: &str,
    key: &str,
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "KeyReleaseAt",
                    &(key, window),
                )
                .await
                .map_err(map_err)?;
            format!("released key `{key}` into window {window}")
        }
        None => {
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
            format!("released key `{key}`")
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}

/// See [`type_text`]'s doc comment for what `window` does and the
/// `None`-is-unchanged guarantee.
pub async fn hotkey(
    bus_name: &str,
    keys: &[String],
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "HotkeyAt",
                    &(keys, window),
                )
                .await
                .map_err(map_err)?;
            format!("pressed `{}` into window {window}", keys.join("+"))
        }
        None => {
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
            format!("pressed `{}`", keys.join("+"))
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}

pub async fn mouse_move(bus_name: &str, dx: i32, dy: i32, json: bool) -> CliResult<()> {
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
pub async fn mouse_move_to(bus_name: &str, x: i32, y: i32, json: bool) -> CliResult<()> {
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
pub async fn mouse_position(bus_name: &str, json: bool) -> CliResult<()> {
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

/// `window` asks the daemon to confirm the pointer is over that window before
/// clicking, and to click nothing if it is not.
///
/// **A different question from [`type_text`]'s `window`**, despite the same
/// flag name: that one is about keyboard focus and corrects it when it can,
/// this one is about what the pointer is over and only ever refuses. See
/// `MouseClickAt` in the daemon for why moving the pointer would be the wrong
/// favour.
pub async fn mouse_click(
    bus_name: &str,
    button: &str,
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "MouseClickAt",
                    &(button, window),
                )
                .await
                .map_err(map_err)?;
            format!("clicked {button} mouse button in window {window}")
        }
        None => {
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
            format!("clicked {button} mouse button")
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}

/// See [`mouse_click`]'s doc comment for what `window` does.
pub async fn mouse_scroll(
    bus_name: &str,
    dx: i32,
    dy: i32,
    window: Option<u32>,
    json: bool,
) -> CliResult<()> {
    let connection = connect().await?;
    let message = match window {
        Some(window) => {
            connection
                .call_method(
                    Some(bus_name),
                    wgaf_common::INPUT_OBJECT_PATH,
                    Some(wgaf_common::INPUT_INTERFACE_NAME),
                    "MouseScrollAt",
                    &(dx, dy, window),
                )
                .await
                .map_err(map_err)?;
            format!("scrolled by ({dx}, {dy}) in window {window}")
        }
        None => {
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
            format!("scrolled by ({dx}, {dy})")
        }
    };
    crate::output::print_ok(json, &message);
    Ok(())
}
