# Changelog

All notable changes to MihoTerm will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-alpha.4] - 2026-08-17

### Added

- Confirmed live switching between stored subscription profiles in the managed
  TUI while preserving the current loopback proxy endpoint.
- Default-on installer choice for a per-user `mihoterm.service`, with explicit
  `--autostart` and `--no-autostart` automation controls.
- An explicit bilingual Global, Rule, and Direct mode chooser.

### Changed

- Cold managed sessions now default to Global mode without modifying stored
  profiles; Global mode shows only the GLOBAL selector and its nodes.
- Installing a different version safely stops the exact old managed process,
  preserves its active profile, and restarts or rolls back atomically.

### Fixed

- Refresh dashboard policy groups and proxies immediately after a profile
  switch, with rollback if Mihomo rejects or MihoTerm cannot persist the
  replacement configuration.

## [0.1.0-alpha.3] - 2026-08-12

### Added

- A read-only live connections view (`c`) backed by the Mihomo `/connections`
  endpoint, with host, network, proxy chain, rule, and per-connection
  throughput columns, plus a real-time aggregate upload/download rate in the
  header.
- A typed `ApiClient::connections` method and connection models.
- A headless `probe` command for running built-in or configured targets through
  an explicitly named Mihomo proxy, with per-target output and aggregate exit
  status.

### Fixed

- Prevent managed profiles from entering a proxy-node DNS bootstrap cycle when
  they enable DNS but omit `proxy-server-nameserver`; MihoTerm now derives the
  dedicated node resolvers from the profile's own `default-nameserver` while
  preserving explicit DNS policy.

## [0.1.0-alpha.2] - 2026-07-27

### Added

- A persistent per-user Mihomo lifecycle with `start`, `stop`, `status`,
  `env`, `shell`, and `exec` commands.
- Generated authentication for the loopback mixed proxy, plus owner-only
  session descriptors and PID-reuse-resistant stop behavior.
- Reversible Bash integration that loads the active proxy in new shells and
  restores pre-existing proxy variables after stop or uninstall.
- A versioned user-local installer and safe uninstaller; profiles and
  configuration are preserved unless `--purge` is requested.
- TUI subscription management with arrow-key navigation, hidden URL input,
  redacted source summaries, validated add/replace, and background update.
- Installer lifecycle coverage in the local CI entry point.

### Changed

- Closing the TUI no longer stops the managed proxy.
- Portable archives now include `install.sh` and the Bash integration hook.

### Fixed

- Wrap status messages and context-sensitive controls in narrow terminals
  instead of silently truncating them.

## [0.1.0-alpha.1] - 2026-07-26

### Added

- Initial project contracts and local validation entry point.
- Typed, timeout-bounded Mihomo API reads for version, runtime configuration,
  and proxy state.
- Mock-controller contract tests covering authentication, URL prefixes,
  response parsing, and error redaction.
- Read-only, non-blocking terminal views for policy groups and proxies.
- Arrow-key focus and movement, list search, manual refresh, and a sanitized
  headless status command.
- Permission checks for controller secret files and reliable terminal
  restoration on normal exit, signals, errors, and panics.
- Confirmed proxy selection and Mihomo mode changes through typed API writes.
- Separate Google, OpenAI/Codex, and GitHub delay probes with explicit expected
  HTTP statuses.
- A validated, redacted custom-probe model for the upcoming XDG configuration.
- Owner-only XDG configuration loading for custom HTTPS probe targets.
- Named profile import from a protected HTTPS URL file or local Mihomo YAML.
- Bounded downloads, structural YAML validation, private storage, serialized
  mutations, atomic updates, and one-step rollback for managed profiles.
- Profile management commands that never alter a running Mihomo instance.
- An opt-in managed runtime with preflight validation, dynamically reserved
  loopback ports, a random API secret, private files, and exact child cleanup.
- A hardened derived configuration that disables TUN, iptables, extra
  listeners, tunnels, inbound servers, external UI, and non-loopback control.
- A minimal child wrapper that clears inherited environment overrides, applies
  `umask 077`, and requests automatic termination if MihoTerm disappears.
- Guided first-run setup with a hidden subscription URL prompt and deterministic
  profile selection.
- Explicit attach mode; the no-argument command now starts an isolated managed
  instance instead of implicitly connecting to another process.
- Static portable bundle tooling with a checksum-pinned official Mihomo core,
  license separation, and reproducible archive metadata.
- Checksum-pinned Mihomo GeoIP and GeoSite data for deterministic cold starts
  when GitHub is unreachable before the proxy is running.
- Ring-based rustls cryptography to reduce the static MihoTerm binary size.

### Fixed

- Request Mihomo-compatible YAML from subscription services that select their
  response format from the client identifier.

[Unreleased]: https://github.com/wxndong/mihoterm/compare/v0.1.0-alpha.4...HEAD
[0.1.0-alpha.4]: https://github.com/wxndong/mihoterm/compare/v0.1.0-alpha.3...v0.1.0-alpha.4
[0.1.0-alpha.3]: https://github.com/wxndong/mihoterm/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com/wxndong/mihoterm/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/wxndong/mihoterm/releases/tag/v0.1.0-alpha.1
