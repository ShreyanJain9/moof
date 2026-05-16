//! moof-zig — phase 1 mark-sweep garbage collector.
//!
//! design + spec: `docs/superpowers/specs/2026-05-11-phase1-gc-
//! dispatch-compression-design.md` §3.
//!
//! algorithm: mark-sweep, **sparse** (no compaction). FormIds are
//! never reused — L11 (FormId stability for the lifetime of a vat)
//! is preserved trivially. when an unmarked Form is swept, the slot
//! in `Heap.forms` becomes a tombstone: empty proto/slots/handlers/
//! meta + `gc_tombstone = true`. allocators continue to extend the
//! list; no callers reach a tombstone via a live reference (if they
//! do, the GC missed a root).
//!
//! trigger: `runTop` exit (the "turn boundary stand-in" — see spec
//! §3.5 option A). only the outermost moof call triggers; inner
//! `runMethod` calls from natives (option α) do not, since the heap
//! is not quiescent mid-native-call.
//!
//! determinism: mark order = root iteration order. all backing tables
//! are `AutoArrayHashMapUnmanaged` (insertion-order — D5). a fresh
//! `ArrayList` worklist is used; we discharge LIFO. for replicated
//! vats (phase D), the iteration order is stable per D5; the LIFO
//! discharge order only changes which Form gets marked first, which
//! has no observable effect post-sweep.
//!
//! statistics: when `gc_stats_enabled` is true on the World, the
//! collect call prints a single-line summary to stderr. enabled by
//! the `MOOF_GC_STATS=1` env var (read at startup) or the `--gc-stats`
//! CLI flag.
//!
//! see also:
//! - `crates/zig-substrate/src/heap.zig` — `Heap.gcResetMarks`,
//!   `Heap.gcMark`, `Heap.gcIsMarked`, `Heap.gcTombstone`.
//! - `crates/zig-substrate/src/form.zig` — `Form.gc_mark`,
//!   `Form.gc_tombstone`.
//! - `crates/zig-substrate/src/vm.zig` — `runTop` calls `World.collect`
//!   after the outermost frame returns when `gc_enabled` is true.
//! - `laws/substrate-laws.md` L10, L11.
//! - `laws/determinism-laws.md` D5, D6.

const std = @import("std");

const value_mod = @import("value.zig");
const Value = value_mod.Value;

const form_mod = @import("form.zig");
const FormId = form_mod.FormId;

const world_mod = @import("world.zig");
const World = world_mod.World;

/// summary of a single collection cycle. printed to stderr when
/// `world.gc_stats_enabled` is true; also returned from `collect`
/// for tests.
pub const GcStats = struct {
    /// number of Forms in the heap before this collection (excluding
    /// the sentinel at index 0).
    total_before: usize,
    /// number of Forms marked live by this cycle (includes the
    /// sentinel — counted at index 0 implicitly as live).
    live: usize,
    /// number of Forms freshly tombstoned by this cycle.
    swept: usize,
    /// total tombstones in the heap after this cycle (cumulative —
    /// includes tombstones from prior cycles).
    tombstones_total: usize,
};

/// run one mark-sweep cycle. callers should hold the World's
/// invariants intact (no in-flight mid-turn mutation in another
/// thread — single-threaded substrate). returns the cycle's stats.
pub fn collect(world: *World) !GcStats {
    const total_before = world.heap.forms.items.len -| 1;

    // 1. reset marks.
    world.heap.gcResetMarks();
    // sentinel at index 0 is conceptually live (it's never an
    // allocation target, but we don't want sweep to touch it).
    if (world.heap.forms.items.len > 0) {
        world.heap.forms.items[0].gc_mark = true;
    }

    // 2. seed worklist with roots, then drain.
    var worklist: std.ArrayList(FormId) = .empty;
    defer worklist.deinit(world.allocator);

    try seedRoots(world, &worklist);
    try drainWorklist(world, &worklist);

    // 3. sweep — tombstone unmarked Forms, drop side-table entries.
    const sweep_result = try sweepHeap(world);

    return GcStats{
        .total_before = total_before,
        .live = sweep_result.live,
        .swept = sweep_result.swept,
        .tombstones_total = sweep_result.tombstones_total,
    };
}

