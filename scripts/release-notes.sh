#!/bin/sh
# Prints the CHANGELOG.md section for version $1; fails if there is none.
awk -v v="$1" '
  /^## / { if (found) exit; found = index($0, "## " v " ") == 1; next }
  found && (started || NF) { started = 1; print }
  END { if (!found) exit 1 }
' "$(dirname "$0")/../CHANGELOG.md"
