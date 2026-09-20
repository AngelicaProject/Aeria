#!/usr/bin/env sh
set -eu

target=linux-x64
if [ "$#" -ge 1 ]; then target=$1; fi
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
manifest="$repo_root/tools/atlas/version.json"
version=$(node -e 'const m=require(process.argv[1]); process.stdout.write(m.version)' "$manifest")
triple=$(node -e 'const m=require(process.argv[1]); process.stdout.write(m.artifacts[process.argv[2]].targetTriple)' "$manifest" "$target")
binary="$repo_root/apps/desktop/src-tauri/binaries/harmonia-atlas-$triple"

test -f "$binary"
actual_version=$("$binary" --version)
test "$actual_version" = "$version"

help_output=$("$binary" --help 2>&1)
printf '%s\n' "$help_output" | grep -Eq 'package[[:space:]]+--game-path.*--language.*--output.*--events[[:space:]]+jsonl'

set +e
package_help_output=$("$binary" package --help 2>&1)
package_help_status=$?
set -e
printf '%s\n' "$package_help_output" | grep -Eq -- '--events[[:space:]]+jsonl'

probe_root=$(mktemp -d)
trap 'rm -rf "$probe_root"' EXIT
set +e
probe_output=$("$binary" package \
  --game-path "$probe_root/empty-game" \
  --language en \
  --output "$probe_root/source.hsp" \
  --events jsonl 2>&1)
probe_status=$?
set -e
if printf '%s\n' "$probe_output" | grep -Eiq 'unknown (command|option)|unrecognized option|unexpected argument'; then
  printf '%s\n' "$probe_output" >&2
  exit 1
fi

printf 'Verified Harmonia Atlas %s package JSONL contract at %s (package --help exit %s, argument probe exit %s)\n' \
  "$version" "$binary" "$package_help_status" "$probe_status"
