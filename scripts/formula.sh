#!/bin/sh
# Prints the Homebrew formula for a release, from that release's SHA256SUMS.
#
#   scripts/formula.sh v0.2.0 path/to/SHA256SUMS > Formula/sharks.rb
set -eu

tag=$1
sums=$2
repo=anson1eechun/ascii-sharks-term

sha() { awk -v f="sharks-$tag-$1.tar.gz" '$2 == f { print $1 }' "$sums"; }
url() { echo "https://github.com/$repo/releases/download/$tag/sharks-$tag-$1.tar.gz"; }

for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  if [ -z "$(sha "$t")" ]; then
    echo "formula.sh: no checksum for $t in $sums" >&2
    exit 1
  fi
done

cat <<EOF
class Sharks < Formula
  desc "ASCII sharks and fish schools in your terminal"
  homepage "https://github.com/$repo"
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
