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
# would rather see what they are installing. It is the staged tree exactly as
# the packages contain it, so it unpacks over / and nothing else.
#
# Deliberately not an installer script. A tarball that runs code is a package
# with none of a package's guarantees — no dependency check, no uninstall, no
# record of what it put where. `tar -tf` shows the whole contents in advance,
# which is the one thing this format is good at.
#
# The work is all in ensure_tarball() — including the install note that goes
# inside it — because package-arch.sh needs the identical file to record its
# checksum against. See that function for what went wrong when they differed.
ensure_tarball
ok "$TARBALL_NAME"
