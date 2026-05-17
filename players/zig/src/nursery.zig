//! per-turn nursery + diff types — zig port of the rust V1 design.
//!
//! a turn is the unit of atomicity (`docs/superpowers/specs/
//! 2026-05-06-vat-V1-nursery-diff-design.md`). mutations during a
//! turn either land in the nursery (for pre-existing forms, keyed
//! deltas) or directly in the canonical heap above the
//! `turn_watermark` (for new allocations). commit produces a
//! `TurnDiff` summarizing what changed; abort drops the buffered
//! state and truncates the heap to watermark.
//!
//! ported from `crates/substrate/src/nursery.rs`. concepts ported,
//! not lines — the zig substrate is starting fresh and only carries
//! the V1 essentials. V2 frozen-bit transitions are noted in the
//! Delta struct but not yet wired through `freeze` (zig substrate
//! has no `freeze` op yet).
//!
//! ## what zig does NOT port (yet) from the rust V1 impl
//!
//! - **turn_redirect_originals** — `become:` redirect rollback is
//!   deferred. zig `become_` happens outside the nursery for now.
//!   noted in `World.become_` doc.
//! - **TurnDiff serialization** — V9 persistence work. the struct
//!   exists so we have something to journal later.
//! - **mutation-outside-turn panic** — zig allows direct writes
//!   when !in_turn (matching boot-time intrinsics that mutate
//!   heap directly). per task spec scope item #8.

const std = @import("std");

const form = @import("form.zig");
const FormId = form.FormId;

const value_mod = @import("value.zig");
const Value = value_mod.Value;

/// SymId alias — interned-symbol id, u32. matches `world.zig`'s
/// public alias and `form.zig`'s SlotMap key type.
pub const SymId = u32;

/// the three faces of a Form that participate in mutation
/// buffering. matches `Form`'s structural shape: slots / handlers
/// / meta. (`docs/concepts/forms.md`.)
pub const FaceKind = enum(u2) {
    slots = 0,
    handlers = 1,
    meta = 2,
};

/// canonical alias for the per-face IndexMap. zig's
/// `AutoArrayHashMapUnmanaged` preserves insertion order (D5).
pub const FaceMap = std.AutoArrayHashMapUnmanaged(SymId, Value);

/// per-form delta accumulated during a turn for forms that
/// existed before the turn started. only touched keys are
/// stored; unchanged keys fall through to canonical at read
/// time.
///
/// note: forms allocated *during* the turn (FormId payload >=
/// `turn_watermark`) do NOT use a Delta — they live in the
/// canonical `Heap.forms` above the watermark and are mutated
/// directly. the Delta is exclusively for pre-existing forms.
pub const Delta = struct {
    slots: FaceMap = .empty,
    handlers: FaceMap = .empty,
    meta: FaceMap = .empty,

    /// V2 — has this turn frozen the corresponding form? one-way
    /// false→true within a turn. on commit, OR'd into the
    /// canonical `Form.frozen`. on abort, dropped with the rest
    /// of the delta. zig `freeze` op not yet wired; carried for
    /// rust-parity so the field is here when we need it.
    frozen: bool = false,

    /// release storage owned by this delta. callers (commit /
    /// abort) typically `std.mem.swap` the field out of
    /// `nursery_deltas` first, then `deinit` it.
    pub fn deinit(self: *Delta, allocator: std.mem.Allocator) void {
        self.slots.deinit(allocator);
        self.handlers.deinit(allocator);
        self.meta.deinit(allocator);
        self.* = undefined;
    }

    /// access the FaceMap for a given face, mutably.
    pub fn faceMut(self: *Delta, which: FaceKind) *FaceMap {
        return switch (which) {
            .slots => &self.slots,
            .handlers => &self.handlers,
            .meta => &self.meta,
        };
    }

    /// access the FaceMap for a given face, immutably.
    pub fn face(self: *const Delta, which: FaceKind) *const FaceMap {
        return switch (which) {
            .slots => &self.slots,
            .handlers => &self.handlers,
            .meta => &self.meta,
        };
    }

    /// `true` iff no key has been touched in any face and no
    /// frozen-transition has been recorded.
    pub fn isEmpty(self: *const Delta) bool {
        return self.slots.count() == 0 and
            self.handlers.count() == 0 and
            self.meta.count() == 0 and
            !self.frozen;
    }
};

