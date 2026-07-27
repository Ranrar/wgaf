# wgaf

**W**ayland **G**NOME **A**utomation **F**ramework

A Wayland-native automation framework for GNOME Shell — window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation.

## What we're building

A Rust daemon and scriptable CLI that provide window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation on GNOME Wayland — built entirely on the APIs the platform actually exposes for this: a GNOME Shell Extension for window management, Linux `uinput` for input synthesis, and AT-SPI for accessibility-based automation.

The GNOME Shell Extension bridge (`wgaf@wgaf.dev`, `extension/`) exposes window/workspace enumeration and control over D-Bus as `org.gnome.Shell.Extensions.Wgaf.V1`. The daemon mirrors that as its own `org.wgaf.Windows1` D-Bus interface, driven from the CLI via `wgaf window list/focus/move/resize/close/workspaces`. Keyboard/mouse synthesis goes through `uinput` via the daemon's own `org.wgaf.Input1` D-Bus interface, driven from the CLI via `wgaf type/key/mouse` — no Shell extension involved. Accessibility-driven UI automation goes through AT-SPI via the daemon's own `org.wgaf.Accessibility1` D-Bus interface, driven from the CLI via `wgaf a11y ...`.

## Why

Wayland's security model is correct, but it left a real gap: there's no modern, native equivalent to `xdotool`/`wmctrl` for GNOME. People still need to script window layouts, automate repetitive UI interactions, and drive applications for testing — without resorting to X11 compatibility hacks or bypassing the platform's protections. The goal is automation that works *with* GNOME's security model instead of around it: explicit, attributable actions through supported APIs, not silent global control.

## Getting Started

### Build and test (no GNOME session needed)

```sh
cargo build --workspace
cargo test --workspace
```

`cargo test --workspace` includes `wgaf-daemon/tests/windows_stub.rs`, which exercises the real daemon binary's `org.wgaf.Windows1` D-Bus interface against a hand-written stub of the GNOME Shell extension — no real GNOME Shell required.

### Smoke test the daemon/CLI

```sh
cargo run -p wgaf-daemon &
cargo run -p wgaf-cli -- ping
```

Should print `pong`. This is Phase 1 functionality and doesn't need the GNOME Shell extension.

### Testing window management — requires the GNOME Shell extension

`wgaf window ...` commands need `extension/` (`wgaf@wgaf.dev`) actually running and enabled somewhere the daemon can reach over D-Bus.

**Option A: nested GNOME Shell session (recommended — doesn't touch your real desktop).**

On Ubuntu this needs a package that isn't installed by default:

```sh
sudo apt install mutter-dev-bin
```

That provides `/usr/libexec/mutter-devkit`. Without it: plain `dbus-run-session -- gnome-shell --wayland` (no `--devkit`) doesn't nest at all on this GNOME/Mutter version — it tries to become an independent display server and fails with `Failed to take control of the session: GDBus.Error:System.Error.EBUSY: Device or resource busy`, since your real session already owns the seat. And `--devkit` without `mutter-dev-bin` installed fails silently (no window appears), with `Failed to launch devkit: Failed to execute child process "/usr/libexec/mutter-devkit": No such file or directory` buried in the logs. Once the package is installed:

```sh
dbus-run-session -- gnome-shell --devkit --wayland
```

This opens a real, disposable nested Shell session (its own D-Bus session bus) safe to crash or experiment in. From a terminal opened *inside* that nested session (via its Activities overview — an outer terminal targets the wrong session bus):

```sh
cd extension && make install
gnome-extensions enable wgaf@wgaf.dev
```

Then, still inside the nested session:

```sh
cargo run -p wgaf-daemon
```

```sh
cargo run -p wgaf-cli -- window list
cargo run -p wgaf-cli -- --json window list
cargo run -p wgaf-cli -- window focus <id>
cargo run -p wgaf-cli -- window move <id> <x> <y>
cargo run -p wgaf-cli -- window resize <id> <width> <height>
cargo run -p wgaf-cli -- window close <id>
cargo run -p wgaf-cli -- window workspaces
```

(`<id>` is a real numeric id from `window list`'s output — Mutter's stable sequence number, not a placeholder.)

**Option B: your real, live GNOME session.**

```sh
cd extension && make install
```

A newly installed extension UUID isn't picked up by an already-running Shell on Wayland — there's no live rescan. Log out and back in first, then:

```sh
gnome-extensions enable wgaf@wgaf.dev
cargo run -p wgaf-daemon
cargo run -p wgaf-cli -- window list
```

### Troubleshooting: "extension bridge unavailable"

If `wgaf window ...` fails immediately with this error even though the extension is enabled, check for a duplicate `wgaf-daemon` process holding the `org.wgaf.Daemon` D-Bus well-known name — a stale backgrounded instance (e.g. a `&`'d job that survived a Ctrl+C) wins the name race and makes a freshly started second instance silently useless:

```sh
pgrep -af wgaf-daemon
pkill -f 'target/debug/wgaf-daemon'
```

For the full D-Bus reference (gdbus/busctl calls, signal watching), see `.vscode/Documentation/phase2-testing.md`. For the daemon/CLI window-management API surface, see `.vscode/Documentation/phase3-window-management-api.md`.

## TODO

- [x] Phase 1: Daemon + CLI scaffolding (D-Bus `Ping`/`Version`)
- [x] Phase 2: GNOME Shell Extension bridge (window enumeration/control)
- [x] Phase 3: Window management via CLI
- [x] Phase 4: Keyboard/mouse automation (`uinput`)
- [x] Phase 5: Accessibility automation (AT-SPI)
- [ ] Phase 6: Permissions & security hardening
- [ ] Phase 7: Packaging & documentation
- [ ] Phase 8: MCP server for AI agent integration

## License

MIT — see [LICENSE](LICENSE).
