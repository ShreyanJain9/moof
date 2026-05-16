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

/// the universal heap kind.
///
/// every conceptually-allocated moof value is a Form. dispatch walks
/// `proto`. user data lives in `slots`. methods live in `handlers`.
/// provenance + annotations live in `meta`.
pub const Form = struct {
    /// the immediate delegation parent. `Value.nil` for the root
    /// `Object` proto; `.form` for everything else.
    /// (`docs/concepts/objects-and-protos.md`.)
    proto: Value,

    /// named bindings. insertion-order — deterministic across
    /// replicas (`laws/determinism-laws.md` D5).
    slots: SlotMap,

    /// selector → method-Form (`Value.form` of a method-shaped
    /// Form). protos populate this; instances rarely do.
    handlers: SlotMap,

    /// metadata: source-loc, doc, journal-id, type, etc. extensible
    /// by user code (`laws/reflection-contract.md` R7).
    meta: SlotMap,

    /// V2 — freezing. once `true`, slot/handler/meta writes raise
    /// `'frozen-form`. one-way; no thaw. transition itself is a
    /// turn-mutation (journals via the V1 nursery; rolls back on
    /// abort).
    frozen: bool,

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
            .frozen = false,
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
    pub fn slot(self: *const Form, name: u32) Value {
        return if (self.slots.get(name)) |v| v else Value.nil;
    }

    /// `true` if `name` is bound in this Form's slots.
    pub fn slotPresent(self: *const Form, name: u32) bool {
        return self.slots.contains(name);
    }

    /// look up a handler by selector. returns `null` if absent —
    /// callers walk the proto chain via the VM dispatch helper.
    pub fn handler(self: *const Form, selector: u32) ?Value {
        return self.handlers.get(selector);
    }

    /// look up a meta entry. returns `Value.nil` if missing.
    pub fn metaAt(self: *const Form, name: u32) Value {
        return if (self.meta.get(name)) |v| v else Value.nil;
    }
};
