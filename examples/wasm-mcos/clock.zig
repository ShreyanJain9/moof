//! core/clock — wall-clock + monotonic-clock wasm mco.
//!
//! exports two methods. each calls WASI's `clock_time_get`
//! directly — the substrate provides WASI as the standard
//! system-services interface; moof's own imports namespace is
//! reserved for moof-specific primitives.
//!
//!   [$clock now]            → wall-clock nanoseconds since unix epoch
//!   [$clock monotonic]      → monotonic nanoseconds since process start
//!
//! moof side:
//!   (def Clock (__loadWasmMco "examples/wasm-mcos/clock.wasm"))
//!   (def $clock [Clock new])
//!   [$clock now]              ;; → 1735689600123456789
//!   [$clock monotonic]        ;; → some-large-number-going-up
//!
//! per `docs/reference/mco-format.md`, the mco doesn't name itself.
//! the moof code that loads it picks any name (`Clock` here, but
//! could be `TimeSource`, `Watch`, anything).

const std = @import("std");
const wasi = std.os.wasi;

inline fn clock_get(id: wasi.clockid_t) i64 {
    var ts: wasi.timestamp_t = 0;
    // precision = 1us is fine for both clocks; the WASI host
    // (wasmtime) honors it as a hint, returns ns precision in
    // practice.
    _ = wasi.clock_time_get(id, 1000, &ts);
    return @intCast(ts);
}

export fn now() i64 {
    return clock_get(.REALTIME);
}

export fn monotonic() i64 {
    return clock_get(.MONOTONIC);
}
