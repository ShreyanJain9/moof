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

    /// V2 — has this turn frozen the corresponding form? one-way
    /// false→true within a turn. on commit, OR'd into the
    /// canonical `Form.frozen`. on abort, dropped with the rest
    /// of the delta.
    pub frozen: bool,
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

    /// V2 — pre-existing forms whose `frozen` bit transitioned
    /// false→true during this turn. forms that were both allocated
    /// AND frozen in the same turn (e.g. born-frozen via `:new` in
    /// FrozenByDefault mode) appear in `new_allocs` but NOT here:
    /// the new-alloc list already implies their final state, and
    /// consumers only need a separate signal for transitions on
    /// already-canonical forms.
    pub freezings: Vec<FormId>,
}
