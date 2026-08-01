<p align="center">
  <img src="docs/assets/img/logo.png" alt="wgaf — Wayland GNOME Automation Framework" width="360">
</p>

Script your GNOME desktop from the terminal — move windows, type into apps,
click buttons by name, drive any app's UI. If you relied on `xdotool` or
`wmctrl` before switching to Wayland, this gets that power back, built the
way GNOME actually allows it: no X11 hacks, nothing fighting the compositor,
nothing that stops working with the next GNOME release.

The aim is a desktop that is programmable the way a user operates it — by
window, by button, by name — and predictable enough to trust with the same
task twice. Everything routes through one gated, auditable service, so what
holds for a command you type holds for a script you run and, eventually, for
an AI agent acting on your behalf.

## Why

Wayland's security model correctly stops one app from spying on or
controlling another — which is exactly why `xdotool`/`wmctrl`-style tools
broke, and why most "fixes" for that are workarounds waiting to break again.

wgaf doesn't take that shortcut. Every capability goes through an explicit,
supported interface:

- **GNOME Shell Extension** — window and workspace management
- **Linux `uinput`** — keyboard and mouse synthesis
- **AT-SPI** — the same accessibility system screen readers use, for driving
  application UIs

So it keeps working instead of racing GNOME's next release.

### Compared to the X11 tools

| | `xdotool` | `wmctrl` | wgaf |
|---|---|---|---|
| Works on GNOME Wayland | No | No | **Yes** |
| Works on X11 | Yes | Yes | Not targeted |
| Uses supported GNOME APIs | No | No | **Yes** |
| Window management | Yes | Yes | Yes |
| Keyboard/mouse synthesis | Yes | No | Yes |
| Find UI elements by name/role | No | No | **Yes** |
| JSON output | No | No | **Yes** |
| Per-capability permissions | No | No | **Yes** |

## Roadmap

Checked items work today.

### Window management
- [x] List windows — id, title, application, position, size, workspace, focused and maximized state
- [x] Focus, move, resize, and close a window
- [x] List workspaces and see which window sits on which
- [ ] Minimize and maximize
- [ ] Watch windows open, close, and change focus, so a script can react

### Keyboard
- [x] Type a string of text
- [x] Press and release individual keys, modifiers and AltGr included
- [x] Every key of a 105-key keyboard, verified against a real application
- [ ] Type correctly on non-US keyboard layouts
- [ ] Refuse to type when the window you meant isn't the focused one

### Mouse
- [x] Move the pointer relative to where it is
- [x] Click any button and scroll in either direction
- [ ] Move the pointer to an absolute position, on any monitor

### Accessibility
- [x] List running applications that expose an accessible interface
- [x] Read an application's UI tree, to whatever depth you ask for
- [x] Find elements by name, role, or description
- [x] Inspect one element — role, name, description, state, children
- [x] Click an element, trigger a named action, focus it, or fill its text
- [ ] Durable element names, so a saved script survives the application restarting
- [ ] Take those names from an application's own GTK UI file, and turn one into a workflow

### Applications
- [ ] Launch an installed application by name
- [ ] Tell whether an application is already running
- [ ] Drive an application through the actions it publishes itself, where it does

### Scripting and workflows
- [x] JSON output on every command
- [x] Named, stable errors, so a script can tell what went wrong
- [ ] Run a saved workflow from a file
- [ ] Record what you do and replay it later
- [ ] Trigger a script when a window opens or a UI element changes

### AI agents
- [ ] An MCP server, so an agent drives the desktop under the permissions you set
- [ ] Agent actions recorded separately from your own

### Safety and transparency
- [x] Allow, deny, or prompt per capability — thirteen of them
- [x] A prompt arrives as a desktop notification you answer
- [x] Caps on how fast, and how much, synthetic input one command can produce
- [x] Config and policy files must be yours alone, or the daemon won't start
- [x] Every action that changes something is recorded
- [x] `wgaf status` — the extension, `/dev/uinput`, the accessibility bus, and the policy in force
- [ ] A panic stop that halts input at once and stays off until you clear it
- [ ] A readable log file, one per day, private to you
- [ ] A panel indicator showing when something is automating your desktop
- [ ] Screenshots, through the desktop's own consent flow

### Install and platform
- [x] One `make install` — binaries, systemd user service, GNOME Shell extension
- [x] Configuration found automatically, no flags needed
- [x] Shell completions for bash, zsh, fish, elvish and PowerShell, plus man pages
- [ ] `.deb` and `.rpm` packages
- [ ] Tested against more than one GNOME release
- [ ] Other Wayland compositors — KDE, Sway, Hyprland

## Use cases

- **Scripted window layouts** — snap a specific set of apps into position
  when you start a work session, instead of dragging windows around by hand
  every time.
- **Repetitive UI tasks** — fill out the same form, click through a
  multi-step dialog, or repeat a sequence of clicks and keystrokes without
  doing it by hand each time.
- **Reliable GUI automation** — drive an app by element name instead of
  screen coordinates, so scripts keep working when a window resizes, a theme
  changes, or display scaling differs.
- **Shell-scriptable desktop control** — every command supports `--json`, so
  `wgaf` drops straight into shell scripts, automation frameworks, or an AI
  agent that needs to query and drive the desktop.

See the [user guide](docs/user-guide.md) for how to use each of these, the
[CLI reference](docs/cli-reference.md) for every command's exact flags, and
the [example walkthrough](docs/example-walkthrough.md) for a complete task
done start to finish.

## How it works

Three separate mechanisms sit behind one CLI — which is why installation has
three parts:

