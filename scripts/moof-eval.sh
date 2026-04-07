#!/bin/bash
# Evaluate moof expressions and return clean output.
# Usage: ./scripts/moof-eval.sh 'expr1' ['expr2' ...]
# Returns: "=> value" for results, "!! error" for errors, quoted strings for print output

set -euo pipefail
cd "$(dirname "$0")/.."

input=""
for arg in "$@"; do
    input+="$arg"$'\n'
done

echo "$input" | cargo run 2>&1 | awk '
    /^moof>/ {
        sub(/^moof> */, "")
        if ($0 == "" || $0 ~ /\(saved\)/ || $0 ~ /^\.\.\. /) next
        print
        next
    }
    /^"/ { print; next }
    /^=> / { print; next }
    /^!! / { print; next }
'
