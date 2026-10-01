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

## Supported release scope

Stable releases currently support Linux x86_64. ARM build targets exist but are
not advertised as validated release assets. The completed alpha milestones are
recorded in `CHANGELOG.md`; they are historical tags and are not renamed.
Release titles, notes, artifact names and contributor documentation use English.
Localized terminal UI text remains supported.

## Recovery gates

Run `scripts/ci-release-local.sh x86_64-unknown-linux-musl`, then test the
executable and core extracted from that exact archive with:

- `scripts/test-subscription.py`;
- `scripts/test-resilience.py`;
- `scripts/test-managed-recovery.py` (also pass `--legacy-binary` when validating
  a staged upgrade from a supported older release).

Use the scripts' `--help` for isolated state paths. Complete an authenticated
Codex canary on a separate proxy before applying production policy. Record the
actual test results and observation window; do not equate short probes or a
single model response with sustained external network availability.

Keep protected production process identities and endpoint credentials unchanged
when staging an update. Retain the executable used by any live owner. Publish
only after local gates pass, inspect GitHub CI separately, and verify the release
commit, main branch, tag and independently downloaded archive hashes agree.

Portable builds use Zig 0.16.0 and cargo-zigbuild 0.23.0. Mihomo asset names,
versions, and SHA-256 values are reviewed in `packaging/mihomo-assets.tsv`.
Standard data commits and SHA-256 values are reviewed in
`packaging/geodata-assets.tsv`.
