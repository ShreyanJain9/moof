//! moof-zig — entrypoint. V4 polyglot substrate.
//!
//! parses argv, loads a V4 vat-image (or runs an inline smoke),
//! instantiates a World, runs.
//!
//! subcommands:
//!   moof-zig                  — built-in smoke (boots a World, runs two
//!                                hand-constructed chunks).
//!   moof-zig decode <path>    — read raw V4 bytecode bytes from <path>
//!                                (use `/dev/stdin` for piped input on
//!                                POSIX systems) and print each decoded
//!                                opcode + operands.
//!                                used by the cross-stack roundtrip smoke
//!                                that verifies OCaml's `moof-seed bytes`
//!                                output decodes byte-for-byte under the
//!                                zig substrate's `bytecode.decodeOp`.
//!   moof-zig load <path>      — load a V4 vat-image from <path> into a
//!                                fresh bare World, then print world
//!                                state (heap.len, syms.len, chunks,
//!                                natives, here_form). V4 Track C.3
//!                                Task 2.5 — pairs with the rust
//!                                v4_export build-time oracle.

const std = @import("std");
const value_mod = @import("value.zig");
const Value = value_mod.Value;
const form_mod = @import("form.zig");
const FormId = form_mod.FormId;
const Form = form_mod.Form;
const bytecode = @import("bytecode.zig");
const opcodes = @import("opcodes.zig");
const world_mod = @import("world.zig");
const World = world_mod.World;
const ICache = world_mod.ICache;
const intrinsics = @import("intrinsics.zig");
const vm = @import("vm.zig");
const image = @import("image.zig");

pub fn main(init: std.process.Init) !void {
    // we still want our own DebugAllocator for the World / heap (its
    // leak-checking is the test harness for substrate lifetimes). the
    // runtime-supplied `init.gpa` and `init.io` are only used for
    // filesystem reads in the decode subcommand.
    var gpa: std.heap.DebugAllocator(.{}) = .init;
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    // ------------------------------------------------------------
    // argv dispatch
    //
    // zig 0.16 dropped `std.process.argsAlloc` and `std.fs.cwd()`. the
    // new pattern is to accept `std.process.Init` (or .Minimal) as
    // main's first parameter, iterate `init.minimal.args`, and route
    // filesystem access through `init.io` + `std.Io.Dir.cwd()`.
    //
    // we just sniff for `decode <path>` here — anything else falls
    // through to the smoke.
    // ------------------------------------------------------------
    var it = init.minimal.args.iterate();
    _ = it.next(); // skip argv[0]
    const sub_raw = it.next();
    const path_raw = it.next();

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "decode")) {
        const path_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(path_copy);
        return runDecode(allocator, init.io, path_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "load")) {
        const path_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(path_copy);
        return runLoad(allocator, init.io, path_copy);
    }

    return runSmoke(allocator);
}

// ============================================================
// decode subcommand
// ============================================================

/// read all bytes from `path` and decode each opcode in turn via
/// `bytecode.decodeOp`. prints one line per op, prefixed with the byte
/// offset, then a final summary.
///
/// `path` may be `/dev/stdin` to read from stdin on POSIX systems —
/// macOS/Linux both expose stdin as a regular file path.
///
/// the byte-offset prefix lets you visually align this output against
/// `xxd` / `hexdump -C` and against OCaml's `moof-seed compile` hex
/// dump, which is the whole point of the cross-stack smoke.
fn runDecode(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !void {
    const p = std.debug.print;

    // 1 MiB cap is plenty for any V4 chunk body the smoke will feed
    // through this command. larger images should use the (future)
    // moof-zig load-image subcommand instead.
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024));
    defer allocator.free(bytes);

    p("=== decoding {d} bytes from {s} ===\n", .{ bytes.len, path });

    var pos: usize = 0;
    var op_count: usize = 0;
    while (pos < bytes.len) {
        const decoded = bytecode.decodeOp(bytes, pos) catch |err| {
            p("  [{d:>4}] decode error at offset {d}: {s} (byte=0x{x:0>2})\n", .{
                pos,
                pos,
                @errorName(err),
                bytes[pos],
            });
            return err;
        };
        printOp(pos, decoded.op);
        pos += decoded.advance;
        op_count += 1;
    }
    p("=== decoded {d} ops in {d} bytes ===\n", .{ op_count, bytes.len });
}

