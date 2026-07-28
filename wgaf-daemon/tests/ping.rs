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
async fn ping_returns_pong() {
    // Unique bus name so this test doesn't collide with a daemon already
    // running under the default name during local development.
    let bus_name = format!("org.wgaf.Daemon.Test{}", std::process::id());
    let config_path =
        std::env::temp_dir().join(format!("wgaf-daemon-test-{}.toml", std::process::id()));
    std::fs::write(
        &config_path,
        format!("bus_name = \"{bus_name}\"\nlog_level = \"error\"\n"),
    )
    .expect("failed to write test config");
    // The daemon requires both files to exist and to not be group/world
    // writable; `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test config permissions");

    // The daemon requires a policy file; an empty [capabilities] table is the
    // explicit way to say "allow everything". Given its own unique path
    // rather than relying on the config sibling, because every test config
    // lives directly in the temp dir and would otherwise share one file.
    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-ping-permissions-{}.toml", std::process::id()));
    std::fs::write(&permissions_path, "[capabilities]\n").expect("failed to write test policy");
    // Explicit mode: the daemon rejects a group/world-writable policy file,
    // and `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &permissions_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test policy permissions");

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    let _guard = DaemonGuard(child);

    let mut last_err = None;
    for _ in 0..50 {
        match try_ping(&bus_name).await {
            Ok(reply) => {
                let _ = std::fs::remove_file(&config_path);
                assert_eq!(reply, "pong");
                return;
            }
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let _ = std::fs::remove_file(&config_path);
    panic!("daemon did not respond to Ping in time: {last_err:?}");
}
