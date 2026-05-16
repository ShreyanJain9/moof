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
//!   moof load <path>      — load a V4 vat-image from <path> into a
//!                                fresh bare World, then print world
//!                                state (heap.len, syms.len, chunks,
//!                                natives, here_form).
//!   moof exec <vat> <chunk-id-or-path>
//!                             — execute a chunk from a vat, or a raw
//!                                bytecode file against a loaded world.
//!   moof eval <vat> "<expr>"
//!                             — parse expr with native reader, compile
//!                                via ocaml-seed subprocess, run.

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
const reader = @import("reader.zig");

/// process-wide GC options, parsed once in main() from env vars and
/// CLI flags. each World built by the various subcommands consults
/// `applyGcOpts` to flip its `gc_enabled` / `gc_stats_enabled`.
///
/// `--no-gc` disables collection entirely (for diagnostic / A-B
/// measurement). `--gc-stats` (or `MOOF_GC_STATS=1`) prints stats to
/// stderr after every collect cycle.
const GcOpts = struct {
    enabled: bool = true,
    stats: bool = false,
};

var g_gc_opts: GcOpts = .{};

fn applyGcOpts(world: *World) void {
    world.gc_enabled = g_gc_opts.enabled;
    world.gc_stats_enabled = g_gc_opts.stats;
}

pub fn main(init: std.process.Init) !void {
    var gpa: std.heap.DebugAllocator(.{}) = .init;
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    // env var: `MOOF_GC_STATS=1` enables single-line GC summary per
    // collection cycle. silently no-op when unset.
    if (init.minimal.environ.getPosix("MOOF_GC_STATS")) |val| {
        if (val.len > 0 and !std.mem.eql(u8, val, "0")) g_gc_opts.stats = true;
    }

    // pre-scan argv for global flags. these flags can appear in any
    // position; consumed before we dispatch to a subcommand. (zig's
    // args iterator is one-shot, so we collect into a list, strip
    // the flags, and rebuild the iteration sequence below.)
    var argv_list: std.ArrayList([]const u8) = .empty;
    defer {
        for (argv_list.items) |s| allocator.free(s);
        argv_list.deinit(allocator);
    }
    {
        var pre_it = init.minimal.args.iterate();
        while (pre_it.next()) |a| {
            if (std.mem.eql(u8, a, "--no-gc")) {
                g_gc_opts.enabled = false;
                continue;
            }
            if (std.mem.eql(u8, a, "--gc-stats")) {
                g_gc_opts.stats = true;
                continue;
            }
            try argv_list.append(allocator, try allocator.dupe(u8, a));
        }
    }

    // re-build iteration over the filtered argv. argv_list now owns
    // duplicated strings; we index by position.
    var arg_idx: usize = 1; // skip argv[0]
    const argc = argv_list.items.len;
    const sub_raw: ?[]const u8 = if (arg_idx < argc) blk: {
        const s = argv_list.items[arg_idx];
        arg_idx += 1;
        break :blk s;
    } else null;
    const path_raw: ?[]const u8 = if (arg_idx < argc) blk: {
        const s = argv_list.items[arg_idx];
        arg_idx += 1;
        break :blk s;
    } else null;
    // small shim so the existing `it.next()` call sites below keep
    // working with minimal churn.
    const ItShim = struct {
        items: []const []const u8,
        idx: *usize,
        fn next(self: @This()) ?[]const u8 {
            if (self.idx.* >= self.items.len) return null;
            const s = self.items[self.idx.*];
            self.idx.* += 1;
            return s;
        }
    };
    var it = ItShim{ .items = argv_list.items, .idx = &arg_idx };

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
        const second_arg = it.next() orelse {
            std.debug.print("usage: moof exec <vat> <chunk-id-or-path>\n", .{});
            return;
        };
        const vat_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(vat_copy);
        if (std.fmt.parseInt(u32, second_arg, 10)) |chunk_id| {
            return runExec(allocator, init.io, vat_copy, chunk_id);
        } else |_| {
            const bytecode_path = try allocator.dupe(u8, second_arg);
            defer allocator.free(bytecode_path);
            return runExecBytecode(allocator, init.io, vat_copy, bytecode_path);
        }
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "inspect-syms")) {
        const vat_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(vat_copy);
        return runInspectSyms(allocator, init.io, vat_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "eval")) {
        const expr_raw = it.next() orelse {
            std.debug.print("usage: moof eval <vat> \"<expr>\"\n", .{});
            return;
        };
        const vat_copy = try allocator.dupe(u8, path_raw.?);
        defer allocator.free(vat_copy);
        const expr_copy = try allocator.dupe(u8, expr_raw);
        defer allocator.free(expr_copy);
        return runEval(allocator, init.io, init.minimal.environ, vat_copy, expr_copy);
    }

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "serialize")) {
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

    if (sub_raw != null and path_raw != null and std.mem.eql(u8, sub_raw.?, "run")) {
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

    if (sub_raw != null and std.mem.eql(u8, sub_raw.?, "stress-recursion")) {
        const depth: u32 = if (path_raw) |s| std.fmt.parseInt(u32, s, 10) catch 10000 else 10000;
        return runStressRecursion(allocator, depth);
    }

    return runSmoke(allocator);
}

fn runDecode(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !void {
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024));
    defer allocator.free(bytes);
    std.debug.print("=== decoding {d} bytes from {s} ===\n", .{ bytes.len, path });
    var pos: usize = 0;
    while (pos < bytes.len) {
        const decoded = try bytecode.decodeOp(bytes, pos);
        printOp(pos, decoded.op);
        pos += decoded.advance;
    }
}

