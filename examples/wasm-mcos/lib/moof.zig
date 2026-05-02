//! moof.zig — the zig-side abi for moof wasm mcos.
//!
//! a wasm mco written in zig imports this module:
//!
//!     const moof = @import("lib/moof.zig");
//!
//! the moof substrate provides imports under the "moof" wasm
//! namespace — these are *moof-specific* primitives only (slot,
//! slotSet, send, raise, intern, make-string, etc). standard
//! system services (clocks, filesystems, network) come through
//! WASI; mcos that need them speak WASI directly via
//! `wasi_snapshot_preview1`.
//!
//! see `crates/substrate/src/wasm.rs::install_moof_imports` for
//! the substrate side. see `docs/reference/mco-format.md` for the
//! full mco model.
//!
//! THIS FILE IS CURRENTLY A STUB. moof-specific imports land here
//! as the substrate exposes them. for the first wave (clock),
//! WASI alone is sufficient; nothing in this file is needed yet.

// ── moof-namespaced imports (none yet) ───────────────────────────
//
// future imports will be declared as:
//
//   pub extern "moof" fn slot(form_handle: u32, sym_handle: u32) u64;
//   pub extern "moof" fn slot_set(form_handle: u32, sym_handle: u32, value_handle: u64) void;
//   pub extern "moof" fn intern(ptr: [*]const u8, len: usize) u32;
//   pub extern "moof" fn make_string(ptr: [*]const u8, len: usize) u32;
//   pub extern "moof" fn send(receiver: u64, selector: u32, args: [*]const u64, argc: usize) u64;
//   pub extern "moof" fn raise(kind: u32, msg_ptr: [*]const u8, msg_len: usize) noreturn;
//
// each grows the abi version (the substrate cross-checks).

// ── comptime helpers (zig-side, zero overhead) ───────────────────

/// duration in ns of executing a no-arg function. monotonic-clock
/// based; uses WASI directly.
pub inline fn timeFn(comptime f: anytype) i64 {
    const std = @import("std");
    const wasi = std.os.wasi;
    var t0: wasi.timestamp_t = 0;
    var t1: wasi.timestamp_t = 0;
    _ = wasi.clock_time_get(.MONOTONIC, 1000, &t0);
    f();
    _ = wasi.clock_time_get(.MONOTONIC, 1000, &t1);
    return @intCast(t1 - t0);
}
