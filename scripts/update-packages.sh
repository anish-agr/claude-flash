#!/usr/bin/env bash
# Points the Scoop manifest and the Homebrew formula at a published release, using
# the checksums the release workflow uploaded.
#
#   scripts/update-packages.sh 2.2.0
set -euo pipefail

version="${1:?usage: scripts/update-packages.sh VERSION}"
root="$(cd "$(dirname "$0")/.." && pwd)"
sums="$(curl -fsSL "https://github.com/anish-agr/claude-flash/releases/download/v$version/SHA256SUMS")"

hash_of() {
  printf '%s\n' "$sums" | awk -v file="$1" '$2 == file { print $1 }'
}
windows="$(hash_of claude-flash-windows-x64.zip)"
macos="$(hash_of claude-flash-macos-universal.tar.gz)"
if [ -z "$windows" ] || [ -z "$macos" ]; then
  echo "SHA256SUMS for v$version does not list both archives" >&2
  exit 1
fi

# Versions start with a digit, which keeps the autoupdate URL's v$version intact.
sed -i.bak -E \
  -e "s/\"version\": \"[0-9][^\"]*\"/\"version\": \"$version\"/" \
  -e "s#/download/v[0-9][^/]*/#/download/v$version/#" \
  -e "s/\"hash\": \"[0-9a-f]{64}\"/\"hash\": \"$windows\"/" \
  "$root/packaging/scoop/claude-flash.json"
sed -i.bak -E \
  -e "s#/download/v[0-9][^/]*/#/download/v$version/#" \
  -e "s/sha256 \"[0-9a-f]{64}\"/sha256 \"$macos\"/" \
  "$root/packaging/homebrew/claude-flash.rb"
rm -f "$root/packaging/scoop/claude-flash.json.bak" "$root/packaging/homebrew/claude-flash.rb.bak"

echo "Scoop manifest and Homebrew formula now point at v$version."
echo "Copy packaging/homebrew/claude-flash.rb to Formula/claude-flash.rb in anish-agr/homebrew-tap."