fn printOp(offset: usize, op: opcodes.Op) void {
    const p = std.debug.print;
    p("  [{d:>4}] {s}", .{ offset, @tagName(op) });
    switch (op) {
        .push_nil, .push_true, .push_false, .pop, .dup, .load_self, .load_here, .return_op => {},
        .load_const => |c| p(" idx={d}", .{c.idx}),
        .load_name => |n| p(" sym={d}", .{n.name}),
        .send, .super_send, .send_self, .send_here => |s| p(" sel={d} argc={d} ic={d}", .{ s.selector, s.argc, s.ic_idx }),
        .tail_send, .tail_send_self, .tail_send_here => |s| p(" sel={d} argc={d}", .{ s.selector, s.argc }),
        .send_dynamic => |s| p(" argc={d} ic={d}", .{ s.argc, s.ic_idx }),
        .jump, .jump_if_false, .jump_if_true => |j| p(" offset={d}", .{j.offset}),
        .push_closure => |c| p(" chunk=0x{x:0>8}", .{@as(u32, @bitCast(c.chunk))}),
        .suspend_op => |s| p(" promise_ic={d}", .{s.promise_ic}),
        .resume_op => |s| p(" frame_ic={d}", .{s.frame_ic}),
    }
    p("\n", .{});
}

fn runLoad(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes, allocator);
    std.debug.print("loaded {s} ({d} bytes)\n", .{ path, bytes.len });
    std.debug.print("  heap.len = {d}, syms.len = {d}, chunks = {d}\n", .{ world.heap.len(), world.syms.len(), world.chunk_bytecode.count() });
}

fn runExec(allocator: std.mem.Allocator, io: std.Io, vat_path: []const u8, chunk_id: u32) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    world.io = io;
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes, allocator);
    const chunk_fid = FormId.vatLocal(@intCast(chunk_id));
    std.debug.print("running chunk {d}...\n", .{chunk_id});
    const result = try vm.runTop(&world, chunk_fid);
    printResult(result, &world);
}

