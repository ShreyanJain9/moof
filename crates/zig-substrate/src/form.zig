//! Form — the universal heap kind. and FormId — its address.
//!
//! per `laws/substrate-laws.md` L1, every conceptually-allocated
//! moof value is a Form. concretely, a Form has the four-faces shape
//! (`docs/concepts/forms.md`):
//!
//! - **structure**: parsed code-Forms are List Forms whose slots
//!   already carry head/tail; no extra fields needed at the
//!   substrate level.
//! - **identity**: `proto` + `slots` + `handlers`.
//! - **history**: `meta`.
//! - **liveness**: not on every Form — vat-Forms get extra slots for
//!   mailbox/behavior at phase B.
//!
//! `slots`, `handlers`, and `meta` are **insertion-order** maps
//! (`std.AutoArrayHashMapUnmanaged`) because:
//!
//! 1. **insertion-order iteration is deterministic**, satisfying
//!    `laws/determinism-laws.md` D5. critical for replication.
//! 2. iteration is in the order users *added* keys — what they
//!    expect in inspectors and serializations.
//!
//! v4-spec correspondence: the FormSection per V4 §10.3 walks the
//! slots / handlers / meta in this same insertion order; the on-disk
//! layout is the in-memory layout.
//!
//! ## phase 2 §4.7 — `handlers` audit (2026-05-16)
//!
//! the perf design considered swapping `handlers` to a non-ordered
//! `AutoHashMapUnmanaged` for faster `.get(selector)`. D5 audit found
//! handlers iteration is **user-observable** at three sites:
//!
//!   - `intrinsics.zig::heapHandlerKeysOf` — reflection method
//!     `[obj :handlerKeysOf]` returns the keys as a list; the order
//!     leaks straight to moof code.
//!   - `image.zig::serializeImage` (FormSection) — writes
//!     `count + (sym, value)` pairs in iteration order; the
//!     resulting bytes feed D9's canonical-hash.
//!   - `image.zig::serializeImage` (NativeRefsSection) — walks
//!     every proto's handlers to emit native-method binding entries;
//!     the on-disk order is observable.
//!
//! decision: **keep `handlers` as `AutoArrayHashMapUnmanaged`**.
//! `.get(selector)` is already O(1) amortized on the ordered map (it's
//! two cache-friendly reads: index → value array). the hot path for
//! handlers is `.get`, not iteration. swapping would force a sort at
//! every observable site to preserve D5, which would cost more than
//! the dispatch saves. the `native_fns` table — which has *no*
//! user-observable iteration — was swapped separately (world.zig).

const std = @import("std");
const Value = @import("value.zig").Value;

/// the bit mask that selects the scope tag in a `FormId`'s u32.
pub const SCOPE_MASK: u32 = 0b11 << 30;
/// the bit mask that selects the payload in a `FormId`'s u32.
pub const PAYLOAD_MASK: u32 = ~SCOPE_MASK;
/// the maximum payload value (exclusive). 2^30 ≈ 1.07 billion forms
/// per scope — vastly more than any reasonable vat needs.
pub const MAX_PAYLOAD: u32 = 1 << 30;

