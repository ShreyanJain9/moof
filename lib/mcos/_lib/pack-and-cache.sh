#!/usr/bin/env bash
# lib/mcos/_lib/pack-and-cache.sh — shared steps for every mco's build.sh.
#
# usage: pack-and-cache.sh <name> <wasm-file> <manifest-path>
#
# 1. invoke mco-pack pack to produce <name>.mco
# 2. compute b3sum of the .mco
# 3. move to .moof/mcos/cache/<hash>.mco
# 4. update lib/mcos/index.moof via mco-pack index-update

set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 <name> <wasm-file> <manifest-path>" >&2
    exit 2
fi

NAME="$1"
WASM_FILE="$2"
MANIFEST_PATH="$3"

CACHE_DIR=".moof/mcos/cache"
MCO_PACK="cargo run --quiet --release --bin mco-pack --"

mkdir -p "$CACHE_DIR"

TMP_MCO="$(dirname "$WASM_FILE")/${NAME}.mco"
$MCO_PACK pack "$WASM_FILE" "$TMP_MCO" "$MANIFEST_PATH"

HASH=$(b3sum "$TMP_MCO" | cut -d' ' -f1)
mv "$TMP_MCO" "$CACHE_DIR/$HASH.mco"

$MCO_PACK index-update "core/$NAME" "$HASH"

echo "  -> $CACHE_DIR/$HASH.mco"
