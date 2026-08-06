mod commands;
mod error;
mod output;

use clap::{Args, CommandFactory, Parser, Subcommand};
use error::{CliResult, Verdict};

/// wgaf — Wayland GNOME automation framework CLI.
#[derive(Parser)]
#[command(name = "wgaf")]
struct Cli {
    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// D-Bus name of the daemon to talk to. Defaults to `org.wgaf.Daemon`.
    ///
    /// Only needed if you changed `bus_name` in `config.toml`.
    #[arg(long, global = true)]
    bus_name: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the daemon is running and responding.
    Ping,

    /// Report whether every subsystem is set up correctly, and what
    /// permission policy the daemon is enforcing.
    ///
    /// Unlike `ping` (which only proves the daemon answers), this checks the
    /// GNOME Shell Extension bridge, `/dev/uinput` access, and the AT-SPI
    /// accessibility bus, and prints the daemon's own guidance for whichever
    /// of them is not working. Start here when something is not behaving.
    ///
    /// Exits non-zero if any subsystem is unavailable, so it can gate a
    /// setup script.
    Status,

    /// Stop all input synthesis immediately — the kill switch.
    ///
    /// Use this when a script has run away with the keyboard or pointer. The
    /// daemon refuses every further `type`/`key`/`mouse` command, aborts one
    /// already in progress, and removes its virtual input device.
    ///
    /// It never un-stops itself: run `wgaf release` when the script is dead.
    /// The stop is forgotten if the daemon restarts, and no permission policy
    /// can take it away from you.
    Stop,

    /// Release the kill switch, allowing input automation again.
    ///
    /// Think of `wgaf stop` as a handbrake: this releases it. It does not
    /// continue whatever was interrupted — the daemon keeps no record of that,
    /// and the script that was refused has already given up. Run your command
    /// again once input is allowed.
    Release,

    /// Window management commands (list/focus/move/resize/close), backed by
    /// the daemon's `org.wgaf.Windows1` D-Bus interface.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },

    /// Workspace commands (list/switch/add/remove/reorder), backed by the
    /// daemon's `org.wgaf.Windows1` D-Bus interface.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },

    /// Monitor commands, backed by the daemon's `org.wgaf.Windows1` D-Bus
    /// interface.
    ///
    /// Unlike every other command here, this one does not need the wgaf GNOME
    /// Shell extension: the layout is read from Mutter's own display
    /// configuration.
    Monitor {
        #[command(subcommand)]
        command: MonitorCommand,
    },

    /// Type a string of text (ASCII/US-QWERTY only), backed by the
    /// daemon's `org.wgaf.Input1` D-Bus interface.
    Type {
        /// The text to type.
        text: String,

        /// Type into this window specifically, correcting focus first if
        /// needed, instead of whatever currently has keyboard focus.
        ///
        /// Only enforced when `verification_level` in `config.toml` is not
        /// `none` — see that setting's docs. Omitting this flag behaves
        /// exactly as before it existed.
        #[arg(long)]
        window: Option<u32>,
    },

    /// Low-level single-key press/release, by evdev key name (`a`, `enter`,
    /// `leftshift`, `up`, `f5`, `altgr`, ...). No ASCII/shift awareness — see
    /// `wgaf type` for that; combine `key press leftshift` + `key press a` +
    /// releases for a capital `A`.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },

    /// Mouse automation commands (move, click, scroll), backed by the
    /// daemon's `org.wgaf.Input1` D-Bus interface.
    ///
    /// Prefer `wgaf a11y` where an element can be found by name or role:
    /// clicking a named button keeps working when a window moves or a theme
    /// changes, and a coordinate does not.
    Mouse {
        #[command(subcommand)]
        command: MouseCommand,
    },

    /// Accessibility automation (AT-SPI): enumerate accessible
    /// applications, find/inspect elements by role/name/description, and
    /// invoke actions (click/focus/set text) on them — backed by the
    /// daemon's `org.wgaf.Accessibility1` D-Bus interface. Preferred over
    /// coordinate-based automation whenever an element can be found this
    /// way.
    A11y {
        #[command(subcommand)]
        command: A11yCommand,
    },

