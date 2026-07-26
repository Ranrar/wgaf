# Makefile for the wgaf GNOME Shell Extension bridge.
#
# Dev-iteration packaging/install only (Phase 2 scope) - full distro
# packaging (.deb/.rpm bundling daemon + extension + udev rules) is Phase 7
# and lives elsewhere.
#
# Usage:
#   make pack      - build wgaf@wgaf.dev.shell-extension.zip via `gnome-extensions pack`
#   make install   - pack, then install to ~/.local/share/gnome-shell/extensions/
#   make enable    - enable the extension in the current Shell (logout/nested-restart may be required)
#   make disable   - disable the extension
#   make uninstall - remove the installed copy
#   make clean     - remove build artifacts

UUID := wgaf@wgaf.dev
SRC_FILES := metadata.json extension.js dbusInterface.js windows.js
PACK_FILE := $(UUID).shell-extension.zip
INSTALL_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(UUID)

.PHONY: pack install enable disable uninstall clean

pack: $(PACK_FILE)

$(PACK_FILE): $(SRC_FILES)
	gnome-extensions pack --force \
		--extra-source=windows.js \
		--extra-source=dbusInterface.js \
		.

install: pack
	gnome-extensions install --force $(PACK_FILE)
	@echo "Installed to $(INSTALL_DIR)"
	@echo "Run 'make enable', then log out/in (X11) or Alt+F2 r Enter (Xorg only)"
	@echo "or restart the nested Shell session (Wayland has no in-session Shell restart)."

enable:
	gnome-extensions enable $(UUID)

disable:
	gnome-extensions disable $(UUID)

uninstall:
	gnome-extensions uninstall $(UUID) || true
	rm -rf $(INSTALL_DIR)

clean:
	rm -f $(PACK_FILE)
