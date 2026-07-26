#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-x86_64-unknown-linux-musl}"
dist_dir="${2:-$project_dir/dist}"
asset_table="$project_dir/packaging/mihomo-assets.tsv"
binary="$project_dir/target/$target/release/mihoterm"

case "$target" in
  x86_64-unknown-linux-musl) release_arch="x86_64" ;;
  aarch64-unknown-linux-musl) release_arch="aarch64" ;;
  armv7-unknown-linux-musleabihf) release_arch="armv7" ;;
  *)
    echo "unsupported portable target: $target" >&2
    exit 2
    ;;
esac

if [[ ! -x "$binary" ]]; then
  echo "portable binary is missing; run scripts/build-portable.sh $target" >&2
  exit 2
fi
if [[ "$(file -b "$binary")" != *"statically linked"* ]]; then
  echo "refusing to package a dynamically linked MihoTerm binary" >&2
  exit 1
fi

crate_version="$(
  awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = / {
      value = $0
      sub(/^version = "/, "", value)
      sub(/"$/, "", value)
      print value
      exit
    }
  ' "$project_dir/Cargo.toml"
)"
mihomo_version="$(awk -F '\t' '$1 == "# version" { print $2 }' "$asset_table")"
source_commit="$(awk -F '\t' '$1 == "# source-commit" { print $2 }' "$asset_table")"
asset_record="$(awk -F '\t' -v target="$target" '$1 == target { print $2 "\t" $3 }' "$asset_table")"
license_record="$(awk -F '\t' '$1 == "# license-url" { print $2 "\t" $3 }' "$asset_table")"

if [[ -z "$crate_version" || -z "$mihomo_version" || -z "$source_commit" || -z "$asset_record" || -z "$license_record" ]]; then
  echo "portable release metadata is incomplete" >&2
  exit 1
fi

IFS=$'\t' read -r mihomo_asset mihomo_sha256 <<<"$asset_record"
IFS=$'\t' read -r license_url license_sha256 <<<"$license_record"
mihomo_url="https://github.com/MetaCubeX/mihomo/releases/download/$mihomo_version/$mihomo_asset"
cache_dir="$project_dir/target/release-assets/$mihomo_version"
mihomo_archive="$cache_dir/$mihomo_asset"
mihomo_license="$cache_dir/LICENSE"

download_verified() {
  local url="$1"
  local destination="$2"
  local expected="$3"
  local partial="${destination}.partial.$$"

  if [[ -f "$destination" ]] && [[ "$(sha256sum "$destination" | awk '{ print $1 }')" == "$expected" ]]; then
    return
  fi

  rm -f -- "$partial"
  curl --fail --location --silent --show-error --output "$partial" "$url"
  if [[ "$(sha256sum "$partial" | awk '{ print $1 }')" != "$expected" ]]; then
    rm -f -- "$partial"
    echo "downloaded file failed SHA-256 verification: $url" >&2
    exit 1
  fi
  mv -- "$partial" "$destination"
}

mkdir -p -- "$cache_dir" "$dist_dir" "$project_dir/target"
download_verified "$mihomo_url" "$mihomo_archive" "$mihomo_sha256"
download_verified "$license_url" "$mihomo_license" "$license_sha256"
gzip -t "$mihomo_archive"

staging="$(mktemp -d "$project_dir/target/package.XXXXXXXX")"
trap 'rm -rf -- "$staging"' EXIT
bundle_name="mihoterm-v${crate_version}-linux-${release_arch}"
bundle_dir="$staging/$bundle_name"
install -d -m 0755 "$bundle_dir/licenses"
install -m 0755 "$binary" "$bundle_dir/mihoterm"
gzip -dc "$mihomo_archive" >"$bundle_dir/mihomo"
chmod 0755 "$bundle_dir/mihomo"
install -m 0644 "$project_dir/LICENSE" "$bundle_dir/LICENSE"
install -m 0644 "$project_dir/THIRD_PARTY_NOTICES.md" "$bundle_dir/THIRD_PARTY_NOTICES.md"
install -m 0644 "$project_dir/THIRD-PARTY-LICENSES.html" "$bundle_dir/THIRD-PARTY-LICENSES.html"
install -m 0644 "$project_dir/packaging/PORTABLE_README.md" "$bundle_dir/README.md"
install -m 0644 "$mihomo_license" "$bundle_dir/licenses/Mihomo-GPL-3.0.txt"

cat >"$bundle_dir/CORE-METADATA.txt" <<EOF
Program: Mihomo
Version: $mihomo_version
Upstream asset: $mihomo_asset
Upstream binary SHA-256: $mihomo_sha256
Source commit: $source_commit
Release: https://github.com/MetaCubeX/mihomo/releases/tag/$mihomo_version
Source: https://github.com/MetaCubeX/mihomo/tree/$mihomo_version
License: GPL-3.0
EOF
chmod 0644 "$bundle_dir/CORE-METADATA.txt"

archive="$dist_dir/${bundle_name}.tar.gz"
source_date_epoch="${SOURCE_DATE_EPOCH:-$(git -C "$project_dir" show -s --format=%ct HEAD)}"
tar \
  --sort=name \
  --mtime="@$source_date_epoch" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "$staging" \
  -cf - \
  "$bundle_name" |
  gzip -n >"$archive"

(
  cd "$dist_dir"
  sha256sum mihoterm-v*-linux-*.tar.gz | sort -k2 >SHA256SUMS
)

printf '%s\n' "$archive"
