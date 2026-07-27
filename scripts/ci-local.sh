#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=build-limits.sh
source "$project_dir/scripts/build-limits.sh"
cd "$project_dir"

cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cargo build --release --locked
./scripts/test-installer.sh
git diff --check

if command -v gitleaks >/dev/null 2>&1; then
  gitleaks git --no-banner --redact
else
  echo "gitleaks not installed; secret-history scan skipped"
fi
