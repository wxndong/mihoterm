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

## Managed runtime

Managed mode is opt-in. It derives a separate runtime configuration and does
not execute the stored profile verbatim. Controller and mixed proxy listeners
bind only to loopback on dynamically reserved ports. TUN, iptables, custom
listeners, tunnels, server inbounds, external UI, and unauthenticated alternate
controller transports are disabled.

The child receives a cleared environment, a private home directory, a random
controller secret stored only in a mode `0600` runtime file, and `umask 077`.
MihoTerm tracks and stops only the child it spawned. It never searches for or
signals processes by name.

## Portable release supply chain

Portable archives use statically linked MihoTerm binaries and unmodified
official Mihomo release assets. The core and standard GeoIP/GeoSite data files,
plus their licenses, are pinned by immutable upstream commits and SHA-256 in
the repository and verified before packaging. Each archive records the exact
upstream version or asset commit, source location, and checksum.

MihoTerm does not silently replace the bundled core at runtime. Core updates
and bundled data revisions are reviewed and shipped through a new MihoTerm
release.
