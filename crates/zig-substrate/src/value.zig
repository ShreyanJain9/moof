//! the runtime value — moof's tagged-immediate.
//!
//! per `laws/substrate-laws.md` L1 every conceptual moof value is a
//! Form. but at the substrate level we tag-immediate the most-common
//! kinds (nil, bool, small int, sym, char, float) to avoid heap
//! traffic. each tagged-immediate has an *implicit proto* the
//! substrate hands out at dispatch time (Nil, Bool, Integer, Symbol,
//! Char, Float). reflection still works on small ints.
//!
//! v4-spec correspondence: V4 §4 ("byte encoding") references this
//! Value union as the operand-payload kind used by chunk const-pools
//! (V4 §10.3 FormSection / ChunkSection — each slot/const is a
//! serialized Value). the canonical wire encoding is byte-tagged
//! big-endian; the in-memory layout is a native zig tagged union and
//! does NOT match the wire format byte-for-byte.
//!
//! later (phase G+) NaN-boxing collapses this into a single u64.
//! phase A keeps the honest tagged enum; the optimization is
//! invisible above this module.

const std = @import("std");
const form_mod = @import("form.zig");
const FormId = form_mod.FormId;

/// a moof value as the runtime sees it.
///
/// every variant is small (≤ 8 bytes payload). Foreign handles are
/// deferred — they belong to the wasm/mco boundary and are not in
/// the data-layer scope. they will likely return as an additional
/// variant once `intrinsics.zig` lands.
pub const Value = union(enum) {
    /// `nil` — also the empty list (`docs/concepts/lists.md`).
    /// proto: `Nil`. natural default.
    nil,
    /// boolean. proto: `Bool`.
    bool_: bool,
    /// 64-bit signed integer. proto: `Integer`. moof's bignum-ready
    /// integer width. values outside the eventually-NaN-boxed range
    /// (e.g. > 51 bits) will promote to a heap BigInt Form in later
    /// phases. for phase A, we use a full i64 to match the bootstrap
    /// rust oracle.
    int: i64,
    /// interned symbol. proto: `Symbol`. payload is a SymId (see
    /// `sym.zig`); a u32 to keep this union plain-data.
    sym: u32,
    /// a single Unicode scalar value (`U+0000..=U+10FFFF` minus
    /// surrogates). proto: `Char`.
    char: u32,
    /// 64-bit IEEE-754 float. proto: `Float`. arithmetic with `int`
    /// auto-promotes (`docs/concepts/numbers.md`).
    float: f64,
    /// reference to a heap-allocated Form. proto is `form.proto`.
    /// FormId carries the 2-bit scope tag per V0.
    form: FormId,

    /// `true` if this value is `nil`.
    pub fn isNil(self: Value) bool {
        return self == .nil;
    }

    /// truthy? falsy values are `nil` and `bool_(false)` (clojure /
    /// lisp tradition; see `docs/syntax/literals.md`).
    pub fn isTruthy(self: Value) bool {
        return switch (self) {
            .nil => false,
            .bool_ => |b| b,
            else => true,
        };
    }

    /// extract the FormId, if this is a heap form. `null` otherwise.
    pub fn asFormId(self: Value) ?FormId {
        return switch (self) {
            .form => |id| id,
            else => null,
        };
    }

    /// extract the SymId (u32), if this is a symbol. `null` otherwise.
    pub fn asSym(self: Value) ?u32 {
        return switch (self) {
            .sym => |s| s,
            else => null,
        };
    }

    /// extract the i64, if this is an integer. `null` otherwise.
    pub fn asInt(self: Value) ?i64 {
        return switch (self) {
            .int => |n| n,
            else => null,
        };
    }

    /// by-value equality. used by table-key lookup, `:is`, the
    /// const-pool deduplication step in the compiler, and the
    /// canonical-bytes encoder.
    ///
    /// notes on edge cases:
    ///
    /// - `nil == nil` is `true`.
    /// - `bool_(x) == bool_(y)` is `x == y`.
    /// - `int(x) == int(y)` is `x == y` (signed compare in i48 space).
    /// - `float` equality is bit-equality after NaN-canonicalization
    ///   isn't done here — IEEE 754 NaN != NaN by spec; we follow IEEE
    ///   for raw `Value`. callers needing hash-key sanity should
    ///   canonicalize the bits before comparing.
    /// - `sym`/`char`/`form` compare their payloads directly.
    /// - cross-variant comparisons are always `false`.
    pub fn equals(self: Value, other: Value) bool {
        if (std.meta.activeTag(self) != std.meta.activeTag(other)) return false;
        return switch (self) {
            .nil => true,
            .bool_ => |x| x == other.bool_,
            .int => |x| x == other.int,
            .sym => |x| x == other.sym,
            .char => |x| x == other.char,
            .float => |x| x == other.float,
            .form => |x| x.payload == other.form.payload and x.scope == other.form.scope,
        };
    }
};
