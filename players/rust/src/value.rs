//! the runtime value enum.
//!
//! per `laws/substrate-laws.md` L1, every conceptual value moof
//! allocates is a Form. at the runtime level, we tag-immediate the
//! most-common kinds (nil, bool, small int, symbol) to avoid heap
//! traffic, and reach for a heap-allocated [`Form`](crate::form::Form)
//! for everything else. each tagged-immediate has an *implicit
//! proto* (Nil, Bool, Integer, Symbol) the substrate hands out
//! during dispatch — reflection still works on small ints.
//!
//! foreign handles ([`ForeignId`](crate::foreign::ForeignId)) are a
//! distinct value variant — they're slot-resident references to
//! rust-allocated state, ferried by mcos. they don't have a moof
//! proto in the usual sense (their proto is `ForeignHandle` for
//! the purposes of `:proto` reflection); they just carry an opaque
//! pointer.
//!
//! later (phase G+) NaN-boxing collapses this into a single u64.
//! phase A keeps the honest tagged enum; the optimization is
//! invisible above this module.

use crate::foreign::ForeignId;
use crate::form::FormId;
use crate::sym::SymId;

/// a moof value as the runtime sees it.
///
/// `Copy` because every variant is small (≤ 8 bytes payload).
/// `Hash` so Tables (and future Set/Map) can use Values as keys.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub enum Value {
    /// `nil` — also the empty list (`docs/concepts/lists.md`).
    /// proto: `Nil`. natural default.
    #[default]
    Nil,
    /// boolean. proto: `Bool`.
    Bool(bool),
    /// 64-bit signed integer. proto: `Integer`. promoted to bigint
    /// on overflow in later phases (`docs/concepts/numbers.md`).
    Int(i64),
    /// 64-bit IEEE-754 float, stored as bits so Value can derive
    /// `Eq` + `Hash` (NaN != NaN by IEEE; we compare-by-bits here
    /// for hashmap-key sanity). proto: `Float`. arithmetic with
    /// `Int` auto-promotes (`docs/concepts/numbers.md`).
    Float(u64),
    /// interned symbol. proto: `Symbol`.
    Sym(SymId),
    /// a single Unicode scalar value (`U+0000..=U+10FFFF` minus
    /// surrogates). proto: `Char`. iterating a String yields
    /// `Char` values; `[s at: i]` returns one. (`docs/concepts/
    /// strings.md`.)
    Char(u32),
    /// reference to a heap-allocated Form. proto is `form.proto`.
    Form(FormId),
    /// reference to a rust-allocated foreign resource. proto is
    /// `ForeignHandle`. handles are vat-local and never serialize.
    /// (`docs/concepts/compiled-objects.md`.)
    Foreign(ForeignId),
}

impl Value {
    /// truthy? falsy values are `nil` and `#false` (clojure / lisp
    /// tradition; see `docs/syntax/literals.md`).
    pub fn is_truthy(self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    /// `true` if this value is `nil`.
    pub fn is_nil(self) -> bool {
        matches!(self, Value::Nil)
    }

    /// `true` if this value is a heap form.
    pub fn is_form(self) -> bool {
        matches!(self, Value::Form(_))
    }

    /// extract the FormId, if this is a heap form.
    pub fn as_form_id(self) -> Option<FormId> {
        if let Value::Form(id) = self {
            Some(id)
        } else {
            None
        }
    }

    /// extract the SymId, if this is a symbol.
    pub fn as_sym(self) -> Option<SymId> {
        if let Value::Sym(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// extract the i64, if this is an integer.
    pub fn as_int(self) -> Option<i64> {
        if let Value::Int(n) = self {
            Some(n)
        } else {
            None
        }
    }

    /// build a float-Value from an f64. canonical form is
    /// `Value::Float(f64::to_bits())`.
    pub fn float(x: f64) -> Self {
        Value::Float(x.to_bits())
    }

    /// extract the f64, if this is a float.
    pub fn as_float(self) -> Option<f64> {
        if let Value::Float(bits) = self {
            Some(f64::from_bits(bits))
        } else {
            None
        }
    }

    /// numeric coercion: Int → f64; Float → f64; everything else → None.
    /// used by promoting arithmetic.
    pub fn as_number_f64(self) -> Option<f64> {
        match self {
            Value::Int(n) => Some(n as f64),
            Value::Float(bits) => Some(f64::from_bits(bits)),
            _ => None,
        }
    }

    /// extract the codepoint, if this is a Char.
    pub fn as_char(self) -> Option<u32> {
        if let Value::Char(c) = self {
            Some(c)
        } else {
            None
        }
    }
}
