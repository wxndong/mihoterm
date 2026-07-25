#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cargo build --release --locked
git diff --check

if command -v gitleaks >/dev/null 2>&1; then
  gitleaks git --no-banner --redact
else
  echo "gitleaks not installed; secret-history scan skipped"
fi
