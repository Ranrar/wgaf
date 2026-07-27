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
broke, and why most "fixes" for that are actually workarounds waiting to
break again. wgaf doesn't take that shortcut: every capability goes through
an explicit, supported API — a GNOME Shell Extension for window management,
Linux `uinput` for input synthesis, and AT-SPI (the same system screen
readers use) for accessibility automation — so it keeps working instead of
racing GNOME's next release.

## What it can do

- **Window management** — list, focus, move, resize, and close windows
  (`wgaf window ...`)
- **Keyboard & mouse** — type text, press keys, move/click/scroll the mouse
  (`wgaf type` / `wgaf key` / `wgaf mouse ...`)
- **Accessibility automation** — find and act on UI elements by name/role
  instead of screen coordinates (`wgaf a11y ...`)
- **Per-capability permissions** — allow, deny, or prompt-before-allowing
  any of the above
- **Shell completions** (bash/zsh/fish/elvish/powershell) and man pages

## Use cases

- **Scripted window layouts** — snap a specific set of apps into position
  when you start a work session, instead of dragging windows around by hand
  every time.
- **Repetitive UI tasks** — fill out the same form, click through a
  multi-step dialog, or repeat a sequence of clicks/keystrokes without doing
  it by hand each time.
- **Reliable GUI automation** — drive an app's UI by button/element name
  instead of screen coordinates, so scripts keep working when a window
  resizes or a theme changes.
- **Shell-scriptable desktop control** — every command supports `--json`
  output, so `wgaf` drops straight into shell scripts or other tooling that
  needs to query or drive the desktop.

See the [user guide](docs/user-guide.md) for how to actually use each of
these, the [CLI reference](docs/cli-reference.md) for every command's exact
flags, and the [example walkthrough](docs/example-walkthrough.md) for a
complete task done start to finish.

## Requirements

- A recent Rust toolchain (the workspace uses the 2024 edition)
- GNOME Shell on Wayland (built and tested against GNOME Shell 50)

## Install

```sh
git clone https://github.com/Ranrar/wgaf.git
cd wgaf
make install
```

This builds and installs `wgaf-daemon`/`wgaf` (via `cargo install`, default
`~/.cargo/bin`), installs a systemd user unit, and installs + enables the
GNOME Shell Extension that window management needs.

Two one-time steps `make install` can't do for you:

**1. GNOME Shell needs to actually load the extension.** Wayland has no
in-session Shell restart — log out and back in once after the first
install.

**2. Keyboard/mouse automation needs `/dev/uinput` access:**

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

## Shell completions

```sh
wgaf completions bash > /etc/bash_completion.d/wgaf
wgaf completions zsh > "${fpath[1]}/_wgaf"
```

(`fish`/`elvish`/`powershell` also supported.) Man pages are optional —
`make man` generates and installs them.

## Configuration

`wgaf-daemon` reads two optional TOML files from `$XDG_CONFIG_HOME/wgaf/`
(defaults to `~/.config/wgaf/`): `config.toml` (daemon settings) and its
sibling `permissions.toml` (per-capability policy). Both are entirely
optional — with neither present, the daemon runs with sane defaults and
every capability allowed.

### `permissions.toml` — per-capability policy

Thirteen capabilities exist, one per gated (mutating) command — read-only
commands (`window list`, `a11y find`, etc.) can't be gated at all:

| Interface | Capabilities |
|---|---|
| `org.wgaf.Windows1` | `FocusWindow`, `MoveWindow`, `ResizeWindow`, `CloseWindow` |
| `org.wgaf.Input1` | `TypeText`, `KeyPress`, `KeyRelease`, `MouseMove`, `MouseClick`, `MouseScroll` |
| `org.wgaf.Accessibility1` | `InvokeAction`, `SetText`, `FocusElement` |

```toml
# ~/.config/wgaf/permissions.toml
[capabilities]
TypeText = "Deny"        # block `wgaf type` entirely
CloseWindow = "Prompt"    # ask via a GNOME notification (Allow/Deny) before closing a window
```

Any capability not listed defaults to `Allow` — this is a personal
automation tool, so permissions are an opt-in *restriction* you configure,
never an opt-in *unlock* you must grant before anything works.

## Uninstall

```sh
make uninstall
```

## Troubleshooting

**"GNOME Shell Extension bridge unavailable"** even though the extension is
enabled — check for a duplicate `wgaf-daemon` process holding the D-Bus
name (a stale instance wins the name race and makes a freshly started one
silently useless):

```sh
pgrep -af wgaf-daemon
pkill -f wgaf-daemon
```

More: [user guide](docs/user-guide.md) · [CLI reference](docs/cli-reference.md) · [example walkthrough](docs/example-walkthrough.md)

## License

MIT — see [LICENSE](LICENSE).
