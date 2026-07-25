# Security Policy

## Supported versions

MihoTerm is pre-release software. Security fixes are applied to the latest
development version until the first stable release.

## Reporting a vulnerability

Use
[GitHub private vulnerability reporting](https://github.com/wxndong/mihoterm/security/advisories/new).
Do not open a public issue for a suspected vulnerability or include live
credentials in a report.

## Sensitive data

Controller secrets and subscription sources are credentials. MihoTerm must:

- store them only in user-owned files with restrictive permissions;
- redact them from logs, errors, diagnostics, and screenshots;
- never include them in fixtures, documentation, release artifacts, or
  telemetry;
- avoid passing them through command-line arguments when a file or environment
  variable is available.
