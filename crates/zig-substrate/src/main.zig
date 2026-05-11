//! moof — entrypoint. V4 polyglot substrate.
//!
//! parses argv, loads a V4 vat-image (or runs an inline smoke),
//! instantiates a World, runs.
//!
//! subcommands:
//!   moof                  — built-in smoke (boots a World, runs two
//!                                hand-constructed chunks).
//!   moof decode <path>    — read raw V4 bytecode bytes from <path>
//!                                (use `/dev/stdin` for piped input on
//!                                POSIX systems) and print each decoded
//!                                opcode + operands.
//!                                used by the cross-stack roundtrip smoke
//!                                that verifies OCaml's `moof-seed bytes`
//!                                output decodes byte-for-byte under the
//!                                zig substrate's `bytecode.decodeOp`.
//!   moof load <path>      — load a V4 vat-image from <path> into a
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

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "exec")) {
        // moof exec <vat> <chunk-id>
        const chunk_id_raw = it.next() orelse {
            std.debug.print("usage: moof exec <vat> <chunk-id>\n", .{});
            return;
        };
        const vat_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(vat_copy);
        const chunk_id = try std.fmt.parseInt(u32, chunk_id_raw, 10);
        return runExec(allocator, init.io, vat_copy, chunk_id);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "serialize")) {
        // moof serialize <in.vat> <out.vat>
        const out_raw = it.next() orelse {
            std.debug.print("usage: moof serialize <in.vat> <out.vat>\n", .{});
            return;
        };
        const in_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(in_copy);
        const out_copy = try allocator.dupe(u8, out_raw);
        defer allocator.free(out_copy);
        return runSerialize(allocator, init.io, in_copy, out_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "smoke-serialize-to")) {
        // moof smoke-serialize-to <out.vat>
        // boots a fresh World with intrinsics, runs a chunk that does
        // `[$here serializeTo: 'out.vat]` from inside the VM, then prints
        // the result. proves the intrinsic is reachable from moof code.
        const out_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(out_copy);
        return runSmokeSerializeTo(allocator, init.io, out_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "build-trivial-vat")) {
        // moof build-trivial-vat <out.vat>
        // emits a minimal V4 vat-image whose top-level chunk evaluates
        // [1 + 2] via real native dispatch — used by the W4 smoke.
        const out_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(out_copy);
        return runBuildTrivialVat(allocator, init.io, out_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "run")) {
        // moof run <vat> [--serialize-to <out>]
        const vat_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(vat_copy);
        var serialize_to: ?[]u8 = null;
        while (it.next()) |a| {
            if (std.mem.eql(u8, a, "--serialize-to")) {
                if (it.next()) |out| {
                    serialize_to = try allocator.dupe(u8, out);
                }
            }
        }
        defer if (serialize_to) |s| allocator.free(s);
        return runRun(allocator, init.io, init.minimal.environ, vat_copy, serialize_to);
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
    // moof load-image subcommand instead.
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

    // 1 GiB cap accommodates current FormLoc-bloated polyglot vats
    // (~360 MB). once the FormLoc dedup task lands this can shrink
    // back toward the original 1-5 MB estimate.
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024 * 1024));
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

    p("moof v0.0.0 — V4 polyglot substrate\n", .{});
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

// ============================================================
// smoke-serialize-to subcommand (W4 :serializeTo: round-trip)
// ============================================================

