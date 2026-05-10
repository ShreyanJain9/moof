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
    pub(crate) forms: Vec<Form>,
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
        // post-V0 the vat-local payload is 30 bits, so the per-vat
        // ceiling is ~1B forms (vs 4B before). still vastly more
        // than any real moof workload.
        assert!(
            (id as u32) < crate::form::MAX_PAYLOAD,
            "vat heap exhausted: {} forms allocated (limit {})",
            id, crate::form::MAX_PAYLOAD
        );
        self.forms.push(form);
        FormId::vat_local(id as u32)
    }

    /// borrow a Form by id.
    pub fn get(&self, id: FormId) -> &Form {
        use crate::form::Scope;
        debug_assert!(!id.is_none(), "Heap::get on FormId::NONE");
        match id.scope() {
            Scope::VatLocal => &self.forms[id.payload() as usize],
            Scope::Shared => panic!(
                "shared segment not yet supported (V6); got id payload {}",
                id.payload()
            ),
            Scope::FarRef => panic!(
                "far-ref table not yet supported (V5); got id payload {}",
                id.payload()
            ),
            Scope::Reserved => panic!(
                "reserved scope: id payload {}",
                id.payload()
            ),
        }
    }

    /// mutably borrow a Form by id.
    pub fn get_mut(&mut self, id: FormId) -> &mut Form {
        use crate::form::Scope;
        debug_assert!(!id.is_none(), "Heap::get_mut on FormId::NONE");
        match id.scope() {
            Scope::VatLocal => &mut self.forms[id.payload() as usize],
            Scope::Shared => panic!(
                "shared segment not yet supported (V6); got id payload {}",
                id.payload()
            ),
            Scope::FarRef => panic!(
                "far-ref table not yet supported (V5); got id payload {}",
                id.payload()
            ),
            Scope::Reserved => panic!(
                "reserved scope: id payload {}",
                id.payload()
            ),
        }
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
