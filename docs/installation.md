# Installation

## System requirements

| | Requirement | Why | If it is missing |
|---|---|---|---|
| **GNOME Shell** | **50** | The extension is built against Mutter 18's API and declares Shell 50 only | GNOME refuses to load the extension. Window, workspace and monitor commands are unavailable; everything else works |
| **Display server** | **Wayland** | wgaf exists to automate Wayland, where the X11 tools cannot work | Nothing works. wgaf does not target X11 |
| **systemd** | user services | Runs the daemon with your session | Only if you want it as a service. `wgaf-daemon &` works without it |
| **`libxkbcommon`** | the library | Reads your keyboard layout so typing produces the characters you asked for | The daemon will not start. Already present on every Wayland desktop |
| **AT-SPI** | enabled | Finding and operating buttons and text fields by name | `wgaf a11y` commands only. On by default on GNOME |
| **`/dev/uinput`** | writable by you | Synthesizing keyboard and mouse input | `wgaf type`, `key` and `mouse` are unavailable. See [first-time setup](#first-time-setup) |

Building from source additionally needs a recent **Rust** toolchain (the
workspace uses the 2024 edition) and the **`libxkbcommon` development
headers** — `libxkbcommon-dev` on Debian and Ubuntu, `libxkbcommon-devel` on
Fedora and openSUSE, `libxkbcommon` on Arch. The packages need neither.

Check what you have:

```sh
gnome-shell --version           # want: 50.x
echo "$XDG_SESSION_TYPE"        # want: wayland
```

**The GNOME Shell requirement is the strict one.** wgaf is split in two: window
and workspace management goes through a GNOME Shell extension, and everything
else — typing, clicking, accessibility — does not. On a GNOME older than 50 the
extension will not load, so you get a working half rather than a broken whole.
`wgaf status` says plainly which half you have.

## Tested on

Every requirement above, per distribution, against the package it would use.
**Windows** is the window, workspace and monitor half — the part that needs the
extension. **Input** is typing, clicking and accessibility, which does not.

| Distribution | Package | GNOME Shell | Wayland | systemd | Windows | Input | Verified |
|---|---|---|---|---|---|---|---|
| **Ubuntu 26.04 LTS** | `.deb` | **50.1** ✅ | ✅ | ✅ | ✅ | ✅ | **Yes — end to end** |
| Ubuntu 25.04 | `.deb` | 48 ❌ | ✅ | ✅ | ❌ | ✅ | No |
| Debian 13 | `.deb` | 48 ❌ | ✅ | ✅ | ❌ | ✅ | No |
| Fedora 44 | `.rpm` | 50 ✅ | ✅ | ✅ | ✅ | ✅ | No |
| Fedora 43 | `.rpm` | 49 ❌ | ✅ | ✅ | ❌ | ✅ | No |
| RHEL 10, CentOS 10 | `.rpm` | 47 ❌ | ✅ | ✅ | ❌ | ✅ | No |
| openSUSE Tumbleweed | `.rpm` | 48–50 ⚠️ | ✅ | ✅ | ⚠️ | ✅ | No |
| Arch, Manjaro, EndeavourOS | `-arch.tar.gz` | 49–50 ⚠️ | ✅ | ✅ | ⚠️ | ✅ | No |
| NixOS | `.tar.gz` | varies ⚠️ | ✅ | ✅ | ⚠️ | ✅ | No |
| Gentoo | `.tar.gz` | varies ⚠️ | ✅ | ⚠️ | ⚠️ | ✅ | No |

✅ meets the requirement · ❌ does not · ⚠️ depends on your install

**Only Ubuntu 26.04 LTS is verified**, and only because it is the machine wgaf
is developed on — the full desktop suite runs there. Everything else in this
table is derived from what each distribution ships, not from installing
anything: the packages are assembled from one staged tree, and the `.deb` and
`.rpm` have been unpacked and inspected, but nobody has installed them on the
distribution they target.

Two caveats the table compresses:

- **Rolling releases move.** openSUSE Tumbleweed and Arch are marked ⚠️ because
  the answer depends on which GNOME they shipped this week. Run
  `gnome-shell --version` — 50 or newer and the window half works.
- **Gentoo can be built without systemd.** wgaf only needs it for the user
  service; on OpenRC, start the daemon yourself with `wgaf-daemon &` and
  everything else is unaffected.

If you try one, [say how it went](https://github.com/Ranrar/wgaf/issues) —
include your distribution, `gnome-shell --version`, and the output of
`wgaf status`. This table is only as good as the reports behind it, and right
now there is one.

## Install from a package

Download the file for your distribution from the
[releases page](https://github.com/Ranrar/wgaf/releases), or build them
yourself with `bash scripts/package.sh`.

### Debian, Ubuntu, Mint, Pop!_OS

```sh
sudo apt install ./wgaf_*_amd64.deb
```

Use `apt`, not `dpkg -i` — `apt` resolves the dependencies, `dpkg` leaves them
to you.

### Fedora, RHEL, CentOS

```sh
sudo dnf install ./wgaf-*.x86_64.rpm
```

### openSUSE

```sh
sudo zypper install --allow-unsigned-rpm ./wgaf-*.x86_64.rpm
```

`--allow-unsigned-rpm` is needed because these packages are not signed. zypper
refuses unsigned packages where dnf only warns; that is the only difference
from Fedora.

### Arch, Manjaro, EndeavourOS

```sh
tar -xzf wgaf-*-arch.tar.gz
cd wgaf-*-arch
makepkg -si
```

The archive holds the `PKGBUILD` and its install scriptlet — a few kilobytes.
`makepkg` downloads the binaries from the same release and checks them against
the hash in the `PKGBUILD`, so there is nothing else to fetch by hand.

Do not run `makepkg` as root; it asks for a password when it needs one.

### Anything else, from the tarball

```sh
tar -tzf wgaf-*-x86_64.tar.gz          # see what it contains first
sudo tar -C / --no-same-owner -xzf wgaf-*-x86_64.tar.gz
```

The tarball has no install step, so two things the packages do are yours:

```sh
# the configuration files
mkdir -p ~/.config/wgaf && chmod 700 ~/.config/wgaf
cp -n /usr/share/doc/wgaf/config.toml ~/.config/wgaf/
cp -n /usr/share/doc/wgaf/permissions.toml ~/.config/wgaf/
chmod 600 ~/.config/wgaf/config.toml ~/.config/wgaf/permissions.toml

# the udev rule, so wgaf can type and click
sudo udevadm control --reload-rules && sudo udevadm trigger
```

`cp -n` so a later tarball never overwrites settings you have edited.

### What a package installs

| Path | What |
|---|---|
| `/usr/bin/wgaf`, `/usr/bin/wgaf-daemon` | The command and the background service |
| `/usr/share/gnome-shell/extensions/wgaf@wgaf.dev/` | The GNOME Shell extension |
| `/usr/lib/systemd/user/wgaf-daemon.service` | Starts the daemon with your session |
| `/usr/lib/udev/rules.d/99-wgaf-uinput.rules` | Lets the `input` group use `/dev/uinput` |
| `/usr/share/doc/wgaf/` | Default config, README, changelog, licence |
| `~/.config/wgaf/` | **Your** config and permission policy, mode 600 |

Your configuration is written only if it is not already there. An upgrade
leaves edited files alone and says which ones it kept — see
[Upgrading](#upgrading).

## Install from source

```sh
git clone https://github.com/Ranrar/wgaf.git
cd wgaf
make install
```

This builds and installs `wgaf` and `wgaf-daemon` (via `cargo install`,
default `~/.cargo/bin`), installs the systemd user unit, and installs and
enables the GNOME Shell Extension that window management needs.

## First-time setup

Three things no installer can do for you. Do all three before logging out, and
you only have to log out once.

**1. Enable the GNOME Shell extension.**

```sh
gnome-extensions enable wgaf@wgaf.dev
```

A source install (`make install`) does this for you; a package does not,
because enabling an extension is a per-user choice and a package runs as root.

**2. Join the `input` group**, for keyboard and mouse automation.

```sh
sudo usermod -aG input $USER
```

**If you installed from source**, the udev rule that makes the group mean
anything is also yours to add — the packages ship it:

```sh
echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-wgaf-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Window, workspace and monitor commands work without any of this. Typing and
clicking do not.

**3. Log out and back in.** All of the above need it: group membership is read
at login, and GNOME Shell on Wayland loads extension code only at login. There
is no way to reload an extension mid-session on Wayland.

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

A package and `make install` both put `config.toml` and `permissions.toml` in
`~/.config/wgaf/` for you, and wgaf runs without them anyway — treating every
capability as allowed. [Configuration](configuration.md) covers every setting
and how to restrict what automation may do.

One thing to know before you automate anything: **Escape is wgaf's emergency
stop.** Press it and all input automation stops immediately; `wgaf release`
allows it again. See the
[user guide](user-guide.md#emergency-stop--pulling-the-handbrake).

Escape is only taken while wgaf is actually running automation. The rest of the
time it belongs to your applications as usual.

## Shell completions

```sh
wgaf completions bash > /etc/bash_completion.d/wgaf
wgaf completions zsh > "${fpath[1]}/_wgaf"
```

(`fish` also supported — run `wgaf completions --help` for the full list of
targets.) Man pages are optional — `make man` generates and installs them.

## Upgrading

Install the new package the same way, or re-run `make install` from an updated
checkout.

**Your configuration is never overwritten.** If `~/.config/wgaf/config.toml` or
`permissions.toml` already exists, the package leaves it and says so. That
matters most for `permissions.toml`: the shipped copy allows every capability,
so replacing yours would silently turn a `Deny` you wrote back into `Allow`. To
pick up new defaults deliberately, compare against `/usr/share/doc/wgaf/`.

**Log out and back in after every upgrade.** A new daemon talking to the old
extension still in memory fails with methods reported as missing. Wayland
cannot reload an extension mid-session.

## Uninstall

From a package:

```sh
sudo apt remove wgaf        # Debian, Ubuntu
sudo dnf remove wgaf        # Fedora, RHEL
sudo zypper remove wgaf     # openSUSE
sudo pacman -R wgaf         # Arch
```

From source:

```sh
make uninstall
```

From the tarball there is no package database keeping track, so delete what the
listing showed:

```sh
sudo rm -rf /usr/bin/wgaf /usr/bin/wgaf-daemon \
    /usr/share/gnome-shell/extensions/wgaf@wgaf.dev \
    /usr/lib/systemd/user/wgaf-daemon.service \
    /usr/lib/udev/rules.d/99-wgaf-uinput.rules \
    /usr/share/doc/wgaf
```

None of these touch `~/.config/wgaf/`, and any udev rule or `input` group
membership you added by hand is left in place.

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
