// Symbol interning.
//
// Every symbol in moof is a string (the human-visible name) with a
// u32 id (the VM-internal tag). The SymbolTable owns the mapping in
// both directions.
//
// Per-heap (i.e. per-vat) today: symbol ids are session-local and
// don't cross vat boundaries — the scheduler re-interns by name
// when it copies values across. A future direction is a shared
// intern table for cross-vat speed; keeping the abstraction
// separate makes that change tractable.

use std::collections::HashMap;

#[derive(Default)]
pub struct SymbolTable {
    names: Vec<String>,
    reverse: HashMap<String, u32>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `name` and return its u32 id. idempotent.
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.reverse.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.reverse.insert(name.to_string(), id);
        id
    }

    /// Name lookup by id. Panics on out-of-range (the VM never
    /// produces an unseen id, so this is a precondition, not an
    /// error).
    pub fn name(&self, id: u32) -> &str {
        &self.names[id as usize]
    }

    /// Find an existing symbol's id WITHOUT interning. Returns
    /// None if the name hasn't been seen.
    pub fn find(&self, name: &str) -> Option<u32> {
        self.reverse.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// All interned names, in id order. Used by image save.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Replace the contents wholesale. Used by image load to
    /// restore the full symbol table in one shot.
    pub fn restore(&mut self, names: Vec<String>) {
        self.names.clear();
        self.reverse.clear();
        for (i, name) in names.iter().enumerate() {
            self.reverse.insert(name.clone(), i as u32);
        }
        self.names = names;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_is_idempotent() {
        let mut t = SymbolTable::new();
        let a = t.intern("foo");
        let b = t.intern("foo");
        assert_eq!(a, b);
        assert_eq!(t.name(a), "foo");
    }

    #[test]
    fn find_vs_intern() {
        let mut t = SymbolTable::new();
        assert_eq!(t.find("foo"), None);
        let id = t.intern("foo");
        assert_eq!(t.find("foo"), Some(id));
    }

    #[test]
    fn distinct_names_get_distinct_ids() {
        let mut t = SymbolTable::new();
        let a = t.intern("alpha");
        let b = t.intern("beta");
        assert_ne!(a, b);
    }

    #[test]
    fn restore_round_trips() {
        let mut t = SymbolTable::new();
        t.intern("a");
        t.intern("b");
        t.intern("c");
        let names = t.names().to_vec();

        let mut t2 = SymbolTable::new();
        t2.restore(names.clone());
        assert_eq!(t2.len(), 3);
        assert_eq!(t2.find("a"), Some(0));
        assert_eq!(t2.find("b"), Some(1));
        assert_eq!(t2.find("c"), Some(2));
    }
}
