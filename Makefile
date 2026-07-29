# Build and install wgaf. See README.md for setup and usage.
#
#   make build      - build everything
#   make install    - install the binaries, config files, systemd user unit,
#                     and GNOME Shell Extension, then print the remaining
#                     /dev/uinput setup steps
#   make uninstall  - reverse all of the above
#   make man        - install man pages (optional)
#   make test-apps  - build the GTK4 applications used by some tests
#                     (needs GTK4 development packages; nothing else does)
#   make clean      - remove build artifacts
#
# Installs to ~/.cargo/bin. If you've set CARGO_INSTALL_ROOT or CARGO_HOME
# elsewhere, edit ExecStart in packaging/systemd/wgaf-daemon.service to match
# before installing.

XDG_CONFIG_HOME ?= $(HOME)/.config
XDG_DATA_HOME ?= $(HOME)/.local/share
WGAF_CONFIG_DIR := $(XDG_CONFIG_HOME)/wgaf
SYSTEMD_USER_DIR := $(XDG_CONFIG_HOME)/systemd/user
SYSTEMD_UNIT := packaging/systemd/wgaf-daemon.service
MAN_DIR := $(XDG_DATA_HOME)/man/man1

.PHONY: build install uninstall man test-apps clean \
	cargo-install cargo-uninstall systemd-install systemd-uninstall \
	config-install

build:
	cargo build --release --workspace

# Installs the template config files, never overwriting an existing one — your
# edits survive reinstalls. Tests for the file rather than using `cp -n`, which
# can't report whether it copied or skipped.
config-install:
	@mkdir -p $(WGAF_CONFIG_DIR)
	@for f in config.toml permissions.toml; do \
		if [ -e "$(WGAF_CONFIG_DIR)/$$f" ]; then \
			echo "Kept existing $(WGAF_CONFIG_DIR)/$$f"; \
		else \
			cp "packaging/$$f" "$(WGAF_CONFIG_DIR)/$$f" && \
				echo "Installed $(WGAF_CONFIG_DIR)/$$f"; \
		fi; \
	done
	@# Set explicitly rather than inheriting the umask (002 on many distros
	@# gives 0664). The daemon refuses to start on a group/world-writable
	@# config or policy file. Re-applied every run.
	@chmod 600 $(WGAF_CONFIG_DIR)/config.toml $(WGAF_CONFIG_DIR)/permissions.toml
	@echo "Set mode 600 on both files (required: they must not be group/world-writable)"

install: cargo-install systemd-install config-install
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

# The applications in tests/apps/ are a workspace of their own, deliberately
# excluded from the root one: building wgaf itself must never require GTK4
# development packages, and only these need them. That is why this is a separate
# step rather than part of 'make build'.
#
# On Debian/Ubuntu the requirement is libgtk-4-dev; on Fedora, gtk4-devel.
test-apps:
	cargo build --manifest-path tests/apps/Cargo.toml

clean:
	cargo clean
	cargo clean --manifest-path tests/apps/Cargo.toml
