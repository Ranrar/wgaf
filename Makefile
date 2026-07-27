# Root Makefile for wgaf — GNOME Wayland Automation Framework.
#
# A convenience wrapper around `cargo install --path` for the wgaf-daemon/
# wgaf binaries, plus the extra one-time steps a `cargo install`-only
# workflow doesn't cover: installing the systemd user unit and installing/
# enabling the GNOME Shell Extension (whose own install/enable/uninstall
# logic already lives in extension/Makefile — this Makefile just invokes
# that, so one `make install` here does the whole job).
#
# This is dev-iteration/self-build install tooling, matching
# extension/Makefile's own scope note — NOT a distro (.deb/.rpm) packaging
# pipeline. Full distro packaging is intentionally out of scope until there
# is a tagged release to package.
#
# Usage:
#   make build      - cargo build --release --workspace
#   make install    - `cargo install --path` for wgaf-daemon/wgaf-cli, install
#                      the systemd user unit, install+enable the GNOME Shell
#                      Extension, print the /dev/uinput setup steps
#   make uninstall  - reverse all of the above
#   make man        - generate man pages and install them to
#                      ~/.local/share/man/man1/ (optional, not part of
#                      `install` — see wgaf-cli/src/main.rs's
#                      `generate_man_pages` test)
#   make clean      - cargo clean
#
# Assumes the default `cargo install` target directory (`~/.cargo/bin`,
# i.e. `$CARGO_INSTALL_ROOT` unset) — packaging/systemd/wgaf-daemon.service's
# `ExecStart=%h/.cargo/bin/wgaf-daemon` assumes the same. If you've set
# `CARGO_INSTALL_ROOT`/`CARGO_HOME` to something else, edit that unit's
# `ExecStart` to match before installing it.

XDG_CONFIG_HOME ?= $(HOME)/.config
XDG_DATA_HOME ?= $(HOME)/.local/share
SYSTEMD_USER_DIR := $(XDG_CONFIG_HOME)/systemd/user
SYSTEMD_UNIT := packaging/systemd/wgaf-daemon.service
MAN_DIR := $(XDG_DATA_HOME)/man/man1

.PHONY: build install uninstall man clean \
	cargo-install cargo-uninstall systemd-install systemd-uninstall

build:
	cargo build --release --workspace

install: cargo-install systemd-install
	$(MAKE) -C extension install
	$(MAKE) -C extension enable
	@echo
	@echo "=== wgaf installed ==="
	@echo "wgaf-daemon/wgaf installed via 'cargo install --path' (default: ~/.cargo/bin)."
	@echo "Systemd user unit installed to $(SYSTEMD_USER_DIR)/wgaf-daemon.service"
	@echo "Start it with: systemctl --user enable --now wgaf-daemon.service"
	@echo "Shell completions: wgaf completions {bash,zsh,fish,elvish,powershell}"
	@echo "Man pages (optional): run 'make man'"
	@echo
	@echo "--- /dev/uinput access (required for keyboard/mouse automation) ---"
	@echo "This Makefile does NOT write udev rules automatically (that needs root)."
	@echo "One-time manual setup:"
	@echo "  1. Create /etc/udev/rules.d/99-wgaf-uinput.rules containing:"
	@echo "       KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\""
	@echo "  2. sudo udevadm control --reload-rules && sudo udevadm trigger"
	@echo "  3. sudo usermod -aG input \$$USER"
	@echo "  4. Log out and back in for the new group membership to take effect."
	@echo
	@echo "GNOME Shell Extension installed and enabled, but GNOME Shell may need a"
	@echo "session restart to actually load it (Wayland has no in-session Shell"
	@echo "restart) — see extension/Makefile's own 'make install' output."

cargo-install: build
	cargo install --path wgaf-daemon --force
	cargo install --path wgaf-cli --force

systemd-install:
	mkdir -p $(SYSTEMD_USER_DIR)
	cp $(SYSTEMD_UNIT) $(SYSTEMD_USER_DIR)/wgaf-daemon.service
	systemctl --user daemon-reload

uninstall: systemd-uninstall cargo-uninstall
	$(MAKE) -C extension uninstall
	@echo "wgaf uninstalled."
	@echo "Note: any manual udev rule / 'input' group membership changes from"
	@echo "'make install' are left in place (this target never touches those)."

systemd-uninstall:
	systemctl --user disable --now wgaf-daemon.service || true
	rm -f $(SYSTEMD_USER_DIR)/wgaf-daemon.service
	systemctl --user daemon-reload || true

cargo-uninstall:
	cargo uninstall wgaf-daemon || true
	cargo uninstall wgaf-cli || true

man:
	cargo test -p wgaf-cli generate_man_pages -- --ignored
	mkdir -p $(MAN_DIR)
	cp target/man/*.1 $(MAN_DIR)/
	@echo "Man pages installed to $(MAN_DIR) — ensure it's on your MANPATH."

clean:
	cargo clean
