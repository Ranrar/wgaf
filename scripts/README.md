# Packaging scripts

Builds distribution packages for wgaf. They land in `dist/`.

```sh
bash scripts/package.sh          # every format this machine can build
bash scripts/package.sh --deb    # just one
bash scripts/package.sh --rpm
bash scripts/package.sh --arch
bash scripts/package.sh --tar
```

**To install the result, see [docs/installation.md](../docs/installation.md)** —
that covers the requirements, the per-distribution commands, first-time setup
and what has actually been tested.

## What each script does

| File | Builds | Needs |
|---|---|---|
| `package.sh` | Builds wgaf, assembles the file tree, then calls the four below | `cargo`, `unzip`, `glib-compile-schemas` |
| `package-deb.sh` | Debian, Ubuntu, Mint, Pop!_OS | `dpkg-deb`, `fakeroot` |
| `package-rpm.sh` | Fedora, RHEL, openSUSE | `rpmbuild` (Debian: `sudo apt install rpm`) |
| `package-arch.sh` | Arch, Manjaro, EndeavourOS | `makepkg` for a `.pkg.tar.zst`; otherwise bundles the `PKGBUILD` for someone on Arch to build |
| `package-tar.sh` | Everything else | nothing |
| `_package-common.sh` | Sourced by the others — paths, output helpers, and the post-install fragment they all embed | — |

No distribution ships all four packaging tools, so a missing one is reported
and skipped rather than failing the run. Build on one machine and the packages
work on the others: they contain compiled binaries, so only the architecture
has to match.

The per-format scripts package a tree that `package.sh` assembles. Running one
on its own without that tree stops with a message saying so rather than a
`cp: cannot stat` — but the normal way in is `package.sh`.

## The one piece worth knowing about

`_package-common.sh` holds `CONFIG_SETUP_SNIPPET`, a POSIX-sh fragment that the
Debian `postinst`, the RPM `%post` and the pacman `.install` all embed. It runs
on the user's machine, as root, and puts `config.toml` and `permissions.toml`
into `~/.config/wgaf/` at mode 600.

It lives in one place because three hand-written copies had already drifted —
the Arch one used `$HOME`, which during a pacman transaction is root's home, so
it wrote the configuration into `/root`.

**It never overwrites an existing file.** The shipped `permissions.toml` allows
every capability, so replacing a user's copy would silently turn a `Deny` they
wrote back into `Allow`.

## Publishing

Build locally, install the package, *then* publish — both bugs found in this
script's first two runs (a stale manual page, and a settings schema that was
never compiled) were only visible by unpacking the artifact.

```sh
bash scripts/package.sh
sudo apt install ./dist/wgaf_<version>_amd64.deb
wgaf status

gh release create v<version> dist/* --title "wgaf <version>" --notes-file CHANGELOG.md
gh release upload v<version> dist/*     # if the release already exists
```
