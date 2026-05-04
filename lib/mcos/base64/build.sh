#!/usr/bin/env bash
# lib/mcos/base64/build.sh — build Base64 mco.

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
  -Mmain=lib/mcos/base64/base64.zig \
  -Mmoof=lib/mcos/_lib/moof.zig \
  -femit-bin=lib/mcos/base64/base64.wasm

lib/mcos/_lib/pack-and-cache.sh base64 \
  lib/mcos/base64/base64.wasm \
  lib/mcos/base64/manifest.moof

rm -f lib/mcos/base64/base64.wasm  # cleanup intermediate
