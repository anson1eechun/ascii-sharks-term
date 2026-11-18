#!/bin/sh
# Assembles the npm package for a release from its downloaded assets: the
# launcher, every target's binary, and the version taken from the tag.
#
#   scripts/npm-package.sh v0.2.0 <assets dir> <output dir>
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
for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  tar -xzf "$assets/sharks-$tag-$t.tar.gz" -C "$unpack" "sharks-$tag-$t/sharks"
  mkdir -p "$out/vendor/$t"
  mv "$unpack/sharks-$tag-$t/sharks" "$out/vendor/$t/sharks"
done