/// the universal heap-id. matches the rust `FormId` layout: 2-bit
/// scope tag in the top, 30-bit payload below. derived from the V0
/// scope-tagging design.
///
/// canonical byte encoding (V4 §4) is big-endian u32 — but in memory
/// we keep the native zig packed-struct layout.
pub const FormId = packed struct(u32) {
    payload: u30,
    scope: Scope,

    /// the four scopes a `FormId` can address. V0 spec §5.
    ///
    /// vat-local is the only one with real implementation in V0 —
    /// shared and far-ref panic until later phases fill them in
    /// (V6 / V5 respectively).
    pub const Scope = enum(u2) {
        /// `00…` — index into this vat's `Heap.forms`.
        vat_local = 0b00,
        /// `01…` — index into the process-wide shared segment (V6).
        shared = 0b01,
        /// `10…` — index into this vat's far-ref table (V5; lazily
        /// populated from V4 §10 FarRefsSection).
        far_ref = 0b10,
        /// `11…` — reserved for future use (NaN-boxed immediates,
        /// bigint pool, segmented heaps).
        reserved = 0b11,
    };

    /// the sentinel "absent FormId". `Heap::alloc` never returns this.
    pub const NONE: FormId = .{ .payload = 0, .scope = .vat_local };

    /// `true` if this is the sentinel.
    pub fn isNone(self: FormId) bool {
        return self.payload == 0 and self.scope == .vat_local;
    }

    /// structural equality on the (scope, payload) pair. since FormId
    /// is a packed-struct backed by u32, this just compares bit
    /// patterns.
    pub fn eql(self: FormId, other: FormId) bool {
        return self.payload == other.payload and self.scope == other.scope;
    }

    /// construct a vat-local FormId. payload must fit in 30 bits.
    /// callers responsible for the bounds check — the packed struct
    /// truncates silently. use `Heap::alloc` rather than calling this
    /// directly when allocating new forms.
    pub fn vatLocal(payload: u30) FormId {
        return .{ .payload = payload, .scope = .vat_local };
    }

    /// reinterpret the FormId as its u32 wire representation. used
    /// by `bytecode.zig` and `image.zig` to serialize FormIds.
    pub fn toU32(self: FormId) u32 {
        return @bitCast(self);
    }

    /// inverse of `toU32` — construct a FormId from its u32 wire
    /// representation. preserves scope tag in top 2 bits.
    pub fn fromU32(raw: u32) FormId {
        return @bitCast(raw);
    }
};

/// canonical alias for the slot/handler/meta map type. keyed by
/// SymId (u32), values are Values. insertion-order preserved per
/// determinism law D5. unmanaged so the Form struct doesn't carry
/// an allocator field — allocators flow in through Form methods.
pub const SlotMap = std.AutoArrayHashMapUnmanaged(u32, Value);

/// phase 2 §5.8b — global SymIds for `car` / `cdr`. set by
/// `setConsSyms` at World.init and after image-load (the loader
/// re-interns the sym table, so the SymIds shift). when both are
/// non-zero, `Form.slot` / `Form.slotPresent` honor the FlatCons
/// inline fields; before they're set, those methods behave like
/// the pre-§5.8b code (treat every Form as general).
///
/// these are *not* per-World — only one vat exists per process in
/// V4 phase α, and the same intrinsic registry serves all. when
/// multi-vat lands (V6), this will need to move onto World.
pub var CONS_CAR_SYM: u32 = 0;
pub var CONS_CDR_SYM: u32 = 0;

/// install the (car, cdr) SymIds for FlatCons accessors. idempotent;
/// safe to call repeatedly. World.init and image.loadVatImage both
/// invoke this after the sym table is populated.
pub fn setConsSyms(car_sym: u32, cdr_sym: u32) void {
    CONS_CAR_SYM = car_sym;
    CONS_CDR_SYM = cdr_sym;
}