    /// Print a shell completion script for the given shell to stdout.
    ///
    /// Redirect the output to wherever your shell loads completions from,
    /// e.g. `wgaf completions bash > /etc/bash_completion.d/wgaf` or `wgaf
    /// completions zsh > "${fpath[1]}/_wgaf"`.
    Completions {
        /// Shell to generate a completion script for.
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// Press (hold down) a key.
    Press {
        /// Evdev key name (e.g. `a`, `KEY_A`, `enter`, `leftshift`, `up`,
        /// `f5`, `altgr`, `kp0`).
        key: String,

        /// Press into this window specifically, correcting focus first if
        /// needed, instead of whatever currently has keyboard focus.
        ///
        /// Only enforced when `verification_level` in `config.toml` is not
        /// `none` — see that setting's docs. Omitting this flag behaves
        /// exactly as before it existed.
        #[arg(long)]
        window: Option<u32>,
    },

    /// Release a previously-pressed key.
    Release {
        /// Evdev key name (e.g. `a`, `KEY_A`, `enter`, `leftshift`, `up`,
        /// `f5`, `altgr`, `kp0`).
        key: String,

        /// Release into this window specifically, correcting focus first if
        /// needed, instead of whatever currently has keyboard focus.
        ///
        /// Only enforced when `verification_level` in `config.toml` is not
        /// `none` — see that setting's docs. Omitting this flag behaves
        /// exactly as before it existed.
        #[arg(long)]
        window: Option<u32>,
    },

    /// Press a key combination — all keys held, then released in reverse.
    ///
    /// e.g. `wgaf key combo ctrl shift t`. Doing this by hand takes six
    /// commands and leaves modifiers stuck down if one of them fails.
    ///
    /// These are physical keys, not characters, so a combination is the same
    /// on every keyboard layout.
    Combo {
        /// Key names, in the order they should be held (e.g. `ctrl shift t`).
        #[arg(required = true, num_args = 1..)]
        keys: Vec<String>,

        /// Press this combination into this window specifically, correcting
        /// focus first if needed, instead of whatever currently has
        /// keyboard focus.
        ///
        /// Only enforced when `verification_level` in `config.toml` is not
        /// `none` — see that setting's docs. Omitting this flag behaves
        /// exactly as before it existed.
        #[arg(long)]
        window: Option<u32>,
    },
}

#[derive(Subcommand)]
enum MouseCommand {
    /// Move the pointer relative to its current position.
    Move {
        /// May be negative (move left).
        #[arg(allow_hyphen_values = true)]
        dx: i32,
        /// May be negative (move up).
        #[arg(allow_hyphen_values = true)]
        dy: i32,
    },

    /// Move the pointer to an absolute screen position.
    ///
    /// Coordinates are in screen pixels, measured from the top-left of your
    /// desktop layout, and are exact — unlike `wgaf mouse move`, which is
    /// relative and subject to pointer acceleration.
    ///
    /// A position that is not on any monitor is refused, and nothing moves.
    /// Note that a desktop with monitors of different sizes has gaps: with a
    /// short monitor beside a tall one, a coordinate can be inside the overall
    /// rectangle and still on no screen. `wgaf mouse position` and the error
    /// message both show the layout.
    MoveTo {
        /// May be negative, for a monitor placed left of the primary one.
        #[arg(allow_hyphen_values = true)]
        x: i32,
        /// May be negative, for a monitor placed above the primary one.
        #[arg(allow_hyphen_values = true)]
        y: i32,
    },

    /// Print the pointer's current screen position.
    Position,

    /// Click (press then release) a mouse button.
    Click {
        /// Mouse button to click: `left`, `right`, or `middle`.
        button: String,
    },

    /// Scroll the mouse wheel.
    Scroll {
        /// Horizontal scroll amount, positive = right. May be negative.
        #[arg(allow_hyphen_values = true)]
        dx: i32,
        /// Vertical scroll amount, positive = up. May be negative.
        #[arg(allow_hyphen_values = true)]
        dy: i32,
    },
}

#[derive(Subcommand)]
enum A11yCommand {
    /// List every currently-registered accessible application.
    ListApps,

    /// Find elements within an application by role/name/description.
    Find {
        /// Application name (matched against `wgaf a11y list-apps`'
        /// output — exact match preferred, falls back to a substring
        /// match).
        #[arg(long)]
        app: String,
        /// AT-SPI role name (e.g. `push button`, `menu item`), matched
        /// case-insensitively as a whole. Empty (the default) matches any
        /// role.
        #[arg(long, default_value = "")]
        role: String,
        /// Case-insensitive substring match against the element's
        /// accessible name. Empty (the default) matches any name.
        #[arg(long, default_value = "")]
        name: String,
        /// Case-insensitive substring match against the element's
        /// accessible description. Empty (the default) matches any
        /// description.
        #[arg(long, default_value = "")]
        description: String,
        /// Maximum number of results to return. `0` (the default) uses the
        /// daemon's built-in default (100); values are hard-capped at 1000
        /// regardless.
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        max_results: i32,
    },

