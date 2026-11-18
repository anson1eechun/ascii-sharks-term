#!/bin/sh
# scripts/npm-package.sh v0.2.0 <release assets dir> <output dir>
set -eu

tag=$1
assets=$2
out=$3
here=$(cd "$(dirname "$0")/.." && pwd)

mkdir -p "$out"
cp -R "$here/npm/bin" "$out/"
cp "$here/LICENSE" "$here/README.md" "$out/"
sed "s/\"version\": \".*\"/\"version\": \"${tag#v}\"/" "$here/npm/package.json" > "$out/package.json"

unpack=$(mktemp -d)
trap 'rm -rf "$unpack"' EXIT
for archive in "$assets"/sharks-"$tag"-*.tar.gz; do
  name=$(basename "$archive" .tar.gz)
  tar -xzf "$archive" -C "$unpack" "$name/sharks"
  mkdir -p "$out/vendor/${name#sharks-"$tag"-}"
  mv "$unpack/$name/sharks" "$out/vendor/${name#sharks-"$tag"-}/sharks"
done
