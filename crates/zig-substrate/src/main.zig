//! moof-zig — entrypoint. V4 polyglot substrate.
//!
//! parses argv, loads a V4 vat-image (or runs an inline smoke),
//! instantiates a World, runs.

const std = @import("std");
const value_mod = @import("value.zig");
const Value = value_mod.Value;
const form_mod = @import("form.zig");
const FormId = form_mod.FormId;
const Form = form_mod.Form;
const bytecode = @import("bytecode.zig");
const world_mod = @import("world.zig");
const World = world_mod.World;
const ICache = world_mod.ICache;
const intrinsics = @import("intrinsics.zig");
const vm = @import("vm.zig");
const image = @import("image.zig");

pub fn main() !void {
    var gpa: std.heap.DebugAllocator(.{}) = .init;
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const p = std.debug.print;

    p("moof-zig v0.0.0 — V4 polyglot substrate\n", .{});
    p("  FormId size: {} bits\n", .{@bitSizeOf(FormId)});
    p("  Value size:  {} bytes\n", .{@sizeOf(Value)});
    p("  Form size:   {} bytes\n", .{@sizeOf(Form)});

    // ------------------------------------------------------------
    // boot a full World
    // ------------------------------------------------------------
    var world = try World.init(allocator);
    defer world.deinit();
    p("  world: heap.len={} syms.len={}\n", .{ world.heap.len(), world.syms.len() });

    // ------------------------------------------------------------
    // install intrinsics
    // ------------------------------------------------------------
    try intrinsics.install(&world);
    p("  intrinsics installed; native_fns.count={}\n", .{world.native_fns.count()});

    // ------------------------------------------------------------
    // hand-construct a tiny chunk: PushNil; Return → result: nil
    // ------------------------------------------------------------
    {
        const chunk_form = Form.withProto(.{ .form = world.protos.chunk });
        const chunk_id = try world.heap.alloc(chunk_form);

        // bytecode: 0x01 (push_nil), 0x33 (return_op)
        const body_bytes = try allocator.dupe(u8, &[_]u8{ 0x01, 0x33 });
        try world.chunk_bytecode.put(allocator, chunk_id, body_bytes);

        const consts = try allocator.alloc(Value, 0);
        try world.chunk_consts.put(allocator, chunk_id, consts);

        const ics = try allocator.alloc(ICache, 0);
        try world.chunk_ics.put(allocator, chunk_id, ics);

        const result = try vm.runTop(&world, chunk_id);
        p("  chunk1 (PushNil; Return) → {s}\n", .{@tagName(result)});
    }

    // ------------------------------------------------------------
    // hand-construct: LoadConst 0 (Int 2); LoadConst 1 (Int 3);
    //                 Send :+ argc=1 ic=0; Return  →  Int 5
    // ------------------------------------------------------------
    {
        const chunk_form = Form.withProto(.{ .form = world.protos.chunk });
        const chunk_id = try world.heap.alloc(chunk_form);

        // selector :+
        const plus_sym = try world.syms.intern("+");

        // encode bytecode using bytecode.encodeOp for safety
        var buf: std.ArrayList(u8) = .empty;
        defer buf.deinit(allocator);
        try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
        try bytecode.encodeOp(.{ .load_const = .{ .idx = 1 } }, &buf, allocator);
        try bytecode.encodeOp(
            .{ .send = .{ .selector = plus_sym, .argc = 1, .ic_idx = 0 } },
            &buf,
            allocator,
        );
        try bytecode.encodeOp(.return_op, &buf, allocator);

        const body_bytes = try allocator.dupe(u8, buf.items);
        try world.chunk_bytecode.put(allocator, chunk_id, body_bytes);

        const consts = try allocator.alloc(Value, 2);
        consts[0] = .{ .int = 2 };
        consts[1] = .{ .int = 3 };
        try world.chunk_consts.put(allocator, chunk_id, consts);

        const ics = try allocator.alloc(ICache, 1);
        ics[0] = ICache.empty;
        try world.chunk_ics.put(allocator, chunk_id, ics);

        const result = try vm.runTop(&world, chunk_id);
        switch (result) {
            .int => |n| p("  chunk2 (2 + 3) → Int {d}\n", .{n}),
            else => p("  chunk2 → unexpected: {s}\n", .{@tagName(result)}),
        }
    }

    // light coverage so image-section symbols stay reachable in the
    // build graph (even though we don't load an image right now).
    _ = image.MAGIC;
    _ = image.VERSION;

    p("  V4 polyglot substrate alive ٩(◕‿◕｡)۶\n", .{});
}
