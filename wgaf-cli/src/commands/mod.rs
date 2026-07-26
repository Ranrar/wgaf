pub mod window;

use zbus::Connection;

pub async fn ping() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::session().await?;
    let reply = connection
        .call_method(
            Some(wgaf_common::BUS_NAME),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await?;
    let response: String = reply.body().deserialize()?;
    println!("{response}");
    Ok(())
}

/// Recognizes the daemon's named `org.wgaf.Windows1` D-Bus errors and
/// returns a short, human-friendly message for them instead of the raw
/// `zbus::Error` debug dump. Returns `None` for anything else, so the
/// caller can fall back to propagating the original error.
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
    } else {
        None
    }
}