/// the universal heap kind.
///
/// every conceptually-allocated moof value is a Form. dispatch walks
/// `proto`. user data lives in `slots`. methods live in `handlers`.
/// provenance + annotations live in `meta`.
///
/// ## phase 2 §5.8b — flat-Cons layout
///
/// per the perf design, ~half of all heap forms during bootstrap are
/// cons cells. the canonical layout stored car/cdr inside the `slots`
/// ArrayHashMap — two `put` calls per allocation, two hashmap walks
/// per `car`/`cdr` read. that's ~30 ns per access and ~80 ns per alloc.
///
/// the FlatCons optimization stores `car` and `cdr` *inline* on the
/// Form struct itself when `is_flat_cons == true`. for fresh Cons
/// allocations (`(cons h t)`, `[h cons: t]`, `(list …)`,
/// `[xs reverse]`, image-load re-flatten):
///   - `slots` stays `.empty` (no allocator traffic for canonical slots)
///   - `formSlot(id, sym_car)` / `formSlot(id, sym_cdr)` return the
///     inline field directly (one branch + load, no hashmap probe)
///   - `formSlotSet(id, sym_car, v)` writes the inline field directly
///   - if user code adds a *non-canonical* slot via
///     `[cell slotSet: 'foo to: 42]`, it lazy-inits `slots` and stores
///     there. canonical slots still flow through inline fields.
///
/// reflection (`[obj slots]`-style iterators) must yield `'car, 'cdr`
/// *before* iterating `slots` — that's the responsibility of every
/// site that previously walked `f.slots.iterator()` for a Cons.
/// `heapSlotKeysOf`, `gc.drainWorklist`, and `image.serializeVat`
/// have been updated.
///
/// the on-disk image format is UNCHANGED — a FlatCons serializes as
/// a Form with `car` / `cdr` synthesized into the slots-section, and
/// the loader re-flattens on read (proto == cons → set is_flat_cons,
/// hoist car/cdr inline).
///
/// ## phase 2 §4.7 — `handlers` (unchanged) — see header comment.
pub const Form = struct {
    /// the immediate delegation parent. `Value.nil` for the root
    /// `Object` proto; `.form` for everything else.
    /// (`docs/concepts/objects-and-protos.md`.)
    proto: Value,

    /// named bindings. insertion-order — deterministic across
    /// replicas (`laws/determinism-laws.md` D5).
    ///
    /// for `is_flat_cons` Forms, canonical `:car` / `:cdr` are NOT
    /// stored here; only user-added non-canonical slots land in this
    /// map (lazy-init on first non-cons slot write).
    slots: SlotMap,

    /// selector → method-Form (`Value.form` of a method-shaped
    /// Form). protos populate this; instances rarely do.
    handlers: SlotMap,

    /// metadata: source-loc, doc, journal-id, type, etc. extensible
    /// by user code (`laws/reflection-contract.md` R7).
    meta: SlotMap,

    /// phase 2 §5.8b — FlatCons inline `:car`. valid iff `is_flat_cons`.
    car_inline: Value,

    /// phase 2 §5.8b — FlatCons inline `:cdr`. valid iff `is_flat_cons`.
    cdr_inline: Value,

    /// V2 — freezing. once `true`, slot/handler/meta writes raise
    /// `'frozen-form`. one-way; no thaw. transition itself is a
    /// turn-mutation (journals via the V1 nursery; rolls back on
    /// abort).
    frozen: bool,

    /// phase 2 §5.8b — `true` if this Form uses the FlatCons inline
    /// layout. when set, `car_inline` and `cdr_inline` are the
    /// canonical `:car` and `:cdr` slot values; `slots` holds only
    /// user-added non-canonical slots (lazy-inited).
    is_flat_cons: bool,

    /// GC mark bit (phase 1 mark-sweep, per
    /// `2026-05-11-phase1-gc-dispatch-compression-design.md` §3).
    /// reset to `false` at the start of every collection cycle; set
    /// to `true` by the mark pass when this Form is reached from a
    /// root. unmarked Forms are tombstoned by the sweep pass.
    ///
    /// also serves as the tombstone discriminator post-sweep: a
    /// tombstoned slot has `gc_mark = false`, `proto = .nil`, and
    /// empty `slots` / `handlers` / `meta`. live forms never carry
    /// `gc_mark = false` outside of an active GC cycle.
    ///
    /// one byte of bloat per Form. negligible — typical Forms are
    /// 100+ bytes via their slot/handler/meta hash-map storage.
    gc_mark: bool,

    /// `true` if this Form is a tombstone (an entry the GC reclaimed
    /// because no root could reach it). callers should never reach a
    /// tombstone via a live root — if they do, the GC missed a root.
    /// distinguishable from a "live Form with no slots" by `proto ==
    /// .nil` AND `slots.count() == 0` AND `handlers.count() == 0`.
    /// for V1 we never reuse tombstone slots (L11 trivially preserved).
    gc_tombstone: bool,

    /// build an empty Form with `nil` proto + empty maps. allocator
    /// is taken on every mutation, so init itself is allocation-free.
    pub fn init() Form {
        return .{
            .proto = .nil,
            .slots = .empty,
            .handlers = .empty,
            .meta = .empty,
            .car_inline = .nil,
            .cdr_inline = .nil,
            .frozen = false,
            .is_flat_cons = false,
            .gc_mark = false,
            .gc_tombstone = false,
        };
    }

    /// build a Form with a given proto and otherwise empty.
    pub fn withProto(proto: Value) Form {
        var f = Form.init();
        f.proto = proto;
        return f;
    }

    /// phase 2 §5.8b — build a fresh FlatCons. proto is set to
    /// `cons_proto_v` (caller passes `Value{.form = world.protos.cons}`);
    /// `car_inline` / `cdr_inline` populated; `slots` empty. allocation-
    /// free (no SlotMap put).
    pub fn flatCons(cons_proto_v: Value, car: Value, cdr: Value) Form {
        return .{
            .proto = cons_proto_v,
            .slots = .empty,
            .handlers = .empty,
            .meta = .empty,
            .car_inline = car,
            .cdr_inline = cdr,
            .frozen = false,
            .is_flat_cons = true,
            .gc_mark = false,
            .gc_tombstone = false,
        };
    }

    /// release backing storage for slots / handlers / meta. does NOT
    /// free any heap-allocated Values inside (those live on the
    /// owning Heap). called by `Heap.deinit` for every form.
    pub fn deinit(self: *Form, allocator: std.mem.Allocator) void {
        self.slots.deinit(allocator);
        self.handlers.deinit(allocator);
        self.meta.deinit(allocator);
        self.* = undefined;
    }

    /// look up a slot by name. returns `Value.nil` if missing —
    /// callers that need to distinguish "missing" from "explicitly
    /// nil" use `slotPresent`.
    ///
    /// phase 2 §5.8b: for FlatCons Forms, `:car` / `:cdr` return the
    /// inline fields. requires `CONS_CAR_SYM` / `CONS_CDR_SYM` to be
    /// set (World.init does this; tests that bypass init may need to
    /// call `setConsSyms` themselves).
    pub fn slot(self: *const Form, name: u32) Value {
        if (self.is_flat_cons) {
            if (name == CONS_CAR_SYM) return self.car_inline;
            if (name == CONS_CDR_SYM) return self.cdr_inline;
        }
        return if (self.slots.get(name)) |v| v else Value.nil;
    }

    /// `true` if `name` is bound in this Form's slots.
    ///
    /// phase 2 §5.8b: for FlatCons Forms, `:car` / `:cdr` are always
    /// present (their inline storage IS the binding, even if the
    /// value is `nil`).
    pub fn slotPresent(self: *const Form, name: u32) bool {
        if (self.is_flat_cons) {
            if (name == CONS_CAR_SYM or name == CONS_CDR_SYM) return true;
        }
        return self.slots.contains(name);
    }

    /// look up a handler by selector. returns `null` if absent —
    /// callers walk the proto chain via the VM dispatch helper.
    ///
    /// FlatCons has no instance handlers (the Cons proto carries
    /// them; instances delegate). this just reads the `handlers` map.
    pub fn handler(self: *const Form, selector: u32) ?Value {
        return self.handlers.get(selector);
    }

    /// look up a meta entry. returns `Value.nil` if missing.
    pub fn metaAt(self: *const Form, name: u32) Value {
        return if (self.meta.get(name)) |v| v else Value.nil;
    }

    /// phase 2 §5.8b — count of *observable* slot bindings, including
    /// the synthesized `:car` / `:cdr` for FlatCons Forms. callers
    /// doing reflection (e.g. image-serializer, `[obj slots]`) should
    /// use this instead of `self.slots.count()`.
    pub fn slotCount(self: *const Form) usize {
        if (self.is_flat_cons) return 2 + self.slots.count();
        return self.slots.count();
    }
};

