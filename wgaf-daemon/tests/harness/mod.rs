//! Shared harness for the integration suites that drive the deterministic GTK4
//! applications in `tests/apps/`.
//!
//! # What this is for
//!
//! The applications draw a fixed UI and write what they observed to a JSON
//! report file. A test drives wgaf, then reads that file. **The assertion never
//! travels back through the code path under test** — `wgaf type "hello"` is
//! verified by reading the application's report, never by asking wgaf what the
//! application contains. This module is the reading half; `tests/apps/report`
//! is the writing half, and its documentation states the contract.
//!
//! # Preconditions, not mysteries
//!
//! Every suite using this harness needs things of the machine running it: a
//! Wayland session, a writable `/dev/uinput`, an application binary that only
//! `make test-apps` builds. Historically those requirements lived in CI
//! configuration as a hand-maintained skip list, and a missing one surfaced as
//! whatever symptom happened first — a `DeviceUnavailable` buried in a D-Bus
//! debug dump, or eight identical "is it installed?" failures.
//!
//! So the `require_*` functions here state the requirement itself, and a suite
//! calls them **before** its test body. A machine that cannot run a test should
//! say so in one line naming what to install or run, not fail partway through
//! doing something else.
//!
//! # Why these suites are `#[ignore]`d
//!
//! They take over the desktop. They synthesize real keystrokes into a real
//! session and raise real windows, so running them as part of an ordinary
//! `cargo test` would type into whatever the developer was doing. Run them
//! deliberately:
//!
//! ```text
//! make test-apps
//! cargo test -p wgaf-daemon --test keyboard_coverage -- --ignored --test-threads=1
//! ```
//!
//! `--test-threads=1` matters: two suites driving input at once would each
//! receive the other's keystrokes, and both would fail confusingly.

// Each suite uses a different part of this module, and Cargo compiles the whole
// of it into every one of them. Without this, the unused half warns in each —
// and CI runs clippy with `-D warnings`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use zbus::Connection;

/// How long to wait for an application to report something before giving up.
///
/// Generous on purpose: this bounds a *failure*, not a success. A passing test
/// returns as soon as the report arrives, so the only thing a longer timeout
/// costs is how long a genuinely broken run takes to say so — and a value tuned
/// tight enough to be fast is a value that flakes on a loaded machine.
pub const REPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the report file is re-read while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Preconditions
// ---------------------------------------------------------------------------

/// Fails unless this is a Wayland session.
///
/// The applications are GTK4 clients that must actually map a window; without a
/// display they exit immediately and every later assertion reports an absent
/// report file, which looks like a wgaf fault and is not one.
pub fn require_wayland_session() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        panic!(
            "this suite needs a Wayland session (WAYLAND_DISPLAY is unset). \
             It drives real GTK4 windows on a real GNOME Shell and cannot run headless."
        );
    }
}

/// Fails unless `/dev/uinput` can be opened for writing.
///
/// This is the same access the daemon needs to synthesize anything at all, so
/// checking it here turns "every input assertion failed" into one line naming
/// the udev rule to install.
pub fn require_uinput() {
    if let Err(err) = std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
        panic!(
            "this suite needs a writable /dev/uinput ({err}). \
             See the first-time setup in docs/installation.md: install the udev rule and join the `input` group."
        );
    }
}

/// Resolves a `tests/apps/` binary, failing with the command that builds it.
///
/// The applications are a separate Cargo workspace, deliberately excluded from
/// the root one so that building wgaf never requires GTK4 development packages.
/// The cost of that decision is exactly this: `cargo test` does not build them,
/// so a suite must look for the binary and say what to run when it is absent.
pub fn require_test_app(name: &str) -> PathBuf {
    let apps_target = repo_root().join("tests/apps/target");
    let candidates = [
        apps_target.join("debug").join(name),
        apps_target.join("release").join(name),
    ];

    for candidate in &candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }

    panic!(
        "this suite needs the `{name}` test application, which was not found at {}. \
         Build it with `make test-apps` (needs GTK4 development packages — see tests/apps/README.md).",
        candidates[0].display()
    );
}

