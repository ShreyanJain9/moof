//! moof.zig — the zig-side abi for moof wasm mcos.
//!
//! a wasm mco written in zig imports this module:
//!
//!     const moof = @import("moof");
//!
//!     export fn now() i64 {
//!         return moof.now_ns();
//!     }
//!
//! and that's it — `moof.now_ns()` resolves at instantiation time
//! to the substrate's clock, the `export fn now` becomes a method
//! on the proto-Form that `[$mco load:]` returns.
//!
//! see `docs/reference/mco-format.md` for the full mco model.
//! see `crates/substrate/src/wasm.rs` for the substrate side.

// ── substrate-provided imports ───────────────────────────────────
//
// every fn here is a `func_wrap`-ed function in the substrate's
// wasmtime Linker, namespaced under the "moof" wasm module.
// declaring them as `extern "moof"` makes them resolve at
// instantiation time.
//
// stable abi: this list grows monotonically; signatures are
// versioned via abi-version in the mco manifest.

/// wall-clock nanoseconds since unix epoch. NOT deterministic;
/// avoid in replicated vats (or use the replication-friendly
/// path that goes through a deterministic-time cap).
pub extern "moof" fn now_ns() i64;

/// monotonic nanoseconds since some unspecified process-local
/// epoch. for measuring durations. must not be compared across
/// vats.
pub extern "moof" fn monotonic_ns() i64;

// ── derived helpers (pure zig) ───────────────────────────────────
//
// these add ergonomics on top of the raw imports. zero overhead
// (zig inlines them when ReleaseSmall/ReleaseFast).

/// wall-clock microseconds since unix epoch. convenience wrapper
/// for the common case where ns precision is overkill.
pub inline fn now_us() i64 {
    return @divTrunc(now_ns(), 1000);
}

/// wall-clock milliseconds since unix epoch.
pub inline fn now_ms() i64 {
    return @divTrunc(now_ns(), 1_000_000);
}

/// duration in ns of executing a closure-shaped fn. returns the
/// number of nanoseconds elapsed.
pub inline fn timeFn(comptime f: anytype) i64 {
    const start = monotonic_ns();
    f();
    return monotonic_ns() - start;
}

// ── method export macro ──────────────────────────────────────────
//
// zig doesn't have lisp-style macros but `comptime` + `inline fn`
// gets close. for more elaborate "macro" surface (e.g. auto-
// generating `extern "C"` boilerplate, validating method names,
// emitting custom-section manifests at compile time), grow this
// section. the simple `export fn` shape is enough for now.
