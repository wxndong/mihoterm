# Release Process

MihoTerm follows Semantic Versioning. Pre-release tags mark capability gates,
not dates.

1. Update `CHANGELOG.md` and the crate version.
2. Run `./scripts/ci-local.sh` from a clean checkout.
3. Build and smoke-test every supported release target.
4. Scan tracked files and Git history for secrets and personal data.
5. Create an annotated tag on a `main` commit.
6. Publish archives and `SHA256SUMS`.
7. Verify the downloaded artifacts independently.

Planned first-release gates:

- `v0.1.0-alpha.1`: usable read-only attach mode;
- `v0.1.0-beta.1`: safe control actions and target probes;
- `v0.1.0-rc.1`: profiles, managed runtime, packaging, and security audit;
- `v0.1.0`: successful isolated canary use and documentation review.
