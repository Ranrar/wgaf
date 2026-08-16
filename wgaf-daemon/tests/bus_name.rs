//! The daemon's well-known bus name is its identity, and these tests pin the
//! two properties that make it one.
//!
//! `org.wgaf.Daemon` is what the CLI addresses and therefore what it trusts.
//! Until 2026-08-12 the daemon requested it with zbus's default flags, which
//! include `AllowReplacement` and `ReplaceExisting` — so any process on the
//! session bus could take the name, and starting a second daemon silently did
//! exactly that, leaving the first running, unreachable and unaware.
//!
//! Both tests need only a session bus: no Wayland, no `/dev/uinput`, no window.
//! They are deliberately **not** `#[ignore]`d, because the behaviour they check
//! is the one a contributor is most likely to break by touching the connection
//! builder, and it costs nothing to run.

use std::process::Command;
use std::time::Duration;

mod harness;

/// Writes the two files the daemon requires at mode 600, returning both paths,
/// so a test can spawn a daemon the harness's `DaemonGuard` will not own.
///
/// The harness spawns daemons that are expected to stay up; here the point is a
/// daemon that exits, so its exit status and stderr are the assertion.
fn config_for(bus_name: &str, label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let nonce = format!("{}-{label}", std::process::id());
    let dir = std::env::temp_dir();
    let config_path = dir.join(format!("wgaf-bus-name-config-{nonce}.toml"));
    let permissions_path = dir.join(format!("wgaf-bus-name-permissions-{nonce}.toml"));

    std::fs::write(
        &config_path,
        format!("bus_name = \"{bus_name}\"\nlog_level = \"error\"\n"),
    )
    .expect("failed to write the test config");
    std::fs::write(&permissions_path, "[capabilities]\n").expect("failed to write the test policy");

    // The daemon refuses a group- or world-writable config or policy file, and
    // `fs::write` honours the umask — 002 on many distributions, giving 0664.
    for path in [&config_path, &permissions_path] {
        std::fs::set_permissions(path, PermissionsExt::from_mode(0o600))
            .expect("failed to tighten test file permissions");
    }

    (config_path, permissions_path)
}

