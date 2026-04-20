// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.

use indexmap::IndexMap;
use serde::{Serialize, Deserialize};
use crate::value::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// General object: parent + named slots + handlers.
    /// Slots are NOT fixed-shape — new slots can be added via slot_set.
    /// (Used to be fixed; we relaxed that when folding Environment in. If
    /// a use-site wants to reject adding new slots, it can check slot_get
    /// first.)
    /// Closures are Generals with code_idx / arity / is_operative / is_pure
    /// slots plus their captures as regular slots.
    General {
        parent: Value,
        slot_names: Vec<u32>,           // symbol IDs
        slot_values: Vec<Value>,        // values, same length as slot_names
        handlers: Vec<(u32, Value)>,    // selector → handler value
    },

    /// Optimized cons pair: parent is always the Cons prototype.
    Pair(Value, Value),

    /// Optimized string.
    Text(String),

    /// Optimized byte buffer (bytecode, raw data, etc.).
    Buffer(Vec<u8>),

    /// Lua-style table: sequential part + keyed part.
    /// `map` is IndexMap-backed → O(1) keyed lookup AND insertion-order
    /// iteration (crucial for stable describe/show of sets + bags). String
    /// keys are content-normalized at insert-time via canonicalize_key —
    /// equal strings land in the same bucket.
    Table {
        seq: Vec<Value>,
        map: IndexMap<Value, Value>,
    },
}

impl HeapObject {
    pub fn new_general(parent: Value, slot_names: Vec<u32>, slot_values: Vec<Value>) -> Self {
        debug_assert_eq!(slot_names.len(), slot_values.len());
        HeapObject::General {
            parent,
            slot_names,
            slot_values,
            handlers: Vec::new(),
        }
    }

    pub fn new_empty(parent: Value) -> Self {
        HeapObject::General {
            parent,
            slot_names: Vec::new(),
            slot_values: Vec::new(),
            handlers: Vec::new(),
        }
    }

    /// Get the parent value (for delegation).
    pub fn parent(&self) -> Value {
        match self {
            HeapObject::General { parent, .. } => *parent,
            // optimized types delegate to their type prototype (resolved by dispatch)
            _ => Value::NIL,
        }
    }

    /// Look up a slot value by name (symbol ID).
    pub fn slot_get(&self, name: u32) -> Option<Value> {
        match self {
            HeapObject::General { slot_names, slot_values, .. } => {
                slot_names.iter().position(|n| *n == name)
                    .map(|i| slot_values[i])
            }
            HeapObject::Pair(_car, _cdr) => None, // handled by dispatch via Cons proto
            _ => None,
        }
    }

    /// Set a slot value by name. Grows the object if the slot doesn't exist —
    /// environments add bindings this way. Always succeeds (returns true for
    /// legacy callers that checked the result).
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

    /// Remove a slot by name. No-op if the slot doesn't exist or the
    /// object can't be shrunk. Used by env_remove during eval's save/restore.
    pub fn slot_remove(&mut self, name: u32) {
        if let HeapObject::General { slot_names, slot_values, .. } = self {
            if let Some(i) = slot_names.iter().position(|n| *n == name) {
                slot_names.remove(i);
                slot_values.remove(i);
            }
        }
    }

    /// Get the slot names (for introspection).
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
                // optimized types can't have per-instance handlers
                // (they use the type prototype's handlers via delegation)
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
