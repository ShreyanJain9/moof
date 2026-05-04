#!/usr/bin/env bash
# lib/mcos/utf8/build.sh — build Utf8 mco (c implementation).
#
# requires: brew install llvm@21 lld@21
# apple clang does not ship with a wasm backend; llvm@21 + lld@21 do.
#
# CLANG override: set MOOF_CLANG env var to use a different clang.
# example: MOOF_CLANG=/path/to/clang ./build.sh

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/../../.."   # repo root

# prefer llvm@21 clang for wasm32 support; fall back to MOOF_CLANG env.
if [ -z "${MOOF_CLANG:-}" ]; then
  LLVM_PREFIX="$(brew --prefix llvm@21 2>/dev/null || true)"
  if [ -n "$LLVM_PREFIX" ] && [ -x "$LLVM_PREFIX/bin/clang" ]; then
    MOOF_CLANG="$LLVM_PREFIX/bin/clang"
  else
    MOOF_CLANG="clang"
  fi
fi

# wasm-ld comes from lld@21; add to PATH if available.
LLD_BIN="$(brew --prefix lld@21 2>/dev/null || true)/bin"
if [ -n "$LLD_BIN" ] && [ -d "$LLD_BIN" ]; then
  export PATH="$LLD_BIN:$PATH"
fi

"$MOOF_CLANG" \
  --target=wasm32-freestanding \
  -nostdlib \
  -Wl,--no-entry \
  -Wl,--export-dynamic \
  -O2 \
  -o lib/mcos/utf8/utf8.wasm \
  lib/mcos/utf8/utf8.c

lib/mcos/_lib/pack-and-cache.sh utf8 \
  lib/mcos/utf8/utf8.wasm \
  lib/mcos/utf8/manifest.moof

rm -f lib/mcos/utf8/utf8.wasm  # cleanup intermediate