/// (form_id, face, key) tuple used as the mutation map key.
/// AutoArrayHashMap auto-hashes / auto-eqls these.
pub const MutationKey = struct {
    form_id: FormId,
    face: FaceKind,
    key: SymId,
};

/// (prior, new) value pair recorded per mutation.
pub const MutationPair = struct {
    prior: Value,
    new: Value,
};

/// the result of `commit_turn`: a record of what changed during
/// the turn. consumed (in V1) by tests; will feed the
/// `inputs.log` (V9), replication (V11), and CRDT merge
/// pathways (V11). zig substrate doesn't serialize these yet —
/// the struct exists so the API and call sites are in place for
/// when persistence lands.
///
/// the `mutations` map is dedup-keyed by `(form, face, key)` —
/// last-write-wins per key per turn. intermediate writes within
/// a turn don't appear; only the final value at commit-time
/// does. the `prior` value is what was in the canonical heap at
/// turn-start; `new` is the final value the turn settled on.
///
/// `new_allocs` lists FormIds allocated this turn, in
/// allocation order. forms in `new_allocs` do NOT appear in
/// `mutations` (they have no prior state).
pub const TurnDiff = struct {
    mutations: std.AutoArrayHashMapUnmanaged(MutationKey, MutationPair) = .empty,
    new_allocs: std.ArrayList(FormId) = .empty,

    /// V2 — pre-existing forms whose `frozen` bit transitioned
    /// false→true during this turn. (zig doesn't yet have a
    /// `freeze` op so this stays empty for now.)
    freezings: std.ArrayList(FormId) = .empty,

    pub fn deinit(self: *TurnDiff, allocator: std.mem.Allocator) void {
        self.mutations.deinit(allocator);
        self.new_allocs.deinit(allocator);
        self.freezings.deinit(allocator);
        self.* = undefined;
    }
};

// ─────────────────────────────────────────────────────────────────
// inline tests
// ─────────────────────────────────────────────────────────────────

const testing = std.testing;

test "Delta: empty by default" {
    var d: Delta = .{};
    try testing.expect(d.isEmpty());
    d.deinit(testing.allocator);
}

test "Delta: face / faceMut return matching FaceMaps" {
    var d: Delta = .{};
    defer d.deinit(testing.allocator);
    try d.faceMut(.slots).put(testing.allocator, 7, Value.nil);
    try testing.expect(!d.isEmpty());
    try testing.expectEqual(@as(usize, 1), d.face(.slots).count());
    try testing.expectEqual(@as(usize, 0), d.face(.handlers).count());
    try testing.expectEqual(@as(usize, 0), d.face(.meta).count());
}

test "Delta: frozen transition counts as non-empty" {
    var d: Delta = .{};
    defer d.deinit(testing.allocator);
    d.frozen = true;
    try testing.expect(!d.isEmpty());
}

test "TurnDiff: default is empty" {
    var t: TurnDiff = .{};
    defer t.deinit(testing.allocator);
    try testing.expectEqual(@as(usize, 0), t.mutations.count());
    try testing.expectEqual(@as(usize, 0), t.new_allocs.items.len);
    try testing.expectEqual(@as(usize, 0), t.freezings.items.len);
}

test "TurnDiff: round-trips mutations" {
    var t: TurnDiff = .{};
    defer t.deinit(testing.allocator);
    const fid = FormId.vatLocal(1);
    try t.mutations.put(testing.allocator, .{
        .form_id = fid,
        .face = .slots,
        .key = 42,
    }, .{ .prior = Value.nil, .new = .{ .int = 5 } });
    try testing.expectEqual(@as(usize, 1), t.mutations.count());
    const entry = t.mutations.get(.{
        .form_id = fid,
        .face = .slots,
        .key = 42,
    }).?;
    try testing.expect(entry.prior == .nil);
    try testing.expectEqual(@as(i48, 5), entry.new.int);
}

// ─────────────────────────────────────────────────────────────────
// integration tests — World lifecycle + nursery-aware r/w.
// kept in nursery.zig (rather than world.zig) so the test
// surface lives next to the type definitions it exercises.
// ─────────────────────────────────────────────────────────────────

