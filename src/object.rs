// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.
//
// Invariant: a General's slot_values[0] is always the object's parent,
// and slot_names[0] is always the interned symbol `parent`. That way
// parent is a first-class slot — slot_get / slot_names / slot_set find
// it naturally, and there's no "parent is a special field" branch
// anywhere in the slot protocol. Construction is done via Heap methods
// that know sym_parent and prepend the slot automatically.

use indexmap::IndexMap;
use serde::{Serialize, Deserialize};
use crate::value::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// General object: named slots + open handlers.
    /// By convention slot_names[0] == sym_parent, slot_values[0] == parent.
    General {
        slot_names: Vec<u32>,           // slot[0] is always `parent`
        slot_values: Vec<Value>,        // slot_values[0] is the parent value
        handlers: Vec<(u32, Value)>,    // selector → handler value
    },

    /// Optimized cons pair: parent is always the Cons prototype.
    Pair(Value, Value),

    /// Optimized string.
    Text(String),

    /// Optimized byte buffer (bytecode, raw data, etc.).
    Buffer(Vec<u8>),

    /// Lua-style table: sequential part + keyed part.
    /// `map` is IndexMap-backed — O(1) keyed lookup with insertion-order
    /// iteration. String keys are canonicalized to interned symbols at
    /// insert-time via Heap::canonicalize_key.
    Table {
        seq: Vec<Value>,
        map: IndexMap<Value, Value>,
    },
}

impl HeapObject {
    /// Construct a General with explicit slot 0 = parent. Callers that
    /// don't have sym_parent available should go through Heap::make_object
    /// or Heap::make_object_with_slots instead.
    pub fn new_general(parent_sym: u32, parent: Value, extra_names: Vec<u32>, extra_values: Vec<Value>) -> Self {
        debug_assert_eq!(extra_names.len(), extra_values.len());
        let mut slot_names = Vec::with_capacity(extra_names.len() + 1);
        let mut slot_values = Vec::with_capacity(extra_values.len() + 1);
        slot_names.push(parent_sym);
        slot_values.push(parent);
        slot_names.extend(extra_names);
        slot_values.extend(extra_values);
        HeapObject::General { slot_names, slot_values, handlers: Vec::new() }
    }

    pub fn new_empty(parent_sym: u32, parent: Value) -> Self {
        HeapObject::General {
            slot_names: vec![parent_sym],
            slot_values: vec![parent],
            handlers: Vec::new(),
        }
    }

    /// Get the parent value. By convention it's always slot_values[0] on
    /// a General; optimized types don't have their own parent, they
    /// delegate to their type prototype (resolved by prototype_of).
    pub fn parent(&self) -> Value {
        match self {
            HeapObject::General { slot_values, .. } => {
                slot_values.first().copied().unwrap_or(Value::NIL)
            }
            _ => Value::NIL,
        }
    }

    /// Look up a slot value by name (symbol ID). Parent falls out of this
    /// naturally because it's stored at slot_names[0] == sym_parent.
    pub fn slot_get(&self, name: u32) -> Option<Value> {
        match self {
            HeapObject::General { slot_names, slot_values, .. } => {
                slot_names.iter().position(|n| *n == name)
                    .map(|i| slot_values[i])
            }
            HeapObject::Pair(_, _) => None, // handled via Cons proto
            _ => None,
        }
    }

    /// Set a slot value by name. Grows the object if the slot doesn't
    /// exist. Writing `parent` reparents (finds slot_names[0]).
    pub fn slot_set(&mut self, name: u32, val: Value) -> bool {
        match self {
            HeapObject::General { slot_names, slot_values, .. } => {
                if let Some(i) = slot_names.iter().position(|n| *n == name) {
                    slot_values[i] = val;
                } else {
                    slot_names.push(name);
                    slot_values.push(val);
                }
                true
            }
            _ => false,
        }
    }

    /// Remove a slot by name. No-op for parent (slot 0) — removing it
    /// would break the invariant. Used by env_remove during eval's
    /// save/restore.
    pub fn slot_remove(&mut self, name: u32) {
        if let HeapObject::General { slot_names, slot_values, .. } = self {
            // don't remove slot 0 — that's parent, and removing it would
            // violate the invariant.
            if let Some(i) = slot_names.iter().position(|n| *n == name) {
                if i == 0 { return; }
                slot_names.remove(i);
                slot_values.remove(i);
            }
        }
    }

    /// Get the slot names (for introspection). Includes 'parent as the
    /// first entry — no wrapping needed.
    pub fn slot_names(&self) -> Vec<u32> {
        match self {
            HeapObject::General { slot_names, .. } => slot_names.clone(),
            _ => Vec::new(),
        }
    }

    /// Look up a handler by selector (symbol ID).
    pub fn handler_get(&self, selector: u32) -> Option<Value> {
        match self {
            HeapObject::General { handlers, .. } => {
                handlers.iter().find(|(s, _)| *s == selector).map(|(_, v)| *v)
            }
            _ => None,
        }
    }

    /// Set (or add) a handler. Handlers are open — always succeeds.
    pub fn handler_set(&mut self, selector: u32, handler: Value) {
        match self {
            HeapObject::General { handlers, .. } => {
                if let Some(entry) = handlers.iter_mut().find(|(s, _)| *s == selector) {
                    entry.1 = handler;
                } else {
                    handlers.push((selector, handler));
                }
            }
            _ => {
                // optimized types use their type prototype's handlers
            }
        }
    }

    /// Get all handler names (for introspection).
    pub fn handler_names(&self) -> Vec<u32> {
        match self {
            HeapObject::General { handlers, .. } => {
                handlers.iter().map(|(s, _)| *s).collect()
            }
            _ => Vec::new(),
        }
    }
}
