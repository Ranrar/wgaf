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

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
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
