#!/usr/bin/env bash
# lib/mcos/hash/build.sh — build Hash mco (blake3 in zig).

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/../../.."   # repo root

zig build-exe \
  -target wasm32-freestanding \
  -fno-entry \
  -rdynamic \
  -O ReleaseFast \
  -fstrip \
  --dep moof \
  -Mmain=lib/mcos/hash/hash.zig \
  -Mmoof=lib/mcos/_lib/moof.zig \
  -femit-bin=lib/mcos/hash/hash.wasm

lib/mcos/_lib/pack-and-cache.sh hash \
  lib/mcos/hash/hash.wasm \
  lib/mcos/hash/manifest.moof

rm -f lib/mcos/hash/hash.wasm  # cleanup intermediate

# write the computed hash to hash.expected-hash for substrate build.rs
HASH=$(grep "core/hash" lib/mcos/index.moof | grep -oE '"[a-f0-9]{64}"' | tail -1 | tr -d '"')
echo "$HASH" > lib/mcos/hash/hash.expected-hash
echo "  -> hash.expected-hash: $HASH"
