# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-07-27

### Added

- `wgaf-daemon`: new `input` module (`src/input/`) — `device.rs` manages raw `/dev/uinput` device lifecycle, with ioctl request numbers (`UI_SET_EVBIT`/`UI_SET_KEYBIT`/`UI_SET_RELBIT`/`UI_DEV_SETUP`/`UI_DEV_CREATE`/`UI_DEV_DESTROY`) computed via a `const fn` mirroring the kernel's own `_IOC` macro rather than hand-copied magic numbers, self-checked against real kernel header values in a unit test, using `libc`'s `input_event`/`input_id`/`uinput_setup` structs (new workspace dependency `libc = "0.2"`) matching `ydotoold`'s own low-level approach rather than a higher-level uinput crate. `codes.rs` provides evdev event/key/button/axis constants, case-insensitive key-name lookup, and an ASCII→(keycode, needs_shift) table for `TypeText`, deliberately US-QWERTY-only for v1 (documented limitation, same as `ydotool`). `keyboard.rs`/`mouse.rs` provide press/release/type_text and relative move/click/scroll primitives — mouse absolute positioning is an explicit non-goal since Wayland has no reliable global absolute-coordinate authority. `mod.rs` exposes `InputBackend` (mirroring `windows::WindowManager`) and `InputError` (`thiserror`); the device is created lazily on first use via `tokio::sync::OnceCell`, cached only on success, so a `/dev/uinput` permissions problem never blocks daemon startup or the other two D-Bus interfaces and recovers without a restart once fixed. Device operations run via `tokio::task::spawn_blocking` guarded by a `std::sync::Mutex`.
- `wgaf-daemon`: every synthesized input action (`type_text`, `key_press`, `key_release`, `mouse_move`, `mouse_click`, `mouse_scroll`) is logged via `tracing::info!` on target `wgaf_daemon::input::audit` before executing — an audit trail only (nothing is blocked), ahead of Phase 6's real permission engine, replacing `ydotool`'s zero-accountability blind-forwarding model. `TypeText` has a `MAX_TYPE_TEXT_LEN` safety cap (4096 chars) as a guard against a runaway/mistaken caller, not a policy decision. `InputError::DeviceUnavailable` explains the udev-rule + `input`-group fix directly in the error message and never suggests root/sudo.
- `wgaf-daemon`: new `org.wgaf.Input1` D-Bus interface at `/org/wgaf/Input` (`src/dbus/input_api.rs`, wired into `src/dbus/mod.rs` and `src/main.rs` alongside `org.wgaf.Daemon1`/`org.wgaf.Windows1`) — `TypeText(s)`, `KeyPress(s)`, `KeyRelease(s)`, `MouseMove(i,i)`, `MouseClick(s)`, `MouseScroll(i,i)`, all returning `()`, with named errors `org.wgaf.Input1.Error.DeviceUnavailable`/`.UnknownKey`/`.InvalidButton` (constants added to `wgaf-common/src/lib.rs` alongside the existing `WINDOWS_*`/`EXTENSION_*` constants). `src/config.rs` gained an `input_device_name` field, mirroring `extension_bus_name`'s precedent for test isolation.
- `wgaf-cli`: new `wgaf type <text>` / `wgaf key press|release <key>` / `wgaf mouse move <dx> <dy>` / `wgaf mouse click <button>` / `wgaf mouse scroll <dx> <dy>` subcommand tree (`src/commands/input.rs`, wired into `src/main.rs` and `src/commands/mod.rs`), following the existing `wgaf window ...` subtree's structure, with `--json` wrapping a success/message status since these calls return `()`.
- `wgaf-daemon/tests/input.rs`: integration test spawning the real `wgaf-daemon` binary and driving an actual kernel `uinput` device — confirms lazy device creation, all six `Input1` methods succeeding, the device appearing in `/proc/bus/input/devices` (`EV=7`, an `eventN` handler), and clean teardown (`UI_DEV_DESTROY`) on daemon exit, plus unit tests for the ioctl-number self-check, the ASCII→keycode table, and CLI argument parsing. Raw per-event readback from the device's own `/dev/input/eventN` node is a documented gap (`EACCES` in this sandbox — needs full `input`-group membership); see `.vscode/Documentation/phase4-input-automation-api.md`.

## [0.3.0] - 2026-07-26

### Added

