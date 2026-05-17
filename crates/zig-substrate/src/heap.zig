//! Form heap — the substrate's allocator.
//!
//! a contiguous `std.ArrayList(Form)` indexed by the FormId's payload
//! bits. allocation pushes; index zero is reserved as the
//! `FormId.NONE` sentinel.
//!
//! per `laws/substrate-laws.md` L11, FormIds are stable for the life
//! of the vat. we therefore do **not** compact / renumber during gc —
//! phase B's gc tombstones dead slots; phase G+ considers an
//! indirection table if heap density becomes a concern.
//!
//! ## tombstone reuse (post-§5.8d perf, 2026-05-16)
//!
//! the GC's sweep pass tombstones unmarked Forms; previously those
//! slots stayed dead forever (`heap.forms.items.len` grew monotonically,
//! peaking near 440k forms / ~99% tombstones on a bootstrap). this
//! left the heap arena bloated and made every subsequent `gcResetMarks`
//! walk pay for the dead slots' cache footprint.
//!
//! fix: the sweep pushes each freshly-tombstoned `payload` onto
//! `Heap.free_list`. `Heap.alloc` pops from the free-list first
//! before extending `forms.items`. the slot is re-initialized in
//! place — the tombstone bit clears, slot/handler/meta maps are
//! freshly empty (sweep already called `Form.deinit` on them).
//!
//! **L11 sanity:** L11 says "FormId is stable for the lifetime of
//! a vat." tombstoning a Form means the original Form-at-that-FormId
//! is GONE — no user-visible reference can reach it (else the mark
//! pass would have kept it). allocating a NEW Form at the same
//! FormId is logically a new identity. no user can observe both the
//! old and the new at the same time, since the old was unreachable
//! at the moment the GC walked. L11's "lifetime" is the lifetime of
//! the original identity — once that identity is gone, the slot is
//! reusable.
//!
//! this matches how every modern GC handles "address stability":
//! a tracing GC may move objects (we don't), but the conceptual
//! id is only meaningful for the duration the reference is live.
//!
//! per `laws/determinism-laws.md` D4, allocation order in a
//! replicated vat is deterministic by turn-seq + per-turn ordinal.
//! phase A is single-vat solo, so the deterministic-id discipline
//! isn't enforced here yet — `Heap.alloc` simply returns the next
//! index. phase D adds a deterministic allocator.
//!
//! v4-spec correspondence: V4 §10.3 FormSection writes the heap in
//! alloc order; FormId payload N → forms[N]. the sentinel slot at
//! index 0 is not written to disk (the on-disk `num_forms` excludes
//! it; loaders push a fresh sentinel before populating).
//!
//! become: indirection. `[a become: b]` registers `a → b` in the
//! redirects table; subsequent `get` / `getMut` calls chase the
//! redirect before indexing `forms`. resolution is bounded — see
//! `MAX_BECOME_HOPS`. enables live proto migration; backed by the
//! V1 nursery for per-turn replication.

const std = @import("std");
const form_mod = @import("form.zig");
const Form = form_mod.Form;
const FormId = form_mod.FormId;

/// max indirection-chain length. `become_` resolves the target
/// before inserting so a fresh insertion never extends a chain;
/// existing chains arise only when two `become_`s race in a way
/// the current scheduler doesn't permit (single-vat). the bound is
/// purely defensive against future-phase scheduler regressions.
pub const MAX_BECOME_HOPS: usize = 32;