// ─────────────────────────────────────────────────────────────────
// phase 2 §5.8b — FlatCons contract tests. exercises the Form-faces
// invariants the spec calls out: structure (car/cdr), identity
// (frozen, becomes), history (meta), reflection (slotCount), and the
// extras path (non-canonical slot writes).
// ─────────────────────────────────────────────────────────────────

const testing = std.testing;

test "FlatCons: slot(:car) returns inline car" {
    setConsSyms(101, 102); // dummy sym ids for the test
    defer setConsSyms(0, 0); // reset so other tests don't see them

    const car = Value{ .int = 7 };
    const cdr = Value.nil;
    const f = Form.flatCons(Value.nil, car, cdr);
    try testing.expect(f.is_flat_cons);
    try testing.expectEqual(@as(usize, 2), f.slotCount());
    try testing.expect(f.slot(101).equals(car));
    try testing.expect(f.slot(102).equals(cdr));
}

test "FlatCons: slotPresent on :car / :cdr is always true" {
    setConsSyms(101, 102);
    defer setConsSyms(0, 0);

    const f = Form.flatCons(Value.nil, Value.nil, Value.nil);
    try testing.expect(f.slotPresent(101));
    try testing.expect(f.slotPresent(102));
    try testing.expect(!f.slotPresent(999));
}

