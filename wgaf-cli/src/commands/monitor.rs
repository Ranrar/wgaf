//! `wgaf monitor ...` subcommands: a thin D-Bus client of the daemon's own
//! `org.wgaf.Windows1` interface. No business logic here: parse args (done by
//! `clap` in `main.rs`), call the daemon, format the reply.
//!
//! Unlike `window.rs` and `workspace.rs`, nothing behind this module talks to
//! the GNOME Shell extension — the daemon reads the layout from Mutter's own
//! `org.gnome.Mutter.DisplayConfig`. So `wgaf monitor list` works on a session
//! where the extension is not installed, which is worth preserving: it is how a
//! user finds out where they are allowed to move the pointer.

use wgaf_common::MonitorRecord;
use wgaf_common::dict::MonitorRecordDict;

use super::{CliResult, connect, map_err};

/// A human-readable name for Mutter's transform enum, or `None` for the
/// unrotated, unflipped case.
///
/// Values `0`–`3` are the quarter turns and `4`–`7` the same four with the
/// image flipped. Returning `None` rather than `"normal"` for `0` keeps the
/// common case out of the output entirely — a rotation column that reads
/// `normal` on every line is noise, and the whole point is that a rotated
/// monitor should stand out.
fn transform_name(transform: u32) -> Option<String> {
    let name = match transform {
        0 => return None,
        1 => "rotated 90",
        2 => "rotated 180",
        3 => "rotated 270",
        4 => "flipped",
        5 => "flipped, rotated 90",
        6 => "flipped, rotated 180",
        7 => "flipped, rotated 270",
        // Not a value Mutter's enum defines. Reported rather than hidden or
        // treated as normal: a monitor whose orientation wgaf does not
        // understand is exactly the case where a user needs to know the
        // geometry above it might be interpreted wrongly.
        other => return Some(format!("transform {other}")),
    };
    Some(name.to_string())
}

pub async fn list(bus_name: &str, json: bool) -> CliResult<()> {
    let connection = connect().await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            wgaf_common::WINDOWS_OBJECT_PATH,
            Some(wgaf_common::WINDOWS_INTERFACE_NAME),
            "GetMonitors",
            &(),
        )
        .await
        .map_err(map_err)?;
    let dicts: Vec<MonitorRecordDict> = reply.body().deserialize()?;
    let monitors: Vec<MonitorRecord> = dicts.into_iter().map(Into::into).collect();

    if json {
        crate::output::print_json(&monitors)?;
    } else if monitors.is_empty() {
        // Mutter reporting no monitors is a fault the daemon already turns into
        // an error, so this line should be unreachable in practice. It stays
        // because an empty list printing nothing at all would be indis-
        // tinguishable from the command not having run.
        println!("No monitors.");
    } else {
        for line in render(&monitors) {
            println!("{line}");
        }
    }
    Ok(())
}

