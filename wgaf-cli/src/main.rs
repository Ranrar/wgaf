mod commands;
mod error;
mod output;

use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Args, CommandFactory, Parser, Subcommand};
use error::{CliResult, Verdict};

/// Colours for `--help` and for clap's own error messages.
///
/// clap 4.4 onward ships a deliberately plain default — bold and underline, no
/// colour — so this is opt-in. The palette is cargo's, near enough: green for
/// section headings, cyan for the things you type. Following the tool every
/// Rust developer already has open costs nothing and means `wgaf --help` does
/// not feel like a different kind of program.
///
/// **Nothing here needs a "should I use colour?" check.** clap writes help
/// through `anstream`, which strips styling when the destination is not a
/// terminal and honours `NO_COLOR`, `CLICOLOR_FORCE` and friends. Piping
/// `wgaf --help` into a file or a pager gets clean text, which is what makes
/// this safe to turn on at all.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
    .valid(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD));

/// The banner shown by `wgaf --help`.
///
/// # Why `--help` and not `-h`
///
/// It is set as the *long* banner, so `-h` stays a dense reference you can scan
/// in a second and `--help` is the one that introduces the tool. Someone typing
/// `-h` for the tenth time today wants the command list at the top of the
/// screen, not four lines of art above it.
///
/// # No colour codes in here
///
/// The block characters carry it on their own, and embedding ANSI would put
/// escape sequences into the generated man page, which renders this same text.
/// Colour is applied to the help *structure* by [`HELP_STYLES`], where anstream
/// can strip it when the output is not a terminal.
const LOGO: &str = concat!(
    "\n",
    " _____                                                          _____ \n",
    "( ___ )                                                        ( ___ )\n",
    " |   |~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~|   | \n",
    " |   |                                                          |   | \n",
    " |   |       wayland GNOME automation framework    ██████       |   | \n",
    " |   |                                            ███░░███      |   | \n",
    " |   |       █████ ███ █████  ███████  ██████    ░███ ░░░       |   | \n",
    " |   |      ░░███ ░███░░███  ███░░███ ░░░░░███  ███████         |   | \n",
    " |   |       ░███ ░███ ░███ ░███ ░███  ███████ ░░░███░          |   | \n",
    " |   |       ░░███████████  ░███ ░███ ███░░███   ░███           |   | \n",
    " |   |        ░░████░████   ░░███████░░████████  █████          |   | \n",
    " |   |         ░░░░ ░░░░     ░░░░░███ ░░░░░░░░  ░░░░░           |   | \n",
    " |   |                       ███ ░███                           |   | \n",
    " |   |                      ░░██████                            |   | \n",
    " |   |                       ░░░░░░                             |   | \n",
    " |   |                                                          |   | \n",
    " |   | https://github.com/Ranrar/wgaf                           |   | \n",
    " |___|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~|___| \n",
    "(_____)                                                        (_____)\n",
    "\n",
    // Kept to the box's own 70 columns so the text sits inside the frame
    // rather than overhanging it. Check the width if you reword these.
    "Automate GNOME on Wayland through its own interfaces, not around them.\n",
    "\n",
    "Windows and workspaces through GNOME Shell, keyboard and mouse through\n",
    "the kernel, buttons and text fields by name through accessibility.\n",
);

// A doc comment on this struct becomes the tool's `--help` banner, so keep
// anything that is not for a user reading `wgaf --help` down here in a plain
// comment. clap takes the first paragraph as the short description and the
// whole thing as the long one, which is how a maintenance note ends up printed
// above the command list.
//
// `version` takes the crate version, so `wgaf --version` reports the version of
// *this command* — not necessarily of the daemon answering it, since a
// long-running daemon can be older than a freshly built CLI. `wgaf status`
// reports the daemon's own, and is the one to quote when the two disagree.

// The one-line doc comment below is the whole of what a user should see here,
// and it stays even though LOGO now carries the longer description, because it
// has three jobs beyond `--help`:
//
//   - `wgaf -h`. The banner is on `--help` only, so without this the terse
//     help opens straight into `Usage:` and never says what the tool is.
//   - The man page's NAME line. `clap_mangen` builds `wgaf - <about>` from it,
//     and that line is what `whatis` and `man -k` index. With no about it
//     renders as a bare `wgaf` and the tool stops being findable by
//     description.
//   - Shell completions, which show it beside the command name.
//
// Deliberately shorter than the logo's wording rather than identical to it:
// `--help` prints the banner and then this, so a verbatim repeat would read as
// a mistake.

