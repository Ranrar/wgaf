# Makefile for the wgaf GNOME Shell Extension bridge.
#
# Dev-iteration packaging/install only - full distro packaging (.deb/.rpm
# bundling daemon + extension + udev rules) is not built here and lives
# elsewhere.
#
# Usage:
#   make schemas   - compile the settings schema (the kill-switch shortcut)
#   make pack      - build wgaf@wgaf.dev.shell-extension.zip via `gnome-extensions pack`
#   make install   - pack, then install to ~/.local/share/gnome-shell/extensions/
#   make enable    - enable the extension in the current Shell (logout/nested-restart may be required)
#   make disable   - disable the extension
#   make uninstall - remove the installed copy
#   make clean     - remove build artifacts

UUID := wgaf@wgaf.dev
SCHEMA := schemas/org.gnome.shell.extensions.wgaf.gschema.xml
SRC_FILES := metadata.json extension.js dbusInterface.js windows.js pointer.js $(SCHEMA)
PACK_FILE := $(UUID).shell-extension.zip
INSTALL_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(UUID)

.PHONY: pack schemas install enable disable uninstall clean

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
		--schema=$(SCHEMA) \
		.

install: schemas pack
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
	rm -f $(PACK_FILE) schemas/gschemas.compiled
