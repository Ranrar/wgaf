# Installation

## Requirements

- GNOME Shell on Wayland (built and tested against GNOME Shell 50)
- A recent Rust toolchain (the workspace uses the 2024 edition)
- The `libxkbcommon` development package, which wgaf builds against to read
  your keyboard layout. The library itself is already on every Wayland desktop;
  it is the headers the build needs. On Debian and Ubuntu that is
  `libxkbcommon-dev`, on Fedora `libxkbcommon-devel`, on Arch `libxkbcommon`.
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

## First-time setup

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

Set up the two configuration files next, if `make install` didn't do it for
you — see [Configuration](configuration.md).

One thing to know before you automate anything: **Escape is wgaf's emergency
stop** while the extension is enabled. Press it and all input automation stops
immediately; `wgaf release` allows it again. See the
[user guide](user-guide.md#emergency-stop--pulling-the-handbrake).

## Shell completions

```sh
wgaf completions bash > /etc/bash_completion.d/wgaf
wgaf completions zsh > "${fpath[1]}/_wgaf"
```

(`fish` also supported — run `wgaf completions --help` for the full list of
targets.) Man pages are optional — `make man` generates and installs them.

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

**The first `wgaf type` or `wgaf mouse click` after starting the daemon does
nothing**, and running it again works — the desktop had not finished picking
up wgaf's virtual input device yet, so the keystrokes went nowhere. wgaf waits
300 ms for this on the first command; raise `input_device_settle_ms` in
`config.toml` if your machine needs longer. You only pay the wait once, on the
first command after the daemon starts.

**A command was refused** — that's the permission policy, not a fault. See
[when a command gets denied](user-guide.md#when-a-command-gets-denied).

---

[Configuration](configuration.md) · [User guide](user-guide.md) ·
[CLI reference](cli-reference.md) · [Example walkthrough](example-walkthrough.md)
