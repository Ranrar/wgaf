//! The channel a test application uses to say what it observed.
//!
//! wgaf's self-testing rests on one rule: **wgaf drives, the application
//! reports, and an assertion never travels back through the code path under
//! test.** Verifying `wgaf type "hello"` with `wgaf a11y info` would let a
//! single AT-SPI bug produce a false pass, and would blame the wrong subsystem
//! when something genuinely broke. So the application writes what it received
//! to a file, and the test reads that file directly — a path wgaf is not
//! involved in anywhere.
//!
//! A plain JSON file is deliberately the dumbest mechanism that works. There is
//! no IPC to get wrong, and the report survives the process: after a failure
//! the file is still on disk, which is exactly when it is wanted. The cost is
//! that a test polls with a timeout rather than waiting on an event, which is a
//! fair trade for a channel that cannot itself be the thing that is broken.
//!
//! # The contract
//!
//! Every report is a single JSON object carrying two envelope fields, with the
//! application's own state flattened alongside them:
//!
//! ```json
//! {
//!   "app": "window-test",
//!   "seq": 3,
//!   "pid": 12345,
//!   "windows": []
//! }
//! ```
//!
//! - **`app`** names the application, so a report read from the wrong path is
//!   obvious rather than mysterious.
//! - **`seq`** starts at 1 and increases by one per report. It is what lets a
//!   test tell three different situations apart: **no file** means the
//!   application has not started, **`seq` unchanged** means it has started and
//!   nothing has happened since, and **`seq` increased** means a new event
//!   arrived. Without it, a test cannot distinguish "the keystroke never
//!   arrived" from "I read the file before it was rewritten", and would either
//!   sleep arbitrarily or flake.
//!
//! Reports are written whole or not at all — see [`Reporter::report`].

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

/// One report as it appears on disk: the envelope, with the application's own
/// state flattened in beside it rather than nested under a key. Flattening
/// keeps the file readable by eye during a failure, which is most of the reason
/// this channel is a file at all.
#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    app: &'a str,
    seq: u64,
    #[serde(flatten)]
    state: &'a T,
}

/// Writes an application's state to its report file.
///
/// Construct one at startup with the path the harness passed in, then call
/// [`Self::report`] on every observed change — including once when the
/// application is first up, so that a test can tell "started, nothing has
/// happened" from "not started".
#[derive(Debug)]
pub struct Reporter {
    app: String,
    path: PathBuf,
    /// Precomputed sibling of `path`, since [`Self::report`] must not fail
    /// halfway through deciding where to write.
    temp_path: PathBuf,
    seq: u64,
}

