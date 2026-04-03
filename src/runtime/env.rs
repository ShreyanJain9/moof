/// First-class environments (§7.3).
///
/// Environments are objects in the heap. They hold bindings (symbol → value)
/// and an optional parent link for lexical scoping. Because they're first-class,
/// `vau` can capture and pass them around — this is what makes the reflective
/// tower possible.

use super::value::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Environment {
    /// Parent environment (heap id), or None for the root.
    pub parent: Option<u32>,
    /// Bindings in this frame: symbol id → value.
    pub bindings: HashMap<u32, Value>,
}

impl Environment {
    pub fn new(parent: Option<u32>) -> Self {
        Environment {
            parent,
            bindings: HashMap::new(),
        }
    }

    pub fn define(&mut self, sym: u32, val: Value) {
        self.bindings.insert(sym, val);
    }

    pub fn lookup_local(&self, sym: u32) -> Option<Value> {
        self.bindings.get(&sym).copied()
    }
}
