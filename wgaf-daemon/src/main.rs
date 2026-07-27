mod accessibility;
mod config;
mod dbus;
mod input;
mod permissions;
mod windows;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use config::Config;
use tracing_subscriber::EnvFilter;

/// wgaf background daemon: owns the session D-Bus service that the CLI and
/// GNOME Shell Extension talk to.
#[derive(Parser)]
struct Args {
    /// Path to a TOML config file.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to a TOML permission-policy file. Same file format as
    /// `--config` (plain TOML), always expected as a *sibling* of wherever
    /// `--config`'s file lives — same "explicit path, or a sensible
    /// default" shape as `Config::load`, except here the *default* itself
    /// is "no file -> every capability defaults to Allow" rather than a
    /// `Default` struct — see `permissions::policy`'s module docs. If
    /// omitted, the daemon looks for `permissions.toml` next to
    /// `--config`'s file (or, if `--config` itself was omitted, does not
    /// look anywhere and simply runs all-Allow — a real default `--config`
    /// path — and thus a real default location for this file too — isn't
    /// implemented yet).
    #[arg(long)]
    permissions: Option<PathBuf>,

    /// Override the configured log level (trace, debug, info, warn, error).
    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut config = Config::load(args.config.as_deref())?;
    if let Some(log_level) = args.log_level {
        config.log_level = log_level;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(bus_name = %config.bus_name, "starting wgaf-daemon");

    // Resolve the permission-policy file path: an explicit `--permissions`
    // wins; otherwise default to a `permissions.toml` sibling of
    // `--config`'s file (same directory), if one was given. `PolicyMap::load`
    // treats a missing/nonexistent path as "every capability defaults to
    // Allow", not an error — see `permissions::policy`'s module docs — so
    // there is deliberately no fallback path to try when `--config` itself
    // was never given (there is, as yet, no default `--config` location to
    // anchor a sibling lookup to at all — the intended eventual default is
    // XDG-based).
    let permissions_path = args.permissions.or_else(|| {
        args.config
            .as_ref()
            .and_then(|c| c.parent())
            .map(|dir| dir.join("permissions.toml"))
    });
    let policy = permissions::PolicyMap::load(permissions_path.as_deref())?;
    let permission_gate = Arc::new(permissions::PermissionGate::new(policy));
    tracing::info!(
        permissions_path = ?permissions_path,
        "loaded permission policy (unmentioned capabilities default to Allow)"
    );

    // A separate session-bus connection used only as a client of the GNOME
    // Shell Extension bridge — independent from the connection the daemon
    // serves its own D-Bus API on below. Building the proxy here does not
    // require the extension to already be running (see
    // `windows::WindowManager`'s doc comments): if it's not installed or
    // not enabled yet, window-management calls fail with a clear error at
    // call time rather than blocking daemon startup, so input automation
    // and future AT-SPI features can still come up independently.
    let extension_connection = zbus::Connection::session().await?;
    let window_manager = windows::WindowManager::connect_to(
        extension_connection,
        &config.extension_bus_name,
        wgaf_common::EXTENSION_OBJECT_PATH,
        wgaf_common::EXTENSION_INTERFACE_NAME,
    )
    .await?;

    // `InputBackend::new` does not touch `/dev/uinput` — like
    // `WindowManager::connect_to` above, the actual resource (the virtual
    // uinput device) is only created lazily on first real use, so a
    // permissions problem here never prevents the daemon from starting or
    // from serving `org.wgaf.Windows1`. See `input::InputBackend`'s doc
    // comments.
    let input_backend = input::InputBackend::new(config.input_device_name.clone());

    // `AccessibilityBackend::new` does not touch the AT-SPI bus — like
    // `WindowManager::connect_to`/`InputBackend::new` above, the actual
    // resource (a connection to `org.a11y.Bus`) is only established lazily
    // on first real use, so accessibility being unavailable/not-yet-enabled
    // for this session never prevents the daemon from starting or from
    // serving the other interfaces. See `accessibility::AccessibilityBackend`'s
    // doc comments.
    let accessibility_backend = accessibility::AccessibilityBackend::new();

    let _connection = zbus::connection::Builder::session()?
        .name(config.bus_name.as_str())?
        .serve_at(wgaf_common::OBJECT_PATH, dbus::Daemon)?
        .serve_at(
            wgaf_common::WINDOWS_OBJECT_PATH,
            dbus::windows_api::WindowsApi::new(window_manager, Arc::clone(&permission_gate)),
        )?
        .serve_at(
            wgaf_common::INPUT_OBJECT_PATH,
            dbus::input_api::InputApi::new(input_backend, Arc::clone(&permission_gate)),
        )?
        .serve_at(
            wgaf_common::ACCESSIBILITY_OBJECT_PATH,
            dbus::accessibility_api::AccessibilityApi::new(
                accessibility_backend,
                Arc::clone(&permission_gate),
            ),
        )?
        .build()
        .await?;

    tracing::info!(
        object_path = wgaf_common::OBJECT_PATH,
        windows_object_path = wgaf_common::WINDOWS_OBJECT_PATH,
        input_object_path = wgaf_common::INPUT_OBJECT_PATH,
        accessibility_object_path = wgaf_common::ACCESSIBILITY_OBJECT_PATH,
        "registered on session bus, waiting for requests"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