fn runExecBytecode(allocator: std.mem.Allocator, io: std.Io, vat_path: []const u8, bytecode_path: []const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    world.io = io;
    const vat_bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(vat_bytes);
    try image.loadVatImage(&world, vat_bytes, allocator);
    const path = if (std.mem.eql(u8, bytecode_path, "-")) "/dev/stdin" else bytecode_path;
    const bytecode_bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024));
    defer allocator.free(bytecode_bytes);
    var pos: usize = 0;
    while (pos < bytecode_bytes.len) {
        const chunk_fid = try image.loadChunk(&world, bytecode_bytes, &pos, allocator);
        const result = try vm.runTop(&world, chunk_fid);
        printResult(result, &world);
    }
}

fn runInspectSyms(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes, allocator);
    std.debug.print("sym table (len={d}):\n", .{world.syms.len()});
    var i: u32 = 1;
    while (i <= world.syms.len()) : (i += 1) {
        std.debug.print("  [{d:>3}] {s}\n", .{ i, world.syms.resolve(i) });
    }
}

fn runEval(allocator: std.mem.Allocator, io: std.Io, environ: std.process.Environ, vat_path: []const u8, expr_src: []const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    world.io = io;
    const vat_bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(vat_bytes);
    try image.loadVatImage(&world, vat_bytes, allocator);
    try intrinsics.installCaps(&world);
    if (resolveLibRoot(allocator, io, environ)) |root| {
        defer allocator.free(root);
        try world.setTransporterRoot(root);
    }
    const main_sym_id = lookupSymId(&world, "main");
    if (main_sym_id != 0 and !world.here_form.isNone()) {
        const here = world.heap.get(world.here_form);
        if (here.slot(main_sym_id).asFormId()) |main_id| {
            _ = try vm.runTop(&world, main_id);
        }
    }
    _ = try reader.readOne(&world, expr_src);
    const seed_exe = "../ocaml-seed/_build/default/bin/seed.exe";
    const vat_abs = try std.fs.path.resolve(allocator, &.{vat_path});
    defer allocator.free(vat_abs);
    const temp_path = "/tmp/moof_eval.moof";
    try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = temp_path, .data = expr_src, .flags = .{ .truncate = true } });
    const argv = [_][]const u8{ seed_exe, "bytes", "--syms", vat_abs, temp_path };
    const run_res = try std.process.run(allocator, io, .{ .argv = &argv });
    defer allocator.free(run_res.stdout);
    defer allocator.free(run_res.stderr);
    if (run_res.term != .exited or run_res.term.exited != 0) {
        std.debug.print("error: moof-seed failed\n{s}\n", .{run_res.stderr});
        return;
    }
    var pos: usize = 0;
    const chunk_fid = try image.loadChunk(&world, run_res.stdout, &pos, allocator);
    const result = try vm.runTop(&world, chunk_fid);
    printResult(result, &world);
}

fn runRun(allocator: std.mem.Allocator, io: std.Io, environ: std.process.Environ, vat_path: []const u8, serialize_to: ?[]const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    world.io = io;
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, vat_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes, allocator);
    try intrinsics.installCaps(&world);
    if (resolveLibRoot(allocator, io, environ)) |root| {
        defer allocator.free(root);
        try world.setTransporterRoot(root);
    }
    const main_sym_id = lookupSymId(&world, "main");
    if (main_sym_id != 0 and !world.here_form.isNone()) {
        const hf = world.heap.get(world.here_form);
        if (hf.slot(main_sym_id).asFormId()) |maybe_chunk| {
            const chunk_to_run = if (world.chunk_bytecode.contains(maybe_chunk)) maybe_chunk else world.formSlot(maybe_chunk, world.body_sym).asFormId() orelse maybe_chunk;
            if (world.chunk_bytecode.contains(chunk_to_run)) {
                const result = try vm.runTop(&world, chunk_to_run);
                printResult(result, &world);
            }
        }
    }
    if (serialize_to) |out_path| {
        var buf: std.ArrayList(u8) = .empty;
        defer buf.deinit(allocator);
        try image.serializeVat(&world, &buf, allocator);
        try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = out_path, .data = buf.items, .flags = .{ .truncate = true } });
    }
}