/// The repository root, derived from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("wgaf-daemon has a parent directory")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// The application under test
// ---------------------------------------------------------------------------

/// One report as it was found on disk.
///
/// Deliberately a `serde_json::Value` rather than a typed mirror of each
/// application's state: a second definition of the same shape is a second thing
/// to keep in step, and a test that reads one field has no use for the rest.
/// The accessors below fail loudly with the field name and the whole document,
/// because a missing field means the application's report changed shape and the
/// document is what says how.
#[derive(Debug, Clone)]
pub struct Report(serde_json::Value);

impl Report {
    /// The report's sequence number. See [`TestApp::wait_for`] for what it is for.
    pub fn seq(&self) -> u64 {
        self.u64("seq")
    }

    pub fn u64(&self, field: &str) -> u64 {
        self.0
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| self.missing(field, "an unsigned integer"))
    }

    pub fn i64(&self, field: &str) -> i64 {
        self.0
            .get(field)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_else(|| self.missing(field, "an integer"))
    }

    pub fn bool(&self, field: &str) -> bool {
        self.0
            .get(field)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_else(|| self.missing(field, "a boolean"))
    }

    pub fn str(&self, field: &str) -> &str {
        self.0
            .get(field)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| self.missing(field, "a string"))
    }

    pub fn array(&self, field: &str) -> &[serde_json::Value] {
        self.0
            .get(field)
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_else(|| self.missing(field, "an array"))
    }

    /// The raw document, for a failure message that needs to show everything.
    pub fn json(&self) -> &serde_json::Value {
        &self.0
    }

    fn missing(&self, field: &str, expected: &str) -> ! {
        panic!(
            "report has no `{field}` field holding {expected}; \
             the application's report shape has changed. Full report:\n{:#}",
            self.0
        );
    }
}

/// A spawned test application, killed when the test ends however it ends.
///
/// The report file lives in a directory of this guard's own, removed on drop:
/// two applications sharing one path would overwrite each other's reports, and
/// a leftover file from an earlier run would be read as though the current run
/// had produced it — which is the failure mode that a `seq` baseline exists to
/// make impossible, but only if the file is genuinely this run's.
pub struct TestApp {
    name: &'static str,
    child: Child,
    dir: PathBuf,
    report_path: PathBuf,
}

