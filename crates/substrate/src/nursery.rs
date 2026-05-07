//! per-turn nursery + diff types.
//!
//! a turn is the unit of atomicity (`docs/superpowers/specs/
//! 2026-05-06-vat-V1-nursery-diff-design.md`). mutations during
//! a turn either land in the nursery (for pre-existing forms,
//! keyed deltas) or directly in the canonical heap above the
//! `turn_watermark` (for new allocations). commit produces a
//! `TurnDiff` summarizing what changed; abort drops the buffered
//! state and truncates the heap to watermark.

use indexmap::IndexMap;

use crate::form::FormId;
use crate::sym::SymId;
use crate::value::Value;

/// the three faces of a Form that participate in mutation
/// buffering. matches `Form`'s structural shape: slots /
/// handlers / meta. (`docs/concepts/forms.md`.)
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FaceKind {
    Slots,
    Handlers,
    Meta,
}

/// per-form delta accumulated during a turn for forms that
/// existed before the turn started. only touched keys are
/// stored; unchanged keys fall through to canonical at read
/// time.
///
/// note: forms allocated *during* the turn (FormId payload
/// >= `turn_watermark`) do NOT use a Delta — they live in the
/// canonical `Vec<Form>` above the watermark and are mutated
/// directly. the Delta is exclusively for pre-existing forms.
#[derive(Default, Debug)]
pub struct Delta {
    pub slots: IndexMap<SymId, Value>,
    pub handlers: IndexMap<SymId, Value>,
    pub meta: IndexMap<SymId, Value>,
}

impl Delta {
    /// access the `IndexMap` for a given face, mutably.
    pub fn face_mut(&mut self, face: FaceKind) -> &mut IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &mut self.slots,
            FaceKind::Handlers => &mut self.handlers,
            FaceKind::Meta => &mut self.meta,
        }
    }

    /// access the `IndexMap` for a given face, immutably.
    pub fn face(&self, face: FaceKind) -> &IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &self.slots,
            FaceKind::Handlers => &self.handlers,
            FaceKind::Meta => &self.meta,
        }
    }

    /// `true` iff no key has been touched in any face.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
            && self.handlers.is_empty()
            && self.meta.is_empty()
    }
}

/// the result of `commit_turn`: a record of what changed
/// during the turn. consumed (in V1) by tests; will feed the
/// `inputs.log` (V9), replication (V11), and CRDT merge
/// pathways (V11).
///
/// the `mutations` map is dedup-keyed by `(form, face, key)` —
/// last-write-wins per key per turn. intermediate writes within
/// a turn don't appear; only the final value at commit-time does.
/// the `prior` value is what was in the canonical heap at
/// turn-start; `new` is the final value the turn settled on.
///
/// `new_allocs` lists FormIds allocated this turn, in
/// allocation order. forms in `new_allocs` do NOT appear in
/// `mutations` (they have no prior state).
#[derive(Default, Debug)]
pub struct TurnDiff {
    pub mutations: IndexMap<(FormId, FaceKind, SymId), (Value, Value)>,
    pub new_allocs: Vec<FormId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_default_is_empty() {
        let d = Delta::default();
        assert!(d.is_empty());
        assert!(d.slots.is_empty());
        assert!(d.handlers.is_empty());
        assert!(d.meta.is_empty());
    }

    #[test]
    fn delta_face_returns_correct_map() {
        let mut d = Delta::default();
        d.slots.insert(SymId(1), Value::Int(10));
        d.handlers.insert(SymId(2), Value::Int(20));
        d.meta.insert(SymId(3), Value::Int(30));

        assert_eq!(d.face(FaceKind::Slots).get(&SymId(1)).copied(), Some(Value::Int(10)));
        assert_eq!(d.face(FaceKind::Handlers).get(&SymId(2)).copied(), Some(Value::Int(20)));
        assert_eq!(d.face(FaceKind::Meta).get(&SymId(3)).copied(), Some(Value::Int(30)));
    }

    #[test]
    fn delta_face_mut_returns_mutable_map() {
        let mut d = Delta::default();
        d.face_mut(FaceKind::Slots).insert(SymId(1), Value::Int(42));
        assert_eq!(d.slots.get(&SymId(1)).copied(), Some(Value::Int(42)));
    }

    #[test]
    fn delta_is_empty_after_only_default_face_mut_lookups() {
        let mut d = Delta::default();
        // touching face_mut without inserting shouldn't make it non-empty.
        let _ = d.face_mut(FaceKind::Slots);
        assert!(d.is_empty());
    }

    #[test]
    fn delta_is_not_empty_after_inserting_into_any_face() {
        let mut d1 = Delta::default();
        d1.slots.insert(SymId(1), Value::Nil);
        assert!(!d1.is_empty());

        let mut d2 = Delta::default();
        d2.handlers.insert(SymId(1), Value::Nil);
        assert!(!d2.is_empty());

        let mut d3 = Delta::default();
        d3.meta.insert(SymId(1), Value::Nil);
        assert!(!d3.is_empty());
    }

    #[test]
    fn turn_diff_default_is_empty() {
        let td = TurnDiff::default();
        assert!(td.mutations.is_empty());
        assert!(td.new_allocs.is_empty());
    }

    #[test]
    fn turn_diff_can_record_mutations_and_allocs() {
        let mut td = TurnDiff::default();
        td.mutations.insert(
            (FormId::vat_local(5), FaceKind::Slots, SymId(7)),
            (Value::Int(1), Value::Int(2)),
        );
        td.new_allocs.push(FormId::vat_local(10));

        assert_eq!(td.mutations.len(), 1);
        assert_eq!(td.new_allocs.len(), 1);
        let entry = td.mutations
            .get(&(FormId::vat_local(5), FaceKind::Slots, SymId(7)))
            .copied();
        assert_eq!(entry, Some((Value::Int(1), Value::Int(2))));
    }

    #[test]
    fn face_kind_variants_distinguish() {
        assert_ne!(FaceKind::Slots, FaceKind::Handlers);
        assert_ne!(FaceKind::Handlers, FaceKind::Meta);
        assert_ne!(FaceKind::Slots, FaceKind::Meta);
    }
}
