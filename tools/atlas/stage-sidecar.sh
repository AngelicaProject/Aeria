#!/usr/bin/env sh
set -eu

target=linux-x64
if [ "$#" -ge 1 ]; then target=$1; fi
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
manifest="$repo_root/tools/atlas/version.json"
version=$(node -e 'const m=require(process.argv[1]); process.stdout.write(m.version)' "$manifest")
artifact=$(node -e 'const m=require(process.argv[1]); const a=m.artifacts[process.argv[2]]; process.stdout.write(JSON.stringify(a))' "$manifest" "$target")
name=$(node -e 'process.stdout.write(JSON.parse(process.argv[1]).name)' "$artifact")
expected_hash=$(node -e 'process.stdout.write(JSON.parse(process.argv[1]).sha256)' "$artifact")
archive_kind=$(node -e 'process.stdout.write(JSON.parse(process.argv[1]).archive)' "$artifact")
executable=$(node -e 'process.stdout.write(JSON.parse(process.argv[1]).executable)' "$artifact")
triple=$(node -e 'process.stdout.write(JSON.parse(process.argv[1]).targetTriple)' "$artifact")
base_url=$(node -e 'const m=require(process.argv[1]); process.stdout.write(m.releaseBaseUrl)' "$manifest")

work_root=/tmp/aeria-atlas-"$version"-"$target"
mkdir -p "$work_root/extract" "$repo_root/apps/desktop/src-tauri/binaries"
archive="$work_root/$name"
curl --fail --location --retry 2 --output "$archive" "$base_url/$name"
actual_hash=$(sha256sum "$archive" | awk '{print $1}')
test "$actual_hash" = "$expected_hash"

if [ "$archive_kind" = "zip" ]; then
  unzip -p "$archive" "$executable" > "$repo_root/apps/desktop/src-tauri/binaries/harmonia-atlas-$triple.exe"
else
  tar -xzf "$archive" -C "$work_root/extract" -- "$executable"
  cp "$work_root/extract/$executable" "$repo_root/apps/desktop/src-tauri/binaries/harmonia-atlas-$triple"
  chmod +x "$repo_root/apps/desktop/src-tauri/binaries/harmonia-atlas-$triple"
fi