#[derive(Parser)]
#[command(
    name = "wgaf",
    version,
    styles = HELP_STYLES,
    before_long_help = LOGO,
    // Keep the description's own line breaks instead of re-flowing it into one
    // paragraph. Without this clap joins consecutive doc-comment lines with
    // spaces and wraps to the terminal width, so a deliberate three-line
    // description arrives as one long sentence at whatever width the reader's
    // window happens to be.
    verbatim_doc_comment,
    // `-h` stays terse and `--help` gives the detail, which clap does on its
    // own for every command whose doc comment has a second paragraph. Do NOT
    // reach for `args_conflicts_with_subcommands` to sharpen the usage line:
    // it makes `--json` conflict with every subcommand, which breaks
    // `wgaf --json window list` — a documented, tested form.
    after_help = "Run `wgaf help <command>` for detail on any command, or see the \
                  full reference at docs/cli-reference.md.",
)]
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

    /// Check that everything is set up correctly.
    ///
    /// Reports whether every subsystem is working, and what permission policy
    /// the daemon is enforcing.
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

    /// List, focus, move, resize and close windows.
    ///
    /// Also sends a window to another workspace. Backed by the daemon's
    /// `org.wgaf.Windows1` D-Bus interface, and needs the GNOME Shell
    /// extension.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },

    /// List workspaces, and switch, add, remove or reorder them.
    ///
    /// Backed by the daemon's `org.wgaf.Windows1` D-Bus interface, and needs
    /// the GNOME Shell extension.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },

    /// List the monitors making up your desktop.
    ///
    /// Unlike every other command here, this one does not need the wgaf GNOME
    /// Shell extension: the layout is read from Mutter's own display
    /// configuration.
    Monitor {
        #[command(subcommand)]
        command: MonitorCommand,
    },

    /// Type a string of text.
    ///
    /// Uses whatever keyboard layout your desktop is set to, so accented and
    /// AltGr characters work. A character the layout cannot produce is
    /// reported rather than silently dropped.
    ///
    /// Goes to whatever currently has keyboard focus unless you pass
    /// `--window`.
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

    /// Press and release individual keys, and key combinations.
    ///
    /// Keys are named the way the kernel names them (`a`, `enter`,
    /// `leftshift`, `up`, `f5`, `altgr`, ...), and are physical keys rather
    /// than characters — so a combination means the same on every layout.
    ///
    /// There is no shift awareness here: `wgaf type` is what turns text into
    /// keystrokes. For a capital `A` by hand you would press `leftshift`, press
    /// `a`, then release both — or just use `wgaf key combo`.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },

    /// Move, click and scroll the mouse.
    ///
    /// Prefer `wgaf a11y` where an element can be found by name or role:
    /// clicking a named button keeps working when a window moves or a theme
    /// changes, and a coordinate does not.
    Mouse {
        #[command(subcommand)]
        command: MouseCommand,
    },

    /// Find and operate UI elements by name, role or description.
    ///
    /// Lists accessible applications, reads their element trees, and clicks or
    /// fills what it finds — through the same accessibility system a screen
    /// reader uses.
    ///
    /// Preferred over coordinate-based automation whenever an element can be
    /// found this way: a named button keeps working when the window moves.
    A11y {
        #[command(subcommand)]
        command: A11yCommand,
    },

    /// Print a shell completion script.
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

    /// Replace an element's text content.
    ///
    /// Requires the element to implement AT-SPI's `EditableText` interface —
    /// most text fields do, and read-only ones report that they do not.
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

    /// Minimize a window.
    ///
    /// Note that typing at a minimized window is refused — `wgaf type --window`
    /// will tell you to restore it first, rather than sending the keystrokes
    /// wherever focus happens to be.
    Minimize(WindowId),

    /// Restore a minimized window.
    ///
    /// This does not focus it. Run `wgaf window focus` afterwards if that is
    /// what you want.
    Unminimize(WindowId),

    /// Maximize a window.
    ///
    /// Fills the work area — the screen minus the top bar and any dock. Use
    /// `wgaf window fullscreen` to cover those too.
    ///
    /// Always both directions. GNOME can maximize a window sideways only, from
    /// its own keyboard shortcuts, but it offers no way for another program to
    /// ask for that.
    Maximize(WindowId),

    /// Unmaximize a window, returning it to its previous size.
    Unmaximize(WindowId),

    /// Make a window fullscreen.
    ///
    /// Covers the top bar and any dock, which maximizing does not.
    Fullscreen(WindowId),

    /// Take a window out of fullscreen.
    Unfullscreen(WindowId),

    /// Keep a window above all others.
    ///
    /// Outranks `wgaf window raise` entirely — a raised ordinary window still
    /// sits below one that is kept above.
    Above(WindowId),

    /// Stop keeping a window above all others.
    Unabove(WindowId),

    /// Show a window on every workspace.
    Stick(WindowId),

    /// Show a window on only its own workspace again.
    ///
    /// Some windows are on every workspace for reasons of the compositor's own
    /// — those are refused, with the reason, rather than silently doing
    /// nothing.
    Unstick(WindowId),

    /// Raise a window to the top of the stack.
    ///
    /// Within its layer: this cannot lift a window past one that is kept above.
    /// Raising does not move keyboard focus, though focusing does raise.
    Raise(WindowId),

    /// Lower a window to the bottom of the stack.
    Lower(WindowId),
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

    /// Show how the workspaces are arranged.
    ///
    /// How many there are, which is active, the grid GNOME lays them out in,
    /// and whether GNOME is managing their number itself.
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

            // Each pair is one daemon method taking a boolean, so the two verbs
            // differ only in what they pass — the CLI is where "unminimize" is
            // a word and the wire is where it is `false`.
            WindowCommand::Minimize(WindowId { id }) => {
                commands::window::set_minimized(bus_name, id, true, json).await?
            }
            WindowCommand::Unminimize(WindowId { id }) => {
                commands::window::set_minimized(bus_name, id, false, json).await?
            }
            WindowCommand::Maximize(WindowId { id }) => {
                commands::window::set_maximized(bus_name, id, true, json).await?
            }
            WindowCommand::Unmaximize(WindowId { id }) => {
                commands::window::set_maximized(bus_name, id, false, json).await?
            }
            WindowCommand::Fullscreen(WindowId { id }) => {
                commands::window::set_fullscreen(bus_name, id, true, json).await?
            }
            WindowCommand::Unfullscreen(WindowId { id }) => {
                commands::window::set_fullscreen(bus_name, id, false, json).await?
            }
            WindowCommand::Above(WindowId { id }) => {
                commands::window::set_above(bus_name, id, true, json).await?
            }
            WindowCommand::Unabove(WindowId { id }) => {
                commands::window::set_above(bus_name, id, false, json).await?
            }
            WindowCommand::Stick(WindowId { id }) => {
                commands::window::set_on_all_workspaces(bus_name, id, true, json).await?
            }
            WindowCommand::Unstick(WindowId { id }) => {
                commands::window::set_on_all_workspaces(bus_name, id, false, json).await?
            }
            WindowCommand::Raise(WindowId { id }) => {
                commands::window::restack(bus_name, id, wgaf_common::Stacking::Raise, json).await?
            }
            WindowCommand::Lower(WindowId { id }) => {
                commands::window::restack(bus_name, id, wgaf_common::Stacking::Lower, json).await?
            }
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

    /// All twelve window-state verbs parse, and each needs an id.
    ///
    /// Pinned as a set rather than one test per verb because the risk they share
    /// is a naming one: these are six pairs, and a pair where only one half
    /// exists is a gap a user meets rather than a compiler does.
    #[test]
    fn parses_every_window_state_verb_and_requires_an_id() {
        for verb in [
            "minimize",
            "unminimize",
            "maximize",
            "unmaximize",
            "fullscreen",
            "unfullscreen",
            "above",
            "unabove",
            "stick",
            "unstick",
            "raise",
            "lower",
        ] {
            Cli::try_parse_from(["wgaf", "window", verb, "42"])
                .unwrap_or_else(|e| panic!("`wgaf window {verb} 42` should parse: {e}"));
            assert!(
                Cli::try_parse_from(["wgaf", "window", verb]).is_err(),
                "`wgaf window {verb}` must require an id"
            );
        }
    }

    /// `maximize` takes no direction flag, and passing one is a parse error
    /// rather than a silently ignored argument.
    ///
    /// `--direction horizontal|vertical|both` existed briefly and was removed:
    /// Mutter 18's `maximize()` always acts on both axes and overwrites the
    /// flags that appear to select one — measured inside the Shell, see
    /// `setWindowMaximized` in `extension/windows.js`.
    ///
    /// **The flag must not come back without a route that works.** An argument
    /// accepted and then ignored is the exact failure this project keeps
    /// naming, and it is what shipped for a few hours before a manual run
    /// caught it. This test is what makes bringing it back a deliberate act.
    #[test]
    fn maximize_takes_no_direction_flag() {
        for verb in ["maximize", "unmaximize"] {
            Cli::try_parse_from(["wgaf", "window", verb, "42"])
                .unwrap_or_else(|e| panic!("`wgaf window {verb} 42` should parse: {e}"));

            for direction in ["both", "horizontal", "vertical"] {
                assert!(
                    Cli::try_parse_from(["wgaf", "window", verb, "42", "--direction", direction])
                        .is_err(),
                    "`wgaf window {verb} --direction {direction}` must be rejected while \
                     Mutter offers no way to honour it"
                );
            }
        }
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

    /// The top-level description is one line written for a user.
    ///
    /// A `///` comment on `Cli` is not documentation — clap prints it in
    /// `wgaf --help`, above the command list. That is easy to forget while
    /// editing, and the result is a maintenance note shown to everyone who runs
    /// the tool: it has happened, with a three-bullet explanation of why the
    /// `about` exists appearing between the banner and `Usage:`.
    ///
    /// Notes for maintainers belong in the plain `//` block above the struct,
    /// which the compiler keeps and clap never sees.
    #[test]
    fn the_top_level_description_is_one_line_written_for_a_user() {
        let command = Cli::command();

        for (which, text) in [
            ("about", command.get_about().map(ToString::to_string)),
            (
                "long_about",
                command.get_long_about().map(ToString::to_string),
            ),
        ] {
            let Some(text) = text else { continue };

            assert!(
                !text.trim_end().contains('\n'),
                "`{which}` is {} lines. Everything here is printed by \
                 `wgaf --help` — move the explanation to the `//` comment above \
                 `struct Cli`:\n{text}",
                text.trim_end().lines().count()
            );
            // Markdown only renders in rustdoc. Its presence means the text was
            // written as documentation rather than as help output.
            for marker in ["**", "[`"] {
                assert!(
                    !text.contains(marker),
                    "`{which}` contains `{marker}`, which only makes sense in \
                     rustdoc — this string is printed verbatim to a terminal:\n{text}"
                );
            }
        }
    }

    /// The global flags work on **either side** of the subcommand.
    ///
    /// `docs/cli-reference.md` promises this in as many words, and it is the
    /// form that reads naturally when the flag applies to the whole run
    /// (`wgaf --json window list`) rather than to the command
    /// (`wgaf window list --json`).
    ///
    /// Written after breaking it: `args_conflicts_with_subcommands` tidies the
    /// usage line and makes every global flag conflict with every subcommand,
    /// so `wgaf --json monitor list` became "the subcommand 'monitor' cannot be
    /// used with '--json'". Nothing in the suite noticed, because every test
    /// here happened to put its flags after the subcommand.
    #[test]
    fn global_flags_work_on_either_side_of_the_subcommand() {
        for args in [
            vec!["wgaf", "--json", "window", "list"],
            vec!["wgaf", "window", "list", "--json"],
            vec!["wgaf", "--bus-name", "org.example.Test", "monitor", "list"],
            vec!["wgaf", "monitor", "list", "--bus-name", "org.example.Test"],
            vec!["wgaf", "--json", "workspace", "switch", "1"],
            vec!["wgaf", "workspace", "switch", "1", "--json"],
        ] {
            let rendered = args.join(" ");
            let cli = Cli::try_parse_from(&args)
                .unwrap_or_else(|e| panic!("`{rendered}` must parse: {e}"));
            assert!(
                cli.json || cli.bus_name.is_some(),
                "`{rendered}` lost its flag"
            );
        }
    }

    /// `--version` and `-V` both report the crate version.
    ///
    /// Every other invocation of `wgaf` requires a subcommand, so this is the
    /// one flag that has to work on its own — a `--version` that errored with
    /// "no subcommand given" would be worse than not having one.
    #[test]
    fn version_is_reported_without_a_subcommand() {
        for flag in ["--version", "-V"] {
            let Err(err) = Cli::try_parse_from(["wgaf", flag]) else {
                panic!("`{flag}` must print the version, not parse as a command");
            };
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::DisplayVersion,
                "`{flag}` must print the version, not fail"
            );
            assert!(
                err.to_string().contains(env!("CARGO_PKG_VERSION")),
                "`{flag}` must report the crate version, got: {err}"
            );
        }
    }

    /// **`--version` deliberately ignores `--json`.**
    ///
    /// clap handles the flag and exits before any of this crate's code runs, so
    /// the output is plain text whatever else was passed. That is left alone
    /// rather than reimplemented: `--version` printing `name x.y.z` is a
    /// convention every other command-line tool shares, and a script parsing it
    /// expects that shape.
    ///
    /// A script that wants a version as JSON wants the *daemon's*, which is
    /// what it is actually talking to — `wgaf status --json` reports it as
    /// `daemon_version`, and the two can legitimately differ when a
    /// long-running daemon is older than a freshly built CLI.
    ///
    /// Pinned as a test so the behaviour is a decision on the record rather
    /// than something noticed later and "fixed" into an inconsistency.
    #[test]
    fn version_ignores_json_because_that_is_the_convention() {
        let Err(err) = Cli::try_parse_from(["wgaf", "--json", "--version"]) else {
            panic!("--version must still short-circuit when --json is present");
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);

        let rendered = err.to_string();
        assert!(
            !rendered.trim_start().starts_with('{'),
            "--version stays plain text under --json, got: {rendered}"
        );
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
