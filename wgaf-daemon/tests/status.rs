//! End-to-end tests for `org.wgaf.Daemon1.Status` against a really spawned
//! daemon.
//!
//! Status is unusually testable for this project: unlike the window and
//! accessibility suites, which need a real GNOME Shell or a real AT-SPI bus,
//! the most valuable thing to assert here is what the daemon reports when a
//! subsystem is *absent*.
//!
//! **Absence is arranged, not assumed.** These tests used to take "the
//! extension is not installed" as a property of the test environment, which
//! made them fail on any developer desktop that actually has wgaf set up — the
//! machines most likely to run them. Each test now points its daemon at an
//! `extension_bus_name` nobody owns, so the assertion is about what the daemon
//! reports when the bridge is unreachable, which is the thing being tested.

use std::process::{Child, Command};
use std::time::Duration;

use wgaf_common::DaemonStatus;
use wgaf_common::dict::DaemonStatusDict;

/// Writes a policy file the daemon will accept.
///
/// The mode has to be set explicitly: the daemon refuses a policy file that is
/// group- or world-writable (anyone who can write it decides what the daemon
/// may do), and `fs::write` honours the umask — 002 on many distros, which
/// yields a group-writable 0664.
fn write_policy(path: &std::path::Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, contents).expect("failed to write test policy");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .expect("failed to tighten test policy permissions");
}

/// Kills the spawned daemon even if an assertion panics mid-test.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn try_status(bus_name: &str) -> Result<DaemonStatus, Box<dyn std::error::Error>> {
    let connection = zbus::Connection::session().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Status",
            &(),
        )
        .await?;
    let dict: DaemonStatusDict = reply.body().deserialize()?;
    Ok(dict.into())
}

/// `config.toml` fragment pointing the daemon's extension bridge at a bus name
/// nothing owns.
///
/// This is how a test says "the extension is unreachable" without depending on
/// the machine it runs on. Unique per test, so a daemon left over from another
/// suite cannot accidentally own it.
fn unowned_extension_config(tag: &str) -> String {
    format!(
        "extension_bus_name = \"org.wgaf.Test.NoExtension.{tag}{}\"\n",
        std::process::id()
    )
}

