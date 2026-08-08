#!/usr/bin/env bash
#
# Builds portable tarball for wgaf (NixOS, Gentoo, and others).
#
# Usage:
#   bash scripts/package-tar.sh     everything else (NixOS, Gentoo, etc.)

source "$(dirname "${BASH_SOURCE[0]}")/_package-common.sh"

step "Building the portable tarball"

require_stage

# For every distribution without a package of its own, and for anyone who
# would rather see what they are installing. It is the staged tree exactly
# as the packages contain it, so it unpacks over / and nothing else.
#
# Deliberately not an installer script. A tarball that runs code is a
# package with none of a package's guarantees — no dependency check, no
# uninstall, no record of what it put where. `tar -tf` shows the whole
# contents in advance, which is the one thing this format is good at.
cat > "$STAGE/usr/share/doc/wgaf/INSTALL.tarball" <<EOF
wgaf $VERSION — portable install

Unpack over the filesystem root, as root:

    sudo tar -C / --no-same-owner -xzf wgaf-$VERSION-$(uname -m).tar.gz

Look before you do, if you like:

    tar -tzf wgaf-$VERSION-$(uname -m).tar.gz

Then set up your configuration. The packages do this for you; a tarball has no
install step to do it in, so it is yours to run — and note the mode, which the
daemon insists on because permissions.toml decides what automation may do:

    mkdir -p ~/.config/wgaf && chmod 700 ~/.config/wgaf
    cp -n /usr/share/doc/wgaf/config.toml ~/.config/wgaf/
    cp -n /usr/share/doc/wgaf/permissions.toml ~/.config/wgaf/
    chmod 600 ~/.config/wgaf/config.toml ~/.config/wgaf/permissions.toml

\`cp -n\` so that unpacking a newer tarball never overwrites settings you have
edited. wgaf runs without these files, treating every capability as allowed.
$POST_INSTALL_NOTE

To remove it again, delete what the listing showed:

    sudo rm -rf /usr/bin/wgaf /usr/bin/wgaf-daemon \\
        /usr/share/gnome-shell/extensions/$UUID \\
        /usr/lib/systemd/user/wgaf-daemon.service \\
        /usr/lib/udev/rules.d/99-wgaf-uinput.rules \\
        /usr/share/doc/wgaf
EOF

# Rebuilt rather than reused, so the INSTALL note written just above is in
# it. `ensure_tarball` may have made one already for the PKGBUILD.
rm -f "$DIST/$TARBALL_NAME"
ensure_tarball
ok "$TARBALL_NAME"
