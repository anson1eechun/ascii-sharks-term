#!/bin/sh
# scripts/formula.sh v0.2.0 path/to/SHA256SUMS > Formula/sharks.rb
set -eu

tag=$1
sums=$2
repo=anson1eechun/ascii-sharks-term

sha() {
  awk -v f="sharks-$tag-$1.tar.gz" '$2 == f { print $1; found = 1 } END { exit !found }' "$sums" ||
    { echo "formula.sh: no checksum for $1 in $sums" >&2; exit 1; }
}
url() { echo "https://github.com/$repo/releases/download/$tag/sharks-$tag-$1.tar.gz"; }

mac_arm=$(sha aarch64-apple-darwin)
mac_intel=$(sha x86_64-apple-darwin)
linux_arm=$(sha aarch64-unknown-linux-musl)
linux_intel=$(sha x86_64-unknown-linux-musl)

cat <<EOF
class Sharks < Formula
  desc "ASCII sharks and fish schools in your terminal"
  homepage "https://github.com/$repo"
  license "MIT"

  on_macos do
    on_arm do
      url "$(url aarch64-apple-darwin)"
      sha256 "$mac_arm"
    end
    on_intel do
      url "$(url x86_64-apple-darwin)"
      sha256 "$mac_intel"
    end
  end

  on_linux do
    on_arm do
      url "$(url aarch64-unknown-linux-musl)"
      sha256 "$linux_arm"
    end
    on_intel do
      url "$(url x86_64-unknown-linux-musl)"
      sha256 "$linux_intel"
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