    /// Walk and print an application's accessible object tree.
    Tree {
        /// Application name — same matching rules as `find --app`.
        #[arg(long)]
        app: String,
        /// Maximum depth to descend, relative to the application's root
        /// object. `0` (the default) uses the daemon's built-in default
        /// (10); values are hard-capped at 64 regardless.
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        max_depth: i32,
    },

    /// Print a single element's current info, re-read directly from its
    /// reference.
    Info {
        /// Element reference, in `bus_name#object_path` form — as printed
        /// by `list-apps`/`find`/`tree`.
        element: wgaf_common::ElementRef,
    },

    /// Invoke an accessible action on an element (click/press/activate).
    Click {
        /// Element reference, in `bus_name#object_path` form.
        element: wgaf_common::ElementRef,
        /// Which action to invoke, by its machine-readable name
        /// (case-insensitive). Empty (the default) invokes the element's
        /// default action (AT-SPI's own convention: action index 0).
        #[arg(long, default_value = "")]
        action: String,
    },

    /// Request keyboard focus for an element.
    Focus {
        /// Element reference, in `bus_name#object_path` form.
        element: wgaf_common::ElementRef,
    },

    /// Replace an element's text content (requires the element to
    /// implement AT-SPI's `EditableText` interface — most text fields do).
    SetText {
        /// Element reference, in `bus_name#object_path` form.
        element: wgaf_common::ElementRef,
        /// The new text content.
        text: String,
    },
}

#[derive(Subcommand)]
enum WindowCommand {
    /// List all windows.
    List,

    /// Stream window events as they happen, until interrupted with Ctrl-C.
    ///
    /// Reports windows opening, closing and taking focus. Each line carries the
    /// window's id; run `wgaf window list` for its title and geometry, since a
    /// window has neither at the instant it is created.
    ///
    /// There is no replay: events that happened before this command started are
    /// gone. With --json each event is one line of JSON, so it can be piped
    /// straight into a program that reads a line at a time.
    ///
    /// Needs the GNOME Shell extension, and the WatchWindows permission.
    Watch,

    /// Focus (activate) a window by id.
    Focus(WindowId),

    /// Move a window by id so its top-left corner is at (x, y).
    Move {
        #[command(flatten)]
        id: WindowId,
        /// May be negative (e.g. on a monitor left of the primary one).
        #[arg(allow_hyphen_values = true)]
        x: i32,
        /// May be negative (e.g. on a monitor above the primary one).
        #[arg(allow_hyphen_values = true)]
        y: i32,
    },

    /// Resize a window by id to (width, height).
    Resize {
        #[command(flatten)]
        id: WindowId,
        /// Target width, in pixels.
        width: i32,
        /// Target height, in pixels.
        height: i32,
    },

    /// Close a window by id.
    Close(WindowId),

    /// Send a window to another workspace.
    ///
    /// The window moves; you stay where you are. Run `wgaf workspace switch`
    /// afterwards to follow it. The command does not return until the window is
    /// actually on that workspace.
    ///
    /// The workspace has to exist already — use `wgaf workspace add` first if
    /// it does not.
    MoveToWorkspace {
        #[command(flatten)]
        id: WindowId,
        /// The workspace index, as reported by `wgaf workspace list`.
        index: i32,
    },
}

#[derive(Args)]
struct WindowId {
    /// The window id, as reported by `wgaf window list`.
    id: u32,
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// List all workspaces, with their window counts and which one is active.
    List,

    /// Show how the workspaces are arranged: how many there are, which is
    /// active, the grid GNOME lays them out in, and whether GNOME is managing
    /// their number itself.
    ///
    /// That last one is worth knowing before using `add` or `remove`: with
    /// dynamic workspaces — GNOME's default — the Shell keeps one empty
    /// workspace at the end and reclaims any other that empties.
    Layout,

    /// Switch to a workspace by index.
    ///
    /// The command does not return until that workspace is actually active, so
    /// a following `wgaf window list` sees the new one.
    Switch(WorkspaceIndex),

    /// Add a workspace at the end, printing its index.
    ///
    /// The new workspace is not switched to — run `wgaf workspace switch` for
    /// that. With dynamic workspaces on, GNOME may reclaim it as soon as it is
    /// left empty; `wgaf workspace layout` says which mode you are in.
    Add,

    /// Remove a workspace by index.
    ///
    /// Windows on it are not closed — GNOME moves them to a neighbouring
    /// workspace. The last remaining workspace cannot be removed.
    Remove(WorkspaceIndex),