/// seed the worklist with every root FormId. roots per spec §3.2.
/// each `addIfForm` is a no-op for `FormId.NONE` and tagged-immediate
/// Values — only `Value{.form = id}` (or a bare live FormId) gets
/// pushed.
fn seedRoots(world: *World, worklist: *std.ArrayList(FormId)) !void {
    // here_form, macros_form — the two canonical Form-roots.
    try addIfFormId(world, worklist, world.here_form);
    try addIfFormId(world, worklist, world.macros_form);

    // all 18 canonical protos (Protos struct).
    try addIfFormId(world, worklist, world.protos.object);
    try addIfFormId(world, worklist, world.protos.nil);
    try addIfFormId(world, worklist, world.protos.bool_);
    try addIfFormId(world, worklist, world.protos.integer);
    try addIfFormId(world, worklist, world.protos.char);
    try addIfFormId(world, worklist, world.protos.sym);
    try addIfFormId(world, worklist, world.protos.cons);
    try addIfFormId(world, worklist, world.protos.string);
    try addIfFormId(world, worklist, world.protos.bytes);
    try addIfFormId(world, worklist, world.protos.method);
    try addIfFormId(world, worklist, world.protos.chunk);
    try addIfFormId(world, worklist, world.protos.closure);
    try addIfFormId(world, worklist, world.protos.env);
    try addIfFormId(world, worklist, world.protos.foreign_handle);
    try addIfFormId(world, worklist, world.protos.table);
    try addIfFormId(world, worklist, world.protos.frame);
    try addIfFormId(world, worklist, world.protos.macros);
    try addIfFormId(world, worklist, world.protos.opcode);

    // VM frames — chunk, env, self_, defining_proto.
    for (world.vm.frames.items) |frame| {
        try addIfFormId(world, worklist, frame.chunk);
        try addIfFormId(world, worklist, frame.env);
        try addIfFormValue(world, worklist, frame.self_);
        try addIfFormId(world, worklist, frame.defining_proto);
    }

    // VM operand stack.
    for (world.vm.stack.items) |v| try addIfFormValue(world, worklist, v);

    // chunk side-tables (keyed by chunk-FormId — the chunk itself is
    // a root) plus their Value payloads (consts, IC slots).
    {
        var it = world.chunk_bytecode.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
        }
    }
    {
        var it = world.chunk_consts.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
            for (entry.value_ptr.*) |v| try addIfFormValue(world, worklist, v);
        }
    }
    {
        var it = world.chunk_ics.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
            for (entry.value_ptr.*) |ic| {
                try addIfFormId(world, worklist, ic.cached_proto);
                try addIfFormId(world, worklist, ic.cached_method);
                try addIfFormId(world, worklist, ic.cached_defining);
                try addIfFormId(world, worklist, ic.cached_singleton);
            }
        }
    }
    {
        var it = world.chunk_params.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
            // params are SymIds (u32), not FormIds — no walk.
        }
    }

    // native_fns keys — the method-Forms bound to native functions.
    {
        var it = world.native_fns.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
        }
    }

    // proto_generation keys — protos with bumped generation counters.
    {
        var it = world.proto_generation.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
        }
    }

    // far_ref_table keys — far-ref FormIds. these are .far_ref scope,
    // so `addIfFormId` will skip them (only vat-local can be marked).
    // we still iterate so that vat-local Forms referenced by far-ref
    // target_form_id could be tracked if we were to add a cross-scope
    // root walk later. for V1, far_ref keys are vat-local-only by
    // construction in image-load, so this is a safe no-op walk.
    {
        var it = world.far_ref_table.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
        }
    }

    // become_ redirects — both keys and values. per spec §11 Q3,
    // a→b means anyone holding `a` still indexes through it; both
    // ends are live.
    {
        var it = world.heap.redirects.iterator();
        while (it.next()) |entry| {
            try addIfFormId(world, worklist, entry.key_ptr.*);
            try addIfFormId(world, worklist, entry.value_ptr.*);
        }
    }
}

