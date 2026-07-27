mod commands;

use clap::{Args, Parser, Subcommand};

/// wgaf — Wayland GNOME automation framework CLI.
#[derive(Parser)]
#[command(name = "wgaf")]
struct Cli {
    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the daemon is running and responding.
    Ping,

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
    /// `leftshift`, ...). No ASCII/shift awareness — see `wgaf type` for
    /// that; combine `key press leftshift` + `key press a` + releases for a
    /// capital `A`.
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
}

#[derive(Subcommand)]
enum KeyCommand {
    /// Press (hold down) a key.
    Press {
        /// Evdev key name (e.g. `a`, `KEY_A`, `enter`, `leftshift`).
        key: String,
    },

    /// Release a previously-pressed key.
    Release {
        /// Evdev key name (e.g. `a`, `KEY_A`, `enter`, `leftshift`).
        key: String,
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
        /// `left`, `right`, or `middle`.
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
        width: i32,
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let json = cli.json;

    match cli.command {
        Command::Ping => commands::ping().await?,
        Command::Window { command } => match command {
            WindowCommand::List => commands::window::list(json).await?,
            WindowCommand::Focus(WindowId { id }) => commands::window::focus(id, json).await?,
            WindowCommand::Move {
                id: WindowId { id },
                x,
                y,
            } => commands::window::move_window(id, x, y, json).await?,
            WindowCommand::Resize {
                id: WindowId { id },
                width,
                height,
            } => commands::window::resize(id, width, height, json).await?,
            WindowCommand::Close(WindowId { id }) => commands::window::close(id, json).await?,
            WindowCommand::Workspaces => commands::window::workspaces(json).await?,
        },
        Command::Type { text } => commands::input::type_text(&text, json).await?,
        Command::Key { command } => match command {
            KeyCommand::Press { key } => commands::input::key_press(&key, json).await?,
            KeyCommand::Release { key } => commands::input::key_release(&key, json).await?,
        },
        Command::Mouse { command } => match command {
            MouseCommand::Move { dx, dy } => commands::input::mouse_move(dx, dy, json).await?,
            MouseCommand::Click { button } => commands::input::mouse_click(&button, json).await?,
            MouseCommand::Scroll { dx, dy } => commands::input::mouse_scroll(dx, dy, json).await?,
        },
        Command::A11y { command } => match command {
            A11yCommand::ListApps => commands::accessibility::list_apps(json).await?,
            A11yCommand::Find {
                app,
                role,
                name,
                description,
                max_results,
            } => {
                commands::accessibility::find(&app, &role, &name, &description, max_results, json)
                    .await?
            }
            A11yCommand::Tree { app, max_depth } => {
                commands::accessibility::tree(&app, max_depth, json).await?
            }
            A11yCommand::Info { element } => {
                commands::accessibility::get_element_info(&element, json).await?
            }
            A11yCommand::Click { element, action } => {
                commands::accessibility::click(&element, &action, json).await?
            }
            A11yCommand::Focus { element } => {
                commands::accessibility::focus(&element, json).await?
            }
            A11yCommand::SetText { element, text } => {
                commands::accessibility::set_text(&element, &text, json).await?
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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
}
