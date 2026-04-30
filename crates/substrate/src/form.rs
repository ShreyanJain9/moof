//! the universal heap kind.
//!
//! per `laws/substrate-laws.md` L1, every conceptually-allocated
//! value is a Form. concretely, a Form has the four-faces shape
//! (`docs/concepts/forms.md`):
//!
//! - **structure**: nothing here yet — phase A doesn't expose
//!   head/args separately. (parsed code-Forms are List Forms whose
//!   slots already carry head/tail; no extra fields needed at the
//!   substrate level.)
//! - **identity**: `proto` + `slots` + `handlers`.
//! - **history**: `meta`.
//! - **liveness**: not on every Form — vat-Forms get extra slots
//!   for mailbox/behavior at phase B.
//!
//! `slots`, `handlers`, and `meta` are `IndexMap`s for two reasons:
//!
//! 1. **insertion-order iteration is deterministic**, satisfying
//!    `laws/determinism-laws.md` D5. critical for replication.
//! 2. iteration is in the order users *added* keys, which is what
//!    they expect in inspectors and serializations.

use indexmap::IndexMap;

use crate::sym::SymId;
use crate::value::Value;

/// the heap-id of a Form. vat-local. stable within a vat
/// (`laws/substrate-laws.md` L11).
///
/// `FormId(0)` is reserved as a sentinel — never returned by
/// `Heap::alloc`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct FormId(pub u32);

impl FormId {
    pub const NONE: FormId = FormId(0);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

/// the universal heap kind.
///
/// every conceptually-allocated moof value is a Form. dispatch
/// walks `proto`. user data lives in `slots`. methods live in
/// `handlers`. provenance + annotations live in `meta`.
#[derive(Default)]
pub struct Form {
    /// the immediate delegation parent. `Value::Nil` for the root
    /// `Object` proto; `Value::Form(_)` for everything else.
    /// (`docs/concepts/objects-and-protos.md`.)
    pub proto: Value,

    /// named bindings. `IndexMap` so iteration order is insertion
    /// order — *deterministic* across replicas
    /// (`laws/determinism-laws.md` D5).
    pub slots: IndexMap<SymId, Value>,

    /// selector → method-Form (`Value::Form` of a method-shaped
    /// Form). protos populate this; instances rarely do.
    pub handlers: IndexMap<SymId, Value>,

    /// metadata: source-loc, doc, journal-id, type, etc.
    /// extensible by user code (`laws/reflection-contract.md` R7).
    pub meta: IndexMap<SymId, Value>,
}

impl Form {
    /// build a Form with a given proto and otherwise empty.
    pub fn with_proto(proto: Value) -> Self {
        Form {
            proto,
            slots: IndexMap::new(),
            handlers: IndexMap::new(),
            meta: IndexMap::new(),
        }
    }

    /// look up a slot by name. returns `Value::Nil` if missing —
    /// callers that need to distinguish "missing" from "explicitly
    /// nil" use [`Form::slot_present`].
    pub fn slot(&self, name: SymId) -> Value {
        self.slots.get(&name).copied().unwrap_or(Value::Nil)
    }

    /// `true` if `name` is bound in this Form's slots.
    pub fn slot_present(&self, name: SymId) -> bool {
        self.slots.contains_key(&name)
    }

    /// look up a handler by selector. returns `None` if absent;
    /// callers walk the proto chain via `Heap::dispatch`.
    pub fn handler(&self, selector: SymId) -> Option<Value> {
        self.handlers.get(&selector).copied()
    }

    /// look up a meta entry. returns `Value::Nil` if missing.
    pub fn meta_at(&self, name: SymId) -> Value {
        self.meta.get(&name).copied().unwrap_or(Value::Nil)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_form_has_no_proto_no_slots() {
        let f = Form::default();
        assert!(f.proto.is_nil());
        assert!(f.slots.is_empty());
        assert!(f.handlers.is_empty());
        assert!(f.meta.is_empty());
    }

    #[test]
    fn with_proto_records_proto() {
        let p = Value::Form(FormId(7));
        let f = Form::with_proto(p);
        assert_eq!(f.proto, p);
    }

    #[test]
    fn slot_returns_nil_for_missing() {
        let f = Form::default();
        assert_eq!(f.slot(SymId(42)), Value::Nil);
        assert!(!f.slot_present(SymId(42)));
    }

    #[test]
    fn slot_returns_explicit_nil_distinguishably() {
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Nil);
        assert_eq!(f.slot(SymId(1)), Value::Nil);
        assert!(f.slot_present(SymId(1))); // present, even though nil
    }

    #[test]
    fn handler_returns_none_for_missing() {
        let f = Form::default();
        assert_eq!(f.handler(SymId(99)), None);
    }

    #[test]
    fn slot_iteration_is_insertion_order() {
        // determinism-laws.md D5.
        let mut f = Form::default();
        for i in (1..=10).rev() {
            f.slots.insert(SymId(i), Value::Int(i as i64));
        }
        let order: Vec<u32> = f.slots.keys().map(|k| k.0).collect();
        assert_eq!(order, vec![10, 9, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn form_id_zero_is_sentinel() {
        assert!(FormId::NONE.is_none());
        assert!(!FormId(1).is_none());
    }

    #[test]
    fn meta_has_independent_entries() {
        let mut f = Form::default();
        f.meta.insert(SymId(1), Value::Int(7));
        assert_eq!(f.meta_at(SymId(1)), Value::Int(7));
        assert_eq!(f.meta_at(SymId(2)), Value::Nil);
    }
}
