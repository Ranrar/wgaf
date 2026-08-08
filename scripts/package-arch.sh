#!/usr/bin/env bash
#
# Builds Arch package (PKGBUILD) for wgaf (Arch, Manjaro, EndeavourOS).
#
# Usage:
#   bash scripts/package-arch.sh    Arch, Manjaro, EndeavourOS

source "$(dirname "${BASH_SOURCE[0]}")/_package-common.sh"

step "Building the Arch package"

# A PKGBUILD is written whether or not this machine can build from it,
# because it is the thing Arch users actually consume: the AUR distributes
# build recipes, not binaries. `makepkg` refuses to run as root and only
# exists on Arch, so a built package is a bonus rather than the point.
require_stage

ARCH_DIR="$REPO_ROOT/target/archpkg"
rm -rf "$ARCH_DIR"
install -d "$ARCH_DIR"
ensure_tarball
cp "$DIST/$TARBALL_NAME" "$ARCH_DIR/"

cat > "$ARCH_DIR/PKGBUILD" <<EOF
# Maintainer: Kim Skov Rasmussen <kim@skovrasmussen.com>
pkgname=wgaf
pkgver=$VERSION
pkgrel=1
pkgdesc="Desktop automation for GNOME on Wayland"
arch=('$(uname -m)')
url="https://github.com/Ranrar/wgaf"
license=('MIT')
depends=('libxkbcommon' 'gnome-shell')
optdepends=('at-spi2-core: automating buttons and text fields by name')
# A full URL rather than a bare filename, so \`makepkg\` fetches the tarball
# from the release instead of the user having to put it here by hand. If the
# file is already in the build directory — which it is when this repository
# builds the package itself — makepkg uses that one and skips the download.
source=("\$url/releases/download/v\$pkgver/wgaf-\$pkgver-\$CARCH.tar.gz")
sha256sums=('$(sha256sum "$DIST/$TARBALL_NAME" | cut -d' ' -f1)')

# The tree is already built and staged, so there is nothing to compile here.
package() {
    cp -a "\$srcdir/usr" "\$pkgdir/usr"
}

# Arch runs udev and systemd reloads through hooks rather than a post-install
# script, and the two things a user still has to do are printed by .install.
EOF

# The same body the .deb's postinst and the .rpm's %post get, for the same
# reason: three hand-maintained copies is how the Arch one came to use $HOME,
# which during a pacman transaction is root's home — so it wrote the
# configuration into /root and the user never saw it.
{
    printf 'post_install() {\n'
    printf '%s\n' "$CONFIG_SETUP_SNIPPET"
    printf "    cat <<'NOTE'\n%s\nNOTE\n}\n\n" "$POST_INSTALL_NOTE"
    cat <<'TAIL'
post_upgrade() {
    post_install
}
TAIL
} > "$ARCH_DIR/wgaf.install"

sh -n "$ARCH_DIR/wgaf.install" || {
    echo "the generated wgaf.install is not valid shell" >&2
    exit 1
}
# Referenced only after it exists, so a PKGBUILD copied out on its own is
# still valid.
printf 'install=wgaf.install\n' >> "$ARCH_DIR/PKGBUILD"

if command -v makepkg >/dev/null; then
    # On Arch itself, build the real thing: a single `.pkg.tar.zst` that
    # installs with `pacman -U` and needs nothing else, exactly as the .deb and
    # .rpm do on their distributions.
    (cd "$ARCH_DIR" && makepkg --nodeps --force >/dev/null 2>&1) &&
        find "$ARCH_DIR" -name '*.pkg.tar.*' -exec cp {} "$DIST/" \; &&
        ok "$(cd "$DIST" && ls -1 wgaf-"$VERSION"*.pkg.tar.* 2>/dev/null | head -1)" ||
        skip "makepkg failed — the build files are in $ARCH_DIR"
else
    # Anywhere else, ship the build files as **one** archive rather than two
    # loose ones.
    #
    # `makepkg` needs both the PKGBUILD and wgaf.install in the same directory —
    # it will not fetch the second, because `install=` files are read locally
    # and never downloaded. Publishing them separately made Arch the only
    # distribution whose users had to collect several files and put them in the
    # right place before anything would run, and forgetting one gave an error
    # about a missing scriptlet rather than about a missing download.
    #
    # The binary tarball is deliberately *not* in here: the PKGBUILD's `source`
    # is a URL to it, so makepkg fetches and checksums it. That keeps this
    # archive at a few kilobytes and means the 5 MB of binaries are downloaded
    # once, by the tool that verifies them.
    ARCH_BUNDLE="wgaf-$VERSION-arch.tar.gz"
    BUNDLE_DIR="wgaf-$VERSION-arch"
    rm -rf "${ARCH_DIR:?}/$BUNDLE_DIR"
    install -d "$ARCH_DIR/$BUNDLE_DIR"
    cp "$ARCH_DIR/PKGBUILD" "$ARCH_DIR/wgaf.install" "$ARCH_DIR/$BUNDLE_DIR/"
    tar -C "$ARCH_DIR" -czf "$DIST/$ARCH_BUNDLE" "$BUNDLE_DIR"
    ok "$ARCH_BUNDLE (PKGBUILD + install scriptlet)"
    skip "makepkg is not here (Arch only), so no .pkg.tar.zst — an Arch user"
    skip "builds one from the bundle with: tar -xzf $ARCH_BUNDLE && cd $BUNDLE_DIR && makepkg -si"
fi
