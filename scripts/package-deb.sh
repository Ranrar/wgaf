#!/usr/bin/env bash
#
# Builds Debian/Ubuntu .deb package for wgaf.
#
# Usage:
#   bash scripts/package-deb.sh     Debian, Ubuntu, Pop!_OS, Mint

source "$(dirname "${BASH_SOURCE[0]}")/_package-common.sh"

step "Building the .deb"

if ! command -v dpkg-deb >/dev/null; then
    skip "dpkg-deb is not installed (Debian/Ubuntu ship it; elsewhere: install dpkg)"
    exit 0
fi

require_stage

DEB_ROOT="$REPO_ROOT/target/deb-root"
rm -rf "$DEB_ROOT"
cp -a "$STAGE" "$DEB_ROOT"
install -d "$DEB_ROOT/DEBIAN"

# Depends: the shared libraries the binaries actually link against,
# rather than the -dev packages needed to build them. libxkbcommon0 is
# on every Wayland desktop already; naming it means apt says so rather
# than the daemon failing to start.
cat > "$DEB_ROOT/DEBIAN/control" <<EOF
Package: wgaf
Version: $VERSION
Section: utils
Priority: optional
Architecture: $(dpkg --print-architecture)
Depends: libc6, libxkbcommon0, gnome-shell (>= 45)
Recommends: at-spi2-core
Maintainer: Kim Skov Rasmussen <kim@skovrasmussen.com>
Homepage: https://github.com/Ranrar/wgaf
Description: Desktop automation for GNOME on Wayland
 wgaf automates GNOME on Wayland through the interfaces GNOME actually
 provides: windows and workspaces through a GNOME Shell extension, keyboard
 and mouse through the kernel's uinput device, and buttons and text fields by
 name through the accessibility bus.
 .
 It does not work around the compositor's security model, which is why it
 needs an extension for window management and cannot see other applications'
 keystrokes.
EOF

# Assembled from pieces rather than written as one heredoc, so the config
# setup and the closing note come from _package-common.sh and cannot drift
# away from the .rpm's and the Arch package's copies of the same thing.
{
    cat <<'HEAD'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    # The rule is only useful once udev has reloaded and applied it.
    if command -v udevadm >/dev/null; then
        udevadm control --reload-rules || true
        udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
    fi
    if command -v systemctl >/dev/null; then
        systemctl daemon-reload || true
    fi
HEAD

    printf '%s\n' "$CONFIG_SETUP_SNIPPET"

    # Quoted delimiter: the note contains `$USER`, which is being shown to the
    # reader as something to type rather than expanded here.
    printf "    cat <<'NOTE'\n%s\nNOTE\nfi\nexit 0\n" "$POST_INSTALL_NOTE"
} > "$DEB_ROOT/DEBIAN/postinst"
chmod 755 "$DEB_ROOT/DEBIAN/postinst"

# A postinst that fails leaves the package half-configured, so it is worth
# knowing it at least parses before shipping it.
sh -n "$DEB_ROOT/DEBIAN/postinst" || {
    echo "the generated postinst is not valid shell" >&2
    exit 1
}

DEB_FILE="$DIST/wgaf_${VERSION}_$(dpkg --print-architecture).deb"
fakeroot dpkg-deb --build "$DEB_ROOT" "$DEB_FILE" >/dev/null
ok "$(basename "$DEB_FILE")"

# The parts worth seeing, rather than all sixty-odd paths. A package
# that installs the wrong thing is much easier to catch here than after
# someone has installed it — the first two builds of this script
# shipped a stale manual page and an uncompiled settings schema, and
# both were visible in a listing.
printf '%s      %s manual pages, extension at %s%s\n' "$C_DIM" \
    "$(dpkg-deb --contents "$DEB_FILE" | grep -c 'man1/.*\.gz$')" \
    "$(dpkg-deb --contents "$DEB_FILE" | awk '/extensions\/.*metadata.json/{print $6}')" \
    "$C_OFF"
