# Build and install wgaf. See README.md for setup and usage.
#
#   make check-deps - check the non-Rust packages the build needs are installed
#                     (runs automatically before 'make build' and 'make install')
#   make build      - build everything
#   make install    - install the binaries, config files, systemd user unit,
#                     and GNOME Shell Extension, then print the remaining
#                     /dev/uinput setup steps
#   make uninstall  - reverse all of the above
#   make man        - install man pages (optional)
#   make test-apps  - build the GTK4 applications used by some tests
#                     (needs GTK4 development packages; nothing else does)
#   make test-desktop - run the tests that drive a real desktop
#                     (opens windows and types on your session — see below.
#                     Start it and walk away; nothing here waits for you)
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

.PHONY: build install uninstall man test-apps test-desktop clean \
	cargo-install cargo-uninstall systemd-install systemd-uninstall \
	config-install check-deps check-gtk

# Everything wgaf needs that Cargo cannot install for you.
#
# Checked before building rather than after, because the alternative is a wall
# of linker output ending in "cannot find -lxkbcommon", which tells you what
# failed but not what to install.
check-deps:
	@printf 'Checking build dependencies... '
	@if ! printf 'int main(void){return 0;}' | cc -x c - -o /dev/null -lxkbcommon 2>/dev/null; then \
		echo "MISSING"; \
		echo; \
		echo "wgaf needs the libxkbcommon development package to build."; \
		echo "It reads your keyboard layout so 'wgaf type' produces the characters"; \
		echo "you asked for on any layout, not just US."; \
		echo; \
		echo "The library itself is already on every Wayland desktop — it is the"; \
		echo "header and linker files that are missing. Install one of:"; \
		echo; \
		echo "  Debian/Ubuntu   sudo apt install libxkbcommon-dev"; \
		echo "  Fedora/RHEL     sudo dnf install libxkbcommon-devel"; \
		echo "  Arch            sudo pacman -S libxkbcommon"; \
		echo "  openSUSE        sudo zypper install libxkbcommon-devel"; \
		echo; \
		exit 1; \
	fi
	@echo "ok (libxkbcommon)"

build: check-deps
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
check-gtk:
	@printf 'Checking test-application dependencies... '
	@if ! pkg-config --exists gtk4 2>/dev/null; then \
		echo "MISSING"; \
		echo; \
		echo "The test applications need the GTK4 development package."; \
		echo "Nothing else in wgaf does — the daemon and CLI build without it,"; \
		echo "which is why this is a separate step."; \
		echo; \
		echo "  Debian/Ubuntu   sudo apt install libgtk-4-dev"; \
		echo "  Fedora/RHEL     sudo dnf install gtk4-devel"; \
		echo "  Arch            sudo pacman -S gtk4"; \
		echo "  openSUSE        sudo zypper install gtk4-devel"; \
		echo; \
		exit 1; \
	fi
	@echo "ok (gtk4)"

test-apps: check-gtk
	cargo build --manifest-path tests/apps/Cargo.toml

# These tests drive your actual desktop: they open windows, move focus, and
# type on the keyboard for real. That is the point — it is the only way to
# check that what wgaf sends is what an application receives — but it means
# they cannot run while you are using the machine for anything else. Leave the
# session alone until they finish.
#
# They are marked ignored so that an ordinary 'cargo test' never starts them by
# accident, and run one at a time because two of them driving the keyboard at
# once would each receive the other's keystrokes.
#
# You can start this and walk away. Everything here runs on its own.
#
# One test is deliberately left out: the emergency-key test needs somebody to
# press Escape on a real keyboard, so it would sit here waiting for a minute
# rather than finishing. The kill-switch line below names the one test it wants
# instead of running the whole file, so that adding a test to that file never
# drags the manual one in by accident. To run the manual one yourself:
#
#   cargo test -p wgaf-daemon --test kill_switch -- --ignored --nocapture \
#       --test-threads=1 a_synthesized_escape
#
# It tells you on screen when it is your turn, and the key press has to go to
# some window other than the terminal you started it from.
test-desktop: test-apps
	cargo test -p wgaf-daemon --test keyboard_coverage --test keyboard_layout \
		--test window_management --test pointer --test combined -- --ignored --test-threads=1
	cargo test -p wgaf-daemon --test kill_switch -- --ignored --test-threads=1 \
		stop_during_a_long_type_text

clean:
	cargo clean
	cargo clean --manifest-path tests/apps/Cargo.toml