```
                    +---------------+
                    |     wgaf      |
                    |     (CLI)     |
                    +-------+-------+
                            |
                            | D-Bus
                            v
                +-----------------------+
                |      wgaf-daemon      |
                |                       |
                |   permission check    |
                |   audit log           |
                +-----------+-----------+
                            |
        +-------------------+-------------------+
        |                   |                   |
        v                   v                   v
+---------------+   +---------------+   +---------------+
|  GNOME Shell  |   |     Linux     |   |    AT-SPI     |
|   Extension   |   |    uinput     |   | accessibility |
|   (windows)   |   |    (input)    |   |     (UI)      |
+---------------+   +---------------+   +---------------+
```

The GNOME Shell Extension is the only sanctioned way to reach Mutter's window
management, which is why it has to be installed and enabled separately.
`uinput` is a kernel interface, which is why it needs a one-time permissions
step. AT-SPI needs nothing extra — it's on by default on GNOME.

Nothing reaches those three directly. Every command that changes something
passes the daemon's permission check first and is recorded afterwards, which is
what makes [`permissions.toml`](docs/configuration.md#permissionstoml--per-capability-policy)
a real boundary rather than a suggestion — and what keeps it one no matter what
is issuing the commands.

## Get started

You need GNOME Shell on Wayland (tested against GNOME Shell 50) and a recent
Rust toolchain.

```sh
git clone https://github.com/Ranrar/wgaf.git
cd wgaf
make install
systemctl --user enable --now wgaf-daemon.service
wgaf ping        # should print: pong
```

Two one-time steps `make install` can't do for you: **log out and back in
once** so GNOME Shell loads the extension, and **grant access to
`/dev/uinput`** so wgaf can synthesize keyboard and mouse input. Both are in
the [installation guide](docs/installation.md#first-time-setup), which also
covers shell completions, man pages, uninstalling, and what to check when
something isn't working.

## Quick examples

Windows are addressed by the numeric id from `wgaf window list` — not by
application name:

```sh
wgaf window list
#    7  ws=0    240,76    800x600   org.gtk.Demo4   Assistant  [focused]

wgaf window focus 7
wgaf window move 7 100 100
wgaf window resize 7 1280 800
wgaf window close 7
```

Type and click:

```sh
wgaf type "Hello from wgaf"
wgaf mouse click left
```

Drive a UI by element name rather than coordinates. Find the element first,
then act on the reference it prints:

```sh
wgaf a11y find --app gtk4-demo --role "push button" --name Save
# push button   Save   :1.87#/org/a11y/atspi/accessible/1234

wgaf a11y click ':1.87#/org/a11y/atspi/accessible/1234'
```

Machine-readable output for scripts:

```sh
wgaf window list --json
```

## Configuration

Two TOML files in `~/.config/wgaf/`, which the daemon finds on its own:

| File | Purpose |
|---|---|
| `config.toml` | Bus name, log level, device name, input safety limits, device settle time |
| `permissions.toml` | What wgaf is allowed to do — `Allow`, `Deny`, or `Prompt` per capability |

`make install` sets both up with the right ownership and mode, and never
overwrites files you already have. Nothing is mandatory beyond the files
existing: an unlisted capability is allowed, so the policy is an opt-in
*restriction* rather than an unlock you must grant before anything works.

```toml
# ~/.config/wgaf/permissions.toml
[capabilities]
TypeText = "Deny"         # block `wgaf type` entirely
CloseWindow = "Prompt"    # ask before closing a window
```

`wgaf status` shows which files are in use and what's restricted. See
[Configuration](docs/configuration.md) for every setting, the full capability
list, and the input safety limits that bound how much synthetic input one
command can produce.

## Known issues

- Typed text and clicks go to whatever has keyboard focus at the moment they
  are sent, and can't be aimed at a particular window — so it's best not to use
  the desktop while a script is running.
- `wgaf type` assumes a US keyboard layout. On other layouts punctuation can
  come out differently, and characters that need AltGr — `@` and `{` on a
  Danish keyboard, for instance — can't be typed at all. Being fixed; see
  [typing on a non-US layout](docs/cli-reference.md#typing-on-a-non-us-layout).

## The goal

A desktop that people, scripts, and AI agents can all operate the same way —
by window, by button, by name — with one service deciding what is allowed and
keeping a record of it, whoever is asking.

The CLI is the first way in. Two more are planned, and both are clients of that
same daemon rather than a second door around it, so the permissions you set
hold for all three:

```
      you            a script          an AI agent
       |                |                   |
       v                v                   v
  +---------+     +-----------+     +----------------+
  |   CLI   |     |  Flow     |     |   MCP server   |
  |  (now)  |     |  Script   |     |   (planned)    |
  |         |     | (planned) |     |                |
  +----+----+     +-----+-----+     +--------+-------+
       |                |                    |
       +----------------+--------------------+
                        |
                        v
              +-----------------------+
              |      wgaf-daemon      |
              |   permission + audit  |
              +-----------+-----------+
                          |
      +-------------+-----+-------+---------------+
      |             |             |               |
      v             v             v               v
   windows        input      accessibility    launching
 (Shell ext.)    (uinput)      (AT-SPI)       (planned)
```

*Flow Script* is a planned plain-text format for describing a task step by
step — launch this, wait for that window, click the button named Save — that
you can read before running it and keep in version control. Shell scripts
around `--json` are today's answer and will keep working.

## Documentation

| | |
|---|---|
| [Installation](docs/installation.md) | Install, first-time setup, completions, uninstall, and what to check when something isn't working |
| [Configuration](docs/configuration.md) | Every setting, the permission policy, and the input safety limits |
| [User guide](docs/user-guide.md) | How to actually use each capability |
| [CLI reference](docs/cli-reference.md) | Every command's exact flags and error messages |
| [Example walkthrough](docs/example-walkthrough.md) | One complete task, start to finish |

## License

MIT — see [LICENSE](LICENSE).
