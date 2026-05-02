#!/usr/bin/env bash
# build.sh — easy zig→wasm-mco build command.
#
# usage:
#   ./build.sh hello
#   ./build.sh clock
#
# produces <name>.wasm from <name>.zig in the current dir.
# auto-discovers exports declared with `export fn` in the source.
# bundles in lib/moof.zig as the `moof` zig module.

set -euo pipefail

NAME="${1:?usage: ./build.sh <name>  (without .zig)}"
SRC="$NAME.zig"
OUT="$NAME.wasm"

if [[ ! -f "$SRC" ]]; then
    echo "no source: $SRC" >&2
    exit 1
fi

# discover exports — every `export fn NAME(` in the source.
EXPORTS=$(grep -oE 'export fn [a-zA-Z_][a-zA-Z0-9_]*' "$SRC" \
    | awk '{print $3}' \
    | sort -u)

if [[ -z "$EXPORTS" ]]; then
    echo "no exports found in $SRC" >&2
    exit 1
fi

# build the --export=… flags.
EXPORT_FLAGS=()
for e in $EXPORTS; do
    EXPORT_FLAGS+=("--export=$e")
done

echo "→ $OUT  exports: $(echo $EXPORTS | tr '\n' ' ')"

zig build-exe \
    -target wasm32-wasi \
    -O ReleaseSmall \
    -fno-entry \
    "${EXPORT_FLAGS[@]}" \
    "$SRC"

ls -la "$OUT"