fn lookupSymId(world: *const World, name: []const u8) u32 {
    const total = world.syms.len();
    var i: u32 = 1;
    while (i <= total) : (i += 1) {
        if (std.mem.eql(u8, world.syms.resolve(i), name)) return i;
    }
    return 0;
}

fn resolveLibRoot(allocator: std.mem.Allocator, io: std.Io, environ: std.process.Environ) ?[]u8 {
    if (environ.getPosix("MOOF_LIB")) |lib| return allocator.dupe(u8, lib) catch null;
    const lib_path = "./lib";
    std.Io.Dir.cwd().access(io, lib_path, .{}) catch return null;
    return allocator.dupe(u8, lib_path) catch null;
}

fn printResult(v: Value, world: *const World) void {
    const p = std.debug.print;
    switch (v) {
        .nil => p("=> nil\n", .{}),
        .bool_ => |b| p("=> {s}\n", .{if (b) "#true" else "#false"}),
        .int => |n| p("=> Int({d})\n", .{n}),
        .sym => |s| p("=> Sym('{s})\n", .{world.syms.resolve(s)}),
        .char => |c| p("=> Char(#\\{u})\n", .{@as(u21, @truncate(@as(u32, @bitCast(c))))}),
        .float => |f| p("=> Float({d})\n", .{f}),
        .form => |id| p("=> Form#{d} (scope={s})\n", .{ id.payload, @tagName(id.scope) }),
    }
}

fn runSmoke(allocator: std.mem.Allocator) !void {
    const p = std.debug.print;
    p("moof v0.0.0 — V4 polyglot substrate\n", .{});
    var world = try World.init(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    try intrinsics.install(&world);
    {
        const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
        const body = try allocator.dupe(u8, &[_]u8{ 0x01, 0x33 });
        try world.chunk_bytecode.put(allocator, chunk_id, body);
        try world.chunk_consts.put(allocator, chunk_id, try allocator.alloc(Value, 0));
        try world.chunk_ics.put(allocator, chunk_id, try allocator.alloc(ICache, 0));
        const result = try vm.runTop(&world, chunk_id);
        p("  chunk1 (PushNil; Return) → {s}\n", .{@tagName(result)});
    }
    p("  V4 polyglot substrate alive ٩(◕‿◕｡)۶\n", .{});
}

fn runSerialize(allocator: std.mem.Allocator, io: std.Io, in_path: []const u8, out_path: []const u8) !void {
    var world = try World.initBare(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, in_path, allocator, .limited(1024 * 1024 * 1024));
    defer allocator.free(bytes);
    try image.loadVatImage(&world, bytes, allocator);
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try image.serializeVat(&world, &buf, allocator);
    try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = out_path, .data = buf.items, .flags = .{ .truncate = true } });
    std.debug.print("wrote {s} ({d} bytes)\n", .{ out_path, buf.items.len });
}

