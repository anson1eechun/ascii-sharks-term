#!/bin/sh
# After the Release workflow has built vX.Y.Z, publish it to the Homebrew tap
# and to npm. Safe to re-run: parts already published are skipped.
# Needs gh logged in, and npm logged in for the npm part.
#
#   scripts/publish.sh v0.1.0 [--dry-run]
set -eu

tag=${1:?usage: scripts/publish.sh vX.Y.Z [--dry-run]}
dry=${2:-}
version=${tag#v}
repo=anson1eechun/ascii-sharks-term
tap=anson1eechun/homebrew-tap
package=ascii-sharks-term
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl"
here=$(cd "$(dirname "$0")/.." && pwd)

cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/rs/Cargo.toml" | head -n 1)
if [ "$cargo_version" != "$version" ]; then
  echo "rs/Cargo.toml says $cargo_version, not $version" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gh release download "$tag" --repo "$repo" --dir "$tmp/assets"
(cd "$tmp/assets" && shasum -a 256 -c SHA256SUMS)

# ── Homebrew: a formula pointing at the release assets ──────────────────
sha() { awk -v f="sharks-$tag-$1.tar.gz" '$2 == f { print $1 }' "$tmp/assets/SHA256SUMS"; }
url() { echo "https://github.com/$repo/releases/download/$tag/sharks-$tag-$1.tar.gz"; }

gh repo clone "$tap" "$tmp/tap" -- --quiet
mkdir -p "$tmp/tap/Formula"
cat > "$tmp/tap/Formula/sharks.rb" <<EOF
class Sharks < Formula
  desc "ASCII sharks and fish schools in your terminal"
  homepage "https://github.com/$repo"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
      url "$(url aarch64-apple-darwin)"
      sha256 "$(sha aarch64-apple-darwin)"
    end
    on_intel do
      url "$(url x86_64-apple-darwin)"
      sha256 "$(sha x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$(url aarch64-unknown-linux-musl)"
      sha256 "$(sha aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "$(url x86_64-unknown-linux-musl)"
      sha256 "$(sha x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "sharks"
  end

  test do
    assert_match "greathammer", shell_output("#{bin}/sharks --list")
  end
end
EOF

git -C "$tmp/tap" add Formula/sharks.rb
if git -C "$tmp/tap" diff --cached --quiet; then
  echo "homebrew: formula already at $version"
elif [ "$dry" = --dry-run ]; then
  cat "$tmp/tap/Formula/sharks.rb"
else
  git -C "$tmp/tap" -c user.name="$(git -C "$here" config user.name)" \
    -c user.email="$(git -C "$here" config user.email)" commit --quiet -m "sharks $version"
  git -C "$tmp/tap" push --quiet origin HEAD:main
  echo "homebrew: published sharks $version"
fi

# ── npm: the launcher plus every target's binary ────────────────────────
if [ "$(npm view "$package@$version" version 2>/dev/null)" = "$version" ]; then
  echo "npm: $package@$version already published"
  exit 0
fi
pkg=$tmp/npm
mkdir -p "$pkg"
cp -R "$here/npm/bin" "$pkg/"
cp "$here/LICENSE" "$here/README.md" "$pkg/"
sed "s/\"version\": \".*\"/\"version\": \"$version\"/" "$here/npm/package.json" > "$pkg/package.json"
for t in $targets; do
  mkdir -p "$pkg/vendor/$t"
  tar -xzf "$tmp/assets/sharks-$tag-$t.tar.gz" -C "$tmp" "sharks-$tag-$t/sharks"
  mv "$tmp/sharks-$tag-$t/sharks" "$pkg/vendor/$t/sharks"
done
if [ "$dry" = --dry-run ]; then
  (cd "$pkg" && npm pack --dry-run)
elif ! npm whoami >/dev/null 2>&1; then
  echo "npm: not logged in; run 'npm login', then re-run this script" >&2
  exit 1
else
  (cd "$pkg" && npm publish --access public)
  echo "npm: published $package@$version"
fi
