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
            .allocator = allocator,
        };
    }

    /// release the heap. walks every Form and deinits its slot /
    /// handler / meta maps before freeing the ArrayList itself.
    pub fn deinit(self: *Heap) void {
        for (self.forms.items) |*f| f.deinit(self.allocator);
        self.forms.deinit(self.allocator);
        self.redirects.deinit(self.allocator);
        self.* = undefined;
    }

    /// allocate a new Form, returning its (vat-local) FormId.
    ///
    /// the id is stable for the heap's lifetime
    /// (`laws/substrate-laws.md` L11). the caller transfers
    /// ownership of `form` (and its inner maps) to the heap.
    pub fn alloc(self: *Heap, form: Form) !FormId {
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
    /// hash-map storage and reset to a clean tombstone marker. the
    /// slot in `forms` stays — FormId stability (L11) requires we
    /// never reuse a tombstoned index in V1.
    pub fn gcTombstone(self: *Heap, payload: u30) void {
        const f = &self.forms.items[payload];
        f.deinit(self.allocator);
        f.* = Form.init();
        f.gc_tombstone = true;
    }
};
