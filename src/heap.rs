//! Form heap — phase 1 minimum.
//!
//! a Vec-backed allocator. ids are indices. allocation is push;
//! there is no GC yet (phase 2 adds it; not needed for the phase 1
//! forcing function).
//!
//! later phases:
//! - GC: tracing collector across the heap (phase 2).
//! - per-vat heaps: each vat has its own; ids are vat-local
//!   (phase 2; concepts/vats.md).
//! - mmap'd persistence: heap pages map to files on disk
//!   (phase 2; concepts/persistence.md).

use crate::form::{Form, FormId};

/// a contiguous heap of Forms.
pub struct Heap {
    forms: Vec<Form>,
}

impl Heap {
    pub fn new() -> Self {
        // index 0 is reserved for FormId::NONE — push a dead placeholder.
        Heap {
            forms: vec![Form::default()],
        }
    }

    /// allocate a new Form. returns its id.
    pub fn alloc(&mut self, form: Form) -> FormId {
        let id = self.forms.len() as u32;
        self.forms.push(form);
        FormId(id)
    }

    /// borrow a Form by id.
    pub fn get(&self, id: FormId) -> &Form {
        debug_assert!(!id.is_none(), "get() on FormId::NONE");
        &self.forms[id.0 as usize]
    }

    /// mutably borrow a Form by id.
    pub fn get_mut(&mut self, id: FormId) -> &mut Form {
        debug_assert!(!id.is_none(), "get_mut() on FormId::NONE");
        &mut self.forms[id.0 as usize]
    }

    /// total Forms allocated (including the placeholder at index 0).
    pub fn len(&self) -> usize {
        self.forms.len()
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}
