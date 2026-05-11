//! moof-zig — entrypoint. V4 polyglot substrate.
//!
//! parses argv, loads a V4 vat-image (or runs an inline smoke),
//! instantiates a World, runs.

const std = @import("std");
const value = @import("value.zig");
const form = @import("form.zig");
const sym = @import("sym.zig");
const heap = @import("heap.zig");
const opcodes = @import("opcodes.zig");
const bytecode = @import("bytecode.zig");

pub fn main() !void {
    var gpa: std.heap.DebugAllocator(.{}) = .init;
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const p = std.debug.print;

    p("moof-zig v0.0.0 — V4 polyglot substrate\n", .{});
    p("  FormId size: {} bits\n", .{@bitSizeOf(form.FormId)});
    p("  Value size:  {} bytes\n", .{@sizeOf(value.Value)});
    p("  Form size:   {} bytes\n", .{@sizeOf(form.Form)});

    // smoke: alloc heap, intern syms, encode an op
    var h = try heap.Heap.init(allocator);
    defer h.deinit();

    var st = try sym.SymTable.init(allocator);
    defer st.deinit();

    const foo = try st.intern("foo");
    const bar = try st.intern("bar");
    p("  interned: foo={}, bar={}, foo-resolve={s}\n",
        .{ foo, bar, st.resolve(foo) });

    // encode + decode an Op roundtrip
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);

    const op = opcodes.Op{ .send = .{ .selector = foo, .argc = 2, .ic_idx = 4 } };
    try bytecode.encodeOp(op, &buf, allocator);
    p("  encoded Send :foo argc=2 ic=4 → {} bytes\n", .{buf.items.len});

    const decoded = try bytecode.decodeOp(buf.items, 0);
    p("  decoded: tag={s}, advance={}\n",
        .{ @tagName(decoded.op), decoded.advance });

    p("  V4 polyglot substrate skeleton ready ٩(◕‿◕｡)۶\n", .{});
}