    /// Move a workspace to a different position.
    ///
    /// Every other workspace shifts to make room, so indices read before this
    /// are out of date afterwards.
    Reorder {
        #[command(flatten)]
        index: WorkspaceIndex,
        /// The position to move it to.
        new_index: i32,
    },
}

#[derive(Args)]
struct WorkspaceIndex {
    /// The workspace index, as reported by `wgaf workspace list`.
    index: i32,
}

#[derive(Subcommand)]
enum MonitorCommand {
    /// List the monitors making up the desktop.
    ///
    /// Positions and sizes are in the same coordinate space as `wgaf window
    /// list` and `wgaf mouse move-to`, and are already adjusted for scaling and
    /// rotation — so a coordinate inside one of these rectangles is one the
    /// pointer can actually be moved to.
    List,
}

#[tokio::main]
async fn main() {
    // Parsed here, rather than inside `run`, so `--json` is known even when
    // `run` returns an `Err` — the three-outcome taxonomy below (ADR-0007,
    // `plan-first-release.md` §16) needs it to choose between the plain-text
    // and JSON failure shapes.
    let cli = Cli::parse();
    let json = cli.json;

    // Errors are printed here rather than returned from `main`, because
    // Rust's `Termination` impl for `Result` formats the error with `Debug`,
    // not `Display` — which wrapped every message in quotes (`Error: "unknown
    // key ..."`) and would print the raw struct for any error type that isn't
    // a plain string. `crate::error::describe_dbus_error` works hard to
    // produce a readable sentence; handing it to `Debug` undid that.
    match run(cli).await {
        Err(err) => {
            if json {
                output::print_outcome_error(err.verdict, &err.message);
            } else {
                match err.verdict {
                    // A genuine error: unchanged from before this taxonomy
                    // existed, `error:` on stderr.
                    Verdict::Error => eprintln!("error: {}", err.message),
                    // A policy denial or a verification failure is not a
                    // fault — per ADR-0007, framing it as one tells a user
                    // their own `permissions.toml` rule (or an honestly
                    // reported focus mismatch) is a bug. The daemon's own
                    // wording already reads as a standalone sentence (e.g.
                    // `` `FocusWindow` denied by permission policy
                    // (permissions.toml)``), so it is printed as-is.
                    Verdict::Denied | Verdict::Unverified => eprintln!("{}", err.message),
                }
            }
            std::process::exit(err.verdict.exit_code());
        }
        // `wgaf status` reports an unhealthy subsystem by exiting non-zero
        // while still printing its full report, so it can gate a setup script
        // (`if wgaf status; then ...`). That is distinct from the error case
        // above: the command succeeded, the system it described did not.
        Ok(Outcome::Unhealthy) => std::process::exit(1),
        Ok(Outcome::Ok) => {}
    }
}

/// Whether the command's *subject* was healthy, as opposed to whether the
/// command itself succeeded. Only `wgaf status` can report `Unhealthy`.
enum Outcome {
    Ok,
    Unhealthy,
}

async fn run(cli: Cli) -> CliResult<Outcome> {
    let json = cli.json;
    // ADDED: `--bus-name` defaults to the daemon's own default rather than
    // being baked into a clap `default_value`, so the "customized bus name"
    // case and the "default" case both flow through the same
    // `Option::unwrap_or_else` path instead of clap needing to stringify a
    // `const` at derive-macro time.
    let bus_name = cli
        .bus_name
        .unwrap_or_else(|| wgaf_common::BUS_NAME.to_string());
    let bus_name = bus_name.as_str();

    match cli.command {
        Command::Ping => commands::ping(bus_name, json).await?,
        Command::Status => {
            return Ok(if commands::status(bus_name, json).await? {
                Outcome::Ok
            } else {
                Outcome::Unhealthy
            });
        }
        Command::Stop => commands::stop(bus_name, json).await?,
        Command::Release => commands::release(bus_name, json).await?,
        Command::Window { command } => match command {
            WindowCommand::List => commands::window::list(bus_name, json).await?,
            WindowCommand::Watch => commands::window::watch(bus_name, json).await?,
            WindowCommand::Focus(WindowId { id }) => {
                commands::window::focus(bus_name, id, json).await?
            }
            WindowCommand::Move {
                id: WindowId { id },
                x,
                y,
            } => commands::window::move_window(bus_name, id, x, y, json).await?,
            WindowCommand::Resize {
                id: WindowId { id },
                width,
                height,
            } => commands::window::resize(bus_name, id, width, height, json).await?,
            WindowCommand::Close(WindowId { id }) => {
                commands::window::close(bus_name, id, json).await?
            }
            WindowCommand::MoveToWorkspace {
                id: WindowId { id },
                index,
            } => commands::window::move_to_workspace(bus_name, id, index, json).await?,
        },
        Command::Workspace { command } => match command {
            WorkspaceCommand::List => commands::workspace::list(bus_name, json).await?,
            WorkspaceCommand::Layout => commands::workspace::layout(bus_name, json).await?,
            WorkspaceCommand::Switch(WorkspaceIndex { index }) => {
                commands::workspace::switch(bus_name, index, json).await?
            }
            WorkspaceCommand::Add => commands::workspace::add(bus_name, json).await?,
            WorkspaceCommand::Remove(WorkspaceIndex { index }) => {
                commands::workspace::remove(bus_name, index, json).await?
            }
            WorkspaceCommand::Reorder {
                index: WorkspaceIndex { index },
                new_index,
            } => commands::workspace::reorder(bus_name, index, new_index, json).await?,
        },
        Command::Monitor { command } => match command {
            MonitorCommand::List => commands::monitor::list(bus_name, json).await?,
        },
        Command::Type { text, window } => {
            commands::input::type_text(bus_name, &text, window, json).await?
        }
        Command::Key { command } => match command {
            KeyCommand::Press { key, window } => {
                commands::input::key_press(bus_name, &key, window, json).await?
            }
            KeyCommand::Release { key, window } => {
                commands::input::key_release(bus_name, &key, window, json).await?
            }
            KeyCommand::Combo { keys, window } => {
                commands::input::hotkey(bus_name, &keys, window, json).await?
            }
        },
        Command::Mouse { command } => match command {
            MouseCommand::Move { dx, dy } => {
                commands::input::mouse_move(bus_name, dx, dy, json).await?
            }
            MouseCommand::MoveTo { x, y } => {
                commands::input::mouse_move_to(bus_name, x, y, json).await?
            }
            MouseCommand::Position => commands::input::mouse_position(bus_name, json).await?,
            MouseCommand::Click { button } => {
                commands::input::mouse_click(bus_name, &button, json).await?
            }
            MouseCommand::Scroll { dx, dy } => {
                commands::input::mouse_scroll(bus_name, dx, dy, json).await?
            }
        },
        Command::A11y { command } => match command {
            A11yCommand::ListApps => commands::accessibility::list_apps(bus_name, json).await?,
            A11yCommand::Find {
                app,
                role,
                name,
                description,
                max_results,
            } => {
                commands::accessibility::find(
                    bus_name,
                    &app,
                    &role,
                    &name,
                    &description,
                    max_results,
                    json,
                )
                .await?
            }
            A11yCommand::Tree { app, max_depth } => {
                commands::accessibility::tree(bus_name, &app, max_depth, json).await?
            }
            A11yCommand::Info { element } => {
                commands::accessibility::get_element_info(bus_name, &element, json).await?
            }
            A11yCommand::Click { element, action } => {
                commands::accessibility::click(bus_name, &element, &action, json).await?
            }
            A11yCommand::Focus { element } => {
                commands::accessibility::focus(bus_name, &element, json).await?
            }
            A11yCommand::SetText { element, text } => {
                commands::accessibility::set_text(bus_name, &element, &text, json).await?
            }
        },
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
        }
    }