/// discharge the worklist LIFO. for each FormId: mark it, then
/// push its outgoing Value-references (proto + slots/handlers/meta).
fn drainWorklist(world: *World, worklist: *std.ArrayList(FormId)) !void {
    while (worklist.items.len > 0) {
        const id = worklist.pop().?;
        if (id.scope != .vat_local) continue;
        if (id.payload >= world.heap.forms.items.len) continue;
        const f = &world.heap.forms.items[id.payload];
        if (f.gc_mark) continue;
        f.gc_mark = true;

        // proto walks like any other Value.
        try addIfFormValue(world, worklist, f.proto);

        // slots / handlers / meta — insertion-order maps (D5).
        var sit = f.slots.iterator();
        while (sit.next()) |e| try addIfFormValue(world, worklist, e.value_ptr.*);

        var hit = f.handlers.iterator();
        while (hit.next()) |e| try addIfFormValue(world, worklist, e.value_ptr.*);

        var mit = f.meta.iterator();
        while (mit.next()) |e| try addIfFormValue(world, worklist, e.value_ptr.*);
    }
}

/// push `id` onto the worklist if it's a vat-local non-sentinel
/// FormId that hasn't been marked yet. tagged immediates / shared /
/// far-ref scopes are not GC'd by V1.
fn addIfFormId(world: *World, worklist: *std.ArrayList(FormId), id: FormId) !void {
    if (id.isNone()) return;
    if (id.scope != .vat_local) return;
    if (id.payload >= world.heap.forms.items.len) return;
    if (world.heap.forms.items[id.payload].gc_mark) return;
    // chase redirects so we mark the canonical target (and the
    // redirect key got pre-marked via seedRoots's redirects walk).
    const resolved = world.heap.resolveId(id);
    try worklist.append(world.allocator, resolved);
}

/// shortcut for `Value`-typed roots — extracts the FormId if present.
fn addIfFormValue(world: *World, worklist: *std.ArrayList(FormId), v: Value) !void {
    switch (v) {
        .form => |id| try addIfFormId(world, worklist, id),
        else => {},
    }
}

const SweepResult = struct {
    live: usize,
    swept: usize,
    tombstones_total: usize,
};

/// walk `heap.forms` and tombstone everything not marked. also drop
/// side-table entries keyed on freshly-tombstoned FormIds (otherwise
/// the side-table contents leak even though the Form is gone).
///
/// side-tables removed from per spec §3.4:
/// - chunk_bytecode (owns a byte slice)
/// - chunk_consts   (owns a Value slice)
/// - chunk_ics      (owns an ICache slice)
/// - chunk_params   (owns a u32 slice)
/// - native_fns     (function pointer; no owned heap)
/// - proto_generation (u32; no owned heap)
///
/// far_ref_table is keyed on .far_ref scope — never tombstoned here.
/// heap.redirects entries point to vat-local ids on both sides; per
/// the spec §3.2 + §11 Q3 they're roots; if the entry's key/value
/// becomes unreachable from any other root, the entry's own root-ness
/// kept it alive — but as soon as no one *else* references the key,
/// the redirect is dead and should drop. for V1 we keep redirects
/// intact (they're always alive) — phase 2 may revisit.
fn sweepHeap(world: *World) !SweepResult {
    var live: usize = 0;
    var swept: usize = 0;
    var tombstones_total: usize = 0;

    // start at i=1 — index 0 is the sentinel, never an allocation.
    var i: usize = 1;
    while (i < world.heap.forms.items.len) : (i += 1) {
        const f = &world.heap.forms.items[i];
        if (f.gc_tombstone) {
            tombstones_total += 1;
            continue;
        }
        if (f.gc_mark) {
            live += 1;
            continue;
        }
        // freshly dead — tombstone it.
        const payload: u30 = @intCast(i);
        const fid = FormId.vatLocal(payload);

        // drop side-table entries first (so chunk_bytecode et al
        // free their owned slices), THEN tombstone the Form itself.
        try sweepSideTables(world, fid);

        world.heap.gcTombstone(payload);
        swept += 1;
        tombstones_total += 1;
    }

    return SweepResult{
        .live = live,
        .swept = swept,
        .tombstones_total = tombstones_total,
    };
}

