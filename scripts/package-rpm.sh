#!/usr/bin/env bash
#
# Builds RPM package for wgaf (Fedora, RHEL, openSUSE).
#
# Usage:
#   bash scripts/package-rpm.sh     Fedora, RHEL, openSUSE

source "$(dirname "${BASH_SOURCE[0]}")/_package-common.sh"

step "Building the .rpm"

if ! command -v rpmbuild >/dev/null; then
    skip "rpmbuild is not installed (Debian/Ubuntu: sudo apt install rpm)"
    exit 0
fi

require_stage

RPM_TOP="$REPO_ROOT/target/rpmbuild"
rm -rf "$RPM_TOP"
install -d "$RPM_TOP"/{BUILD,RPMS,SOURCES,SPECS,BUILDROOT}

# Written in three parts, and the middle one is why.
#
# The header needs $VERSION and $STAGE substituted, so its heredoc is
# unquoted. The %post body must NOT be: it contains `${SUDO_USER:-}` and half a
# dozen other shell variables that belong to the machine installing the
# package, and an unquoted heredoc would expand them here — to nothing — and
# ship a script that sets up configuration for a user called "".
{
cat <<EOF
Name:           wgaf
Version:        $VERSION
Release:        1
Summary:        Desktop automation for GNOME on Wayland
License:        MIT
URL:            https://github.com/Ranrar/wgaf
BuildArch:      $(uname -m)
# Library dependencies are left to rpm's own dependency generator, which reads
# them out of the binaries. Naming libxkbcommon by hand would be wrong on at
# least one target: Fedora calls it libxkbcommon, openSUSE libxkbcommon0, and a
# package that names the other one is uninstallable rather than merely
# imprecise. gnome-shell has the same name on both, and cannot be inferred from
# an ELF header, so it is the one stated outright.
Requires:       gnome-shell
Recommends:     at-spi2-core

%description
wgaf automates GNOME on Wayland through the interfaces GNOME actually
provides: windows and workspaces through a GNOME Shell extension, keyboard
and mouse through the kernel's uinput device, and buttons and text fields by
name through the accessibility bus.

It does not work around the compositor's security model, which is why it
needs an extension for window management and cannot see other applications'
keystrokes.

# The tree is already built and staged; rpmbuild only has to package it.
%install
cp -a $STAGE/. %{buildroot}/

EOF

cat <<'POST'
%post
if command -v udevadm >/dev/null; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
fi
if command -v systemctl >/dev/null; then
    systemctl daemon-reload || true
fi
POST

printf '%s\n' "$CONFIG_SETUP_SNIPPET"
printf "cat <<'NOTE'\n%s\nNOTE\n\n" "$POST_INSTALL_NOTE"

cat <<EOF
%files
/usr/bin/wgaf
/usr/bin/wgaf-daemon
/usr/share/gnome-shell/extensions/$UUID
/usr/lib/systemd/user/wgaf-daemon.service
/usr/lib/udev/rules.d/99-wgaf-uinput.rules
/usr/share/doc/wgaf
%{_mandir}/man1/*
EOF
} > "$RPM_TOP/SPECS/wgaf.spec"

# rpmbuild is noisy on success and noisier on failure, and on a machine with no
# RPM database it prints errors about one that do not stop the build. Captured
# so that a real failure is the thing shown rather than something to find in
# forty lines of shell trace.
if ! rpmbuild --define "_topdir $RPM_TOP" -bb "$RPM_TOP/SPECS/wgaf.spec" \
        > "$RPM_TOP/build.log" 2>&1; then
    echo "rpmbuild failed:" >&2
    cat "$RPM_TOP/build.log" >&2
    exit 1
fi
find "$RPM_TOP/RPMS" -name '*.rpm' -exec cp {} "$DIST/" \;
ok "$(cd "$DIST" && ls -1 wgaf-"$VERSION"*.rpm 2>/dev/null | head -1)"
