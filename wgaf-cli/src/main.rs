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
}
