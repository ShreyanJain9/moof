//! Form heap — the substrate's allocator.
//!
//! a `Vec<Form>` indexed by `FormId`. allocation pushes; index
//! zero is reserved as the [`FormId::NONE`] sentinel.
//!
//! per `laws/substrate-laws.md` L11, FormIds are stable for the
//! life of the vat. we therefore do **not** compact / renumber
//! during gc — phase B's gc tombstones dead slots; phase G+
//! considers an indirection table if heap density becomes a
//! concern.
//!
//! per `laws/determinism-laws.md` D4, allocation order in a
//! replicated vat is deterministic by turn-seq + per-turn
//! ordinal. phase A is single-vat solo, so the deterministic-id
//! discipline isn't enforced here yet — `Heap::alloc` simply
//! returns the next index. phase D adds a deterministic allocator.

use crate::form::{Form, FormId};

/// a contiguous, single-vat heap of Forms.
pub struct Heap {
    forms: Vec<Form>,
}

impl Heap {
    pub fn new() -> Self {
        // index 0 is reserved for FormId::NONE — push a placeholder
        // so we never hand it out.
        Heap {
            forms: vec![Form::default()],
        }
    }

    /// allocate a new Form, returning its id.
    ///
    /// the id is stable for the heap's lifetime
    /// (`laws/substrate-laws.md` L11).
    pub fn alloc(&mut self, form: Form) -> FormId {
        let id = self.forms.len();
        // `usize` could in principle exceed `u32`. on 64-bit, this
        // is a 4-billion-form ceiling — way more than any real moof
        // workload should reach. enforce it explicitly.
        assert!(id < u32::MAX as usize, "heap exhausted: 4G forms allocated");
        self.forms.push(form);
        FormId(id as u32)
    }

    /// borrow a Form by id.
    pub fn get(&self, id: FormId) -> &Form {
        debug_assert!(!id.is_none(), "Heap::get on FormId::NONE");
        &self.forms[id.0 as usize]
    }

    /// mutably borrow a Form by id.
    pub fn get_mut(&mut self, id: FormId) -> &mut Form {
        debug_assert!(!id.is_none(), "Heap::get_mut on FormId::NONE");
        &mut self.forms[id.0 as usize]
    }

    /// total Forms allocated (including the placeholder at index 0).
    pub fn len(&self) -> usize {
        self.forms.len()
    }

    /// `true` if no real allocations have happened yet.
    pub fn is_empty(&self) -> bool {
        // index 0 is always present; "empty" means only the
        // sentinel slot is occupied.
        self.forms.len() == 1
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sym::SymId;
    use crate::value::Value;

    #[test]
    fn alloc_returns_distinct_ids() {
        let mut h = Heap::new();
        let a = h.alloc(Form::default());
        let b = h.alloc(Form::default());
        assert_ne!(a, b);
        assert!(!a.is_none());
        assert!(!b.is_none());
    }

    #[test]
    fn ids_start_at_one() {
        let mut h = Heap::new();
        let id = h.alloc(Form::default());
        assert_eq!(id.0, 1, "id 0 is reserved for FormId::NONE");
    }

    #[test]
    fn get_returns_what_was_put() {
        let mut h = Heap::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(42));
        let id = h.alloc(f);
        assert_eq!(h.get(id).slot(SymId(7)), Value::Int(42));
    }

    #[test]
    fn get_mut_can_modify_in_place() {
        let mut h = Heap::new();
        let id = h.alloc(Form::default());
        h.get_mut(id).slots.insert(SymId(1), Value::Int(99));
        assert_eq!(h.get(id).slot(SymId(1)), Value::Int(99));
    }

    #[test]
    fn ids_are_stable_across_other_allocs() {
        // L11: allocation of more forms must not invalidate
        // existing ids.
        let mut h = Heap::new();
        let a = h.alloc(Form::default());
        h.get_mut(a).slots.insert(SymId(1), Value::Int(100));
        for _ in 0..50 {
            h.alloc(Form::default());
        }
        // a still resolves; its slot value is intact.
        assert_eq!(h.get(a).slot(SymId(1)), Value::Int(100));
    }

    #[test]
    fn len_includes_sentinel_slot() {
        let mut h = Heap::new();
        assert_eq!(h.len(), 1, "fresh heap holds only the sentinel");
        assert!(h.is_empty());
        h.alloc(Form::default());
        assert_eq!(h.len(), 2);
        assert!(!h.is_empty());
    }

    #[test]
    #[should_panic(expected = "FormId::NONE")]
    fn get_on_none_panics_in_debug() {
        let h = Heap::new();
        let _ = h.get(FormId::NONE);
    }
}
