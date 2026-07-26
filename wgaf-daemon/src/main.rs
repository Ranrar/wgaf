mod config;
mod dbus;
mod windows;

use std::path::PathBuf;

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

    let _connection = zbus::connection::Builder::session()?
        .name(config.bus_name.as_str())?
        .serve_at(wgaf_common::OBJECT_PATH, dbus::Daemon)?
        .serve_at(
            wgaf_common::WINDOWS_OBJECT_PATH,
            dbus::windows_api::WindowsApi::new(window_manager),
        )?
        .build()
        .await?;

    tracing::info!(
        object_path = wgaf_common::OBJECT_PATH,
        windows_object_path = wgaf_common::WINDOWS_OBJECT_PATH,
        "registered on session bus, waiting for requests"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
