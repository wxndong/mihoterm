#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=build-limits.sh
source "$project_dir/scripts/build-limits.sh"
target="${1:-x86_64-unknown-linux-musl}"

case "$target" in
  x86_64-unknown-linux-musl | aarch64-unknown-linux-musl | armv7-unknown-linux-musleabihf) ;;
  *)
    echo "unsupported portable target: $target" >&2
    exit 2
    ;;
esac

for command_name in cargo cargo-zigbuild file rustup zig; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "required release tool is missing: $command_name" >&2
    exit 2
  fi
done

if ! rustup target list --installed | grep -Fxq "$target"; then
  echo "Rust target is not installed: $target" >&2
  echo "Run: rustup target add $target" >&2
  exit 2
fi

cd "$project_dir"
cargo zigbuild --release --locked --target "$target"

binary="$project_dir/target/$target/release/mihoterm"
description="$(file -b "$binary")"
if [[ "$description" != *"statically linked"* ]]; then
  echo "portable build is not statically linked: $description" >&2
  exit 1
fi

printf '%s\n' "$binary"