impl TestApp {
    /// Starts the application and returns once it has written its first report.
    ///
    /// Waiting here rather than leaving it to the caller is deliberate: a test
    /// that drives wgaf before the window exists is testing nothing, and its
    /// failure would name whichever assertion came first rather than the fact
    /// that the application never started.
    pub async fn spawn(name: &'static str) -> Self {
        let binary = require_test_app(name);

        let dir = std::env::temp_dir().join(format!(
            "wgaf-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the report directory");
        let report_path = dir.join("report.json");

        let child = Command::new(&binary)
            .arg("--report")
            .arg(&report_path)
            // Take the session's input method out of the path — measured, not
            // precautionary.
            //
            // On Wayland a GTK4 client talks to the input method over
            // `text-input-v3`, and a key the method consumes is **never
            // delivered to the client at all**. The compositor is doing exactly
            // what the protocol says; the application simply cannot see it, and
            // no controller in any propagation phase can either.
            //
            // Which keys those are depends on the layout, which made this look
            // like a wgaf bug: on the `dk` keymap evdev 13 is the dead acute
            // key, so IBus swallowed it and then swallowed the `backspace`
            // after it to cancel the pending composition. Two keys out of
            // ninety-eight went missing on a Danish desktop and would have
            // arrived on a US one.
            //
            // These applications exist to answer "did the key arrive", not "did
            // the input method compose it", so the simple built-in context is
            // the right one: it keeps the xkb keymap and does its composition
            // inside GTK, where the raw key event reaches the application first.
            .env("GTK_IM_MODULE", "gtk-im-context-simple")
            .stdout(std::process::Stdio::null())
            // Kept, not silenced: the application prints report-write failures
            // here, and those are exactly what explains an empty report file.
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap_or_else(|err| panic!("failed to start {}: {err}", binary.display()));

        let app = Self {
            name,
            child,
            dir,
            report_path,
        };
        app.wait_until_reporting().await;
        app
    }

    /// The path being written to, for a failure message that wants to name it.
    pub fn report_path(&self) -> &Path {
        &self.report_path
    }

    /// Reads the current report, or `None` if the application has not written
    /// one yet.
    ///
    /// A parse failure is *not* treated as "not yet": reports are written whole
    /// or not at all (the `report` crate renames a complete file into place), so
    /// invalid JSON means something is genuinely wrong and silently retrying
    /// would hide it until the timeout.
    pub fn read(&self) -> Option<Report> {
        let bytes = std::fs::read(&self.report_path).ok()?;
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!(
                "{}'s report at {} is not valid JSON ({err}). \
                 Reports are renamed into place whole, so this is not a torn read.",
                self.name,
                self.report_path.display()
            )
        });
        Some(Report(value))
    }

    /// Waits for the first report, proving the application started and mapped.
    async fn wait_until_reporting(&self) -> Report {
        let deadline = tokio::time::Instant::now() + REPORT_TIMEOUT;
        loop {
            if let Some(report) = self.read() {
                return report;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "{} never wrote a report to {} within {:?}. \
                     It either failed to start or could not open a display.",
                    self.name,
                    self.report_path.display(),
                    REPORT_TIMEOUT
                );
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// The current `seq`, to be taken **before** driving wgaf.
    ///
    /// Mapping the windows already produces a burst of reports — `window-test`
    /// reaches `seq` 9 before a test does anything — so a test waits for `seq`
    /// to exceed a baseline it captured, never for a particular value.
    pub fn seq(&self) -> u64 {
        self.read()
            .expect("the application has stopped reporting")
            .seq()
    }

    /// Polls until `predicate` accepts a report, or fails naming `what` was
    /// expected and showing the last report seen.
    ///
    /// The report in the failure message is the point: a bare "condition not met
    /// in 10s" says nothing about whether the input never arrived, arrived as
    /// something else, or arrived at a window that was not focused — and the
    /// application already wrote down which.
    pub async fn wait_for(&self, what: &str, predicate: impl Fn(&Report) -> bool) -> Report {
        match self.try_wait_for(REPORT_TIMEOUT, &predicate).await {
            Ok(report) => report,
            Err(Some(last)) => panic!(
                "timed out after {REPORT_TIMEOUT:?} waiting for {what} from {}. \
                 Last report:\n{:#}",
                self.name,
                last.json()
            ),
            Err(None) => panic!(
                "timed out after {REPORT_TIMEOUT:?} waiting for {what}: \
                 {} wrote no report at all",
                self.name
            ),
        }
    }

    /// As [`Self::wait_for`], but returns the last report seen instead of
    /// failing, and takes its own timeout.
    ///
    /// For a test that checks many things in one run and wants to report *all*
    /// of what went wrong: a sweep over every key would otherwise stop at the
    /// first one that never arrived, and the run after the fix would stop at the
    /// second. The short timeout belongs to the caller for the same reason —
    /// ninety failing checks at ten seconds each is not a test anybody waits for.
    pub async fn try_wait_for(
        &self,
        timeout: Duration,
        predicate: impl Fn(&Report) -> bool,
    ) -> Result<Report, Option<Report>> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last = None;
        loop {
            if let Some(report) = self.read() {
                if predicate(&report) {
                    return Ok(report);
                }
                last = Some(report);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(last);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Waits for any new report after `baseline`.
    pub async fn wait_for_change(&self, baseline: u64) -> Report {
        self.wait_for(&format!("a report newer than seq {baseline}"), |report| {
            report.seq() > baseline
        })
        .await
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// The daemon under test
// ---------------------------------------------------------------------------

/// A spawned `wgaf-daemon`, killed on drop along with its configuration.
///
/// The files outlive `spawn` deliberately: the daemon reads them
/// asynchronously, and removing them immediately races its own startup.
pub struct DaemonGuard {
    child: Child,
    config_path: PathBuf,
    permissions_path: PathBuf,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_file(&self.permissions_path);
    }
}

/// Spawns the real daemon binary with a private bus name and configuration.
///
/// `extra_config` is appended verbatim, for a suite that needs a specific
/// setting — an input suite gives its virtual device a unique name, since
/// `/proc/bus/input/devices` is machine-global and has no notion of which
/// process created an entry.
///
/// **`input_device_settle_ms` is deliberately left at its default here**, unlike
/// `tests/input.rs`, which sets it to `0` so that its synthesized keystrokes go
/// nowhere. These suites have a window of their own to aim at and want delivery,
/// which is the entire reason they exist.
pub fn spawn_daemon(suite: &str, bus_name: &str, extra_config: &str) -> DaemonGuard {
    let nonce = format!("{}-{:?}", std::process::id(), std::thread::current().id());

    let config_path = std::env::temp_dir().join(format!("wgaf-{suite}-config-{nonce}.toml"));
    std::fs::write(
        &config_path,
        format!("bus_name = \"{bus_name}\"\nlog_level = \"error\"\n{extra_config}"),
    )
    .expect("failed to write the test config");

    // An empty capability table is the explicit way to say "allow everything".
    let permissions_path =
        std::env::temp_dir().join(format!("wgaf-{suite}-permissions-{nonce}.toml"));
    std::fs::write(&permissions_path, "[capabilities]\n").expect("failed to write the test policy");

    // The daemon rejects a group- or world-writable config or policy file, and
    // `fs::write` honours the umask — 002 on many distributions, giving 0664.
    for path in [&config_path, &permissions_path] {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .expect("failed to tighten test file permissions");
    }

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

/// Connects to the session bus and waits for the daemon to answer `Ping`.
pub async fn wait_for_daemon(bus_name: &str) -> Connection {
    let connection = Connection::session()
        .await
        .expect("failed to connect to the session bus");

    for _ in 0..100 {
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
    panic!("the daemon did not answer Ping within 10s of starting");
}

/// Calls a method on one of the daemon's interfaces.
pub async fn call<R, A>(
    connection: &Connection,
    bus_name: &str,
    object_path: &str,
    interface: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(Some(bus_name), object_path, Some(interface), method, args)
        .await?;
    reply.body().deserialize()
}

/// Calls a method on `org.wgaf.Input1`.
pub async fn input<R, A>(
    connection: &Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    call(
        connection,
        bus_name,
        wgaf_common::INPUT_OBJECT_PATH,
        wgaf_common::INPUT_INTERFACE_NAME,
        method,
        args,
    )
    .await
}

/// Calls a method on `org.wgaf.Windows1`.
pub async fn windows<R, A>(
    connection: &Connection,
    bus_name: &str,
    method: &str,
    args: &A,
) -> zbus::Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    A: serde::Serialize + zbus::zvariant::Type,
{
    call(
        connection,
        bus_name,
        wgaf_common::WINDOWS_OBJECT_PATH,
        wgaf_common::WINDOWS_INTERFACE_NAME,
        method,
        args,
    )
    .await
}

/// Creates the virtual input device and waits out its settle delay, so that the
/// first assertion of a test body is not the one that pays for it.
///
/// The daemon creates its `uinput` device lazily, on the first synthesis, and
/// then waits for the compositor to notice it. Until that wait elapses the
/// events are written to a device nobody is reading, and are simply lost —
/// silently, with the call reporting success (filed in `issues.md`). A test
/// whose first keystroke lands in that window fails while wgaf is working, so
/// the harness spends the first command on something with no observable effect.
///
/// `MouseMove(0, 0)` is that command: it goes through the same path as any other
/// synthesis, so it creates the device and pays the settle wait, while moving
/// the pointer nowhere and typing nothing. A keystroke would have had to land
/// somewhere.
pub async fn warm_up_input_device(connection: &Connection, bus_name: &str) {
    input::<(), _>(connection, bus_name, "MouseMove", &(0i32, 0i32))
        .await
        .expect("failed to warm up the input device");
}
