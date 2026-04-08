// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.
//
// Fixed-shape slots: an object's slot NAMES are sealed at creation.
// Only values can change. Handlers are open — add them anytime.

use crate::value::Value;

#[derive(Debug, Clone)]
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

    /// Mutable indexed collection.
    Array(Vec<Value>),

    /// Mutable key-value collection.
    Map(Vec<(Value, Value)>),
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
            HeapObject::Pair(car, cdr) => {
                // car=0 and cdr=1 are resolved by the caller using interned symbols
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
            _ => false,
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
