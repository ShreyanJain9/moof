// lib/mcos/utf8/utf8.c — utf-8 codepoint validation + length.
//
// second-language mco (c) — validates the wasm mco abi from c.
//
// exports (wasm export name = moof selector verbatim):
//   valid?:(bytes_handle: u32) -> i64   1 if valid utf-8, 0 otherwise
//   length:(bytes_handle: u32) -> i64   codepoint count; raises 'utf8-invalid
//
// both return i64 — the trampoline marshals i64 as moof Int directly.
// i32 returns are treated as handle-table indices (for heap-allocated
// values), so integer results must use i64.
//
// note on export-name convention:
//   zig keeps colons literal (encode:, decode:, seedFrom:). this c
//   binding does the same: export names = moof selectors verbatim.
//   '?' is a valid wasm identifier character and is kept as-is.
//   the manifest lists selectors verbatim; the host matches them
//   by string equality against the wasm export table.

#include "../_lib/moof.h"

static uint8_t io_buf[65536];

// ── valid?: ───────────────────────────────────────────────────────
// returns 1 if the bytes form contains well-formed utf-8, 0 otherwise.
// returns i64 so the trampoline marshals it as a moof Int directly
// (i32 returns are treated as handle-table indices, not integers).

__attribute__((export_name("valid?:")))
int64_t valid_q_col_(uint32_t bytes_handle) {
    size_t n = moof_bytes_data(bytes_handle, io_buf, sizeof io_buf);
    if (n > sizeof io_buf) n = sizeof io_buf;
    size_t i = 0;
    while (i < n) {
        uint8_t b = io_buf[i];
        size_t need;
        if      ((b & 0x80) == 0x00) need = 1;
        else if ((b & 0xE0) == 0xC0) need = 2;
        else if ((b & 0xF0) == 0xE0) need = 3;
        else if ((b & 0xF8) == 0xF0) need = 4;
        else return 0;
        if (i + need > n) return 0;
        for (size_t j = 1; j < need; j++) {
            if ((io_buf[i + j] & 0xC0) != 0x80) return 0;
        }
        i += need;
    }
    return 1;
}

// ── length: ───────────────────────────────────────────────────────
// returns count of unicode codepoints.
// raises 'utf8-invalid on malformed input.

__attribute__((export_name("length:")))
int64_t length_col_(uint32_t bytes_handle) {
    size_t n = moof_bytes_data(bytes_handle, io_buf, sizeof io_buf);
    if (n > sizeof io_buf) n = sizeof io_buf;
    int64_t count = 0;
    size_t i = 0;
    while (i < n) {
        uint8_t b = io_buf[i];
        size_t need;
        if      ((b & 0x80) == 0x00) need = 1;
        else if ((b & 0xE0) == 0xC0) need = 2;
        else if ((b & 0xF0) == 0xE0) need = 3;
        else if ((b & 0xF8) == 0xF0) need = 4;
        else moof_raise_kind("utf8-invalid", "invalid utf-8 lead byte");
        if (i + need > n) moof_raise_kind("utf8-invalid", "truncated utf-8 sequence");
        for (size_t j = 1; j < need; j++) {
            if ((io_buf[i + j] & 0xC0) != 0x80) {
                moof_raise_kind("utf8-invalid", "bad utf-8 continuation byte");
            }
        }
        count++;
        i += need;
    }
    return count;
}
