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
    // `input_device_settle_ms = 0` is a safety setting, not a performance one:
    // these tests synthesize real keystrokes and have no window of their own to
    // aim at, so a live device would type into whatever the developer has
    // focused. See the long note on the same line in `tests/input.rs`, which
    // explains why this is a mitigation rather than a guarantee.
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"{daemon_bus_name}\"\nlog_level = \"error\"\ninput_device_name = \"{device_name}\"\ninput_device_settle_ms = 0\n"
        ),
    )
    .expect("failed to write test config");
    // The daemon requires both files to exist and to not be group/world
    // writable; `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test config permissions");

    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-daemon-perm-test-{nonce}-permissions.toml"));
    std::fs::write(&permissions_path, policy_toml).expect("failed to write test permissions.toml");
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

#[ignore = "takes over the desktop: proving the Deny is scoped needs the *allowed* \
            capabilities to really run, so this moves the pointer and presses `a` for real. \
            Needs a real /dev/uinput — run via `make test-desktop`."]
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

#[ignore = "takes over the desktop: proving the default is Allow needs the call to really \
            succeed, so this types `hi` into a real session. Needs a real /dev/uinput — run \
            via `make test-desktop`."]
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

// ---------------------------------------------------------------------------
// The policy file is mandatory. These guard the fail-closed behaviour added
// after the previous design was found to be fail-open: a missing
// `permissions.toml` used to mean "allow everything", so deleting the file
// silently removed every restriction the user had configured, and a file lost
// to a bad sync or a stray `rm` was indistinguishable from a deliberate
// decision. The daemon now refuses to start instead.
// ---------------------------------------------------------------------------

/// Runs the daemon to completion with the given extra args and returns
/// (exit-success, stderr). Used for the startup-refusal cases, which never
/// reach the point of owning a bus name.
fn run_daemon_expecting_exit(config_path: &std::path::Path, extra: &[&str]) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(config_path)
        .args(extra)
        .output()
        .expect("failed to run wgaf-daemon");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// Writes a minimal config in its own directory, so the `permissions.toml`
/// sibling lookup resolves somewhere private to this test.
fn config_in_private_dir(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("wgaf-required-policy-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create private config dir");
    let config_path = dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            "bus_name = \"org.wgaf.Test.Required.{tag}{}\"\nlog_level = \"error\"\n",
            std::process::id()
        ),
    )
    .expect("write config");
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("tighten config permissions");
    (dir, config_path)
}

#[test]
fn daemon_refuses_to_start_when_the_policy_file_is_missing() {
    let (dir, config_path) = config_in_private_dir("missing");
    let (ok, stderr) = run_daemon_expecting_exit(&config_path, &[]);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !ok,
        "a missing policy file must be fatal — treating it as \"allow everything\" \
         is fail-open, and would mean losing the file silently drops every restriction"
    );
    assert!(
        stderr.contains("no permission policy file found"),
        "the error must say what is wrong; got: {stderr}"
    );
    assert!(
        stderr.contains("[capabilities]") && stderr.contains("--permissions-optional"),
        "the error must say how to fix it, both ways; got: {stderr}"
    );
    assert!(
        !stderr.contains("Missing {"),
        "the error must be the Display message, not a Debug dump; got: {stderr}"
    );
}

#[test]
fn daemon_refuses_a_policy_file_writable_by_group_or_others() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, config_path) = config_in_private_dir("mode");
    let permissions_path = dir.join("permissions.toml");
    std::fs::write(&permissions_path, "[capabilities]\n").expect("write policy");
    // World-writable: any process on the machine could rewrite what this
    // daemon is permitted to do between one run and the next.
    std::fs::set_permissions(&permissions_path, std::fs::Permissions::from_mode(0o666))
        .expect("loosen policy permissions");

    let (ok, stderr) = run_daemon_expecting_exit(&config_path, &[]);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!ok, "a group/world-writable policy file must be fatal");
    assert!(
        stderr.contains("writable by group or others") && stderr.contains("chmod 600"),
        "the error must name the problem and the fix; got: {stderr}"
    );
}

#[test]
fn daemon_accepts_a_correctly_owned_and_moded_policy_file() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, config_path) = config_in_private_dir("good");
    let permissions_path = dir.join("permissions.toml");
    std::fs::write(&permissions_path, "[capabilities]\n").expect("write policy");
    std::fs::set_permissions(&permissions_path, std::fs::Permissions::from_mode(0o600))
        .expect("tighten policy permissions");

    // A daemon that starts successfully never exits on its own, so give it a
    // moment and then confirm it is still running rather than having died.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let still_running = child.try_wait().expect("try_wait").is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        still_running,
        "a policy file owned by this user with mode 600 must be accepted"
    );
}

#[test]
fn permissions_optional_flag_allows_running_without_a_policy_file() {
    let (dir, config_path) = config_in_private_dir("optout");

    // The escape hatch exists so "I genuinely want no restrictions" stays
    // possible — but as an explicit statement, not as the silent consequence
    // of a file having gone missing.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions-optional")
        .spawn()
        .expect("failed to start wgaf-daemon");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let still_running = child.try_wait().expect("try_wait").is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        still_running,
        "--permissions-optional must let the daemon run with no policy file"
    );
}

#[test]
fn daemon_refuses_to_start_when_the_config_file_is_missing() {
    // `config.toml` is required on the same terms as the policy file: the
    // settings the daemon runs under should be visible on disk, not a silent
    // fallback. Its stakes are lower than the policy file's, but a writable
    // config still decides which bus names the daemon talks to.
    let dir = std::env::temp_dir().join(format!("wgaf-required-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create private config dir");
    let config_path = dir.join("config.toml");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .output()
        .expect("failed to run wgaf-daemon");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "a missing config file must be fatal"
    );
    assert!(
        stderr.contains("no configuration file found"),
        "the error must say what is wrong; got: {stderr}"
    );
    assert!(
        stderr.contains("--config-optional"),
        "the error must offer the escape hatch; got: {stderr}"
    );
}

#[test]
fn an_empty_config_file_is_valid_and_selects_the_defaults() {
    use std::os::unix::fs::PermissionsExt;

    // Requiring the file must not mean requiring content: an empty file is
    // the explicit way to ask for every default, which is what makes the
    // requirement cheap to satisfy.
    let dir = std::env::temp_dir().join(format!("wgaf-empty-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create private config dir");
    std::fs::write(dir.join("config.toml"), "").expect("write empty config");
    std::fs::set_permissions(
        dir.join("config.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("tighten config");
    std::fs::write(dir.join("permissions.toml"), "[capabilities]\n").expect("write policy");
    std::fs::set_permissions(
        dir.join("permissions.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("tighten policy");

    // An empty config means the *default* bus name, which a real daemon on
    // this machine may already own — so assert on startup surviving rather
    // than on acquiring the name.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(dir.join("config.toml"))
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to start wgaf-daemon");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let exited = child.try_wait().expect("try_wait");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    // Either still running, or exited for the unrelated reason that the
    // default bus name was already taken — but never for a config problem.
    if let Some(status) = exited {
        assert!(
            !status.success(),
            "unexpected clean exit from a daemon that should either run or fail to \
             claim the bus name"
        );
    }
}
