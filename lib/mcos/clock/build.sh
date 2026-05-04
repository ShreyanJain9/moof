#!/usr/bin/env bash
# lib/mcos/clock/build.sh — build Clock mco.
#
# clock uses wasm32-wasi (needs WASI clock_time_get).
# the substrate links in wasmtime-wasi so WASI imports resolve.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/../../.."   # repo root

zig build-exe \
  -target wasm32-wasi \
  -fno-entry \
  -rdynamic \
  -O ReleaseFast \
  -fstrip \
  -Mmain=lib/mcos/clock/clock.zig \
  -femit-bin=lib/mcos/clock/clock.wasm

lib/mcos/_lib/pack-and-cache.sh clock \
  lib/mcos/clock/clock.wasm \
  lib/mcos/clock/manifest.moof

rm -f lib/mcos/clock/clock.wasm  # cleanup intermediate
