#!/bin/sh
# Differential test: the Rust port must draw exactly what upstream's TypeScript
# draws, cell for cell, on the same seed. Compares 34 frames over the first ~2 s
# (after that, 1-ulp libm differences get amplified by the chaotic fish sim and
# the runs drift apart, which is expected), half of them with eating and
# respawning forced early.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

(cd "$here/../rs" && cargo build --release --tests -q)

frames=0 bad=0
for steps in $(seq 2 7 116); do
  for force in "" 1; do
    SHARKS_DUMP_FORCE=$force bun "$here/upstream-dump.ts" "$tmp/ts.txt" "$steps"
    (cd "$here/../rs" && SHARKS_DUMP_FORCE=$force SHARKS_DUMP="$tmp/rs.txt" SHARKS_DUMP_STEPS=$steps \
      cargo test --release -q dump_frame >/dev/null 2>&1)
    frames=$((frames + 1))
    if ! cmp -s "$tmp/ts.txt" "$tmp/rs.txt"; then
      bad=$((bad + 1))
      echo "MISMATCH at $steps steps${force:+ (forced)}:"
      diff "$tmp/ts.txt" "$tmp/rs.txt" | head -5
    fi
  done
done
echo "$frames frames compared, $bad mismatched"
[ "$bad" -eq 0 ]
