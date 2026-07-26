mod config;
mod dbus;

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

    let _connection = zbus::connection::Builder::session()?
        .name(config.bus_name.as_str())?
        .serve_at(wgaf_common::OBJECT_PATH, dbus::Daemon)?
        .build()
        .await?;

    tracing::info!(
        object_path = wgaf_common::OBJECT_PATH,
        "registered on session bus, waiting for requests"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
