#!/usr/bin/env bash
#
# Common setup and functions for package-*.sh distro builders.
# This file is sourced by each distro-specific packaging script.

set -euo pipefail

# Initialize if not already done
if [ -z "${_COMMON_LOADED:-}" ]; then
    _COMMON_LOADED=1

    # Set up paths and variables
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(dirname "$SCRIPT_DIR")"

    UUID="wgaf@wgaf.dev"
    VERSION="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$REPO_ROOT/Cargo.toml" | head -1)"
    DIST="$REPO_ROOT/dist"
    STAGE="$REPO_ROOT/target/package-root"

    # Color codes
    if [ -t 1 ]; then
        C_OK=$'\033[32m'; C_WARN=$'\033[33m'; C_DIM=$'\033[2m'; C_OFF=$'\033[0m'
    else
        C_OK=""; C_WARN=""; C_DIM=""; C_OFF=""
    fi

    # Helper functions
    step() { printf '\n%s==>%s %s\n' "$C_DIM" "$C_OFF" "$1"; }
    ok()   { printf '%s  ok%s  %s\n' "$C_OK" "$C_OFF" "$1"; }
    skip() { printf '%sskip%s  %s\n' "$C_WARN" "$C_OFF" "$1"; }

    # Version check
    [ -n "$VERSION" ] || { echo "could not read the version from Cargo.toml" >&2; exit 1; }

    # The staged tree as one file. Both a product in its own right — the fallback
    # for distributions with no package here — and the source the PKGBUILD builds
    # from, so there is one tarball rather than two identical ones under different
    # names.
    TARBALL_NAME="wgaf-$VERSION-$(uname -m).tar.gz"
    ensure_tarball() {
        [ -f "$DIST/$TARBALL_NAME" ] || tar -C "$STAGE" -czf "$DIST/$TARBALL_NAME" .
    }

    # Refuses to build a package out of a tree that was never assembled.
    #
    # The per-format scripts can be run on their own, and every one of them
    # starts by copying $STAGE. Without this, doing so fails with
    # `cp: cannot stat '.../package-root'`, which names the symptom and not the
    # cause.
    require_stage() {
        [ -d "$STAGE" ] && [ -x "$STAGE/usr/bin/wgaf" ] && return 0
        cat >&2 <<EOF

Nothing has been built yet — $STAGE does not exist.

The per-format scripts package a tree that scripts/package.sh assembles. Run
that instead, either for everything or for one format:

    bash scripts/package.sh
    bash scripts/package.sh --deb

EOF
        exit 1
    }

    # ---------------------------------------------------------------------------
    # The post-install fragment every package embeds.
    # ---------------------------------------------------------------------------
    #
    # Written once, in POSIX sh, because it runs inside a Debian postinst, an
    # RPM %post and a pacman .install — three files that cannot source anything
    # from this repository, since by then they are on someone else's machine.
    # Keeping three copies is how they came to disagree: the Arch one used
    # $HOME, which is root's home during a pacman transaction, so it wrote the
    # configuration into /root.
    #
    # **It never overwrites an existing file.** An earlier version copied the
    # packaged defaults over whatever was there, keeping one `.old` backup. For
    # config.toml that loses your settings on every upgrade; for
    # permissions.toml it is worse than that, because the shipped file allows
    # every capability — so an upgrade would silently turn `TypeText = "Deny"`
    # back into `Allow`. A package must not quietly re-grant a permission its
    # owner took away. This matches the root Makefile, which has always said it
    # never overwrites an existing config.
    CONFIG_SETUP_SNIPPET='
# wgaf reads ~/.config/wgaf/, which is per-user, while this script runs as
# root. Work out who to set it up for, and say so plainly when that cannot be
# determined rather than writing into /root.
wgaf_user="${SUDO_USER:-}"
if [ -z "$wgaf_user" ] && [ -n "${PKEXEC_UID:-}" ]; then
    wgaf_user="$(getent passwd "$PKEXEC_UID" | cut -d: -f1)"
fi

if [ -z "$wgaf_user" ] || [ "$wgaf_user" = "root" ]; then
    echo "wgaf: could not tell whose configuration to set up. To do it yourself:"
    echo "        mkdir -p ~/.config/wgaf && chmod 700 ~/.config/wgaf"
    echo "        cp /usr/share/doc/wgaf/config.toml ~/.config/wgaf/"
    echo "        cp /usr/share/doc/wgaf/permissions.toml ~/.config/wgaf/"
    echo "        chmod 600 ~/.config/wgaf/config.toml ~/.config/wgaf/permissions.toml"
else
    wgaf_home="$(getent passwd "$wgaf_user" | cut -d: -f6)"
    wgaf_group="$(id -g "$wgaf_user")"
    wgaf_dir="$wgaf_home/.config/wgaf"

    if [ ! -d "$wgaf_dir" ]; then
        mkdir -p "$wgaf_dir"
        chown "$wgaf_user:$wgaf_group" "$wgaf_dir"
        chmod 700 "$wgaf_dir"
    fi

    for wgaf_file in config.toml permissions.toml; do
        if [ -f "$wgaf_dir/$wgaf_file" ]; then
            echo "wgaf: kept your existing $wgaf_file"
        elif [ -f "/usr/share/doc/wgaf/$wgaf_file" ]; then
            cp "/usr/share/doc/wgaf/$wgaf_file" "$wgaf_dir/$wgaf_file"
            chown "$wgaf_user:$wgaf_group" "$wgaf_dir/$wgaf_file"
            # The daemon refuses to read either file if it is group- or
            # world-readable, since permissions.toml decides what automation is
            # allowed to do.
            chmod 600 "$wgaf_dir/$wgaf_file"
            echo "wgaf: installed a default $wgaf_file in $wgaf_dir"
        fi
    done
fi
'

    # What a user has to do that a package cannot do for them, written once and
    # printed by all four. `$USER` is left literal on purpose — it is being
    # shown as a command to type, not expanded here.
    POST_INSTALL_NOTE='
wgaf is installed. Three things are left, and all of them need you:

  1. Enable the GNOME Shell extension:
       gnome-extensions enable wgaf@wgaf.dev

  2. To let wgaf type and click, join the input group:
       sudo usermod -aG input $USER

  3. Log out and back in. Both of the above need it, and so does the
     extension — GNOME Shell on Wayland loads extension code only at login.

Then: wgaf status
'

fi
