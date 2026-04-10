// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.
//
// Fixed-shape slots: an object's slot NAMES are sealed at creation.
// Only values can change. Handlers are open — add them anytime.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::value::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// General object: parent + fixed named slots + open handlers.
    General {
        parent: Value,
        slot_names: Vec<u32>,           // symbol IDs, fixed at creation
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
    /// One data structure replaces both Array and HashMap.
    Table {
        seq: Vec<Value>,              // sequential (integer-indexed, 0-based)
        map: Vec<(Value, Value)>,     // keyed (arbitrary key-value pairs)
    },

    /// Closure: a compiled function or operative.
    /// Parent chain leads to Block prototype → Object.
    Closure {
        parent: Value,                // Block/Closure prototype (→ Object)
        code_idx: usize,             // index into VM's closure_descs
        arity: u8,
        is_operative: bool,
        captures: Vec<(u32, Value)>, // (name_sym, captured_value)
        handlers: Vec<(u32, Value)>, // per-instance handlers (e.g. call:)
    },

    /// Environment: dynamic-shape binding object.
    /// Unlike General (fixed shape), new bindings can be added at any time.
    /// Used as the root namespace — `def` writes here, `GetGlobal` reads here.
    Environment {
        parent: Value,                        // scope chain (NIL for root)
        bindings: HashMap<u32, Value>,        // sym → value, O(1), dynamic
        handlers: Vec<(u32, Value)>,          // like all objects
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
            HeapObject::General { parent, .. } |
            HeapObject::Closure { parent, .. } |
            HeapObject::Environment { parent, .. } => *parent,
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
            HeapObject::Closure { captures, .. } => {
                // captured values are accessible as slots
                captures.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
            }
            HeapObject::Environment { bindings, .. } => {
                bindings.get(&name).copied()
            }
            HeapObject::Pair(_car, _cdr) => {
                None // handled by dispatch via Cons prototype
            }
            _ => None,
        }
    }

    /// Set a slot value by name. Returns false if slot doesn't exist (shape is fixed).
    pub fn slot_set(&mut self, name: u32, val: Value) -> bool {
        match self {
            HeapObject::General { slot_names, slot_values, .. } => {
                if let Some(i) = slot_names.iter().position(|n| *n == name) {
                    slot_values[i] = val;
                    true
                } else {
                    false // shape is fixed — can't add slots
                }
            }
            HeapObject::Closure { captures, .. } => {
                if let Some(cap) = captures.iter_mut().find(|(n, _)| *n == name) {
                    cap.1 = val;
                    true
                } else {
                    false
                }
            }
            HeapObject::Environment { bindings, .. } => {
                bindings.insert(name, val);
                true // dynamic shape — always succeeds
            }
            _ => false,
        }
    }

    /// Get the slot names (for introspection).
    pub fn slot_names(&self) -> Vec<u32> {
        match self {
            HeapObject::General { slot_names, .. } => slot_names.clone(),
            HeapObject::Closure { captures, .. } => captures.iter().map(|(n, _)| *n).collect(),
            HeapObject::Environment { bindings, .. } => bindings.keys().copied().collect(),
            _ => Vec::new(),
        }
    }

    /// Look up a handler by selector (symbol ID).
    pub fn handler_get(&self, selector: u32) -> Option<Value> {
        match self {
            HeapObject::General { handlers, .. } |
            HeapObject::Closure { handlers, .. } |
            HeapObject::Environment { handlers, .. } => {
                handlers.iter().find(|(s, _)| *s == selector).map(|(_, v)| *v)
            }
            _ => None,
        }
    }

    /// Set (or add) a handler. Handlers are open — always succeeds.
    pub fn handler_set(&mut self, selector: u32, handler: Value) {
        match self {
            HeapObject::General { handlers, .. } |
            HeapObject::Closure { handlers, .. } |
            HeapObject::Environment { handlers, .. } => {
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
            HeapObject::General { handlers, .. } |
            HeapObject::Closure { handlers, .. } |
            HeapObject::Environment { handlers, .. } => {
                handlers.iter().map(|(s, _)| *s).collect()
            }
            _ => Vec::new(),
        }
    }
}