/// remove `fid` from every owned side-table, freeing inner slices.
/// safe to call on a FormId not in any side-table (the swapRemove
/// returns false; nothing happens).
fn sweepSideTables(world: *World, fid: FormId) !void {
    if (world.chunk_bytecode.fetchSwapRemove(fid)) |kv| {
        world.allocator.free(kv.value);
    }
    if (world.chunk_consts.fetchSwapRemove(fid)) |kv| {
        world.allocator.free(kv.value);
    }
    if (world.chunk_ics.fetchSwapRemove(fid)) |kv| {
        world.allocator.free(kv.value);
    }
    if (world.chunk_params.fetchSwapRemove(fid)) |kv| {
        world.allocator.free(kv.value);
    }
    _ = world.native_fns.remove(fid);
    _ = world.proto_generation.swapRemove(fid);
}

/// print stats to stderr. caller checks `world.gc_stats_enabled`.
pub fn printStats(stats: GcStats) void {
    std.debug.print(
        "[gc] total_before={d} live={d} swept={d} tombstones_total={d}\n",
        .{ stats.total_before, stats.live, stats.swept, stats.tombstones_total },
    );
}

// ─────────────────────────────────────────────────────────────────
// inline tests (zig 0.16 `zig test`)
// ─────────────────────────────────────────────────────────────────

const testing = std.testing;
const Form = form_mod.Form;

test "GC: unreachable Form gets tombstoned" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const garbage_id = try world.heap.alloc(Form.init());
    try testing.expect(!world.heap.forms.items[garbage_id.payload].gc_tombstone);
    const stats = try collect(&world);
    try testing.expect(stats.swept >= 1);
    try testing.expect(world.heap.forms.items[garbage_id.payload].gc_tombstone);
}

test "GC: Form reachable via here_form survives" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const live_id = try world.heap.alloc(Form.init());
    const slot_sym = try world.syms.intern("live-test");
    try world.envBind(world.here_form, slot_sym, .{ .form = live_id });
    const stats = try collect(&world);
    _ = stats;
    try testing.expect(!world.heap.forms.items[live_id.payload].gc_tombstone);
}

test "GC: Form reachable only via VM stack survives" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const live_id = try world.heap.alloc(Form.init());
    try world.vm.stack.append(world.allocator, .{ .form = live_id });
    _ = try collect(&world);
    try testing.expect(!world.heap.forms.items[live_id.payload].gc_tombstone);
    _ = world.vm.stack.pop();
}

test "GC: Form reachable via chunk_consts survives" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const live_id = try world.heap.alloc(Form.init());
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    const consts = try world.allocator.alloc(value_mod.Value, 1);
    consts[0] = .{ .form = live_id };
    try world.chunk_consts.put(world.allocator, chunk_id, consts);
    // chunk_id must also be reachable, else its side-table entry would
    // be swept along with it. install it on here_form to keep it alive.
    const chunk_slot = try world.syms.intern("test-chunk");
    try world.envBind(world.here_form, chunk_slot, .{ .form = chunk_id });
    _ = try collect(&world);
    try testing.expect(!world.heap.forms.items[live_id.payload].gc_tombstone);
    try testing.expect(!world.heap.forms.items[chunk_id.payload].gc_tombstone);
}