/// boot a fresh World, install intrinsics, run a chunk that calls
/// `[$here serializeTo: 'out_sym]` from bytecode. proves the
/// in-moof primitive works end-to-end and that the written file
/// loads correctly.
///
/// path_sym is interned as a literal symbol so we don't need
/// String-Form storage at the boot smoke layer.
fn runSmokeSerializeTo(allocator: std.mem.Allocator, io: std.Io, out_path: []const u8) !void {
    const p = std.debug.print;

    var world = try World.init(allocator);
    defer world.deinit();
    world.io = io;
    try intrinsics.install(&world);

    // intern path as a symbol so :serializeTo: pulls it through the
    // sym variant of valueToString (avoids cons-of-Char-Forms).
    const path_sym = try world.syms.intern(out_path);
    const serialize_sel = try world.syms.intern("serializeTo:");

    const chunk_form = form_mod.Form.withProto(.{ .form = world.protos.chunk });
    const chunk_id = try world.heap.alloc(chunk_form);

    // bytecode: LoadHere; LoadConst 0; Send :serializeTo: argc=1 ic=0; Return
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try bytecode.encodeOp(.load_here, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(
        .{ .send = .{ .selector = serialize_sel, .argc = 1, .ic_idx = 0 } },
        &buf,
        allocator,
    );
    try bytecode.encodeOp(.return_op, &buf, allocator);

    const body_bytes = try allocator.dupe(u8, buf.items);
    try world.chunk_bytecode.put(allocator, chunk_id, body_bytes);

    const consts = try allocator.alloc(Value, 1);
    consts[0] = .{ .sym = path_sym };
    try world.chunk_consts.put(allocator, chunk_id, consts);

    const ics = try allocator.alloc(ICache, 1);
    ics[0] = ICache.empty;
    try world.chunk_ics.put(allocator, chunk_id, ics);

    const params = try allocator.alloc(u32, 0);
    try world.chunk_params.put(allocator, chunk_id, params);

    p("running [$here serializeTo: '{s}]...\n", .{out_path});
    const result = vm.runTop(&world, chunk_id) catch |err| {
        p("vm error: {s}\n", .{@errorName(err)});
        return;
    };
    printResult(result, &world);
    p("if the smoke succeeded, {s} now exists.\n", .{out_path});
}

// ============================================================
// build-trivial-vat subcommand (W4 smoke helper)
// ============================================================

/// build a tiny World with one chunk that evaluates `[1 + 2]` and
/// serialize it as a V4 vat-image. used by the W4 round-trip smoke:
///
///   moof build-trivial-vat /tmp/trivial.vat
///   moof exec /tmp/trivial.vat <chunk-id>   # → Int(3)
///
/// the chunk_id of the trivial chunk is printed to stderr.
fn runBuildTrivialVat(allocator: std.mem.Allocator, io: std.Io, out_path: []const u8) !void {
    const p = std.debug.print;

    var world = try World.init(allocator);
    defer world.deinit();
    world.io = io;
    try intrinsics.install(&world);

    // first chunk: PushConst 0; Return — const-only "Int(3)".
    {
        const trivial_form = form_mod.Form.withProto(.{ .form = world.protos.chunk });
        const trivial_id = try world.heap.alloc(trivial_form);
        var tbuf: std.ArrayList(u8) = .empty;
        defer tbuf.deinit(allocator);
        try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &tbuf, allocator);
        try bytecode.encodeOp(.return_op, &tbuf, allocator);
        const tbody = try allocator.dupe(u8, tbuf.items);
        try world.chunk_bytecode.put(allocator, trivial_id, tbody);
        const tconsts = try allocator.alloc(Value, 1);
        tconsts[0] = .{ .int = 3 };
        try world.chunk_consts.put(allocator, trivial_id, tconsts);
        try world.chunk_ics.put(allocator, trivial_id, try allocator.alloc(ICache, 0));
        try world.chunk_params.put(allocator, trivial_id, try allocator.alloc(u32, 0));
        p("  loadconst-only chunk id = {d}  (=> Int(3))\n", .{trivial_id.payload});
    }

    // hand-construct [1 + 2]: LoadConst 0; LoadConst 1; Send :+ argc=1 ic=0; Return.
    const plus_sym = try world.syms.intern("+");

    const chunk_form = form_mod.Form.withProto(.{ .form = world.protos.chunk });
    const chunk_id = try world.heap.alloc(chunk_form);

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
    consts[0] = .{ .int = 1 };
    consts[1] = .{ .int = 2 };
    try world.chunk_consts.put(allocator, chunk_id, consts);

    const ics = try allocator.alloc(ICache, 1);
    ics[0] = ICache.empty;
    try world.chunk_ics.put(allocator, chunk_id, ics);

    const params = try allocator.alloc(u32, 0);
    try world.chunk_params.put(allocator, chunk_id, params);

    // serialize it.
    var out_buf: std.ArrayList(u8) = .empty;
    defer out_buf.deinit(allocator);
    try image.serializeVat(&world, &out_buf, allocator);

    try std.Io.Dir.cwd().writeFile(io, .{
        .sub_path = out_path,
        .data = out_buf.items,
        .flags = .{ .truncate = true },
    });

    p("wrote {s} ({d} bytes)\n", .{ out_path, out_buf.items.len });
    p("  trivial chunk id = {d}\n", .{chunk_id.payload});
    p("  exec smoke:\n", .{});
    p("    moof exec {s} {d}   # → Int(3)\n", .{ out_path, chunk_id.payload });
}

// ============================================================
// exec subcommand (W4 Piece 1)
// ============================================================