/// How long a daemon that must refuse to start is given to do so.
///
/// It refuses before serving anything, so this bounds a *failure*: a passing
/// run returns as soon as the process exits, in single-digit milliseconds.
const REFUSAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Waits up to [`REFUSAL_TIMEOUT`] for a child to exit, returning `None` if it
/// is still running.
///
/// `Child::wait` has no timeout and `wait_timeout` is a dependency this suite
/// does not need for one poll loop.
fn wait_briefly(child: &mut std::process::Child) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + REFUSAL_TIMEOUT;
    while std::time::Instant::now() < deadline {
        match child.try_wait().expect("failed to poll the daemon process") {
            Some(status) => return Some(status),
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    None
}

/// Starting a second daemon on a name that is already owned must fail that
/// second daemon, not the first.
///
/// This is the regression test for the measured behaviour: before the fix, the
/// second process logged `registered on session bus, waiting for requests` and
/// took the name, while the first kept running and owned nothing.
#[tokio::test]
async fn a_second_daemon_refuses_to_start_and_leaves_the_first_serving() {
    let bus_name = format!("org.wgaf.Daemon.BusNameTest{}", std::process::id());

    let _first = harness::spawn_daemon("bus-name", &bus_name, "");
    let connection = harness::wait_for_daemon(&bus_name).await;

    let (config_path, permissions_path) = config_for(&bus_name, "second");

    // Deliberately **not** `Command::output()`, which waits for the child's
    // stderr to close and therefore never returns while it is still running.
    // The whole failure being tested for is a second daemon that starts
    // successfully and stays up, so `output()` turns this test's own failure
    // into an indefinite hang — verified by reverting the two request flags,
    // where it hung rather than failing. A test that cannot fail legibly is
    // not much of a test.
    let mut second = Command::new(env!("CARGO_BIN_EXE_wgaf-daemon"))
        .arg("--config")
        .arg(&config_path)
        .arg("--permissions")
        .arg(&permissions_path)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to start the second wgaf-daemon");

    let status = wait_briefly(&mut second);

    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_file(&permissions_path);

    // Kill first, read second. `read_to_string` on the child's stderr blocks
    // until the pipe closes, which needs the child to be gone — so reading it
    // before this check reintroduces exactly the hang `output()` caused.
    let Some(status) = status else {
        let _ = second.kill();
        let _ = second.wait();
        panic!(
            "the second daemon was still running after {REFUSAL_TIMEOUT:?} on \
             an already-owned name. It has taken `{bus_name}` from the first \
             daemon, which is now unreachable — check \
             `allow_name_replacements` / `replace_existing_names` on the \
             connection builder in `main.rs`"
        );
    };

    let mut stderr = String::new();
    if let Some(mut pipe) = second.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }

    assert!(
        !status.success(),
        "the second daemon exited successfully; it should have refused the \
         already-owned name.\nstderr: {stderr}"
    );

    // Assert on the *message*, not merely on the failure. A bare
    // `zbus::Error::NameTaken` reaching the user says "name already taken on
    // the bus" and names neither the name nor what to do, which is the half of
    // this fix a test checking only the exit status would let rot.
    assert!(
        stderr.contains("another wgaf-daemon already owns the bus name"),
        "the second daemon's error does not explain itself.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&bus_name),
        "the second daemon's error does not name the bus name it wanted.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("busctl --user list"),
        "the second daemon's error does not say how to find the other one.\nstderr: {stderr}"
    );

    // The property that was actually broken: the first daemon is still the one
    // answering. A `NameTaken` that still displaced the owner would pass every
    // assertion above.
    let reply: String = harness::call(
        &connection,
        &bus_name,
        wgaf_common::OBJECT_PATH,
        wgaf_common::INTERFACE_NAME,
        "Ping",
        &(),
    )
    .await
    .expect("the first daemon stopped answering Ping after the second tried to start");
    assert_eq!(reply, "pong");
}

/// A plain bus client must not be able to take the name away from a running
/// daemon.
///
/// This is the security half, and it is the one that cannot be checked by
/// reading the daemon's own source: `ReplaceExisting` from a requester only
/// succeeds if the current owner set `AllowReplacement`, so a request that
/// comes back `InQueue` rather than `PrimaryOwner` is external confirmation
/// that the owner did not.
#[tokio::test]
async fn an_outside_process_cannot_take_the_name_from_a_running_daemon() {
    use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

    let bus_name = format!("org.wgaf.Daemon.BusNameSteal{}", std::process::id());

    let _daemon = harness::spawn_daemon("bus-name-steal", &bus_name, "");
    let connection = harness::wait_for_daemon(&bus_name).await;

    // A separate connection, standing in for any other process on the session
    // bus — which is exactly the adversary this flag admits.
    let outsider = zbus::Connection::session()
        .await
        .expect("failed to connect to the session bus");
    let proxy = DBusProxy::new(&outsider)
        .await
        .expect("failed to reach org.freedesktop.DBus");

    let reply = proxy
        .request_name(
            bus_name.as_str().try_into().expect("invalid bus name"),
            RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
        )
        .await
        .expect("RequestName failed outright");

    assert_ne!(
        reply,
        RequestNameReply::PrimaryOwner,
        "an outside connection took `{bus_name}` from the running daemon, so \
         AllowReplacement is set again. Anything on the session bus can now \
         impersonate the daemon to the CLI"
    );
    assert_eq!(reply, RequestNameReply::Exists);

    // And the daemon is still there, which is the observable consequence.
    let reply: String = tokio::time::timeout(
        Duration::from_secs(5),
        harness::call(
            &connection,
            &bus_name,
            wgaf_common::OBJECT_PATH,
            wgaf_common::INTERFACE_NAME,
            "Ping",
            &(),
        ),
    )
    .await
    .expect("Ping did not return within 5s")
    .expect("the daemon stopped answering Ping after the name was requested");
    assert_eq!(reply, "pong");
}