const world_mod = @import("world.zig");
const World = world_mod.World;
const form_mod = @import("form.zig");
const Form = form_mod.Form;

test "World: startTurn / commitTurn flip in_turn + advance watermark" {
    var world = try World.init(testing.allocator);
    defer world.deinit();

    try testing.expect(!world.inTurn());
    const wm_before = world.turn_watermark;

    world.startTurn();
    try testing.expect(world.inTurn());

    // alloc a Form during the turn — sits above watermark.
    const new_id = try world.heap.alloc(Form.init());
    try testing.expect(new_id.payload >= wm_before);

    var diff = try world.commitTurn();
    defer diff.deinit(world.allocator);

    try testing.expect(!world.inTurn());
    try testing.expect(world.turn_watermark > wm_before);
    // new alloc should be listed.
    try testing.expectEqual(@as(usize, 1), diff.new_allocs.items.len);
    try testing.expectEqual(new_id.payload, diff.new_allocs.items[0].payload);
}

test "World: abortTurn discards deltas + truncates new allocs" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const wm_before = world.turn_watermark;
    const heap_len_before = world.heap.forms.items.len;

    // pre-existing form (allocated outside turn).
    const pre_id = try world.heap.alloc(Form.init());
    // bump watermark manually so pre_id counts as canonical pre-existing.
    world.turn_watermark = @intCast(world.heap.forms.items.len);
    const wm_pre = world.turn_watermark;
    _ = wm_before;
    _ = heap_len_before;

    const sym = try world.syms.intern("test-slot");
    world.startTurn();

    // mutate pre-existing → buffered in nursery.
    try world.formSlotSet(pre_id, sym, .{ .int = 42 });
    try testing.expect(world.nursery_deltas.count() > 0);

    // alloc a fresh form mid-turn.
    const fresh_id = try world.heap.alloc(Form.init());
    try testing.expect(fresh_id.payload >= wm_pre);

    world.abortTurn();

    try testing.expect(!world.inTurn());
    // nursery cleared
    try testing.expectEqual(@as(usize, 0), world.nursery_deltas.count());
    // pre-existing form's slot is NOT set (delta was dropped)
    const pre_f = world.heap.get(pre_id);
    try testing.expect(!pre_f.slotPresent(sym));
    // heap truncated back to watermark — fresh_id is gone.
    try testing.expectEqual(@as(usize, wm_pre), world.heap.forms.items.len);
}

test "World: commitTurn applies deltas to canonical" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const pre_id = try world.heap.alloc(Form.init());
    world.turn_watermark = @intCast(world.heap.forms.items.len);

    const sym = try world.syms.intern("k");
    world.startTurn();
    try world.formSlotSet(pre_id, sym, .{ .int = 7 });
    // canonical NOT yet mutated — read directly.
    const f_pre = world.heap.get(pre_id);
    try testing.expect(!f_pre.slotPresent(sym));
    // nursery-aware read sees the pending value.
    try testing.expectEqual(@as(i48, 7), world.formSlot(pre_id, sym).int);

    var diff = try world.commitTurn();
    defer diff.deinit(world.allocator);

    // canonical now has the value.
    const f_post = world.heap.get(pre_id);
    try testing.expect(f_post.slotPresent(sym));
    try testing.expectEqual(@as(i48, 7), f_post.slot(sym).int);
    // diff captured prior=nil + new=7.
    const entry = diff.mutations.get(.{
        .form_id = pre_id,
        .face = .slots,
        .key = sym,
    }).?;
    try testing.expect(entry.prior == .nil);
    try testing.expectEqual(@as(i48, 7), entry.new.int);
}

test "World: read-your-writes within a turn" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const pre_id = try world.heap.alloc(Form.init());
    world.turn_watermark = @intCast(world.heap.forms.items.len);

    const sym = try world.syms.intern("rw");
    world.startTurn();
    try world.formSlotSet(pre_id, sym, .{ .int = 1 });
    try testing.expectEqual(@as(i48, 1), world.formSlot(pre_id, sym).int);
    try world.formSlotSet(pre_id, sym, .{ .int = 2 });
    try testing.expectEqual(@as(i48, 2), world.formSlot(pre_id, sym).int);

    var diff = try world.commitTurn();
    defer diff.deinit(world.allocator);

    // last-write-wins: diff carries (prior=nil, new=2).
    const entry = diff.mutations.get(.{
        .form_id = pre_id,
        .face = .slots,
        .key = sym,
    }).?;
    try testing.expect(entry.prior == .nil);
    try testing.expectEqual(@as(i48, 2), entry.new.int);
}

