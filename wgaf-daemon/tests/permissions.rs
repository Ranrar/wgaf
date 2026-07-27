//! Integration tests for the permission gate against the *real*
//! `wgaf-daemon` binary and its *real* `org.wgaf.Input1` interface — same
//! "spawn the real binary, talk to it over D-Bus" approach as
//! `tests/input.rs`/`tests/windows_stub.rs`, not a unit test of
//! `permissions::PermissionGate` in isolation (that's covered by the unit
//! tests inside `src/permissions/mod.rs`/`src/permissions/policy.rs`
//! instead).
//!
//! `org.wgaf.Input1` was chosen as the interface to exercise here (rather
//! than `Windows1`/`Accessibility1`) because it's the one that can be
//! driven against a real backend without a real GNOME Shell session or a real
//! AT-SPI-registered application — `/dev/uinput` is confirmed writable in
//! this sandbox (see `tests/input.rs`'s module docs), so `Deny`-ing one
//! `Input1` capability while leaving another `Allow`'d proves real
//! enforcement: the *same* mutating interface, same backend, same running
//! daemon process, with one specific capability refused and a different one
//! still working.
//!
//! **What is NOT covered here (documented gap, not silently skipped):** the
//! interactive `Prompt` policy value. There is no way to click a desktop
//! notification's "Allow"/"Deny" action button from an automated test in
//! this sandbox (no real notification daemon/user present) — this is the
//! same category of gap as `input.rs`'s "no real GUI verification"/
//! `accessibility.rs`'s "no `FocusElement` success demonstrated" caveats:
//! investigated, not silently assumed to work, and called out honestly
//! here. `Prompt`'s *cache* behavior and its
//! `resolve_prompt`/`PermissionGate::check` control flow
//! are still covered by `src/permissions/mod.rs`'s unit tests using a
//! pre-seeded policy — just not the real notification round-trip.

use std::process::{Child, Command};
use std::time::Duration;

use zbus::Connection;

/// Kills the spawned daemon even if an assertion panics mid-test, and keeps
/// both the config file and the permissions file alive until then (removing
/// either earlier would race the child's own async `Config::load`/
/// `PolicyMap::load`, per the existing convention in
/// `input.rs`/`windows_stub.rs`).
struct DaemonGuard {
    child: Child,
    config_path: std::path::PathBuf,
    permissions_path: std::path::PathBuf,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_file(&self.permissions_path);
    }
}

/// Spawns the real `wgaf-daemon` binary with a test-private bus name, a
/// test-private unique `input_device_name` (see `tests/input.rs`'s module
/// docs for why that has to be unique per test), and an explicit
/// `--permissions` file containing `policy_toml` verbatim.
fn spawn_daemon_with_policy(
    daemon_bus_name: &str,
    device_name: &str,
    nonce: &str,
    policy_toml: &str,
) -> DaemonGuard {
    let config_path = std::env::temp_dir().join(format!("wgaf-daemon-perm-test-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{daemon_bus_name}\"\nlog_level = \"error\"\ninput_device_name = \"{device_name}\"\n"
        ),
    )
    .expect("failed to write test config");

    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-daemon-perm-test-{nonce}-permissions.toml"));
    std::fs::write(&permissions_path, policy_toml).expect("failed to write test permissions.toml");

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    DaemonGuard {
        child,
        config_path,
        permissions_path,
    }
}

async fn wait_for_daemon(bus_name: &str) -> Connection {
    let connection = Connection::session().await.expect("connect to session bus");
    for _ in 0..50 {
        let reply = connection
            .call_method(
                Some(bus_name),
                wgaf_common::OBJECT_PATH,
                Some(wgaf_common::INTERFACE_NAME),
                "Ping",
                &(),
            )
            .await;
        if reply.is_ok() {
            return connection;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("daemon did not respond to Ping in time");
}

async fn call_input<R, A>(
    connection: &Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::INPUT_OBJECT_PATH,
            Some(wgaf_common::INPUT_INTERFACE_NAME),
            method,
            args,
        )
        .await?;
    reply.body().deserialize()
}

#[tokio::test]
async fn denied_capability_is_refused_while_other_capabilities_still_succeed() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Permissions.Deny{pid}");
    let device_name = format!("wgaf test device permdeny {pid}");

    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &device_name,
        &format!("deny{pid}"),
        "[capabilities]\nTypeText = \"Deny\"\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    // The gated, denied capability: TypeText must fail with the new
    // PermissionDenied error, not succeed and not fail with some other
    // (e.g. device-related) error.
    let err = call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &("hello",))
        .await
        .expect_err("TypeText should be refused by the Deny policy");
    match err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(
                name.as_str(),
                wgaf_common::INPUT_ERROR_PERMISSION_DENIED,
                "TypeText should fail specifically with PermissionDenied"
            );
        }
        other => panic!("expected a MethodError, got {other:?}"),
    }

    // A different, non-denied capability on the *same* Input1 interface
    // (and the same running daemon/policy file) must still succeed —
    // proving the Deny is scoped to the one configured capability, not a
    // blanket refusal.
    call_input::<(), _>(&connection, &daemon_bus_name, "MouseMove", &(3i32, 4i32))
        .await
        .expect("MouseMove should still succeed — only TypeText is denied");
    call_input::<(), _>(&connection, &daemon_bus_name, "KeyPress", &("a",))
        .await
        .expect("KeyPress should still succeed — only TypeText is denied");
    call_input::<(), _>(&connection, &daemon_bus_name, "KeyRelease", &("a",))
        .await
        .expect("KeyRelease should still succeed — only TypeText is denied");
}

#[tokio::test]
async fn absent_permissions_file_defaults_every_capability_to_allow() {
    let pid = std::process::id();
    let daemon_bus_name = format!("org.wgaf.Test.Permissions.Absent{pid}");
    let device_name = format!("wgaf test device permabsent {pid}");

    // An empty (no `[capabilities]` table) TOML file is the equivalent of
    // "no file at all" (see `permissions::policy`'s module docs and unit
    // tests) — used here rather than an actually-missing `--permissions`
    // flag only so this test can still clean up a real file via
    // `DaemonGuard`; the "genuinely no --permissions flag given" path is
    // covered directly by `wgaf-daemon/src/permissions/policy.rs`'s
    // `missing_file_path_defaults_every_capability_to_allow` unit test.
    let _daemon = spawn_daemon_with_policy(
        &daemon_bus_name,
        &device_name,
        &format!("absent{pid}"),
        "# no [capabilities] table at all\n",
    );
    let connection = wait_for_daemon(&daemon_bus_name).await;

    call_input::<(), _>(&connection, &daemon_bus_name, "TypeText", &("hi",))
        .await
        .expect("TypeText should succeed under an empty (all-default-Allow) policy");
}
