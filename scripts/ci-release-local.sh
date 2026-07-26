#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-x86_64-unknown-linux-musl}"

cd "$project_dir"
for command_name in cargo-about cargo-audit; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "required release tool is missing: $command_name" >&2
    exit 2
  fi
done

license_check="$(mktemp "$project_dir/target/third-party-licenses.XXXXXXXX.html")"
trap 'rm -f -- "$license_check"' EXIT
./scripts/generate-licenses.sh "$license_check"
if ! cmp --silent "$license_check" THIRD-PARTY-LICENSES.html; then
  echo "THIRD-PARTY-LICENSES.html is stale" >&2
  exit 1
fi

cargo audit
./scripts/ci-local.sh
./scripts/build-portable.sh "$target"
archive="$(./scripts/package-portable.sh "$target")"
first_sha256="$(sha256sum "$archive" | awk '{ print $1 }')"
archive="$(./scripts/package-portable.sh "$target")"
second_sha256="$(sha256sum "$archive" | awk '{ print $1 }')"

if [[ "$first_sha256" != "$second_sha256" ]]; then
  echo "portable archive is not reproducible" >&2
  exit 1
fi

if tar -tzf "$archive" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
  echo "portable archive contains an unsafe path" >&2
  exit 1
fi

verification_dir="$(mktemp -d "$project_dir/target/release-verify.XXXXXXXX")"
trap 'rm -f -- "$license_check"; rm -rf -- "$verification_dir"' EXIT
tar -xzf "$archive" -C "$verification_dir"
bundle_dir="$(find "$verification_dir" -mindepth 1 -maxdepth 1 -type d -print -quit)"

for required in \
  "$bundle_dir/mihoterm" \
  "$bundle_dir/mihomo" \
  "$bundle_dir/LICENSE" \
  "$bundle_dir/THIRD_PARTY_NOTICES.md" \
  "$bundle_dir/THIRD-PARTY-LICENSES.html" \
  "$bundle_dir/CORE-METADATA.txt" \
  "$bundle_dir/licenses/Mihomo-GPL-3.0.txt"; do
  if [[ ! -f "$required" ]]; then
    echo "portable archive is missing: ${required##*/}" >&2
    exit 1
  fi
done

for executable in "$bundle_dir/mihoterm" "$bundle_dir/mihomo"; do
  if [[ ! -x "$executable" ]] || [[ "$(file -b "$executable")" != *"statically linked"* ]]; then
    echo "portable executable is not static and executable: ${executable##*/}" >&2
    exit 1
  fi
done

(
  cd "$project_dir/dist"
  sha256sum -c SHA256SUMS
)

printf 'release archive verified: %s\n' "$archive"
