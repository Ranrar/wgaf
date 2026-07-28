# wgaf

**W**ayland **G**NOME **A**utomation **F**ramework

Script your GNOME desktop from the terminal — move windows, type into apps,
click buttons by name, drive any app's UI. If you relied on `xdotool` or
`wmctrl` before switching to Wayland, this gets that power back, built the
way GNOME actually allows it: no X11 hacks, nothing fighting the compositor,
nothing that stops working with the next GNOME release.

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

## What it can do

| Capability | Description | Examples |
|---|---|---|
| Window management | List, focus, move, resize, and close windows via the GNOME Shell integration. | `wgaf window list`<br>`wgaf window focus 7`<br>`wgaf window move 7 100 100` |
| Keyboard automation | Type text and send individual key events through Linux `uinput`. | `wgaf type "hello"`<br>`wgaf key press leftshift` |
| Mouse automation | Move the pointer, click, and scroll. | `wgaf mouse move 100 -50`<br>`wgaf mouse click left`<br>`wgaf mouse scroll 0 -3` |
| Accessibility automation | Find UI elements by name or role and act on them, instead of using screen coordinates. | `wgaf a11y find --app gtk4-demo --name Save`<br>`wgaf a11y click <element>` |
| Permission control | Allow, deny, or prompt-before-allowing each mutating capability. | `permissions.toml` |
| Script integration | JSON output on every command, for shell scripts and other tooling. | `wgaf window list --json` |
| Developer tooling | Shell completions and man pages. | `wgaf completions bash`<br>`make man` |

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
                    +---------------+
                    |  wgaf-daemon  |
                    +-------+-------+
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

## Requirements

- GNOME Shell on Wayland (built and tested against GNOME Shell 50)
- A recent Rust toolchain (the workspace uses the 2024 edition)
- AT-SPI enabled — the default on GNOME
- `systemd` user services, only if you want the daemon to run as a service

## Install

```sh
git clone https://github.com/Ranrar/wgaf.git
cd wgaf
make install
```

This builds and installs `wgaf` and `wgaf-daemon` (via `cargo install`,
default `~/.cargo/bin`), installs the systemd user unit, and installs and
enables the GNOME Shell Extension that window management needs.

### First-time setup

Two one-time steps `make install` can't do for you.

**1. Let GNOME Shell load the extension.** Wayland has no in-session Shell
restart, so log out and back in once after the first install.

**2. Grant access to `/dev/uinput`** for keyboard and mouse automation:

```sh
echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-wgaf-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
sudo usermod -aG input $USER
```

Then log out and back in again for the new group membership to apply.

## Run it

```sh
systemctl --user enable --now wgaf-daemon.service
```

or just `wgaf-daemon &` if you'd rather not run it as a service.

Then:

```sh
wgaf ping
```

should print `pong`.

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

## Shell completions

```sh
wgaf completions bash > /etc/bash_completion.d/wgaf
wgaf completions zsh > "${fpath[1]}/_wgaf"
```

(`fish` also supported — run `wgaf completions --help` for the full list of
targets.) Man pages are optional — `make man` generates and installs them.

## Configuration

`wgaf-daemon` reads two optional TOML files from `$XDG_CONFIG_HOME/wgaf/`
(defaults to `~/.config/wgaf/`): `config.toml` (daemon settings) and its
sibling `permissions.toml` (per-capability policy). Both are entirely
optional — with neither present, the daemon runs with sane defaults and
every capability allowed.

### `permissions.toml` — per-capability policy

Thirteen capabilities exist, one per gated (mutating) command. Read-only
commands (`window list`, `a11y find`, etc.) can't be gated at all:

| Interface | Capabilities |
|---|---|
| `org.wgaf.Windows1` | `FocusWindow`, `MoveWindow`, `ResizeWindow`, `CloseWindow` |
| `org.wgaf.Input1` | `TypeText`, `KeyPress`, `KeyRelease`, `MouseMove`, `MouseClick`, `MouseScroll` |
| `org.wgaf.Accessibility1` | `InvokeAction`, `SetText`, `FocusElement` |

Each can be `Allow`, `Deny`, or `Prompt`:

```toml
# ~/.config/wgaf/permissions.toml
[capabilities]
TypeText = "Deny"         # block `wgaf type` entirely
CloseWindow = "Prompt"    # ask via a GNOME notification (Allow/Deny) before closing a window
```

With the above, `wgaf type "secret"` is refused outright, and
`wgaf window close 7` raises a desktop notification and waits for your
answer.

Any capability not listed defaults to `Allow` — this is a personal
automation tool, so permissions are an opt-in *restriction* you configure,
never an opt-in *unlock* you must grant before anything works.

## Uninstall

```sh
make uninstall
```

Any udev rule or `input` group membership you added by hand is left in
place — `make uninstall` never touches those.

## Troubleshooting

**Start with `wgaf status`.** It checks the GNOME Shell extension bridge,
`/dev/uinput` access, and the accessibility bus in one go, and prints what to
fix for any that aren't working — usually faster than guessing which of the
sections below applies. It exits non-zero if anything is unavailable, and
`wgaf status --json` is the most useful thing to attach to a bug report.

**"GNOME Shell Extension bridge unavailable"** even though the extension is
enabled — check for a duplicate `wgaf-daemon` process holding the D-Bus
name. A stale instance wins the name race and makes a freshly started one
silently useless:

```sh
pgrep -af wgaf-daemon
pkill -f wgaf-daemon
systemctl --user restart wgaf-daemon.service
```

**"input device unavailable"** — `/dev/uinput` isn't accessible. Re-check
step 2 of [first-time setup](#first-time-setup), and confirm the group
membership actually applied (`id -nG | grep input`); it only takes effect
after a full log out and back in.

## Documentation

[User guide](docs/user-guide.md) · [CLI reference](docs/cli-reference.md) ·
[Example walkthrough](docs/example-walkthrough.md)

## License

MIT — see [LICENSE](LICENSE).