- `wgaf-common`: plain-`serde` DTOs `WindowRecord`/`WorkspaceRecord`, plus a new `dict` module (`wgaf-common/src/dict.rs`) with `WindowRecordDict`/`WorkspaceRecordDict` — `zvariant` `a{sv}`-derived wire types with `From`/`Into` conversions to the plain DTOs, needed because zvariant's dict derive wraps every field in a `Variant` and isn't interchangeable with plain JSON. New D-Bus naming constants for the daemon's own `org.wgaf.Windows1` interface (object path, interface name, named errors `WindowNotFound`/`ExtensionUnavailable`) and for the extension's existing `org.gnome.Shell.Extensions.Wgaf.V1` interface (client-side bus name, object path, interface name, `WindowNotFound` error).
- `wgaf-daemon`: new `windows` module (`src/windows/mod.rs`, `src/windows/proxy.rs`) — a `zbus`-proxy client of the GNOME Shell Extension bridge, with extension-availability discovery via `DBus.NameHasOwner` and `Introspectable.Introspect` (checking the introspection XML for the versioned `V1` interface node), caching only successful checks so enabling the extension later doesn't require a daemon restart. Closes the Phase 2 TODO on daemon/extension version negotiation. Translates the extension's `WindowNotFound` error into the daemon's own error type.
- `wgaf-daemon`: new `org.wgaf.Windows1` D-Bus interface (`src/dbus/windows_api.rs`, `src/dbus.rs` split into `src/dbus/mod.rs` + this new module) — `ListWindows`, `FocusWindow(id)`, `MoveWindow(id,x,y)`, `ResizeWindow(id,w,h)`, `CloseWindow(id)`, `GetWorkspaces`, delegating to the new `WindowManager`, with its own named D-Bus errors (`org.wgaf.Windows1.Error.WindowNotFound` / `.ExtensionUnavailable`) distinct from the extension's, via a `zbus::DBusError` derive. `src/config.rs` gained an `extension_bus_name` config field (defaults to the real extension's bus name, overridable for the stub-based integration test). `src/main.rs` now serves both `org.wgaf.Daemon1` and `org.wgaf.Windows1` on the same connection/bus name.
- `wgaf-cli`: new `wgaf window list/focus/move/resize/close/workspaces` subcommand tree (`src/main.rs`, new `src/commands/window.rs`) — a thin D-Bus client of `org.wgaf.Windows1`, converting wire dicts to the shared DTOs, plus a `--json` flag (valid on either side of the subcommand) for machine-readable output via `serde_json`. `move`/`resize` use `allow_hyphen_values = true` to accept negative coordinates. `src/commands/mod.rs` gained a `describe_dbus_error` helper turning the daemon's named D-Bus errors into short human-readable messages instead of a raw `zbus::Error` debug dump.
- `wgaf-daemon/tests/windows_stub.rs`: integration test running the daemon against a hand-written stub GNOME Shell Extension, covering the success, `WindowNotFound`, and `ExtensionUnavailable` paths end-to-end — the roadmap's documented mocked-extension testing strategy, used because real end-to-end verification against a live GNOME Shell extension in a nested session needs interactive GUI steps not scriptable in this environment (documented gap; see `.vscode/Documentation/phase3-window-management-api.md` §5). Plus DTO/wire-format round-trip and signature unit tests, and CLI argument-parsing tests.

## [0.2.0] - 2026-07-26

### Added

- `extension/`: GNOME Shell extension bridge (uuid `wgaf@wgaf.dev`, ESM `Extension` subclass, targets GNOME Shell 50) exporting window/workspace control over D-Bus as `org.gnome.Shell.Extensions.Wgaf` (object path `/org/gnome/Shell/Extensions/Wgaf`, interface `org.gnome.Shell.Extensions.Wgaf.V1`).
- `dbusInterface.js`: D-Bus contract for the bridge — methods `ListWindows`, `FocusWindow`, `MoveWindow`, `ResizeWindow`, `CloseWindow`, `GetWorkspaces`; signals `WindowCreated`, `WindowClosed`, `WindowFocusChanged`; named error `org.gnome.Shell.Extensions.Wgaf.Error.WindowNotFound`. Interface is explicitly versioned (`V1`); breaking changes are to ship as an additive `V2` on the same object path rather than mutating `V1`.
- `windows.js`: D-Bus-agnostic Mutter/`Meta` window manager — enumerates windows via `global.display.list_all_windows()`, tracks focus via `notify::focus-window` and per-window close via each window's `unmanaging` signal, and filters out override-redirect windows from `listWindows()`. Uses `Meta.Window.get_stable_sequence()` (not `get_id()`, which is the X11 XID and always `0` on native Wayland clients) as the stable, protocol-agnostic D-Bus window `id`.
- `extension/Makefile`: `pack`/`install`/`enable`/`disable`/`uninstall`/`clean` targets for dev-iteration packaging via `gnome-extensions pack`.

## [0.1.0] - 2026-07-26

### Added

- Initial Cargo workspace: `wgaf-common`, `wgaf-daemon`, `wgaf-cli`.
- `wgaf-daemon`: TOML config loading, structured logging (`tracing`), and a `zbus`
  D-Bus service (`org.wgaf.Daemon`, interface `org.wgaf.Daemon1`) exposing `Ping`
  and `Version`.
- `wgaf-cli` (`wgaf` binary): `ping` subcommand.
- Optional systemd user unit (`packaging/systemd/wgaf-daemon.service`).
- Integration test exercising daemon startup and `Ping` over D-Bus.
- Project documentation: README, SECURITY policy, MIT license.
