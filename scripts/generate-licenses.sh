#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=build-limits.sh
source "$project_dir/scripts/build-limits.sh"
output="${1:-$project_dir/THIRD-PARTY-LICENSES.html}"
if [[ "$output" != /* ]]; then
  output="$project_dir/$output"
fi

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "required release tool is missing: cargo-about" >&2
  exit 2
fi

temporary="$(mktemp "$project_dir/target/third-party-licenses.XXXXXXXX.raw")"
trap 'rm -f -- "$temporary"' EXIT

cd "$project_dir"
cargo about generate \
  --locked \
  --all-features \
  --fail \
  --output-file "$temporary" \
  packaging/about.hbs
sed -E 's/[[:space:]]+$//' "$temporary" >"$output"
