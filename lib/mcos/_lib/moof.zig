//! lib/mcos/_lib/moof.zig — zig binding for the wasm mco abi.
//!
//! see docs/reference/native-abi.md for the canonical contract.
//! every import below is implemented in players/rust/src/wasm.rs.

// ── moof-namespaced imports ──────────────────────────────────────

pub extern "moof" fn moof_raise(kind_handle: u32, msg_ptr: [*]const u8, msg_len: usize) noreturn;
pub extern "moof" fn moof_make_string(ptr: [*]const u8, len: usize) u32;
pub extern "moof" fn moof_make_bytes(ptr: [*]const u8, len: usize) u32;
pub extern "moof" fn moof_string_text(handle: u32, buf: [*]u8, cap: usize) usize;
pub extern "moof" fn moof_bytes_data(handle: u32, buf: [*]u8, cap: usize) usize;
pub extern "moof" fn moof_intern(ptr: [*]const u8, len: usize) u32;

// ── ergonomic helpers ─────────────────────────────────────────────

pub inline fn raise(kind: []const u8, msg: []const u8) noreturn {
    const k = moof_intern(kind.ptr, kind.len);
    moof_raise(k, msg.ptr, msg.len);
}

pub inline fn makeString(s: []const u8) u32 {
    return moof_make_string(s.ptr, s.len);
}

pub inline fn makeBytes(b: []const u8) u32 {
    return moof_make_bytes(b.ptr, b.len);
}

pub fn readString(handle: u32, buf: []u8) []const u8 {
    const n = moof_string_text(handle, buf.ptr, buf.len);
    const actual = if (n > buf.len) buf.len else n;
    return buf[0..actual];
}

pub fn readBytes(handle: u32, buf: []u8) []const u8 {
    const n = moof_bytes_data(handle, buf.ptr, buf.len);
    const actual = if (n > buf.len) buf.len else n;
    return buf[0..actual];
}