test "GC: FormIds of survivors are unchanged (L11)" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const slot_sym = try world.syms.intern("anchor");
    const a = try world.heap.alloc(Form.init());
    _ = try world.heap.alloc(Form.init()); // garbage
    const c = try world.heap.alloc(Form.init());
    _ = try world.heap.alloc(Form.init()); // garbage
    try world.envBind(world.here_form, slot_sym, .{ .form = a });
    const c_slot = try world.syms.intern("c-anchor");
    try world.envBind(world.here_form, c_slot, .{ .form = c });
    const a_payload = a.payload;
    const c_payload = c.payload;
    _ = try collect(&world);
    // L11: survivor FormIds unchanged.
    try testing.expectEqual(a_payload, a.payload);
    try testing.expectEqual(c_payload, c.payload);
    try testing.expect(!world.heap.forms.items[a_payload].gc_tombstone);
    try testing.expect(!world.heap.forms.items[c_payload].gc_tombstone);
}

test "GC: tombstones not reused — next alloc gets a fresh FormId" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const garbage = try world.heap.alloc(Form.init());
    const garbage_payload = garbage.payload;
    _ = try collect(&world);
    try testing.expect(world.heap.forms.items[garbage_payload].gc_tombstone);

    const fresh = try world.heap.alloc(Form.init());
    // tombstones not reused: fresh.payload != garbage_payload.
    try testing.expect(fresh.payload != garbage_payload);
}

test "GC: --no-gc path skips collection" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.gc_enabled = false;
    const garbage = try world.heap.alloc(Form.init());
    const result = try world.collect();
    try testing.expect(result == null);
    // garbage still alive (no collection happened).
    try testing.expect(!world.heap.forms.items[garbage.payload].gc_tombstone);
}

test "GC: chunk side-tables stay attached while chunk is alive" {
    // sanity test: per spec §3.2 chunk_bytecode/consts/ics KEYS are
    // themselves roots — a chunk-Form is live as long as its side-
    // table entries exist. so allocating + side-table-installing a
    // chunk and immediately GCing should NOT sweep it.
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    const body = try world.allocator.dupe(u8, &[_]u8{ 0x01, 0x33 });
    try world.chunk_bytecode.put(world.allocator, chunk_id, body);
    try world.chunk_consts.put(world.allocator, chunk_id, try world.allocator.alloc(value_mod.Value, 0));
    try world.chunk_ics.put(world.allocator, chunk_id, try world.allocator.alloc(world_mod.ICache, 0));
    _ = try collect(&world);
    try testing.expect(world.chunk_bytecode.contains(chunk_id));
    try testing.expect(!world.heap.forms.items[chunk_id.payload].gc_tombstone);
}

test "GC: chunk side-tables freed when chunk is explicitly removed" {
    // counterpart: if the user removes a chunk from chunk_bytecode
    // (e.g. via a hypothetical chunk-eviction op), and the chunk-Form
    // isn't reachable from any other root, both the chunk and its
    // remaining side-table entries should sweep.
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const chunk_id = try world.heap.alloc(Form.withProto(.{ .form = world.protos.chunk }));
    const body = try world.allocator.dupe(u8, &[_]u8{ 0x01, 0x33 });
    try world.chunk_bytecode.put(world.allocator, chunk_id, body);
    try world.chunk_consts.put(world.allocator, chunk_id, try world.allocator.alloc(value_mod.Value, 0));
    try world.chunk_ics.put(world.allocator, chunk_id, try world.allocator.alloc(world_mod.ICache, 0));
    // simulate eviction: remove bytecode + consts + ics entries.
    // chunk_id no longer reachable from any root.
    if (world.chunk_bytecode.fetchSwapRemove(chunk_id)) |kv| world.allocator.free(kv.value);
    if (world.chunk_consts.fetchSwapRemove(chunk_id)) |kv| world.allocator.free(kv.value);
    if (world.chunk_ics.fetchSwapRemove(chunk_id)) |kv| world.allocator.free(kv.value);
    _ = try collect(&world);
    try testing.expect(world.heap.forms.items[chunk_id.payload].gc_tombstone);
}