/// load a vat-image, find the chunk by id, run it via vm.runTop, print
/// the result. used to prove end-to-end dispatch through re-bound
/// natives on a hydrated World.
///
/// the chunk-id arg is the raw FormId payload (u32). e.g. for a
/// system.vat where the chunk for `[1 + 2]` lives at FormId(42), say
/// `moof exec /tmp/system.vat 42`.
fn runExec(allocator: std.mem.Allocator, io: std.Io, vat_path: []const u8, chunk_id: u32) !void {
    const p = std.debug.print;

    var world = try World.initBare(allocator);
    defer world.deinit();
    world.io = io;

    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);

    try image.loadVatImage(&world, bytes, allocator);
    p("loaded {s} ({d} bytes)\n", .{ vat_path, bytes.len });
    p("  heap.len = {d}, syms.len = {d}, chunks = {d}, natives = {d}\n", .{
        world.heap.len(),
        world.syms.len(),
        world.chunk_bytecode.count(),
        world.native_fns.count(),
    });

    // look up the chunk Form by id. the side-tables are keyed by
    // chunk_id; if it's not there, give a hint at the available range.
    const chunk_fid = FormId.vatLocal(@intCast(chunk_id));
    if (!world.chunk_bytecode.contains(chunk_fid)) {
        p("error: no chunk with id={d} in this vat\n", .{chunk_id});
        // list a few valid ones so the user has somewhere to go.
        var it = world.chunk_bytecode.iterator();
        var shown: usize = 0;
        p("  valid chunk ids (first ~10): ", .{});
        while (it.next()) |entry| {
            if (shown >= 10) break;
            p("{d} ", .{entry.key_ptr.*.payload});
            shown += 1;
        }
        p("...\n", .{});
        return;
    }

    p("running chunk {d}...\n", .{chunk_id});
    const result = vm.runTop(&world, chunk_fid) catch |err| {
        p("vm error: {s}\n", .{@errorName(err)});
        return;
    };

    printResult(result, &world);
}

/// print a Value in a human-readable form. used by exec / run.
fn printResult(v: Value, world: *const World) void {
    const p = std.debug.print;
    switch (v) {
        .nil => p("=> nil\n", .{}),
        .bool_ => |b| p("=> {s}\n", .{if (b) "#true" else "#false"}),
        .int => |n| p("=> Int({d})\n", .{n}),
        .sym => |s| p("=> Sym('{s})\n", .{world.syms.resolve(s)}),
        .char => |cp| p("=> Char(U+{x:0>4})\n", .{cp}),
        .float => |f| p("=> Float({d})\n", .{f}),
        .form => |id| p("=> Form#{d} (scope={s})\n", .{ @as(u32, id.payload), @tagName(id.scope) }),
    }
}

// ============================================================
// serialize subcommand (W4 Piece 2 — load + write roundtrip)
// ============================================================

/// load a vat-image and immediately re-serialize it. used to verify
/// byte-equivalence with rust's v4_export — `diff in.vat out.vat`
/// after roundtrip should match (modulo footer hash which both stub).
fn runSerialize(allocator: std.mem.Allocator, io: std.Io, in_path: []const u8, out_path: []const u8) !void {
    const p = std.debug.print;

    var world = try World.initBare(allocator);
    defer world.deinit();
    world.io = io;

    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, in_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);

    try image.loadVatImage(&world, bytes, allocator);
    p("loaded {s} ({d} bytes)\n", .{ in_path, bytes.len });

    // serialize into a buffer, then dump it.
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try image.serializeVat(&world, &buf, allocator);

    try std.Io.Dir.cwd().writeFile(io, .{
        .sub_path = out_path,
        .data = buf.items,
        .flags = .{ .truncate = true },
    });

    p("wrote {s} ({d} bytes)\n", .{ out_path, buf.items.len });
}

// ============================================================
// transporter lib-root resolution (port of rust transporter.rs::resolve_lib_root)
// ============================================================

/// check if `path` is a directory via io. zig 0.16 dropped
/// `std.fs.cwd()` in favor of `std.Io.Dir.cwd()`; the helper here
/// keeps the resolve-loop terse.
fn isDir(io: std.Io, path: []const u8) bool {
    var dir = std.Io.Dir.cwd().openDir(io, path, .{}) catch return false;
    defer dir.close(io);
    // openDir succeeded → it's a directory (Zig errors on file-as-dir).
    return true;
}

/// resolve the moof lib root: MOOF_LIB env var → ./lib.
/// (the <exe>/../lib path is intentionally skipped — selfExePath
/// requires Io plumbing that's not yet stable in 0.16; main.zig
/// callers usually run from the workspace root so ./lib suffices.)
/// returns the first directory that exists; caller owns the slice.
/// returns null if none found.
fn resolveLibRoot(allocator: std.mem.Allocator, io: std.Io, environ: std.process.Environ) ?[]u8 {
    // 1. MOOF_LIB
    if (std.process.Environ.getAlloc(environ, allocator, "MOOF_LIB")) |env_path| {
        if (isDir(io, env_path)) return env_path;
        allocator.free(env_path);
    } else |_| {}

    // 2. ./lib
    {
        const candidate = allocator.dupe(u8, "./lib") catch return null;
        if (isDir(io, candidate)) return candidate;
        allocator.free(candidate);
    }

    return null;
}

