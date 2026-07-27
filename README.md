# wgaf

**W**ayland **G**NOME **A**utomation **F**ramework

A Wayland-native automation framework for GNOME Shell — window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation.

## What we're building

A Rust daemon and scriptable CLI that provide window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation on GNOME Wayland — built entirely on the APIs the platform actually exposes for this: a GNOME Shell Extension for window management, Linux `uinput` for input synthesis, and AT-SPI for accessibility-based automation.

The GNOME Shell Extension bridge (`wgaf@wgaf.dev`, `extension/`) exposes window/workspace enumeration and control over D-Bus as `org.gnome.Shell.Extensions.Wgaf.V1`. The daemon mirrors that as its own `org.wgaf.Windows1` D-Bus interface, driven from the CLI via `wgaf window list/focus/move/resize/close/workspaces`. Keyboard/mouse synthesis goes through `uinput` via the daemon's own `org.wgaf.Input1` D-Bus interface, driven from the CLI via `wgaf type/key/mouse` — no Shell extension involved. Accessibility-driven UI automation goes through AT-SPI via the daemon's own `org.wgaf.Accessibility1` D-Bus interface, driven from the CLI via `wgaf a11y ...`.

## Why

Wayland's security model is correct, but it left a real gap: there's no modern, native equivalent to `xdotool`/`wmctrl` for GNOME. People still need to script window layouts, automate repetitive UI interactions, and drive applications for testing — without resorting to X11 compatibility hacks or bypassing the platform's protections. The goal is automation that works *with* GNOME's security model instead of around it: explicit, attributable actions through supported APIs, not silent global control.

## Getting Started

### Build

```sh
cargo build --release --workspace
```

Produces `target/release/wgaf-daemon` and `target/release/wgaf` (the CLI).

### Enable the GNOME Shell extension (needed for window management only)

```sh
cd extension && make install
```

A newly installed extension isn't picked up by an already-running Shell on Wayland — log out and back in once, then:

```sh
gnome-extensions enable wgaf@wgaf.dev
```

`wgaf type/key/mouse` and `wgaf a11y ...` don't need this — they talk to the daemon directly via `uinput`/AT-SPI, no Shell extension involved.

### Run the daemon

```sh
wgaf-daemon &
```

See "Configuration" below for `config.toml`/`permissions.toml`, or `packaging/systemd/wgaf-daemon.service` to run it as a systemd user service instead of backgrounding it manually.

### Use the CLI

```sh
wgaf ping                                        # daemon health check

wgaf window list
wgaf window focus/move/resize/close <id>
wgaf window workspaces

wgaf type "hello"
wgaf key press/release <key>
wgaf mouse move/click/scroll ...

wgaf a11y list-apps
wgaf a11y find --app <name> --role <role> --name <name>
wgaf a11y tree --app <name>
wgaf a11y info/click/focus/set-text <element-ref> ...
```

Add `--json` (before or after the subcommand) to any command for machine-readable output.

### Troubleshooting: "extension bridge unavailable"

If `wgaf window ...` fails immediately with this error even though the extension is enabled, check for a duplicate `wgaf-daemon` process holding the `org.wgaf.Daemon` D-Bus name — a stale instance wins the name race and makes a freshly started second one silently useless:

```sh
pgrep -af wgaf-daemon
pkill -f wgaf-daemon
```

For development/testing procedures, see the test suites under `wgaf-daemon/tests/`.

## Configuration

`wgaf-daemon` optionally reads a `permissions.toml` file controlling per-capability policy. It must live in the **same directory** as `config.toml` (pass `--config` and `wgaf-daemon` looks for `permissions.toml` right next to it automatically), or point at it directly with `--permissions /path`. Neither file has a default filesystem location yet.

### `permissions.toml` — per-capability policy

Thirteen capabilities exist, one per gated (mutating) D-Bus method — read-only methods (`ListWindows`, `GetTree`, `FindElements`, ...) can't be gated at all:

| Interface | Capabilities |
|---|---|
| `org.wgaf.Windows1` | `FocusWindow`, `MoveWindow`, `ResizeWindow`, `CloseWindow` |
| `org.wgaf.Input1` | `TypeText`, `KeyPress`, `KeyRelease`, `MouseMove`, `MouseClick`, `MouseScroll` |
| `org.wgaf.Accessibility1` | `InvokeAction`, `SetText`, `FocusElement` |

```toml
# permissions.toml
[capabilities]
TypeText = "Deny"        # block `wgaf type` entirely
CloseWindow = "Prompt"    # ask via a GNOME notification (Allow/Deny) before closing a window
```

Any capability not listed defaults to `Allow` — this is a dev tool, so permissions are an opt-in *restriction* you configure, never an opt-in *unlock* you must grant before anything works.

## TODO

- [x] Phase 1: Daemon + CLI scaffolding (D-Bus `Ping`/`Version`)
- [x] Phase 2: GNOME Shell Extension bridge (window enumeration/control)
- [x] Phase 3: Window management via CLI
- [x] Phase 4: Keyboard/mouse automation (`uinput`)
- [x] Phase 5: Accessibility automation (AT-SPI)
- [x] Phase 6: Permissions & security hardening
- [ ] Phase 7: Packaging & documentation
- [ ] Phase 8: MCP server for AI agent integration

## License

MIT — see [LICENSE](LICENSE).
