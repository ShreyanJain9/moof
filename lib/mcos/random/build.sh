#!/usr/bin/env bash
# lib/mcos/random/build.sh — build Random mco.

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
  -Mmain=lib/mcos/random/random.zig \
  -Mmoof=lib/mcos/_lib/moof.zig \
  -femit-bin=lib/mcos/random/random.wasm

lib/mcos/_lib/pack-and-cache.sh random \
  lib/mcos/random/random.wasm \
  lib/mcos/random/manifest.moof

rm -f lib/mcos/random/random.wasm  # cleanup intermediate