    Ok(Outcome::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        // Catches clap definition errors (duplicate args, bad defaults,
        // etc.) at test time rather than only at first real invocation.
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_ping() {
        let cli = Cli::try_parse_from(["wgaf", "ping"]).expect("should parse");
        assert!(matches!(cli.command, Command::Ping));
        assert!(!cli.json);
    }

    #[test]
    fn parses_status() {
        let cli = Cli::try_parse_from(["wgaf", "status"]).expect("should parse");
        assert!(matches!(cli.command, Command::Status));
        assert!(!cli.json);
    }

    #[test]
    fn parses_status_with_json_and_bus_name() {
        // `status` is the command most likely to be scripted or pasted into a
        // bug report, so both global flags have to work on it.
        let cli = Cli::try_parse_from(["wgaf", "status", "--json", "--bus-name", "org.example.X"])
            .expect("should parse");
        assert!(matches!(cli.command, Command::Status));
        assert!(cli.json);
        assert_eq!(cli.bus_name.as_deref(), Some("org.example.X"));
    }

    #[test]
    fn parses_window_list_with_json_flag() {
        let cli = Cli::try_parse_from(["wgaf", "--json", "window", "list"]).expect("should parse");
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Command::Window {
                command: WindowCommand::List
            }
        ));
    }

    #[test]
    fn json_flag_works_after_subcommand_too() {
        // `global = true` should make --json valid on either side of the
        // subcommand, since scripts may reasonably write it either way.
        let cli = Cli::try_parse_from(["wgaf", "window", "list", "--json"]).expect("should parse");
        assert!(cli.json);
    }

    #[test]
    fn parses_window_focus() {
        let cli = Cli::try_parse_from(["wgaf", "window", "focus", "42"]).expect("should parse");
        match cli.command {
            Command::Window {
                command: WindowCommand::Focus(WindowId { id }),
            } => assert_eq!(id, 42),
            _ => panic!("expected Window(Focus)"),
        }
    }

    #[test]
    fn parses_window_move() {
        let cli =
            Cli::try_parse_from(["wgaf", "window", "move", "7", "100", "-50"]).expect("parse");
        match cli.command {
            Command::Window {
                command: WindowCommand::Move { id, x, y },
            } => {
                assert_eq!(id.id, 7);
                assert_eq!(x, 100);
                assert_eq!(y, -50);
            }
            _ => panic!("expected Window(Move)"),
        }
    }

    #[test]
    fn parses_window_resize() {
        let cli =
            Cli::try_parse_from(["wgaf", "window", "resize", "7", "800", "600"]).expect("parse");
        match cli.command {
            Command::Window {
                command: WindowCommand::Resize { id, width, height },
            } => {
                assert_eq!(id.id, 7);
                assert_eq!(width, 800);
                assert_eq!(height, 600);
            }
            _ => panic!("expected Window(Resize)"),
        }
    }

    #[test]
    fn parses_window_close() {
        let cli = Cli::try_parse_from(["wgaf", "window", "close", "3"]).expect("parse");
        match cli.command {
            Command::Window {
                command: WindowCommand::Close(WindowId { id }),
            } => assert_eq!(id, 3),
            _ => panic!("expected Window(Close)"),
        }
    }

    #[test]
    fn parses_workspace_list() {
        let cli = Cli::try_parse_from(["wgaf", "workspace", "list"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Workspace {
                command: WorkspaceCommand::List
            }
        ));
    }

    /// Workspace listing moved out of `wgaf window` when the workspace noun
    /// gained its mutating verbs — switching a workspace is not an operation
    /// on a window, and two spellings of one command is how a CLI drifts.
    /// Pinned so it cannot quietly come back as a second way to do this.
    #[test]
    fn workspaces_is_no_longer_a_window_subcommand() {
        assert!(Cli::try_parse_from(["wgaf", "window", "workspaces"]).is_err());
    }

    #[test]
    fn parses_workspace_switch() {
        let cli = Cli::try_parse_from(["wgaf", "workspace", "switch", "2"]).expect("parse");
        match cli.command {
            Command::Workspace {
                command: WorkspaceCommand::Switch(WorkspaceIndex { index }),
            } => assert_eq!(index, 2),
            _ => panic!("expected Workspace(Switch)"),
        }
    }

    #[test]
    fn parses_workspace_add_and_layout() {
        assert!(matches!(
            Cli::try_parse_from(["wgaf", "workspace", "add"])
                .expect("parse")
                .command,
            Command::Workspace {
                command: WorkspaceCommand::Add
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["wgaf", "workspace", "layout"])
                .expect("parse")
                .command,
            Command::Workspace {
                command: WorkspaceCommand::Layout
            }
        ));
    }

    #[test]
    fn parses_workspace_reorder_with_both_positions() {
        let cli = Cli::try_parse_from(["wgaf", "workspace", "reorder", "3", "1"]).expect("parse");
        match cli.command {
            Command::Workspace {
                command:
                    WorkspaceCommand::Reorder {
                        index: WorkspaceIndex { index },
                        new_index,
                    },
            } => {
                assert_eq!(index, 3);
                assert_eq!(new_index, 1);
            }
            _ => panic!("expected Workspace(Reorder)"),
        }
    }

    #[test]
    fn rejects_workspace_commands_missing_their_index() {
        assert!(Cli::try_parse_from(["wgaf", "workspace", "switch"]).is_err());
        assert!(Cli::try_parse_from(["wgaf", "workspace", "remove"]).is_err());
        assert!(Cli::try_parse_from(["wgaf", "workspace", "reorder", "1"]).is_err());
    }

    #[test]
    fn parses_monitor_list() {
        let cli = Cli::try_parse_from(["wgaf", "monitor", "list"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Monitor {
                command: MonitorCommand::List
            }
        ));
    }

    #[test]
    fn rejects_missing_window_id() {
        assert!(Cli::try_parse_from(["wgaf", "window", "focus"]).is_err());
    }

    #[test]
    fn parses_type() {
        let cli = Cli::try_parse_from(["wgaf", "type", "hello world"]).expect("parse");
        match cli.command {
            Command::Type { text, window } => {
                assert_eq!(text, "hello world");
                assert_eq!(window, None);
            }
            _ => panic!("expected Type"),
        }
    }

    #[test]
    fn parses_type_with_window() {
        let cli = Cli::try_parse_from(["wgaf", "type", "hello", "--window", "42"]).expect("parse");
        match cli.command {
            Command::Type { text, window } => {
                assert_eq!(text, "hello");
                assert_eq!(window, Some(42));
            }
            _ => panic!("expected Type"),
        }
    }

    #[test]
    fn parses_key_press_and_release() {
        let cli = Cli::try_parse_from(["wgaf", "key", "press", "a"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Press { key, window },
            } => {
                assert_eq!(key, "a");
                assert_eq!(window, None);
            }
            _ => panic!("expected Key(Press)"),
        }

        let cli = Cli::try_parse_from(["wgaf", "key", "release", "leftshift"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Release { key, window },
            } => {
                assert_eq!(key, "leftshift");
                assert_eq!(window, None);
            }
            _ => panic!("expected Key(Release)"),
        }
    }

    #[test]
    fn parses_key_press_with_window() {
        let cli =
            Cli::try_parse_from(["wgaf", "key", "press", "a", "--window", "7"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Press { key, window },
            } => {
                assert_eq!(key, "a");
                assert_eq!(window, Some(7));
            }
            _ => panic!("expected Key(Press)"),
        }
    }

    #[test]
    fn parses_key_release_with_window() {
        let cli =
            Cli::try_parse_from(["wgaf", "key", "release", "a", "--window", "7"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Release { key, window },
            } => {
                assert_eq!(key, "a");
                assert_eq!(window, Some(7));
            }
            _ => panic!("expected Key(Release)"),
        }
    }

    #[test]
    fn parses_key_combo_with_window() {
        let cli = Cli::try_parse_from([
            "wgaf", "key", "combo", "ctrl", "shift", "t", "--window", "7",
        ])
        .expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Combo { keys, window },
            } => {
                assert_eq!(keys, vec!["ctrl", "shift", "t"]);
                assert_eq!(window, Some(7));
            }
            _ => panic!("expected Key(Combo)"),
        }
    }

    #[test]
    fn parses_mouse_move_with_negative_values() {
        let cli = Cli::try_parse_from(["wgaf", "mouse", "move", "-10", "20"]).expect("parse");
        match cli.command {
            Command::Mouse {
                command: MouseCommand::Move { dx, dy },
            } => {
                assert_eq!(dx, -10);
                assert_eq!(dy, 20);
            }
            _ => panic!("expected Mouse(Move)"),
        }
    }

    #[test]
    fn parses_mouse_click() {
        let cli = Cli::try_parse_from(["wgaf", "mouse", "click", "left"]).expect("parse");
        match cli.command {
            Command::Mouse {
                command: MouseCommand::Click { button },
            } => assert_eq!(button, "left"),
            _ => panic!("expected Mouse(Click)"),
        }
    }

    #[test]
    fn parses_mouse_scroll_with_negative_values() {
        let cli = Cli::try_parse_from(["wgaf", "mouse", "scroll", "0", "-5"]).expect("parse");
        match cli.command {
            Command::Mouse {
                command: MouseCommand::Scroll { dx, dy },
            } => {
                assert_eq!(dx, 0);
                assert_eq!(dy, -5);
            }
            _ => panic!("expected Mouse(Scroll)"),
        }
    }

    #[test]
    fn rejects_missing_mouse_move_args() {
        assert!(Cli::try_parse_from(["wgaf", "mouse", "move", "10"]).is_err());
    }

    #[test]
    fn parses_a11y_list_apps() {
        let cli = Cli::try_parse_from(["wgaf", "a11y", "list-apps"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::A11y {
                command: A11yCommand::ListApps
            }
        ));
    }

    #[test]
    fn parses_a11y_find_with_all_filters() {
        let cli = Cli::try_parse_from([
            "wgaf",
            "a11y",
            "find",
            "--app",
            "gtk4-demo",
            "--role",
            "push button",
            "--name",
            "Save",
            "--description",
            "Saves the file",
            "--max-results",
            "5",
        ])
        .expect("parse");
        match cli.command {
            Command::A11y {
                command:
                    A11yCommand::Find {
                        app,
                        role,
                        name,
                        description,
                        max_results,
                    },
            } => {
                assert_eq!(app, "gtk4-demo");
                assert_eq!(role, "push button");
                assert_eq!(name, "Save");
                assert_eq!(description, "Saves the file");
                assert_eq!(max_results, 5);
            }
            _ => panic!("expected A11y(Find)"),
        }
    }

    #[test]
    fn parses_a11y_find_with_only_required_app_filter() {
        let cli =
            Cli::try_parse_from(["wgaf", "a11y", "find", "--app", "gtk4-demo"]).expect("parse");
        match cli.command {
            Command::A11y {
                command:
                    A11yCommand::Find {
                        app,
                        role,
                        name,
                        description,
                        max_results,
                    },
            } => {
                assert_eq!(app, "gtk4-demo");
                assert_eq!(role, "");
                assert_eq!(name, "");
                assert_eq!(description, "");
                assert_eq!(max_results, 0);
            }
            _ => panic!("expected A11y(Find)"),
        }
    }

    #[test]
    fn rejects_a11y_find_without_app() {
        assert!(Cli::try_parse_from(["wgaf", "a11y", "find"]).is_err());
    }

    #[test]
    fn parses_a11y_tree() {
        let cli = Cli::try_parse_from([
            "wgaf",
            "a11y",
            "tree",
            "--app",
            "gtk4-demo",
            "--max-depth",
            "3",
        ])
        .expect("parse");
        match cli.command {
            Command::A11y {
                command: A11yCommand::Tree { app, max_depth },
            } => {
                assert_eq!(app, "gtk4-demo");
                assert_eq!(max_depth, 3);
            }
            _ => panic!("expected A11y(Tree)"),
        }
    }

    #[test]
    fn parses_a11y_element_ref_argument() {
        let cli = Cli::try_parse_from([
            "wgaf",
            "a11y",
            "focus",
            ":1.87#/org/a11y/atspi/accessible/1234",
        ])
        .expect("parse");
        match cli.command {
            Command::A11y {
                command: A11yCommand::Focus { element },
            } => {
                assert_eq!(element.bus_name, ":1.87");
                assert_eq!(element.object_path, "/org/a11y/atspi/accessible/1234");
            }
            _ => panic!("expected A11y(Focus)"),
        }
    }

    #[test]
    fn rejects_malformed_a11y_element_ref() {
        assert!(Cli::try_parse_from(["wgaf", "a11y", "focus", "no-hash-here"]).is_err());
    }

    #[test]
    fn parses_a11y_click_with_action() {
        let cli = Cli::try_parse_from([
            "wgaf",
            "a11y",
            "click",
            ":1.87#/org/a11y/atspi/accessible/1234",
            "--action",
            "press",
        ])
        .expect("parse");
        match cli.command {
            Command::A11y {
                command: A11yCommand::Click { element, action },
            } => {
                assert_eq!(element.to_string(), ":1.87#/org/a11y/atspi/accessible/1234");
                assert_eq!(action, "press");
            }
            _ => panic!("expected A11y(Click)"),
        }
    }

    #[test]
    fn parses_a11y_set_text() {
        let cli = Cli::try_parse_from([
            "wgaf",
            "a11y",
            "set-text",
            ":1.87#/org/a11y/atspi/accessible/1234",
            "hello world",
        ])
        .expect("parse");
        match cli.command {
            Command::A11y {
                command: A11yCommand::SetText { element, text },
            } => {
                assert_eq!(element.bus_name, ":1.87");
                assert_eq!(text, "hello world");
            }
            _ => panic!("expected A11y(SetText)"),
        }
    }

    #[test]
    fn parses_completions_bash() {
        let cli = Cli::try_parse_from(["wgaf", "completions", "bash"]).expect("parse");
        match cli.command {
            Command::Completions { shell } => assert_eq!(shell, clap_complete::Shell::Bash),
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn parses_completions_zsh() {
        let cli = Cli::try_parse_from(["wgaf", "completions", "zsh"]).expect("parse");
        match cli.command {
            Command::Completions { shell } => assert_eq!(shell, clap_complete::Shell::Zsh),
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn rejects_unknown_completions_shell() {
        assert!(Cli::try_parse_from(["wgaf", "completions", "not-a-shell"]).is_err());
    }

    /// Regenerates `wgaf`'s man pages (one `.1` file per subcommand,
    /// recursively) into `target/man/`. Not run as part of the normal test
    /// suite (`#[ignore]`) since it writes files rather than asserting
    /// anything — run it on demand with:
    ///
    ///     cargo test -p wgaf-cli generate_man_pages -- --ignored
    ///
    /// `clap_mangen` is a `[dev-dependencies]`-only dependency (see
    /// `wgaf-cli/Cargo.toml`), so none of this ships in the release binary —
    /// there is no packaging pipeline yet to consume a build-time artifact,
    /// so this is deliberately an on-demand dev step rather than a
    /// `build.rs`.
    #[test]
    #[ignore]
    fn generate_man_pages() {
        let out_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/man");
        std::fs::create_dir_all(out_dir).expect("create target/man");
        clap_mangen::generate_to(Cli::command(), out_dir).expect("generate man pages");
        println!("man pages written to {out_dir}");
    }
}
