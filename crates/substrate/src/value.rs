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
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
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
    /// interned symbol. proto: `Symbol`.
    Sym(SymId),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_is_default() {
        assert_eq!(Value::default(), Value::Nil);
    }

    #[test]
    fn truthiness() {
        assert!(!Value::Nil.is_truthy(), "nil is falsy");
        assert!(!Value::Bool(false).is_truthy(), "#false is falsy");
        assert!(Value::Bool(true).is_truthy());
        assert!(Value::Int(0).is_truthy(), "0 is truthy (clojure tradition)");
        assert!(Value::Int(-1).is_truthy());
        assert!(Value::Sym(SymId(1)).is_truthy());
        assert!(Value::Form(FormId(1)).is_truthy());
    }

    #[test]
    fn copy_is_pointer_safe() {
        // Value is Copy; cloning by assignment is safe.
        let a = Value::Int(42);
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn equality_is_by_value() {
        assert_eq!(Value::Int(5), Value::Int(5));
        assert_ne!(Value::Int(5), Value::Int(6));
        assert_ne!(Value::Int(5), Value::Sym(SymId(5)));
        assert_eq!(Value::Sym(SymId(7)), Value::Sym(SymId(7)));
    }

    #[test]
    fn extractors() {
        assert_eq!(Value::Int(42).as_int(), Some(42));
        assert_eq!(Value::Nil.as_int(), None);
        assert_eq!(Value::Sym(SymId(3)).as_sym(), Some(SymId(3)));
        assert_eq!(Value::Form(FormId(9)).as_form_id(), Some(FormId(9)));
        assert!(Value::Form(FormId(9)).is_form());
        assert!(!Value::Int(0).is_form());
    }

    #[test]
    fn size_is_small() {
        // copy-by-value remains cheap; if Value grows past 16 bytes
        // we want to know.
        assert!(
            std::mem::size_of::<Value>() <= 16,
            "Value should fit in 16 bytes; got {}",
            std::mem::size_of::<Value>()
        );
    }
}