impl Reporter {
    /// Prepares to report as `app` at `path`.
    ///
    /// Fails if `path` names no file, or if its parent directory does not
    /// exist — both are harness mistakes, and catching them at startup gives a
    /// message naming the path instead of a puzzling failure on the first
    /// event.
    pub fn new(app: impl Into<String>, path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();

        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("report path `{}` does not name a file", path.display()),
            )
        })?;

        // The temporary file must be a sibling of the target: `rename(2)` is
        // only atomic within one filesystem, and a conventional temp directory
        // is very often a different one. The pid keeps two applications
        // reporting into the same directory from colliding.
        let temp_name = format!(
            ".{}.tmp.{}",
            file_name.to_string_lossy(),
            std::process::id()
        );
        let temp_path = path.with_file_name(temp_name);

        let parent = temp_path.parent().unwrap_or(Path::new("."));
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "report directory `{}` does not exist (for report path `{}`)",
                    parent.display(),
                    path.display()
                ),
            ));
        }

        Ok(Self {
            app: app.into(),
            path,
            temp_path,
            seq: 0,
        })
    }

    /// The `seq` of the most recent report; `0` before the first one.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The path being written to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Increments `seq` and writes `state` to the report file.
    ///
    /// **A reader never observes a partial report.** The whole document is
    /// written to a sibling temporary file, flushed to disk, and then
    /// `rename(2)`d over the target — an atomic replacement, so a test polling
    /// the file sees either the previous report or the new one, never a
    /// truncated mixture. Writing in place would eventually hand a reader half
    /// a document, and the resulting parse failure would look like a wgaf bug
    /// rather than a test-harness one.
    ///
    /// `state` must serialize as a JSON object, since its fields are flattened
    /// into the envelope; a sequence or a bare value has nothing to flatten
    /// into and fails here.
    pub fn report<T: Serialize>(&mut self, state: &T) -> io::Result<()> {
        self.seq += 1;

        let envelope = Envelope {
            app: &self.app,
            seq: self.seq,
            state,
        };
        let mut payload = serde_json::to_vec_pretty(&envelope).map_err(io::Error::other)?;
        payload.push(b'\n');

        let mut file = File::create(&self.temp_path)?;
        file.write_all(&payload)?;
        // Flush to the filesystem before the rename, so a report that a test
        // can see named is also a report that is fully on disk.
        file.sync_all()?;
        drop(file);

        std::fs::rename(&self.temp_path, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[derive(Serialize)]
    struct Typed {
        typed: String,
        clicks: u32,
    }

    /// A directory that removes itself, so the tests leave nothing behind.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "wgaf-report-test-{}-{}-{:?}",
                tag,
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&path).expect("failed to create temp dir");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: &Path) -> serde_json::Value {
        let bytes = std::fs::read(path).expect("report file missing");
        serde_json::from_slice(&bytes).expect("report was not valid JSON")
    }

    #[test]
    fn a_report_carries_the_app_name_and_a_sequence_number() {
        let dir = TempDir::new("envelope");
        let path = dir.join("report.json");
        let mut reporter = Reporter::new("input-test", &path).expect("failed to build reporter");

        reporter
            .report(&Typed {
                typed: "hello".into(),
                clicks: 0,
            })
            .expect("failed to report");

        let value = read(&path);
        assert_eq!(value["app"], "input-test");
        assert_eq!(value["seq"], 1);
    }

    /// The distinction the whole channel exists to support: a test must be able
    /// to wait for the *next* event rather than re-reading a stale one.
    #[test]
    fn seq_starts_at_one_and_increases_by_one_per_report() {
        let dir = TempDir::new("seq");
        let path = dir.join("report.json");
        let mut reporter = Reporter::new("window-test", &path).expect("failed to build reporter");

        assert_eq!(reporter.seq(), 0, "nothing reported yet");

        for expected in 1..=5 {
            reporter
                .report(&Typed {
                    typed: String::new(),
                    clicks: expected,
                })
                .expect("failed to report");
            assert_eq!(reporter.seq(), expected as u64);
            assert_eq!(read(&path)["seq"], expected);
        }
    }

    #[test]
    fn application_state_sits_alongside_the_envelope_not_nested_under_it() {
        let dir = TempDir::new("flatten");
        let path = dir.join("report.json");
        let mut reporter = Reporter::new("input-test", &path).expect("failed to build reporter");

        reporter
            .report(&Typed {
                typed: "Hello".into(),
                clicks: 2,
            })
            .expect("failed to report");

        let value = read(&path);
        assert_eq!(value["typed"], "Hello");
        assert_eq!(value["clicks"], 2);
        assert!(value.get("state").is_none(), "state must not be nested");
    }

    #[test]
    fn the_temporary_file_never_survives_a_report() {
        let dir = TempDir::new("tempfile");
        let path = dir.join("report.json");
        let mut reporter = Reporter::new("window-test", &path).expect("failed to build reporter");

        reporter
            .report(&Typed {
                typed: String::new(),
                clicks: 1,
            })
            .expect("failed to report");

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .expect("failed to list temp dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "report.json")
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    /// Two applications reporting into one directory must not fight over the
    /// same temporary file. Within a process the target name distinguishes
    /// them; across processes the pid does.
    #[test]
    fn separate_reports_use_separate_temporary_files() {
        let dir = TempDir::new("distinct");
        let first = Reporter::new("window-test", dir.join("a.json")).expect("failed to build");
        let second = Reporter::new("input-test", dir.join("b.json")).expect("failed to build");

        assert_ne!(first.temp_path, second.temp_path);
        assert_eq!(first.temp_path.parent(), first.path.parent());
    }

    /// The property that makes this channel safe to poll: a reader running
    /// concurrently with a writer always sees a complete document. Without the
    /// write-then-rename this fails, and the resulting parse error would be
    /// mistaken for a wgaf bug.
    #[test]
    fn a_concurrent_reader_never_observes_a_partial_report() {
        let dir = TempDir::new("atomic");
        let path = dir.join("report.json");
        let writer_path = path.clone();

        let (stop_tx, stop_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let mut reporter =
                Reporter::new("input-test", &writer_path).expect("failed to build reporter");
            for i in 0..300 {
                reporter
                    .report(&Typed {
                        // Vary the length so a torn write would leave a
                        // different number of bytes behind than the last
                        // complete report did.
                        typed: "x".repeat(i % 64),
                        clicks: i as u32,
                    })
                    .expect("failed to report");
            }
            let _ = stop_tx.send(());
        });

        let mut complete_reads = 0;
        loop {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .expect("reader observed a partial report");
                    complete_reads += 1;
                }
                // The writer has not produced the first report yet.
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => panic!("unexpected read failure: {err}"),
            }

            if stop_rx.try_recv().is_ok() {
                break;
            }
        }

        writer.join().expect("writer panicked");
        assert!(complete_reads > 0, "the reader never saw a report at all");
    }

    #[test]
    fn a_report_path_in_a_missing_directory_fails_at_startup() {
        let dir = TempDir::new("missing");
        let err = Reporter::new("window-test", dir.join("nope").join("report.json"))
            .expect_err("a missing directory must be rejected");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        // The message must name the path, since this is a harness mistake and
        // the harness author is the one reading it.
        assert!(err.to_string().contains("nope"), "{err}");
    }
}
