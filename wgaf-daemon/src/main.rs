mod accessibility;
mod config;
mod dbus;
mod input;
mod permissions;
mod secure_file;
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
    /// Path to a TOML config file. Defaults to
    /// `$XDG_CONFIG_HOME/wgaf/config.toml` (or `$HOME/.config/wgaf/config.toml`
    /// if `$XDG_CONFIG_HOME` is unset) if not given — see
    /// `config::default_config_path`.
    ///
    /// The file is expected to be present, owned by this user, and mode 600.
    /// An empty file is valid and selects every built-in default; to run with
    /// no file at all, pass `--config-optional`.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to a TOML permission-policy file. Same file format as `--config`
    /// (plain TOML). If omitted, the daemon looks for `permissions.toml` next
    /// to the resolved `--config` path (explicit or default).
    ///
    /// The file is expected to be present, owned by this user, and mode 600.
    /// To allow every capability, write one containing an empty
    /// `[capabilities]` table, or pass `--permissions-optional`.
    #[arg(long)]
    permissions: Option<PathBuf>,

    /// Run without a `config.toml`, using the built-in defaults.
    ///
    /// Prefer creating an empty file, which selects the same defaults but
    /// stays visible in your config.
    #[arg(long)]
    config_optional: bool,

    /// Run without a permission policy file, allowing every capability.
    ///
    /// Prefer writing a file with an empty `[capabilities]` table, which
    /// allows everything just the same but stays visible in your config.
    #[arg(long)]
    permissions_optional: bool,

    /// Override the configured log level (trace, debug, info, warn, error).
    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() {
    // Errors are printed here rather than returned from `main`, because
    // Rust's `Termination` impl for `Result` formats them with `Debug`, not
    // `Display` — which would print `Missing { path: "..." }` instead of the
    // multi-line, actionable guidance `secure_file::SecureFileError` carries.
    // The CLI had the same defect; see `wgaf-cli/src/main.rs`.
    if let Err(err) = run().await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // ADDED: resolve a real default `--config` location
    // (`$XDG_CONFIG_HOME/wgaf/config.toml`, falling back to
    // `$HOME/.config/wgaf/config.toml`) when `--config` wasn't given
    // explicitly, closing the gap flagged during Phase 6 where the daemon
    // had no on-disk config location at all short of an explicit flag. An
    // explicit `--config` always wins; `config::default_config_path`
    // returns `None` (no default at all) only if neither
    // `$XDG_CONFIG_HOME` nor `$HOME` is usable.
    let config_path = args.config.clone().or_else(config::default_config_path);

    // Both configuration files are required (see `secure_file`'s module docs
    // for why absence is fatal rather than defaulting silently). Note this
    // runs before tracing is initialized, so a failure here surfaces through
    // `main`'s `eprintln!` rather than the log — which is what we want for a
    // startup refusal the user must read.
    let mut config = match (&config_path, args.config_optional) {
        (Some(path), false) => Config::load_required(path)?,
        // `--config-optional`, or no resolvable location at all (neither
        // $XDG_CONFIG_HOME nor $HOME usable, so there is nowhere a file could
        // live in the first place).
        _ => Config::load(config_path.as_deref())?,
    };
    if let Some(log_level) = args.log_level {
        config.log_level = log_level;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        bus_name = %config.bus_name,
        config_path = ?config_path,
        "starting wgaf-daemon"
    );

    // Resolve the permission-policy file path: an explicit `--permissions`
    // wins; otherwise default to a `permissions.toml` sibling of the
    // resolved `--config` path (same directory) — `config_path`, not
    // `args.config`, so this now finds `permissions.toml` next to the new
    // XDG default `config.toml` location automatically, without needing an
    // explicit `--config` to anchor the sibling lookup.
    let permissions_path = args.permissions.or_else(|| {
        config_path
            .as_ref()
            .and_then(|c| c.parent())
            .map(|dir| dir.join("permissions.toml"))
    });
    // The policy file is expected unless explicitly waived — see
    // `secure_file`'s module docs.
    let policy = match (&permissions_path, args.permissions_optional) {
        (Some(path), false) => permissions::PolicyMap::load_required(path)?,
        // `--permissions-optional`, or no resolvable location at all (neither
        // $XDG_CONFIG_HOME nor $HOME usable, so there is nowhere a file could
        // even live).
        _ => permissions::PolicyMap::load(permissions_path.as_deref())?,
    };
    let permission_gate = Arc::new(permissions::PermissionGate::new(policy));
    if args.permissions_optional {
        tracing::warn!(
            "running with --permissions-optional: every capability is ALLOWED and \
             no policy file is being enforced"
        );
    } else {
        tracing::info!(
            permissions_path = ?permissions_path,
            "loaded permission policy (unmentioned capabilities default to Allow)"
        );
    }

    // A separate session-bus connection used only as a client of the GNOME
    // Shell Extension bridge — independent from the connection the daemon
    // serves its own D-Bus API on below. Building the proxy here does not
    // require the extension to already be running (see
    // `windows::WindowManager`'s doc comments): if it's not installed or
    // not enabled yet, window-management calls fail with a clear error at
    // call time rather than blocking daemon startup, so input automation
    // and future AT-SPI features can still come up independently.
    let extension_connection = zbus::Connection::session().await?;
    let window_manager = Arc::new(
        windows::WindowManager::connect_to(
            extension_connection,
            &config.extension_bus_name,
            wgaf_common::EXTENSION_OBJECT_PATH,
            wgaf_common::EXTENSION_INTERFACE_NAME,
            &config.display_config_bus_name,
        )
        .await?,
    );

    // `InputBackend::new` does not touch `/dev/uinput` — like
    // `WindowManager::connect_to` above, the actual resource (the virtual
    // uinput device) is only created lazily on first real use, so a
    // permissions problem here never prevents the daemon from starting or
    // from serving `org.wgaf.Windows1`. See `input::InputBackend`'s doc
    // comments. The same is true of the keyboard layout, which needs a live
    // Wayland session to read.
    let input_backend = Arc::new(input::InputBackend::new(
        config.input_device_name.clone(),
        input::InputLimits {
            max_events_per_second: config.input_max_events_per_second,
            max_type_text_chars: config.input_max_type_text_chars,
            device_settle: std::time::Duration::from_millis(config.input_device_settle_ms),
        },
        config.input_keyboard_layout.clone(),
    ));

    // Resolve the keyboard layout now rather than at the first `wgaf type`, so
    // a misconfigured `input_keyboard_layout` is reported at startup instead of
    // surfacing later as a failed command.
    //
    // The two failures are treated differently on purpose. A layout that does
    // not resolve is the *configuration* being wrong and stops the daemon,
    // exactly as a malformed `config.toml` does — falling back to some other
    // layout would silently type the wrong characters, which is the failure
    // this whole feature exists to end. A keymap that cannot be read is the
    // *environment* being absent, and must not stop anything: window and
    // accessibility commands need no keymap, and a daemon started before its
    // session is ready recovers on the next call without a restart.
    match input_backend.typing().await {
        Ok(_) => {}
        Err(e @ input::InputError::KeyboardLayoutInvalid(_)) => {
            eprintln!("wgaf-daemon: {e}");
            std::process::exit(1);
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "keyboard layout not resolved at startup; `wgaf type` will retry on first use"
            );
        }
    }

    // `AccessibilityBackend::new` does not touch the AT-SPI bus — like
    // `WindowManager::connect_to`/`InputBackend::new` above, the actual
    // resource (a connection to `org.a11y.Bus`) is only established lazily
    // on first real use, so accessibility being unavailable/not-yet-enabled
    // for this session never prevents the daemon from starting or from
    // serving the other interfaces. See `accessibility::AccessibilityBackend`'s
    // doc comments.
    let accessibility_backend = Arc::new(accessibility::AccessibilityBackend::new());

    // Taken before `input_backend` is moved into `Input1` below. Drives the
    // `InputDeviceActive` property's change notifications, which is how the
    // Shell extension knows when to hold the emergency-key grab and when to
    // give the key back to the desktop.
    let device_presence = input_backend.device_present_watch();

    let connection = zbus::connection::Builder::session()?
        .name(config.bus_name.as_str())?
        .serve_at(
            wgaf_common::OBJECT_PATH,
            dbus::Daemon::new(
                config.bus_name.clone(),
                // Both of these are *resolved* locations that need not exist
                // (a missing file means "use defaults", not an error — see
                // `Config::load`/`PolicyMap::load`). `Status` reports the
                // path either way, with a separate presence flag: the path of
                // a file nobody wrote is exactly what someone asking "where
                // does config go?" needs, while reporting it *without* the
                // flag would imply a config is in effect when none is.
                config_path.clone(),
                permissions_path.clone(),
                Arc::clone(&window_manager),
                Arc::clone(&input_backend),
                Arc::clone(&accessibility_backend),
                Arc::clone(&permission_gate),
            ),
        )?
        .serve_at(
            wgaf_common::WINDOWS_OBJECT_PATH,
            dbus::windows_api::WindowsApi::new(
                Arc::clone(&window_manager),
                Arc::clone(&permission_gate),
            ),
        )?
        .serve_at(
            wgaf_common::INPUT_OBJECT_PATH,
            // `Input1` gets the window manager as well as the input backend:
            // absolute pointer positioning is served through the Shell
            // extension, not through `uinput`. See `InputApi`'s documentation
            // for why the two live on one interface despite that.
            dbus::input_api::InputApi::new(
                input_backend,
                Arc::clone(&window_manager),
                Arc::clone(&permission_gate),
            ),
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

    tokio::spawn(announce_device_presence(
        connection.clone(),
        device_presence,
    ));

    // Re-emits the extension's window signals on `org.wgaf.Windows1`. Spawned
    // unconditionally and never awaited: it retries forever, so a session with
    // no extension installed — or one installed after the daemon started —
    // costs a wakeup every few seconds and nothing else. That mirrors how every
    // other window call recovers without a daemon restart.
    tokio::spawn(dbus::windows_api::forward_window_events(
        window_manager,
        zbus::object_server::SignalEmitter::new(&connection, wgaf_common::WINDOWS_OBJECT_PATH)?,
    ));

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

/// Emits `PropertiesChanged` for `org.wgaf.Daemon1.InputDeviceActive` whenever
/// the virtual input device appears or goes away.
///
/// The GNOME Shell Extension holds its emergency-key grab only while that
/// property is `true`, so this task is what hands `Escape` back to the
/// desktop between runs. A missed notification is a real fault and is logged
/// as one — too few and the key stays captured after a run; too few in the
/// other direction and the panic key is absent during one.
async fn announce_device_presence(
    connection: zbus::Connection,
    mut presence: tokio::sync::watch::Receiver<bool>,
) {
    while presence.changed().await.is_ok() {
        let active = *presence.borrow_and_update();

        let interface = match connection
            .object_server()
            .interface::<_, dbus::Daemon>(wgaf_common::OBJECT_PATH)
            .await
        {
            Ok(interface) => interface,
            Err(error) => {
                // The object server outlives this task in every ordinary run,
                // so this means shutdown is under way. Stopping is correct:
                // there is nothing left to notify.
                tracing::debug!(%error, "stopped announcing input-device presence");
                return;
            }
        };

        if let Err(error) = interface
            .get()
            .await
            .input_device_active_changed(interface.signal_emitter())
            .await
        {
            tracing::warn!(
                %error,
                active,
                "failed to announce a change in input-device presence; the \
                 emergency-key shortcut may now be armed when it should not be, \
                 or absent when it should be armed"
            );
        }
    }
}
