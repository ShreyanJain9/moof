//! values — the substrate's tagged union.
//!
//! every conceptual moof value is a Form (docs/concepts/forms.md,
//! laws/substrate-laws.md L1). at the runtime level, the most-
//! common immediates (nil, booleans, small ints, symbols) are tag-
//! immediate to avoid heap traffic. they still have implicit protos
//! (Nil, Bool, Integer, Symbol) — proto-of-receiver is computed from
//! the tag during send dispatch.
//!
//! later phases compress this into NaN-boxing for cache friendliness.
//! phase 1 is an honest tagged enum; the optimization is internal
//! and invisible above this module.

use crate::form::FormId;
use crate::sym::SymId;

/// a moof value as the runtime sees it.
///
/// derives `Copy` because every variant is small (≤ 8 bytes payload).
/// later: NaN-box this into a single u64.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum Value {
    /// `nil` — also the empty list (docs/concepts/lists.md).
    /// proto: `Nil`. also the natural default value.
    #[default]
    Nil,
    /// boolean. proto: `Bool`.
    Bool(bool),
    /// 64-bit signed integer. proto: `Integer`. promoted to bigint
    /// on overflow in later phases (concepts/numbers.md).
    Int(i64),
    /// interned symbol. proto: `Symbol`.
    Sym(SymId),
    /// reference to a heap-allocated Form. proto is `form.proto`.
    Form(FormId),
}

impl Value {
    /// truthy? falsy values are `nil` and `#false` (clojure / lisp
    /// tradition; see syntax/literals.md).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }
}
