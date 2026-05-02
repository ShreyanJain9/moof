#!/usr/bin/env bash
# build.sh — easy zig→.mco build pipeline.
#
# usage:
#   ./build.sh hello
#   ./build.sh clock
#
# produces <name>.mco from <name>.zig in the current dir.
#   1. zig build-exe (target wasm32-wasi) → <name>.wasm
#   2. mco-pack appends a moof.manifest custom section → <name>.mco
#
# the manifest declares which exports become methods; the loader
# cross-validates manifest vs wasm exports before installing.
#
# auto-discovers exports declared with `export fn` in the source.

set -euo pipefail

NAME="${1:?usage: ./build.sh <name>  (without .zig)}"
SRC="$NAME.zig"
WASM="$NAME.wasm"
MCO="$NAME.mco"

if [[ ! -f "$SRC" ]]; then
    echo "no source: $SRC" >&2
    exit 1
fi

EXPORTS=$(grep -oE 'export fn [a-zA-Z_][a-zA-Z0-9_]*' "$SRC" \
    | awk '{print $3}' \
    | sort -u)

if [[ -z "$EXPORTS" ]]; then
    echo "no exports found in $SRC" >&2
    exit 1
fi

EXPORT_FLAGS=()
for e in $EXPORTS; do
    EXPORT_FLAGS+=("--export=$e")
done

echo "→ zig: $WASM  exports: $(echo $EXPORTS | tr '\n' ' ')"

zig build-exe \
    -target wasm32-wasi \
    -O ReleaseSmall \
    -fno-entry \
    "${EXPORT_FLAGS[@]}" \
    "$SRC"

# build the manifest as moof source-text. methods list is the
# discovered exports (in deterministic sorted order). parent is
# Object — the default. abi-version 1.
METHODS=$(echo $EXPORTS | tr '\n' ' ' | sed 's/ *$//')
MANIFEST="((abi-version 1) (parent Object) (methods ($METHODS)))"

# locate mco-pack — built once via cargo.
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PACK="$ROOT/target/debug/mco-pack"
if [[ ! -x "$PACK" ]]; then
    echo "→ building mco-pack (one-time)..."
    (cd "$ROOT" && cargo build -p mco-pack 2>&1 | tail -3)
fi

echo "→ pack: $WASM + manifest → $MCO"
"$PACK" "$WASM" "$MCO" "$MANIFEST"

ls -la "$MCO"