// ============================================================
// run subcommand (W4 Piece 3 — boot + run main + optional serialize)
// ============================================================

/// load a vat-image, look up its `main` chunk on `$here` (if any), run
/// it to completion. if `--serialize-to <out>` was given, write the
/// final world as a V4 vat-image at `out`.
///
/// the main-chunk convention isn't yet canonicalized by rust's
/// v4_export (which doesn't emit any specific "main" pointer); we
/// look for a `'main` slot on `here_form` and run its chunk if
/// present, otherwise treat the vat as already-bootstrapped and
/// skip directly to the optional serialize step.
fn runRun(allocator: std.mem.Allocator, io: std.Io, environ: std.process.Environ, vat_path: []const u8, serialize_to: ?[]const u8) !void {
    const p = std.debug.print;

    var world = try World.initBare(allocator);
    defer world.deinit();
    world.io = io;

    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);

    try image.loadVatImage(&world, bytes, allocator);
    p("loaded {s} ({d} bytes)\n", .{ vat_path, bytes.len });
    p("  heap.len = {d}, syms.len = {d}, chunks = {d}, natives = {d}\n", .{
        world.heap.len(),
        world.syms.len(),
        world.chunk_bytecode.count(),
        world.native_fns.count(),
    });

    // install primordial caps ($transporter / $compiler / $reader)
    // post-load. these can't ride the NativeRefsSection rebind path
    // because rust v4_export labels their anonymous protos with
    // <anon-N>:selector names that vary per-image. mirrors the rust
    // boot sequence (install_compiler_cap / install_reader_cap /
    // transporter::install run AFTER image hydrate).
    intrinsics.installCaps(&world) catch |err| {
        p("warning: installCaps failed: {s}\n", .{@errorName(err)});
    };

    // resolve transporter root: MOOF_LIB > ./lib > nothing.
    if (resolveLibRoot(allocator, io, environ)) |root| {
        defer allocator.free(root);
        world.setTransporterRoot(root) catch |err| {
            p("warning: setTransporterRoot failed: {s}\n", .{@errorName(err)});
        };
        p("  transporter root = {s}\n", .{root});
    } else {
        p("  transporter root = <unset> (no MOOF_LIB, no <exe>/../lib, no ./lib)\n", .{});
    }

    // is there a `main` slot on $here? if so, it should hold either
    // a chunk FormId directly OR a method-Form whose :body is a chunk.
    const main_sym_id = blk: {
        const total = world.syms.len();
        var i: u32 = 1;
        while (i <= total) : (i += 1) {
            if (std.mem.eql(u8, world.syms.resolve(i), "main")) break :blk i;
        }
        break :blk @as(u32, 0);
    };

    if (main_sym_id != 0 and !world.here_form.isNone()) {
        const here = world.heap.get(world.here_form);
        if (here.slot(main_sym_id).asFormId()) |maybe_chunk| {
            // could be a chunk directly, or a method whose body is a chunk
            const chunk_to_run = if (world.chunk_bytecode.contains(maybe_chunk))
                maybe_chunk
            else
                world.formSlot(maybe_chunk, world.body_sym).asFormId() orelse maybe_chunk;

            if (world.chunk_bytecode.contains(chunk_to_run)) {
                p("running main chunk #{d}...\n", .{chunk_to_run.payload});
                const result = vm.runTop(&world, chunk_to_run) catch |err| {
                    p("vm error in main: {s}\n", .{@errorName(err)});
                    return;
                };
                printResult(result, &world);
            } else {
                p("no main chunk found (`main` slot doesn't point at a chunk); skipping run\n", .{});
            }
        } else {
            p("no main slot on $here; treating vat as already-bootstrapped\n", .{});
        }
    } else {
        p("no `main` symbol or no here_form; treating vat as already-bootstrapped\n", .{});
    }

    if (serialize_to) |out_path| {
        var buf: std.ArrayList(u8) = .empty;
        defer buf.deinit(allocator);
        try image.serializeVat(&world, &buf, allocator);

        try std.Io.Dir.cwd().writeFile(io, .{
            .sub_path = out_path,
            .data = buf.items,
            .flags = .{ .truncate = true },
        });
        p("wrote {s} ({d} bytes)\n", .{ out_path, buf.items.len });
    }
}
