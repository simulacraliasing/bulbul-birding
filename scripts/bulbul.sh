#!/usr/bin/env bash
set -euo pipefail

skill_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$skill_dir/assets/runtime.json"
version="$(sed -n 's/^[[:space:]]*"version": *"\([^"]*\)".*/\1/p' "$manifest")"
repository="$(sed -n 's/^[[:space:]]*"repository": *"\([^"]*\)".*/\1/p' "$manifest")"
fail() { printf '%s\n' "Bulbul: $*" >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]] || fail 'Invalid runtime version.'
[[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail 'Invalid distribution repository.'

os="$(uname -s)"
arch="$(uname -m)"
suffix=''
case "$os/$arch" in
  Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
  MINGW*/x86_64|MSYS*/x86_64) target=x86_64-pc-windows-msvc; suffix=.exe ;;
  *) fail "Unsupported platform: $os/$arch." ;;
esac

# A locally built or packaged executable can be used without a download.
local_binary="$skill_dir/bin/bulbul$suffix"
if [[ -f "$local_binary" ]] && [[ "$("$local_binary" --version 2>/dev/null || true)" == "bulbul $version" ]]; then
  exec "$local_binary" "$@"
fi

asset="bulbul-$version-$target$suffix"
cache_dir="$skill_dir/bin/$version/$target"
binary="$cache_dir/bulbul$suffix"
digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
expected_digest() { awk -v name="$asset" '$2 == name && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ {print tolower($1)}' "$1"; }
if [[ -f "$binary" && -f "$binary.sha256" ]]; then
  expected="$(expected_digest "$binary.sha256")"
  if [[ -n "$expected" && "$(digest "$binary")" == "$expected" ]]; then
    chmod +x "$binary"
    [[ "$("$binary" --version 2>/dev/null || true)" == "bulbul $version" ]] || fail 'Cached executable has an unexpected version.'
    exec "$binary" "$@"
  fi
fi

command -v curl >/dev/null 2>&1 || fail 'Please install curl to download the executable.'
mkdir -p "$cache_dir"
partial="$(mktemp "$cache_dir/.download.XXXXXXXX")"
checksum="$partial.sha256"
trap 'rm -f -- "$partial" "$checksum"' EXIT
base="https://github.com/$repository/releases/download/v$version"
printf '%s\n' "Bulbul: preparing version $version for $target..." >&2
download() {
  curl --fail --location --silent --show-error --proto '=https' --proto-redir '=https' \
    --connect-timeout 10 --max-time 120 --max-filesize 67108864 --output "$2" "$1"
}
download "$base/$asset.sha256" "$checksum" || fail "Version $version is not available yet. See https://github.com/$repository/releases."
expected="$(expected_digest "$checksum")"
[[ -n "$expected" && "$expected" != *$'\n'* ]] || fail 'Invalid executable checksum.'
download "$base/$asset" "$partial" || fail 'Executable download failed. Try again later.'
[[ "$(digest "$partial")" == "$expected" ]] || fail 'Executable checksum verification failed.'
chmod +x "$partial"
[[ "$("$partial" --version 2>/dev/null || true)" == "bulbul $version" ]] || fail 'Downloaded executable has an unexpected version.'
mv -f -- "$partial" "$binary"
mv -f -- "$checksum" "$binary.sha256"
trap - EXIT
exec "$binary" "$@"
