#!/bin/sh
# Manual fallback for what the Release workflow does after building: publish a
# release to the Homebrew tap and to npm. Safe to re-run: parts already
# published are skipped. The npm part needs `npm login` and, with 2FA, an
# interactive terminal.
#
#   scripts/publish.sh v0.2.0 [--dry-run]
set -eu

tag=${1:?usage: scripts/publish.sh vX.Y.Z [--dry-run]}
dry=${2:-}
version=${tag#v}
repo=anson1eechun/ascii-sharks-term
tap=anson1eechun/homebrew-tap
package=ascii-sharks-term
here=$(cd "$(dirname "$0")/.." && pwd)

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gh release download "$tag" --repo "$repo" --dir "$tmp/assets"
(cd "$tmp/assets" && shasum -a 256 -c SHA256SUMS)

# ── Homebrew ─────────────────────────────────────────────────────────────
gh repo clone "$tap" "$tmp/tap" -- --quiet
mkdir -p "$tmp/tap/Formula"
"$here/scripts/formula.sh" "$tag" "$tmp/assets/SHA256SUMS" > "$tmp/tap/Formula/sharks.rb"
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

# ── npm ──────────────────────────────────────────────────────────────────
if [ "$(npm view "$package@$version" version 2>/dev/null)" = "$version" ]; then
  echo "npm: $package@$version already published"
  exit 0
fi
"$here/scripts/npm-package.sh" "$tag" "$tmp/assets" "$tmp/npm"
if [ "$dry" = --dry-run ]; then
  (cd "$tmp/npm" && npm pack --dry-run)
elif ! npm whoami >/dev/null 2>&1; then
  echo "npm: not logged in; run 'npm login', then re-run this script" >&2
  exit 1
else
  (cd "$tmp/npm" && npm publish --access public)
  echo "npm: published $package@$version"
fi