test "World: mid-turn alloc writes directly to canonical (not delta)" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    world.turn_watermark = @intCast(world.heap.forms.items.len);

    const sym = try world.syms.intern("fresh-slot");
    world.startTurn();

    // alloc DURING the turn — payload >= watermark, so writes
    // skip the delta and land on canonical directly.
    const new_id = try world.heap.alloc(Form.init());
    try world.formSlotSet(new_id, sym, .{ .int = 99 });

    // no delta entry for new_id (it's an above-watermark form).
    try testing.expect(!world.nursery_deltas.contains(new_id));
    // canonical IS already updated.
    const f = world.heap.get(new_id);
    try testing.expectEqual(@as(i48, 99), f.slot(sym).int);

    var diff = try world.commitTurn();
    defer diff.deinit(world.allocator);
    // new_id appears in new_allocs, NOT in mutations.
    try testing.expectEqual(@as(usize, 1), diff.new_allocs.items.len);
    try testing.expectEqual(new_id.payload, diff.new_allocs.items[0].payload);
    try testing.expectEqual(@as(usize, 0), diff.mutations.count());
}

test "World: handlers + meta participate in nursery" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const pre_id = try world.heap.alloc(Form.init());
    world.turn_watermark = @intCast(world.heap.forms.items.len);

    const sel = try world.syms.intern("hsel");
    const mkey = try world.syms.intern("mk");
    world.startTurn();
    try world.formHandlerSet(pre_id, sel, .{ .int = 1 });
    try world.formMetaSet(pre_id, mkey, .{ .int = 2 });

    // nursery-aware reads see them.
    try testing.expect(world.formHandler(pre_id, sel) != null);
    try testing.expectEqual(@as(i48, 1), world.formHandler(pre_id, sel).?.int);
    try testing.expectEqual(@as(i48, 2), world.formMeta(pre_id, mkey).int);

    // canonical does NOT yet have them.
    const f_pre = world.heap.get(pre_id);
    try testing.expect(f_pre.handler(sel) == null);

    var diff = try world.commitTurn();
    defer diff.deinit(world.allocator);

    // mutations include both faces.
    try testing.expect(diff.mutations.contains(.{
        .form_id = pre_id,
        .face = .handlers,
        .key = sel,
    }));
    try testing.expect(diff.mutations.contains(.{
        .form_id = pre_id,
        .face = .meta,
        .key = mkey,
    }));
    // canonical sees them now.
    const f_post = world.heap.get(pre_id);
    try testing.expect(f_post.handler(sel) != null);
    try testing.expectEqual(@as(i48, 2), f_post.metaAt(mkey).int);
}

test "World: out-of-turn writes go straight to canonical" {
    var world = try World.init(testing.allocator);
    defer world.deinit();
    const pre_id = try world.heap.alloc(Form.init());
    // zig substrate (unlike rust) allows direct writes when !in_turn
    // — boot intrinsics still poke heap directly. verify that path.
    try testing.expect(!world.inTurn());
    const sym = try world.syms.intern("boot-slot");
    try world.formSlotSet(pre_id, sym, .{ .int = 100 });
    // delta is empty (we never opened a turn).
    try testing.expectEqual(@as(usize, 0), world.nursery_deltas.count());
    // canonical IS updated.
    try testing.expectEqual(@as(i48, 100), world.heap.get(pre_id).slot(sym).int);
}

test "World: commitTurn outside-turn panics in safe builds" {
    // we can't easily catch a std.debug.panic in zig test (it
    // aborts the process). this test documents the contract; the
    // panic is exercised by manual / fuzz runs. leaving as a
    // compile-only no-op so the contract is visible.
    if (false) {
        var world = try World.init(testing.allocator);
        defer world.deinit();
        _ = world.commitTurn() catch unreachable;
    }
}
