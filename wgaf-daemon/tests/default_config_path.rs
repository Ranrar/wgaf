//! End-to-end coverage of the Phase 7 default `--config` path (see
//! `wgaf-daemon/src/config.rs`'s `default_config_path`): with `--config`
//! omitted entirely, the daemon must still find and load
//! `$XDG_CONFIG_HOME/wgaf/config.toml`. Complements the pure unit tests on
//! `resolve_default_config_path` in `config.rs` itself (which check the
//! path-computation logic in isolation) with a real spawned-binary test,
//! same convention as `wgaf-daemon/tests/ping.rs`.
//!
//! `XDG_CONFIG_HOME` is set only on the *spawned child's* environment via
//! `Command::env`, never on this test process's own environment — so this
//! is safe to run alongside other tests in the same `cargo test` invocation
//! without any risk of racing another test that reads/depends on real
//! process env state.

use std::process::{Child, Command};
use std::time::Duration;

/// Kills the spawned daemon even if an assertion panics mid-test.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn try_ping(bus_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let connection = zbus::Connection::session().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await?;
    Ok(reply.body().deserialize()?)
}

#[tokio::test]
async fn daemon_loads_config_from_xdg_default_path_when_config_flag_omitted() {
    let pid = std::process::id();
    let bus_name = format!("org.wgaf.Daemon.XdgTest{pid}");

    // A private, unique XDG_CONFIG_HOME so this doesn't collide with a real
    // config a developer might have at ~/.config/wgaf/config.toml, and so
    // parallel test runs (different pid) never share a directory.
    let xdg_config_home = std::env::temp_dir().join(format!("wgaf-xdg-config-home-{pid}"));
    let config_dir = xdg_config_home.join("wgaf");
    std::fs::create_dir_all(&config_dir).expect("create fake $XDG_CONFIG_HOME/wgaf");
    let config_path = config_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!("bus_name = \"{bus_name}\"\nlog_level = \"error\"\n"),
    )
    .expect("write config.toml at the default XDG location");
    // The daemon requires both files to exist and to not be group/world
    // writable; `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test config permissions");

    // The daemon requires a policy file as a sibling of the resolved config;
    // an empty [capabilities] table means "allow everything", stated
    // explicitly rather than implied by absence. This also exercises the
    // sibling lookup working off the *default* config path.
    let permissions_path = config_dir.join("permissions.toml");
    std::fs::write(&permissions_path, "[capabilities]\n")
        .expect("write permissions.toml alongside it");
    // Explicit mode: the daemon rejects a group/world-writable policy file,
    // and `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &permissions_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test policy permissions");

    // Deliberately no `--config` flag: the whole point of this test is that
    // the daemon finds the file on its own via $XDG_CONFIG_HOME.
    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .env("XDG_CONFIG_HOME", &xdg_config_home)
        .spawn()
        .expect("failed to start wgaf-daemon");
    let _guard = DaemonGuard(child);

    let mut last_err = None;
    for _ in 0..50 {
        match try_ping(&bus_name).await {
            Ok(reply) => {
                let _ = std::fs::remove_dir_all(&xdg_config_home);
                assert_eq!(reply, "pong");
                return;
            }
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let _ = std::fs::remove_dir_all(&xdg_config_home);
    panic!(
        "daemon did not pick up config from $XDG_CONFIG_HOME default path in time: {last_err:?}"
    );
}
