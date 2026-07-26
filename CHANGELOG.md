# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
