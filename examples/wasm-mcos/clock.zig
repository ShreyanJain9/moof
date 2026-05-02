//! core/clock — a wall-clock + monotonic-clock wasm mco written in zig.
//!
//! exports two methods. each is a wasm export that the substrate
//! wraps into a moof method-Form on the loaded proto.
//!
//!   [$clock now]            → wall-clock nanoseconds since unix epoch
//!   [$clock monotonic]      → monotonic nanoseconds since process start
//!
//! moof side:
//!   (def Clock (__loadWasmMco "examples/wasm-mcos/clock.wasm"))
//!   (def $clock [Clock new])
//!   [$clock now]              ;; → 1735689600123456789
//!   [$clock monotonic]        ;; → 12345
//!
//! per `docs/reference/mco-format.md`, the mco doesn't name itself.
//! the moof code that loads it picks any name (`Clock` here, but
//! could be `TimeSource`, `Watch`, anything).

const moof = @import("lib/moof.zig");

export fn now() i64 {
    return moof.now_ns();
}

export fn monotonic() i64 {
    return moof.monotonic_ns();
}
