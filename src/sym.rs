//! symbol interner.
//!
//! a symbol is a Form (concepts/forms.md), but its identity is
//! mediated through an integer id. the interner is a cheap rust
//! data structure that maps name ↔ id; once we have a real vat with
//! a heap, the symtab is itself a Form on the heap.
//!
//! for phase 1: process-global symtab. when phase 2 introduces
//! per-vat heaps, this moves into the World/Vat and ids become
//! vat-local.

use std::collections::HashMap;

/// stable identity for an interned symbol.
///
/// `0` is a sentinel "no symbol." real symbols start at 1.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SymId(pub u32);

impl SymId {
    pub const NONE: SymId = SymId(0);
}

/// symbol table.
pub struct SymTab {
    by_name: HashMap<String, SymId>,
    by_id: Vec<String>,
}

impl SymTab {
    pub fn new() -> Self {
        // index 0 is reserved for SymId::NONE — push a placeholder
        SymTab {
            by_name: HashMap::new(),
            by_id: vec![String::new()],
        }
    }

    /// intern a name, returning its id. idempotent.
    pub fn intern(&mut self, name: &str) -> SymId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = SymId(self.by_id.len() as u32);
        self.by_id.push(name.to_string());
        self.by_name.insert(name.to_string(), id);
        id
    }

    /// look up the name of an id. panics if id is NONE or unknown.
    pub fn name(&self, id: SymId) -> &str {
        debug_assert!(id != SymId::NONE, "name() called on SymId::NONE");
        &self.by_id[id.0 as usize]
    }

    /// lookup by name, without interning.
    pub fn get(&self, name: &str) -> Option<SymId> {
        self.by_name.get(name).copied()
    }
}

impl Default for SymTab {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_idempotent() {
        let mut t = SymTab::new();
        let a = t.intern("foo");
        let b = t.intern("foo");
        let c = t.intern("bar");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn name_roundtrip() {
        let mut t = SymTab::new();
        let id = t.intern("hello");
        assert_eq!(t.name(id), "hello");
    }
}