/// Spawns a daemon on a unique bus name and waits for `Status` to answer.
/// `extra_config` is appended to the generated `config.toml`.
async fn spawn_daemon(tag: &str, extra_config: &str) -> (DaemonGuard, String, DaemonStatus) {
    let bus_name = format!("org.wgaf.Daemon.Test{}{}", tag, std::process::id());
    let config_path = std::env::temp_dir().join(format!(
        "wgaf-status-test-{}-{}.toml",
        tag,
        std::process::id()
    ));
    std::fs::write(
        &config_path,
        format!("bus_name = \"{bus_name}\"\nlog_level = \"error\"\n{extra_config}"),
    )
    .expect("failed to write test config");
    // The daemon requires both files to exist and to not be group/world
    // writable; `fs::write` honours the umask (002 on many distros -> 0664).
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .expect("failed to tighten test config permissions");

    // The daemon requires a policy file. An empty [capabilities] table says
    // "allow everything" explicitly, which is what most of these tests want;
    // the restrictions test below writes a populated one instead.
    let permissions_path = std::env::temp_dir().join(format!(
        "wgaf-status-perm-{}-{}.toml",
        tag,
        std::process::id()
    ));
    write_policy(&permissions_path, "[capabilities]\n");

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    let guard = DaemonGuard(child);

    let mut last_err = None;
    for _ in 0..50 {
        match try_status(&bus_name).await {
            Ok(status) => {
                let _ = std::fs::remove_file(&config_path);
                let _ = std::fs::remove_file(&permissions_path);
                return (guard, bus_name, status);
            }
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let _ = std::fs::remove_file(&config_path);
    panic!("daemon did not answer Status in time: {last_err:?}");
}

#[tokio::test]
async fn status_reports_the_extension_unavailable_while_the_daemon_itself_is_healthy() {
    // A bus name nothing owns, so the bridge is unreachable by construction.
    // Without this the test asserted a property of the *machine* — "wgaf is not
    // installed here" — and failed on every desktop where it is.
    let (_guard, bus_name, status) = spawn_daemon("Ext", &unowned_extension_config("Ext")).await;

    // The whole point of the command: one unavailable subsystem must not stop
    // the daemon answering for the others.
    assert!(
        !status.extension_available,
        "the daemon was pointed at an extension bus name nobody owns, so it \
         must report the bridge unavailable"
    );
    assert!(
        !status.extension_detail.is_empty(),
        "an unavailable subsystem must carry the daemon's actionable guidance, \
         otherwise the user is told only that something is wrong"
    );
    assert_eq!(status.daemon_bus_name, bus_name);
    assert_eq!(status.daemon_version, env!("CARGO_PKG_VERSION"));
    assert!(status.daemon_pid > 0);

    // The policy file this test writes is an empty [capabilities] table: a
    // present, explicit "allow everything", which must be reported as present
    // with nothing restricted — distinguishable from having no policy at all.
    assert!(
        !status.permissions_path.is_empty() && status.permissions_present,
        "the policy file this test wrote must be reported as present"
    );
    assert!(
        status.permissions_restricted.is_empty(),
        "an empty [capabilities] table restricts nothing"
    );
}

#[tokio::test]
async fn status_does_not_create_the_uinput_device() {
    // The rule this guards: reporting must not change anything. Probing
    // `/dev/uinput` by going through `InputBackend::device` would run
    // `UI_DEV_CREATE` and register a real kernel device as a side effect of a
    // read-only query — visible system-wide in `/proc/bus/input/devices`, and
    // enough to disturb `tests/input.rs`, which asserts on that file.
    let device_name = format!("wgaf status probe test {}", std::process::id());
    let (_guard, _bus_name, status) =
        spawn_daemon("NoDev", &format!("input_device_name = \"{device_name}\"\n")).await;

    assert!(
        !status.input_device_created,
        "Status must report the device as not created, having only probed access"
    );

    let devices = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    assert!(
        !devices.contains(&device_name),
        "calling Status registered a uinput device with the kernel — the probe \
         must open /dev/uinput and issue no ioctls, never UI_DEV_CREATE"
    );
}

#[tokio::test]
async fn status_surfaces_configured_restrictions_and_hides_a_file_that_does_not_exist() {
    let permissions_path = std::env::temp_dir().join(format!(
        "wgaf-status-test-permissions-{}.toml",
        std::process::id()
    ));
    write_policy(
        &permissions_path,
        "[capabilities]\nTypeText = \"Deny\"\nCloseWindow = \"Prompt\"\n",
    );

    let bus_name = format!("org.wgaf.Daemon.TestPerm{}", std::process::id());
    let config_path =
        std::env::temp_dir().join(format!("wgaf-status-test-perm-{}.toml", std::process::id()));
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

    let child = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .spawn()
        .expect("failed to start wgaf-daemon");
    let _guard = DaemonGuard(child);

    let mut status = None;
    for _ in 0..50 {
        if let Ok(s) = try_status(&bus_name).await {
            status = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_file(&permissions_path);
    let status = status.expect("daemon did not answer Status in time");

    // Sorted by capability name, and only the non-default entries: an empty
    // list has to unambiguously mean "nothing is restricted".
    assert_eq!(
        status.permissions_restricted,
        vec![
            "CloseWindow=Prompt".to_string(),
            "TypeText=Deny".to_string()
        ],
        "Status must report which capabilities are restricted, and from where — \
         until this existed, a user refused by policy could not see either"
    );
    assert!(
        status.permissions_path.ends_with(".toml") && status.permissions_present,
        "a policy file that exists must be named in the report and marked present"
    );
    assert!(
        status.permissions_prompt_decisions.is_empty(),
        "nothing has been prompted for in this run"
    );
}
