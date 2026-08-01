mod commands;
mod output;

use clap::{Args, CommandFactory, Parser, Subcommand};

/// wgaf — Wayland GNOME automation framework CLI.
#[derive(Parser)]
#[command(name = "wgaf")]
struct Cli {
    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// D-Bus well-known bus name of the daemon to talk to. Defaults to the
    /// daemon's own default (`org.wgaf.Daemon`, [`wgaf_common::BUS_NAME`]) —
    /// only pass this if the target daemon was started with a customized
    /// `bus_name` in its `config.toml` (see `Config::bus_name`).
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

    /// Window management commands (list/focus/move/resize/close, plus
    /// workspace listing), backed by the daemon's `org.wgaf.Windows1`
    /// D-Bus interface.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },

    /// Type a string of text (ASCII/US-QWERTY only), backed by the
    /// daemon's `org.wgaf.Input1` D-Bus interface.
    Type {
        /// The text to type.
        text: String,
    },

    /// Low-level single-key press/release, by evdev key name (`a`, `enter`,
    /// `leftshift`, `up`, `f5`, `altgr`, ...). No ASCII/shift awareness — see
    /// `wgaf type` for that; combine `key press leftshift` + `key press a` +
    /// releases for a capital `A`.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },

    /// Mouse automation commands (relative move, click, scroll), backed by
    /// the daemon's `org.wgaf.Input1` D-Bus interface. There is no
    /// absolute-move command.
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
    },

    /// Release a previously-pressed key.
    Release {
        /// Evdev key name (e.g. `a`, `KEY_A`, `enter`, `leftshift`, `up`,
        /// `f5`, `altgr`, `kp0`).
        key: String,
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

    /// List all workspaces.
    Workspaces,
}

#[derive(Args)]
struct WindowId {
    /// The window id, as reported by `wgaf window list`.
    id: u32,
}

#[tokio::main]
async fn main() {
    // Errors are printed here rather than returned from `main`, because
    // Rust's `Termination` impl for `Result` formats the error with `Debug`,
    // not `Display` — which wrapped every message in quotes (`Error: "unknown
    // key ..."`) and would print the raw struct for any error type that isn't
    // a plain string. `commands::describe_dbus_error` works hard to produce a
    // readable sentence; handing it to `Debug` undid that.
    match run().await {
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
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

async fn run() -> Result<Outcome, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
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
        Command::Window { command } => match command {
            WindowCommand::List => commands::window::list(bus_name, json).await?,
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
            WindowCommand::Workspaces => commands::window::workspaces(bus_name, json).await?,
        },
        Command::Type { text } => commands::input::type_text(bus_name, &text, json).await?,
        Command::Key { command } => match command {
            KeyCommand::Press { key } => commands::input::key_press(bus_name, &key, json).await?,
            KeyCommand::Release { key } => {
                commands::input::key_release(bus_name, &key, json).await?
            }
            KeyCommand::Combo { keys } => commands::input::hotkey(bus_name, &keys, json).await?,
        },
        Command::Mouse { command } => match command {
            MouseCommand::Move { dx, dy } => {
                commands::input::mouse_move(bus_name, dx, dy, json).await?
            }
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
    fn parses_window_workspaces() {
        let cli = Cli::try_parse_from(["wgaf", "window", "workspaces"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Window {
                command: WindowCommand::Workspaces
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
            Command::Type { text } => assert_eq!(text, "hello world"),
            _ => panic!("expected Type"),
        }
    }

    #[test]
    fn parses_key_press_and_release() {
        let cli = Cli::try_parse_from(["wgaf", "key", "press", "a"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Press { key },
            } => assert_eq!(key, "a"),
            _ => panic!("expected Key(Press)"),
        }

        let cli = Cli::try_parse_from(["wgaf", "key", "release", "leftshift"]).expect("parse");
        match cli.command {
            Command::Key {
                command: KeyCommand::Release { key },
            } => assert_eq!(key, "leftshift"),
            _ => panic!("expected Key(Release)"),
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
