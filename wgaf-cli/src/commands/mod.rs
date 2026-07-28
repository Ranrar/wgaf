pub mod accessibility;
pub mod input;
pub mod window;

use zbus::Connection;

pub async fn ping(bus_name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::session().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await?;
    let response: String = reply.body().deserialize()?;
    // FIXED: `--json` is a global flag advertised on every subcommand's
    // `--help` (including this one), but this command silently ignored it
    // and always printed plain text — found during the Phase 7 --help
    // consistency pass. Every other command family already honors it.
    crate::output::print_ok_response(json, &response);
    Ok(())
}

/// Recognizes the daemon's named `org.wgaf.Windows1`/`org.wgaf.Input1`
/// D-Bus errors and returns a short, human-friendly message for them
/// instead of the raw `zbus::Error` debug dump. Returns `None` for anything
/// else, so the caller can fall back to propagating the original error.
pub(crate) fn describe_dbus_error(err: &zbus::Error) -> Option<String> {
    let zbus::Error::MethodError(name, description, _) = err else {
        return None;
    };

    let detail = description
        .as_deref()
        .map(|d| format!(": {d}"))
        .unwrap_or_default();

    if name.as_str() == wgaf_common::WINDOWS_ERROR_WINDOW_NOT_FOUND {
        Some(format!("window not found{detail}"))
    } else if name.as_str() == wgaf_common::WINDOWS_ERROR_EXTENSION_UNAVAILABLE {
        Some(format!("GNOME Shell Extension bridge unavailable{detail}"))
    } else if name.as_str() == wgaf_common::INPUT_ERROR_DEVICE_UNAVAILABLE {
        Some(format!("input device unavailable{detail}"))
    } else if name.as_str() == wgaf_common::INPUT_ERROR_UNKNOWN_KEY {
        Some(format!("unknown key{detail}"))
    } else if name.as_str() == wgaf_common::INPUT_ERROR_INVALID_BUTTON {
        Some(format!("invalid mouse button{detail}"))
    } else if name.as_str() == wgaf_common::ACCESSIBILITY_ERROR_BUS_UNAVAILABLE {
        Some(format!("AT-SPI accessibility bus unavailable{detail}"))
    } else if name.as_str() == wgaf_common::ACCESSIBILITY_ERROR_APP_NOT_FOUND {
        Some(format!("accessible application not found{detail}"))
    } else if name.as_str() == wgaf_common::ACCESSIBILITY_ERROR_ELEMENT_NOT_FOUND {
        Some(format!("accessible element not found{detail}"))
    } else if name.as_str() == wgaf_common::ACCESSIBILITY_ERROR_ACTION_NOT_SUPPORTED {
        Some(format!("action not supported{detail}"))
    } else if name.as_str() == wgaf_common::WINDOWS_ERROR_PERMISSION_DENIED
        || name.as_str() == wgaf_common::INPUT_ERROR_PERMISSION_DENIED
        || name.as_str() == wgaf_common::ACCESSIBILITY_ERROR_PERMISSION_DENIED
    {
        Some(format!("permission denied{detail}"))
    } else {
        None
    }
}
