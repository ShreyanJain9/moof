//! moof-zig — entrypoint. for now: a hello-world that confirms
//! the toolchain compiles + a stub FormId/Value smoke that proves
//! tagged-union encoding works the way we want.
//!
//! once phase γ tasks start landing, this file becomes a tiny CLI
//! shim (parse argv, load bytecode, instantiate World, run).

const std = @import("std");

/// the universal heap-id. matches the rust `FormId` layout: 2-bit
/// scope tag in the top, 30-bit payload below. derived from the V0
/// scope-tagging design.
pub const FormId = packed struct(u32) {
    payload: u30,
    scope: Scope,

    pub const Scope = enum(u2) {
        vat_local = 0b00,
        shared = 0b01,
        far_ref = 0b10,
        reserved = 0b11,
    };

    pub const NONE: FormId = .{ .payload = 0, .scope = .vat_local };

    pub fn isNone(self: FormId) bool {
        return self.payload == 0 and self.scope == .vat_local;
    }

    pub fn vatLocal(payload: u30) FormId {
        return .{ .payload = payload, .scope = .vat_local };
    }
};

/// the runtime value. tagged-immediate per the V0 design.
pub const Value = union(enum) {
    nil,
    bool_: bool,
    int: i48,  // moof's bignum-ready integer width
    sym: u32,  // SymId
    char: u32, // codepoint
    float: f64,
    form: FormId,

    pub fn isNil(self: Value) bool {
        return self == .nil;
    }

    pub fn isTruthy(self: Value) bool {
        return switch (self) {
            .nil => false,
            .bool_ => |b| b,
            else => true,
        };
    }
};

pub fn main() !void {
    const allocator = std.heap.page_allocator;
    const p = std.debug.print;

    p("moof-zig v0.0.0 — V4 substrate prototype\n", .{});
    p("  FormId size: {} bits\n", .{@bitSizeOf(FormId)});
    p("  Value size: {} bytes\n", .{@sizeOf(Value)});

    // tiny smoke: alloc a few FormIds, push them onto an ArrayList,
    // verify the scope-tag round-trips. this proves the toolchain
    // + the packed-struct encoding work as expected.
    var ids: std.ArrayList(FormId) = .empty;
    defer ids.deinit(allocator);

    try ids.append(allocator, FormId.vatLocal(1));
    try ids.append(allocator, FormId.vatLocal(42));
    try ids.append(allocator, FormId{ .payload = 7, .scope = .shared });
    try ids.append(allocator, FormId.NONE);

    p("  allocated {} FormIds:\n", .{ids.items.len});
    for (ids.items, 0..) |id, i| {
        p("    [{}] scope={s} payload={} none?={}\n",
            .{ i, @tagName(id.scope), id.payload, id.isNone() });
    }

    // tiny smoke on Value too
    const values = [_]Value{
        .nil,
        .{ .bool_ = true },
        .{ .int = 42 },
        .{ .form = FormId.vatLocal(99) },
    };
    p("  values:\n", .{});
    for (values, 0..) |v, i| {
        p("    [{}] tag={s} truthy?={}\n",
            .{ i, @tagName(v), v.isTruthy() });
    }
}
