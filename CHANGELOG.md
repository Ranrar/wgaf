# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-26

### Added

- Initial Cargo workspace: `wgaf-common`, `wgaf-daemon`, `wgaf-cli`.
- `wgaf-daemon`: TOML config loading, structured logging (`tracing`), and a `zbus`
  D-Bus service (`org.wgaf.Daemon`, interface `org.wgaf.Daemon1`) exposing `Ping`
  and `Version`.
- `wgaf-cli` (`wgaf` binary): `ping` subcommand.
- Optional systemd user unit (`packaging/systemd/wgaf-daemon.service`).
- Integration test exercising daemon startup and `Ping` over D-Bus.
- Project documentation: README, SECURITY policy, MIT license.
