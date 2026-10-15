#!/usr/bin/env bash
set -euo pipefail

skill_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$skill_dir/assets/runtime.json"
version="$(sed -n 's/^[[:space:]]*"version": *"\([^"]*\)".*/\1/p' "$manifest")"
repository="$(sed -n 's/^[[:space:]]*"repository": *"\([^"]*\)".*/\1/p' "$manifest")"
fail() { printf '%s\n' "Bulbul: $*" >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]] || fail 'Invalid runtime version.'
[[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail 'Invalid distribution repository.'

# Use the generated, compact discovery contract without downloads or credentials.
if [[ "${1:-}" == list ]]; then
  shift
  schemas="$skill_dir/assets/tools.jsonl"
  selected=()
  if [[ $# -eq 0 ]]; then
    while IFS= read -r line; do selected+=("${line%$'\r'}"); done < "$schemas"
  else
    for name in "$@"; do
      if [[ "$name" == --help || "$name" == -h ]]; then printf '%s\n' 'Usage: bulbul list [TOOL ...]'; exit 0; fi
      [[ "$name" =~ ^[a-z][a-z0-9_]*$ ]] || { printf '%s\n' '{"error":"命令参数无效，请运行 bulbul list --help 查看用法。","code":"invalid_arguments"}' >&2; exit 2; }
      line="$(sed -n '/^{"name":"'"$name"'",/p' "$schemas")"
      [[ -n "$line" ]] || { printf '%s\n' '{"error":"没有这个 Bulbul 工具，请运行 list 查看可用工具。","code":"operation_failed"}' >&2; exit 1; }
      selected+=("${line%$'\r'}")
    done
  fi
  printf '['
  separator=''
  for line in "${selected[@]}"; do printf '%s%s' "$separator" "$line"; separator=,; done
  printf ']\n'
  exit 0
fi

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
cache_root="${BULBUL_CACHE_DIR:-}"
if [[ -z "$cache_root" ]]; then
  directory="$PWD"
  while [[ -n "$directory" ]]; do
    if [[ -e "$directory/.git" ]]; then cache_root="$directory/.tmp/bulbul"; break; fi
    [[ "$directory" != / ]] || break
    directory="$(dirname -- "$directory")"
  done
fi
if [[ -z "$cache_root" ]]; then
  case "$os" in
    Darwin) cache_root="$HOME/Library/Caches/bulbul-birding" ;;
    MINGW*|MSYS*) [[ -n "${LOCALAPPDATA:-}" ]] || fail 'Cannot locate the cache. Run in a writable project or set BULBUL_CACHE_DIR.'; cache_root="$LOCALAPPDATA/bulbul-birding/cache" ;;
    *) cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/bulbul-birding" ;;
  esac
fi
if [[ "$suffix" == .exe ]]; then cache_root="$(cygpath -u "$cache_root")"; fi
cache_dir="$cache_root/$version/$target"
binary="$cache_dir/bulbul$suffix"
digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
expected_digest() { awk -v name="$asset" '$2 == name && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ {print tolower($1)}' "$1"; }
for candidate in "$binary" "$skill_dir/bin/$version/$target/bulbul$suffix"; do
 if [[ -f "$candidate" && -f "$candidate.sha256" ]]; then
  expected="$(expected_digest "$candidate.sha256")"
  if [[ -n "$expected" && "$(digest "$candidate")" == "$expected" ]]; then
    [[ "$("$candidate" --version 2>/dev/null || true)" == "bulbul $version" ]] || fail 'Cached executable has an unexpected version.'
    exec "$candidate" "$@"
  fi
 fi
done

command -v curl >/dev/null 2>&1 || fail 'Please install curl to download the executable.'
mkdir -p "$cache_dir" || fail 'Program cache is not writable. Run in a writable project or set BULBUL_CACHE_DIR; your AI tool must allow access to that directory.'
partial="$(mktemp "$cache_dir/.download.XXXXXXXX")" || fail 'Program cache is not writable. Run in a writable project or set BULBUL_CACHE_DIR; your AI tool must allow access to that directory.'
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
