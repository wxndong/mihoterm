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
