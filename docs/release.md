# Release Process

MihoTerm follows Semantic Versioning. Pre-release tags mark capability gates,
not dates.

1. Update `CHANGELOG.md` and the crate version.
2. Run `./scripts/ci-local.sh` from a clean checkout.
3. Build and smoke-test every supported release target.
4. Verify pinned Mihomo core and GeoIP/GeoSite assets before assembling
   portable archives.
5. Scan tracked files and Git history for secrets and personal data.
6. Create an annotated tag on a `main` commit.
7. Publish archives, third-party notices, and `SHA256SUMS`.
8. Download and verify the published artifacts independently.

Planned first-release gates:

- `v0.1.0-alpha.1`: x86_64 portable bundle, guided setup, safe controls,
  profiles, and isolated canary use;
- `v0.1.0-alpha.2`: persistent authenticated proxy lifecycle, user-local
  installation, safe uninstall, shell integration, and TUI source management;
- `v0.1.0-alpha.3`: headless probes, live connection and throughput views, and
  managed proxy-node DNS bootstrap compatibility;
- `v0.1.0-alpha.4`: default Global workflow, explicit mode guidance, optional
  boot startup, and rollback-safe replacement of a running older version;
- `v0.1.0-beta.1`: validated aarch64 and armv7 bundles plus compatibility
  review;
- `v0.1.0-rc.1`: security audit, compatibility review, and release automation;
- `v0.1.0`: sustained canary use and documentation review.

Portable builds use Zig 0.16.0 and cargo-zigbuild 0.23.0. Mihomo asset names,
versions, and SHA-256 values are reviewed in `packaging/mihomo-assets.tsv`.
Standard data commits and SHA-256 values are reviewed in
`packaging/geodata-assets.tsv`.
