# Contributing

Thank you for considering a contribution to MihoTerm.

## Before opening a pull request

1. Keep the change focused on one responsibility.
2. Do not include real subscription URLs, controller secrets, proxy names,
   personal paths, or production screenshots.
3. Add or update tests for observable behavior.
4. Run `./scripts/ci-local.sh`.
5. Explain the user impact and validation evidence in the pull request.

Release changes must also run `./scripts/ci-release-local.sh` for every
architecture being published.

## Code style

- Prefer responsibility-based module names over generic `utils`, `helpers`, or
  `common` modules.
- Keep I/O at the boundaries and business state testable without a terminal or
  live Mihomo instance.
- Avoid blocking the UI thread with network or filesystem work.
- Treat all remote profile content as untrusted input.

## Commit and pull request scope

Use short imperative commit subjects. A pull request should introduce one
coherent capability and should remain independently reviewable.
