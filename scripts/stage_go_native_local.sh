#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

case "$(uname -s)" in
Darwin)
  ext=dylib
  case "$(uname -m)" in
  arm64 | aarch64) plat=macos-arm64 ;;
  *) plat=macos-x86_64 ;;
  esac
  ;;
Linux)
  ext=so
  case "$(uname -m)" in
  aarch64 | arm64) plat=linux-aarch64 ;;
  *) plat=linux-x86_64 ;;
  esac
  ;;
MINGW* | MSYS* | CYGWIN*)
  ext=dll
  plat=windows-x86_64
  ;;
*)
  echo "Unsupported platform: $(uname -s)" >&2
  exit 1
  ;;
esac

src="target/release/libxberg_ffi.${ext}"
if [ "$ext" = "dll" ]; then
  src="target/release/xberg_ffi.${ext}"
fi

if [ ! -f "$src" ]; then
  echo "ERROR: $src not found. Run: cargo build --release -p xberg-ffi" >&2
  exit 1
fi

dst_dir="packages/go/.lib/${plat}"
mkdir -p "$dst_dir"
cp -f "$src" "$dst_dir/"

echo "Staged $(basename "$src") -> $dst_dir/"
