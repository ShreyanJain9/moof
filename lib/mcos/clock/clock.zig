//! lib/mcos/clock/clock.zig — wall-clock + monotonic-clock wasm mco.
//!
//! exports four functions:
//!   now()        → wall-clock nanoseconds since unix epoch  (i64)
//!   monotonic()  → monotonic nanoseconds since process start (i64)
//!   next()       → alias for now()  (DataSource: polled-flavor)
//!   peek()       → alias for now()  (DataSource: polled-flavor)
//!
//! compiled for wasm32-wasi; uses `std.os.wasi.clock_time_get`
//! directly.

const std = @import("std");
const wasi = std.os.wasi;

// ── internal clock helper ─────────────────────────────────────────

inline fn clock_get(id: wasi.clockid_t) i64 {
    var ts: wasi.timestamp_t = 0;
    // precision hint = 1us; wasmtime typically gives ns precision.
    _ = wasi.clock_time_get(id, 1000, &ts);
    return @intCast(ts);
}

// ── exports ───────────────────────────────────────────────────────

export fn now() i64 {
    return clock_get(.REALTIME);
}

export fn monotonic() i64 {
    return clock_get(.MONOTONIC);
}

/// DataSource: polled-flavor.  `next` reads wall-clock (same as now).
export fn next() i64 {
    return clock_get(.REALTIME);
}

/// DataSource: polled-flavor.  `peek` reads wall-clock without
/// advancing state (for a polled source, peek == next == now).
export fn peek() i64 {
    return clock_get(.REALTIME);
}
