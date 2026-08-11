# Makefile for the wgaf GNOME Shell Extension bridge.
#
# Dev-iteration packaging/install only - full distro packaging (.deb/.rpm
# bundling daemon + extension + udev rules) is not built here and lives
# elsewhere.
#
# Usage:
#   make test      - run the extension's unit tests (needs only gjs)
#   make schemas   - compile the settings schema (the kill-switch shortcut)
#   make pack      - build wgaf@wgaf.dev.shell-extension.zip via `gnome-extensions pack`
#   make install   - pack, then install to ~/.local/share/gnome-shell/extensions/
#                    Says whether that is a first install, an update or a
#                    reinstall, and refuses to replace a newer version with an
#                    older one. Add ALLOW_DOWNGRADE=1 if you mean to go back.
#   make enable    - enable the extension (log out and back in for it to load)
#   make disable   - disable the extension
#   make uninstall - remove the installed copy
#   make clean     - remove build artifacts

UUID := wgaf@wgaf.dev
SCHEMA := schemas/org.gnome.shell.extensions.wgaf.gschema.xml
SRC_FILES := metadata.json extension.js dbusInterface.js windows.js pointer.js confirm.js $(SCHEMA)
PACK_FILE := $(UUID).shell-extension.zip
INSTALL_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(UUID)

# The version about to be installed, and the one already there. Both are read
# from the same field of the same file, so they are always comparable.
#
# Read now rather than during the install, so that what is reported is the state
# beforehand — by the time the recipe runs, the new manifest has replaced the
# old one and there is nothing left to compare against.
VERSION := $(shell sed -n 's/.*"version-name": *"\([^"]*\)".*/\1/p' metadata.json)
INSTALLED_VERSION := $(shell sed -n 's/.*"version-name": *"\([^"]*\)".*/\1/p' \
	$(INSTALL_DIR)/metadata.json 2>/dev/null)

.PHONY: pack schemas install version-check enable disable uninstall clean test

# Unit tests that run outside GNOME Shell.
#
# Only the parts of the bridge that do not reach for `global.display`,
# `global.workspace_manager` or Clutter's seat can be covered this way, because
# those exist only inside a running Shell and there is no way to start one for a
# test. Today that is confirm.js. It is also the piece where an untested bug is
# quietest: every mutating method routes its "did that actually happen?" check
# through it, and a broken check reports success for changes that never landed.
#
# Deliberately NOT in SRC_FILES above - tests are not shipped in the bundle.
TESTS := tests/confirm-test.js tests/workspace-grid-test.js tests/window-state-test.js \
	tests/pointer-target-test.js

test:
	@for t in $(TESTS); do \
		echo "--- $$t"; \
		gjs -m $$t || exit 1; \
	done

pack: $(PACK_FILE)

# The settings schema holds the kill-switch keyboard shortcut. GNOME reads the
# compiled form, so it is compiled here as well as inside the bundle - that way
# the shortcut also works when running the extension straight from this
# directory.
schemas: schemas/gschemas.compiled

schemas/gschemas.compiled: $(SCHEMA)
	glib-compile-schemas schemas/

$(PACK_FILE): $(SRC_FILES)
	gnome-extensions pack --force \
		--extra-source=windows.js \
		--extra-source=dbusInterface.js \
		--extra-source=pointer.js \
		--extra-source=confirm.js \
		--schema=$(SCHEMA) \
		.

# Says what is about to happen, and stops if it would go backwards.
#
# Installing an older extension over a newer one is the awkward case worth
# refusing: it succeeds, and the daemon then reports missing methods on a
# session that was working an hour ago. The version is the only warning
# available before you have logged out and found out.
#
# Runs before the install rather than after, so a refusal costs nothing.
version-check:
	@if [ -z "$(VERSION)" ]; then \
		echo "Cannot read version-name from metadata.json." >&2; \
		exit 1; \
	elif [ -z "$(INSTALLED_VERSION)" ]; then \
		echo "Installing $(UUID) $(VERSION) — first install."; \
	elif [ "$(INSTALLED_VERSION)" = "$(VERSION)" ]; then \
		echo "Reinstalling $(UUID) $(VERSION) — same version as the one already installed."; \
	elif [ "$$(printf '%s\n%s\n' '$(INSTALLED_VERSION)' '$(VERSION)' \
			| sort -V | head -1)" = "$(VERSION)" ]; then \
		if [ -n "$(ALLOW_DOWNGRADE)" ]; then \
			echo "Downgrading $(UUID): $(INSTALLED_VERSION) -> $(VERSION)."; \
		else \
			echo "Refusing to install $(UUID) $(VERSION) over the newer $(INSTALLED_VERSION)." >&2; \
			echo >&2; \
			echo "Going backwards leaves a session where wgaf reports methods as missing," >&2; \
			echo "and you will not find out until you have logged out and back in." >&2; \
			echo >&2; \
			echo "If you mean it:  make install ALLOW_DOWNGRADE=1" >&2; \
			exit 1; \
		fi; \
	else \
		echo "Updating $(UUID): $(INSTALLED_VERSION) -> $(VERSION)."; \
		echo "See CHANGELOG.md for what changed."; \
	fi

install: version-check schemas pack
	gnome-extensions install --force $(PACK_FILE)
	@echo
	@echo "Installed $(UUID) $(VERSION) to $(INSTALL_DIR)"
	@echo
	@echo "If this is the first install, enable it:"
	@echo "    gnome-extensions enable $(UUID)"
	@echo
	@echo "Then log out and back in. GNOME Shell on Wayland loads an extension's"
	@echo "code only at login, so a new version never takes effect until you do."

enable:
	gnome-extensions enable $(UUID)

disable:
	gnome-extensions disable $(UUID)

uninstall:
	gnome-extensions uninstall $(UUID) || true
	rm -rf $(INSTALL_DIR)

clean:
	rm -f $(PACK_FILE) schemas/gschemas.compiled