/// The human listing: **exactly one line per monitor**, in order.
///
/// # The usable area is not here, on purpose
///
/// The monitor minus the top bar and any docks is in `--json` as `work_area`
/// and deliberately absent from this listing. It was here briefly and was the
/// wrong call twice over: printed through the monitor row's own format string
/// it read as an extra monitor, so a two-monitor desktop appeared to have three
/// displays; re-indenting fixed the confusion and left it noise in a listing
/// whose job is "where are my screens". A script placing windows needs it and
/// reads JSON. A person at a terminal is asking a simpler question.
///
/// Returning lines rather than printing them is what lets the one-line-per-
/// monitor rule be a test instead of a comment.
fn render(monitors: &[MonitorRecord]) -> Vec<String> {
    monitors
        .iter()
        .map(|m| {
            // Scale and rotation appear only when they are not the boring
            // default, so an ordinary single-monitor desktop prints one clean
            // line and an unusual setup prints exactly what makes it unusual.
            let mut notes: Vec<String> = Vec::new();
            if m.primary {
                notes.push("primary".to_string());
            }
            if let Some(name) = transform_name(m.transform) {
                notes.push(name);
            }
            if m.scale != 1.0 {
                notes.push(format!("scale {}", m.scale));
            }
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!("  [{}]", notes.join(", "))
            };

            format!(
                "{:<10} {:>5}x{:<5} at {:>6},{:<6}{}",
                m.connector, m.width, m.height, m.x, m.y, notes
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unrotated_case_has_no_name_to_print() {
        assert_eq!(transform_name(0), None);
    }

    /// The maintainer's own layout has a `transform: 1` panel, and reading the
    /// geometry without knowing that is how coordinate maths goes wrong.
    #[test]
    fn every_transform_mutter_defines_has_a_name() {
        assert_eq!(transform_name(1).as_deref(), Some("rotated 90"));
        assert_eq!(transform_name(2).as_deref(), Some("rotated 180"));
        assert_eq!(transform_name(3).as_deref(), Some("rotated 270"));
        assert_eq!(transform_name(4).as_deref(), Some("flipped"));
        assert_eq!(transform_name(7).as_deref(), Some("flipped, rotated 270"));
    }

    #[test]
    fn a_value_outside_the_enum_is_reported_rather_than_treated_as_normal() {
        assert_eq!(transform_name(42).as_deref(), Some("transform 42"));
    }

    /// The maintainer's real layout, the one that produced the bug report.
    fn two_monitors() -> Vec<MonitorRecord> {
        vec![
            MonitorRecord {
                connector: "DP-3".to_string(),
                x: 1080,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                transform: 0,
                primary: true,
                // The panel reserves 32px, so this differs from the monitor —
                // which is exactly the case that used to print a second line.
                work_area: Some(wgaf_common::Rect {
                    x: 1080,
                    y: 0,
                    width: 2560,
                    height: 1408,
                }),
            },
            MonitorRecord {
                connector: "HDMI-1".to_string(),
                x: 0,
                y: 0,
                width: 1080,
                height: 1920,
                scale: 1.0,
                transform: 1,
                primary: false,
                work_area: None,
            },
        ]
    }

    /// Two monitors must print two lines.
    ///
    /// This listing briefly carried a second line per monitor for the usable
    /// area, and a two-monitor desktop then appeared to have three displays —
    /// "why are there 3 displays?" was the report, which is the whole bug in
    /// one sentence.
    #[test]
    fn two_monitors_print_two_lines() {
        let lines = render(&two_monitors());
        assert_eq!(lines.len(), 2, "one line per monitor, got: {lines:#?}");
    }

    /// A monitor whose usable area differs must still print only its own line.
    ///
    /// Targets the exact condition the old code branched on, so this fails if
    /// the work area is ever reintroduced to the human listing — which is a
    /// deliberate product decision, not an oversight to be quietly reversed.
    #[test]
    fn a_reserved_work_area_does_not_add_a_line_or_a_mention() {
        let lines = render(&two_monitors());
        let joined = lines.join("\n");

        assert!(
            !joined.contains("usable") && !joined.contains("1408"),
            "the usable area belongs to --json, not the listing: {joined}"
        );
        // And each line still starts with the monitor it is about, so nothing
        // can be mistaken for a nameless extra display.
        assert!(lines[0].starts_with("DP-3"), "{}", lines[0]);
        assert!(lines[1].starts_with("HDMI-1"), "{}", lines[1]);
    }

    /// The notes that *are* worth a human's attention still appear.
    #[test]
    fn the_listing_still_flags_the_primary_and_a_rotation() {
        let lines = render(&two_monitors());
        assert!(lines[0].contains("[primary]"), "{}", lines[0]);
        assert!(lines[1].contains("rotated 90"), "{}", lines[1]);
        assert!(lines[0].contains("2560x1440"), "{}", lines[0]);
    }
}
