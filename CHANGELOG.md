# Changelog

All notable changes to MihoTerm will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