/// print one decoded op on a single line, prefixed by its byte offset.
/// the operand-field naming mirrors `opcodes.zig` exactly so that grep
/// for e.g. `selector=` works across both stacks' diagnostics.
fn printOp(offset: usize, op: opcodes.Op) void {
    const p = std.debug.print;
    p("  [{d:>4}] {s}", .{ offset, @tagName(op) });
    switch (op) {
        // 1-byte ops: tag only, no operands.
        .push_nil,
        .push_true,
        .push_false,
        .pop,
        .dup,
        .load_self,
        .load_here,
        .return_op,
        => {},

        .load_const => |c| p(" idx={d}", .{c.idx}),
        .load_name => |n| p(" sym={d}", .{n.name}),

        // 8-byte sends (with IC)
        .send, .super_send, .send_self, .send_here => |s| {
            p(" sel={d} argc={d} ic={d}", .{ s.selector, s.argc, s.ic_idx });
        },

        // 6-byte tail sends (no IC)
        .tail_send, .tail_send_self, .tail_send_here => |s| {
            p(" sel={d} argc={d}", .{ s.selector, s.argc });
        },

        .send_dynamic => |s| p(" argc={d} ic={d}", .{ s.argc, s.ic_idx }),

        .jump, .jump_if_false, .jump_if_true => |j| {
            p(" offset={d}", .{j.offset});
        },

        .push_closure => |c| {
            // print the raw u32 + the structured fields — handy when
            // cross-checking against OCaml's `chunk#N` which encodes
            // as a u32 FormId on the wire.
            const raw: u32 = @bitCast(c.chunk);
            p(" chunk=0x{x:0>8} (scope={s} payload={d})", .{
                raw,
                @tagName(c.chunk.scope),
                @as(u32, c.chunk.payload),
            });
        },

        .suspend_op => |s| p(" promise_ic={d}", .{s.promise_ic}),
        .resume_op => |s| p(" frame_ic={d}", .{s.frame_ic}),
    }
    p("\n", .{});
}

// ============================================================
// load subcommand (V4 Track C.3 Task 2.5)
// ============================================================

/// load a V4 vat-image from `path` into a fresh bare World and print
/// world state. used to verify the rust v4_export → zig image-load
/// pipeline end-to-end.
///
/// pairs with Track 1 (the rust `moof export-v4` subcommand). a
/// successful run means: bytes parse, sym table is populated from
/// the image (replace semantics per V4 §10), every Form alloc lands
/// in the heap with its proto/slots/handlers/meta, chunk side-tables
/// are filled with byte-encoded bodies + const-pool + zero-initialized
/// ICs + param-sym lists, and every "ProtoName:selector" native ref
/// resolves to a fn pointer via the comptime REGISTRY.
fn runLoad(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !void {
    const p = std.debug.print;

    p("loading {s}...\n", .{path});

    // bare world — no protos, no $here, no Macros. the image carries
    // the canonical FormIds for those; image.loadVatImage installs
    // them after the heap is populated.
    var world = try World.initBare(allocator);
    defer world.deinit();

    // 64 MiB cap is plenty for a stdlib vat-image (rust's bootstrap
    // serialization is ~1-5 MB per estimates in the C.3 plan).
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(64 * 1024 * 1024));
    defer allocator.free(bytes);

    try image.loadVatImage(&world, bytes, allocator);

    p("loaded {s} ({d} bytes)\n", .{ path, bytes.len });
    p("  heap.len = {d}\n", .{world.heap.len()});
    p("  syms.len = {d}\n", .{world.syms.len()});
    p("  chunks   = {d}\n", .{world.chunk_bytecode.count()});
    p("  natives  = {d}\n", .{world.native_fns.count()});
    p("  here_form  = scope={s} payload={d}\n", .{
        @tagName(world.here_form.scope),
        @as(u32, world.here_form.payload),
    });
    p("  macros_form = scope={s} payload={d}\n", .{
        @tagName(world.macros_form.scope),
        @as(u32, world.macros_form.payload),
    });
    p("V4 vat-image alive ٩(◕‿◕｡)۶\n", .{});
}

// ============================================================
// default smoke (unchanged behavior — boots a World, runs chunks)
// ============================================================

fn runSmoke(allocator: std.mem.Allocator) !void {
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