/// a contiguous, single-vat heap of Forms. owns the per-form slot /
/// handler / meta storage and the `become_` redirects table.
///
/// NB: the spec sketch said `std.AutoArrayHashMap(FormId, FormId)`
/// for `redirects` but in zig 0.16 only the *unmanaged* variant
/// exists in std (the managed variant was removed). we therefore
/// hold an allocator field on Heap itself and pass it into the
/// redirects table on every operation. functionally equivalent.
pub const Heap = struct {
    /// every Form allocated in this vat. index 0 is the
    /// `FormId.NONE` sentinel — never returned by `alloc`.
    forms: std.ArrayList(Form),
    /// `become:` indirection: `[a become: b]` registers `a → b`.
    /// `get` / `getMut` chase before indexing `forms`. ArrayHashMap
    /// preserves insertion order — important for D5 determinism
    /// when the redirects are part of a vat snapshot.
    redirects: std.AutoArrayHashMapUnmanaged(FormId, FormId),
    /// **tombstone free-list (post-§5.8d).** GC sweep pushes the
    /// payload of each freshly-tombstoned slot here; `alloc` pops
    /// from here before extending `forms.items`. LIFO so the most
    /// recently freed slot is reused first (better cache locality —
    /// the slot's hash-map storage may still be hot when the next
    /// alloc lands).
    ///
    /// D5 determinism: free-list discharge order is deterministic
    /// (LIFO over deterministic sweep order). replicas sweep in the
    /// same order (index-ascending across `forms.items`), push in
    /// the same order, pop in the same order → reused FormIds match
    /// across replicas.
    ///
    /// L11: see heap.zig header. tombstoned slots have no live
    /// references, so reusing their FormId for a new identity is
    /// observably safe.
    free_list: std.ArrayListUnmanaged(u30),
    /// shared allocator for `forms` growth, redirects growth, and
    /// each Form's slot/handler/meta maps.
    allocator: std.mem.Allocator,

    /// build an empty heap. allocates the sentinel slot so that
    /// `FormId.NONE` is never confused with a real allocation.
    pub fn init(allocator: std.mem.Allocator) !Heap {
        var forms: std.ArrayList(Form) = .empty;
        // sentinel placeholder so the first real alloc lands at
        // payload >= 1.
        try forms.append(allocator, Form.init());
        return .{
            .forms = forms,
            .redirects = .empty,
            .free_list = .empty,
            .allocator = allocator,
        };
    }

    /// release the heap. walks every Form and deinits its slot /
    /// handler / meta maps before freeing the ArrayList itself.
    pub fn deinit(self: *Heap) void {
        for (self.forms.items) |*f| f.deinit(self.allocator);
        self.forms.deinit(self.allocator);
        self.redirects.deinit(self.allocator);
        self.free_list.deinit(self.allocator);
        self.* = undefined;
    }

    /// allocate a new Form, returning its (vat-local) FormId.
    ///
    /// the id is stable for the heap's lifetime
    /// (`laws/substrate-laws.md` L11). the caller transfers
    /// ownership of `form` (and its inner maps) to the heap.
    ///
    /// **§5.8d free-list reuse:** if `free_list` is non-empty, pop
    /// a tombstoned slot's payload and re-initialize it in place.
    /// the previous occupant's `slots`/`handlers`/`meta` were freed
    /// by `gcTombstone` (which calls `Form.deinit`), so overwriting
    /// is safe — no double-free, no stale capacity. otherwise,
    /// extend `forms.items` and return the fresh index.
    pub fn alloc(self: *Heap, form: Form) !FormId {
        // perf: count form allocs via the vm profile counter table.
        @import("vm.zig").PROFILE.forms_allocated += 1;
        if (self.free_list.pop()) |payload| {
            // re-init the tombstoned slot in place. the Form value
            // assigned here is the caller's `form` — its maps are
            // owned by the caller and now transfer to the heap.
            // gc_tombstone bit gets cleared by the assignment (we
            // overwrite the whole Form struct).
            self.forms.items[payload] = form;
            return FormId.vatLocal(payload);
        }
        const id = self.forms.items.len;
        // post-V0 the vat-local payload is 30 bits, so the per-vat
        // ceiling is ~1B forms.
        std.debug.assert(id < form_mod.MAX_PAYLOAD);
        try self.forms.append(self.allocator, form);
        return FormId.vatLocal(@intCast(id));
    }

    /// chase the redirects table to find the canonical FormId for
    /// `id`. when `id` is not a redirect source, returns `id`. used
    /// internally by `get` / `getMut` — direct callers rare.
    ///
    /// panics if the chain exceeds `MAX_BECOME_HOPS` (indicates a
    /// cycle, which the `become_` precondition forbids).
    pub fn resolveId(self: *const Heap, id: FormId) FormId {
        var cur = id;
        var hops: usize = 0;
        while (hops < MAX_BECOME_HOPS) : (hops += 1) {
            const next = self.redirects.get(cur) orelse return cur;
            if (next.payload == cur.payload and next.scope == cur.scope) return cur;
            cur = next;
        }
        std.debug.panic(
            "become: redirect chain exceeds {} hops starting at FormId payload {} — cycle?",
            .{ MAX_BECOME_HOPS, id.payload },
        );
    }

    /// borrow a Form by id. chases `become:` redirects before
    /// indexing — every reference catches up automatically.
    ///
    /// panics on `FormId.NONE`, shared/far-ref scopes (deferred to
    /// later phases), or an out-of-range vat-local payload.
    pub fn get(self: *const Heap, id: FormId) *const Form {
        std.debug.assert(!id.isNone());
        const r = self.resolveId(id);
        return switch (r.scope) {
            .vat_local => &self.forms.items[r.payload],
            .shared => std.debug.panic(
                "shared segment not yet supported (V6); got id payload {}",
                .{r.payload},
            ),
            .far_ref => std.debug.panic(
                "far-ref table not yet supported (V5); got id payload {}",
                .{r.payload},
            ),
            .reserved => std.debug.panic(
                "reserved scope: id payload {}",
                .{r.payload},
            ),
        };
    }

    /// mutably borrow a Form by id. chases `become:` redirects.
    /// same panic discipline as `get`.
    pub fn getMut(self: *Heap, id: FormId) *Form {
        std.debug.assert(!id.isNone());
        const r = self.resolveId(id);
        return switch (r.scope) {
            .vat_local => &self.forms.items[r.payload],
            .shared => std.debug.panic(
                "shared segment not yet supported (V6); got id payload {}",
                .{r.payload},
            ),
            .far_ref => std.debug.panic(
                "far-ref table not yet supported (V5); got id payload {}",
                .{r.payload},
            ),
            .reserved => std.debug.panic(
                "reserved scope: id payload {}",
                .{r.payload},
            ),
        };
    }

    /// `[a become: b]` — record an indirection from `a` to `b`.
    ///
    /// resolves `b` first so a fresh redirect never extends an
    /// existing chain (i.e. if `b` was already redirected to `c`,
    /// we record `a → c` directly, keeping chains short).
    ///
    /// trailing underscore avoids zig's `become` keyword (there
    /// isn't one as of 0.16, but rust has one and the rust
    /// substrate uses `become_` — we match for cross-language
    /// readability).
    ///
    /// callers (typically `World.become_`) are expected to journal
    /// the pre-turn original via the V1 nursery so abort can roll
    /// the redirect back.
    pub fn become_(self: *Heap, a: FormId, b: FormId) !void {
        std.debug.assert(!a.isNone());
        std.debug.assert(!b.isNone());
        const target = self.resolveId(b);
        try self.redirects.put(self.allocator, a, target);
    }

    /// total Forms allocated, including the sentinel at index 0.
    pub fn len(self: *const Heap) usize {
        return self.forms.items.len;
    }

    /// `true` if no real allocations have happened yet (only the
    /// sentinel is present).
    pub fn isEmpty(self: *const Heap) bool {
        return self.forms.items.len == 1;
    }

    // ─────────────────────────────────────────────────────────────
    // GC primitives (phase 1 mark-sweep). called from `World.collect`.
    // ─────────────────────────────────────────────────────────────

    /// reset every Form's `gc_mark` bit to `false`. called at the
    /// start of every collection cycle. tombstones stay tombstones.
    pub fn gcResetMarks(self: *Heap) void {
        for (self.forms.items) |*f| f.gc_mark = false;
    }

    /// `true` if `id` is a (resolved) vat-local FormId pointing at a
    /// live, marked Form. external FormIds (shared / far-ref scopes)
    /// or out-of-range payloads return `false` (caller should not
    /// recurse into them).
    pub fn gcIsMarked(self: *const Heap, id: FormId) bool {
        if (id.isNone()) return false;
        if (id.scope != .vat_local) return false;
        if (id.payload >= self.forms.items.len) return false;
        return self.forms.items[id.payload].gc_mark;
    }

    /// mark `id`'s Form as live. caller should test `gcIsMarked` first
    /// to avoid redundant work; this just flips the bit.
    pub fn gcMark(self: *Heap, id: FormId) void {
        if (id.isNone()) return;
        if (id.scope != .vat_local) return;
        if (id.payload >= self.forms.items.len) return;
        self.forms.items[id.payload].gc_mark = true;
    }

    /// tombstone the Form at `payload`: free its slot/handler/meta
    /// hash-map storage, reset to a clean tombstone marker, and push
    /// the payload onto `free_list` so the next `alloc` can reuse it.
    ///
    /// **L11 (FormId stability):** see heap.zig header comment. once
    /// a Form is tombstoned, no live reference can reach it; the
    /// FormId's "identity" is gone. allocating a new Form at the
    /// same FormId is logically a new identity — there's no overlap
    /// in observability.
    ///
    /// the push can fail (OOM) but we treat that as fatal — a heap
    /// that can't track its own tombstones is too broken to continue.
    /// (the alternative is silent leak: skip the push and the slot
    /// stays tombstoned forever. but then alloc throughput regresses
    /// silently. fail loud.)
    pub fn gcTombstone(self: *Heap, payload: u30) void {
        const f = &self.forms.items[payload];
        f.deinit(self.allocator);
        f.* = Form.init();
        f.gc_tombstone = true;
        self.free_list.append(self.allocator, payload) catch |err| {
            std.debug.panic("gcTombstone: free_list OOM ({s})", .{@errorName(err)});
        };
    }
};
