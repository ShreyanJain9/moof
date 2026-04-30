//! symbol interning.
//!
//! every symbol literal moof reads (`'foo`, `'+`, `'at:put:`) is
//! interned through this table. interning has two purposes:
//!
//! 1. `[a is b]` for two symbols with the same text returns `#true`
//!    in O(1) — comparing `SymId` is comparing two `u32`s.
//! 2. send dispatch's hash key is small and cache-friendly.
//!
//! per `laws/substrate-laws.md` L11, symbol identity is stable for
//! the lifetime of a vat. the same symbol text always interns to
//! the same `SymId`.
//!
//! the symbol *text* is preserved verbatim — case, dashes, colons,
//! everything. there is no normalization. `'Foo` and `'foo` are
//! different symbols. (case conventions are user discipline,
//! `docs/syntax/sigils.md`.)

use std::collections::HashMap;

/// the interned identity of a symbol. cheap to copy and compare.
///
/// the value zero is reserved as `SymId::NONE` for places where
/// the substrate needs an "absent symbol" sentinel.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SymId(pub u32);

impl SymId {
    /// the reserved "absent symbol" sentinel.
    pub const NONE: SymId = SymId(0);

    /// `true` if this is the sentinel.
    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

/// the interning table itself.
///
/// uses an `IndexMap`-equivalent shape (a `HashMap` plus a parallel
/// `Vec`) so we can both look up by text in O(1) *and* recover
/// the text from a `SymId` in O(1).
///
/// not thread-safe — vats are single-threaded; the `World` owns
/// its `SymTable`.
pub struct SymTable {
    by_id: Vec<String>,
    by_name: HashMap<String, SymId>,
}

impl SymTable {
    pub fn new() -> Self {
        // index 0 is reserved for SymId::NONE.
        let mut by_id = Vec::with_capacity(64);
        by_id.push(String::new()); // SymId(0) is "" the sentinel
        SymTable {
            by_id,
            by_name: HashMap::with_capacity(64),
        }
    }

    /// intern a symbol by name. same name ⇒ same id forever.
    pub fn intern(&mut self, name: &str) -> SymId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = SymId(self.by_id.len() as u32);
        self.by_id.push(name.to_string());
        self.by_name.insert(name.to_string(), id);
        id
    }

    /// recover the text for an interned id.
    ///
    /// panics if the id was never interned (i.e., is from a
    /// different table or fabricated).
    pub fn resolve(&self, id: SymId) -> &str {
        &self.by_id[id.0 as usize]
    }

    /// `true` if `name` has ever been interned.
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// number of interned symbols (excluding the `NONE` sentinel).
    pub fn len(&self) -> usize {
        self.by_id.len() - 1
    }

    /// `true` if no symbols have been interned.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for SymTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_same_name_returns_same_id() {
        let mut t = SymTable::new();
        let a = t.intern("foo");
        let b = t.intern("foo");
        assert_eq!(a, b);
    }

    #[test]
    fn intern_distinct_names_distinct_ids() {
        let mut t = SymTable::new();
        let a = t.intern("foo");
        let b = t.intern("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_returns_original_name() {
        let mut t = SymTable::new();
        let id = t.intern("hello-world");
        assert_eq!(t.resolve(id), "hello-world");
    }

    #[test]
    fn case_is_preserved() {
        let mut t = SymTable::new();
        let lower = t.intern("foo");
        let upper = t.intern("Foo");
        assert_ne!(lower, upper);
        assert_eq!(t.resolve(lower), "foo");
        assert_eq!(t.resolve(upper), "Foo");
    }

    #[test]
    fn keyword_selectors_intern() {
        // smalltalk-style keyword selectors are full symbols
        // (`syntax/sigils.md`).
        let mut t = SymTable::new();
        let sel = t.intern("at:put:");
        assert_eq!(t.resolve(sel), "at:put:");
    }

    #[test]
    fn operator_symbols_intern() {
        let mut t = SymTable::new();
        let plus = t.intern("+");
        let minus = t.intern("-");
        assert_ne!(plus, minus);
        assert_eq!(t.resolve(plus), "+");
    }

    #[test]
    fn none_sentinel_is_distinct() {
        let mut t = SymTable::new();
        let foo = t.intern("foo");
        assert_ne!(foo, SymId::NONE);
        assert!(SymId::NONE.is_none());
        assert!(!foo.is_none());
    }

    #[test]
    fn empty_string_interns() {
        // edge case: symbol text "" is legal (though unusual).
        // it interns to a fresh id — *not* SymId::NONE.
        let mut t = SymTable::new();
        let empty = t.intern("");
        assert_ne!(empty, SymId::NONE);
        assert_eq!(t.resolve(empty), "");
    }

    #[test]
    fn contains_and_len_track_interning() {
        let mut t = SymTable::new();
        assert!(t.is_empty());
        t.intern("a");
        assert_eq!(t.len(), 1);
        assert!(t.contains("a"));
        assert!(!t.contains("b"));
        t.intern("a"); // duplicate
        assert_eq!(t.len(), 1);
        t.intern("b");
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn ids_are_dense_starting_at_one() {
        let mut t = SymTable::new();
        let a = t.intern("a");
        let b = t.intern("b");
        let c = t.intern("c");
        assert_eq!(a.0, 1);
        assert_eq!(b.0, 2);
        assert_eq!(c.0, 3);
    }
}