fn runSmokeSerializeTo(allocator: std.mem.Allocator, io: std.Io, out_path: []const u8) !void {
    var world = try World.init(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    world.io = io;
    try intrinsics.install(&world);
    const path_sym = try world.syms.intern(out_path);
    const serialize_sel = try world.syms.intern("serializeTo:");
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try bytecode.encodeOp(.load_here, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = serialize_sel, .argc = 1, .ic_idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.return_op, &buf, allocator);
    try world.chunk_bytecode.put(allocator, chunk_id, try allocator.dupe(u8, buf.items));
    const consts = try allocator.alloc(Value, 1);
    consts[0] = .{ .sym = path_sym };
    try world.chunk_consts.put(allocator, chunk_id, consts);
    try world.chunk_ics.put(allocator, chunk_id, try allocator.alloc(ICache, 1));
    world.chunk_ics.get(chunk_id).?[0] = ICache.empty;
    try world.chunk_params.put(allocator, chunk_id, try allocator.alloc(u32, 0));
    std.debug.print("running [$here serializeTo: '{s}]...\n", .{out_path});
    const result = try vm.runTop(&world, chunk_id);
    printResult(result, &world);
}

fn runBuildTrivialVat(allocator: std.mem.Allocator, io: std.Io, out_path: []const u8) !void {
    var world = try World.init(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    try intrinsics.install(&world);
    const plus_sym = try world.syms.intern("+");
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 1 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = plus_sym, .argc = 1, .ic_idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.return_op, &buf, allocator);
    try world.chunk_bytecode.put(allocator, chunk_id, try allocator.dupe(u8, buf.items));
    const consts = try allocator.alloc(Value, 2);
    consts[0] = .{ .int = 1 };
    consts[1] = .{ .int = 2 };
    try world.chunk_consts.put(allocator, chunk_id, consts);
    try world.chunk_ics.put(allocator, chunk_id, try allocator.alloc(ICache, 1));
    world.chunk_ics.get(chunk_id).?[0] = ICache.empty;
    try world.chunk_params.put(allocator, chunk_id, try allocator.alloc(u32, 0));
    var out_buf: std.ArrayList(u8) = .empty;
    defer out_buf.deinit(allocator);
    try image.serializeVat(&world, &out_buf, allocator);
    try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = out_path, .data = out_buf.items, .flags = .{ .truncate = true } });
    std.debug.print("wrote trivial vat to {s}\n", .{out_path});
}

fn runStressRecursion(allocator: std.mem.Allocator, depth: u32) !void {
    var world = try World.init(allocator);
    defer world.deinit();
    applyGcOpts(&world);
    try intrinsics.install(&world);
    std.debug.print("stress-recursion: depth = {d}\n", .{depth});
    const n_sym = try world.syms.intern("n");
    const rec_sym = try world.syms.intern("rec:");
    const plus_sym = try world.syms.intern("+");
    const minus_sym = try world.syms.intern("-");
    const gt_sym = try world.syms.intern(">");
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(allocator);
    try bytecode.encodeOp(.{ .load_name = .{ .name = n_sym } }, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = gt_sym, .argc = 1, .ic_idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .jump_if_false = .{ .offset = 37 } }, &buf, allocator);
    try bytecode.encodeOp(.load_self, &buf, allocator);
    try bytecode.encodeOp(.{ .load_name = .{ .name = n_sym } }, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 1 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = minus_sym, .argc = 1, .ic_idx = 1 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = rec_sym, .argc = 1, .ic_idx = 2 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.{ .send = .{ .selector = plus_sym, .argc = 1, .ic_idx = 3 } }, &buf, allocator);
    try bytecode.encodeOp(.return_op, &buf, allocator);
    try bytecode.encodeOp(.{ .load_const = .{ .idx = 0 } }, &buf, allocator);
    try bytecode.encodeOp(.return_op, &buf, allocator);
    try world.chunk_bytecode.put(allocator, chunk_id, try allocator.dupe(u8, buf.items));
    const consts = try allocator.alloc(Value, 2);
    consts[0] = .{ .int = 0 };
    consts[1] = .{ .int = 1 };
    try world.chunk_consts.put(allocator, chunk_id, consts);
    try world.chunk_ics.put(allocator, chunk_id, try allocator.alloc(ICache, 4));
    for (world.chunk_ics.get(chunk_id).?) |*ic| ic.* = ICache.empty;
    const params = try allocator.alloc(u32, 1);
    params[0] = n_sym;
    try world.chunk_params.put(allocator, chunk_id, params);
    const proto_id = try world.heap.alloc(Form.init());
    try world.heap.getMut(proto_id).handlers.put(allocator, rec_sym, .{ .form = chunk_id });
    const instance_id = try world.heap.alloc(Form.withProto(.{ .form = proto_id }));
    const result = try world.send(.{ .form = instance_id }, rec_sym, &.{.{ .int = depth }});
    std.debug.print("=> {s}\n", .{@tagName(result)});
}