test "FlatCons: extras slot in lazy SlotMap" {
    setConsSyms(101, 102);
    defer setConsSyms(0, 0);

    var f = Form.flatCons(Value.nil, Value{ .int = 1 }, Value{ .int = 2 });
    defer f.deinit(testing.allocator);
    // car / cdr should not be in the SlotMap
    try testing.expect(!f.slots.contains(101));
    try testing.expect(!f.slots.contains(102));
    // user adds a non-canonical slot — goes to the SlotMap
    try f.slots.put(testing.allocator, 999, Value{ .int = 42 });
    try testing.expectEqual(@as(usize, 3), f.slotCount()); // car + cdr + 999
    try testing.expect(f.slot(999).equals(Value{ .int = 42 }));
    // canonical car/cdr still work
    try testing.expect(f.slot(101).equals(Value{ .int = 1 }));
    try testing.expect(f.slot(102).equals(Value{ .int = 2 }));
}

test "FlatCons: not-flat Form unaffected by CONS_CAR_SYM" {
    setConsSyms(101, 102);
    defer setConsSyms(0, 0);

    var f = Form.init();
    defer f.deinit(testing.allocator);
    // an empty general Form — slot(101) returns nil; not "always-present"
    try testing.expect(f.slot(101).equals(Value.nil));
    try testing.expect(!f.slotPresent(101));
}

test "FlatCons: car/cdr/handlers/meta all still independent" {
    setConsSyms(101, 102);
    defer setConsSyms(0, 0);

    var f = Form.flatCons(Value.nil, Value{ .int = 1 }, Value{ .int = 2 });
    defer f.deinit(testing.allocator);
    try f.handlers.put(testing.allocator, 500, Value{ .sym = 99 });
    try f.meta.put(testing.allocator, 600, Value{ .sym = 88 });
    try testing.expect(f.handler(500).?.equals(Value{ .sym = 99 }));
    try testing.expect(f.metaAt(600).equals(Value{ .sym = 88 }));
    // slot lookup for handler / meta keys: not slot bindings.
    try testing.expect(f.slot(500).equals(Value.nil));
    try testing.expect(f.slot(600).equals(Value.nil));
}
