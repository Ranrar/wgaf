# wgaf

**W**ayland **G**NOME **A**utomation **F**ramework

A Wayland-native automation framework for GNOME Shell — window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation.

## What we're building

A Rust daemon and scriptable CLI that provide window management, keyboard/mouse automation, application discovery, and accessibility-driven UI automation on GNOME Wayland — built entirely on the APIs the platform actually exposes for this: a GNOME Shell Extension for window management, Linux `uinput` for input synthesis, and AT-SPI for accessibility-based automation.

## Why

Wayland's security model is correct, but it left a real gap: there's no modern, native equivalent to `xdotool`/`wmctrl` for GNOME. People still need to script window layouts, automate repetitive UI interactions, and drive applications for testing — without resorting to X11 compatibility hacks or bypassing the platform's protections. The goal is automation that works *with* GNOME's security model instead of around it: explicit, attributable actions through supported APIs, not silent global control.

## TODO

- [x] Phase 1: Daemon + CLI scaffolding (D-Bus `Ping`/`Version`)
- [ ] Phase 2: GNOME Shell Extension bridge (window enumeration/control)
- [ ] Phase 3: Window management via CLI
- [ ] Phase 4: Keyboard/mouse automation (`uinput`)
- [ ] Phase 5: Accessibility automation (AT-SPI)
- [ ] Phase 6: Permissions & security hardening
- [ ] Phase 7: Packaging & documentation
- [ ] Phase 8: MCP server for AI agent integration

## License

MIT — see [LICENSE](LICENSE).
