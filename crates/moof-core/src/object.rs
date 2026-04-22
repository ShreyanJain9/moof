// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.
//
// Prototype delegation is a VM-internal mechanism: every General has
// a `proto` field used only for message-dispatch chain walking. It's
// NOT a slot. It doesn't appear in slotNames. It can't be read via
// slotAt:. If userland wants a chain-walk concept (e.g. Environments
// pointing at outer scopes for variable lookup), they put that on a
// real slot — `bindings`, `outer`, whatever — and the semantics are
// defined by the type, not by the VM.

use crate::value::Value;
use crate::foreign::ForeignData;

#[derive(Debug, Clone)]
pub enum HeapObject {
    /// The one heap variant. A prototype pointer (VM-internal, for
    /// dispatch) plus named slots, handlers, and an optional foreign
    /// payload (Ruby-style rust wrapping). Pair, Text, Bytes, Table,
    /// Vec3, and every user-plugin type flow through this — the only
    /// distinction is whether `foreign` is None (plain moof object)
    /// or Some(…) (rust-backed, with vtable for GC / serialize /
    /// cross-vat / virtual slots). Foreign payloads are immutable.
    General {
        proto: Value,                   // VM-internal dispatch pointer (NOT a slot)
        slot_names: Vec<u32>,
        slot_values: Vec<Value>,
        handlers: Vec<(u32, Value)>,
        foreign: Option<ForeignData>,
    },
}

impl HeapObject {
    pub fn new_general(proto: Value, slot_names: Vec<u32>, slot_values: Vec<Value>) -> Self {
        debug_assert_eq!(slot_names.len(), slot_values.len());
        HeapObject::General {
            proto,
            slot_names,
            slot_values,
            handlers: Vec::new(),
            foreign: None,
        }
    }

    pub fn new_empty(proto: Value) -> Self {
        HeapObject::General {
            proto,
            slot_names: Vec::new(),
            slot_values: Vec::new(),
            handlers: Vec::new(),
            foreign: None,
        }
    }

    pub fn new_foreign(proto: Value, foreign: ForeignData) -> Self {
        HeapObject::General {
            proto,
            slot_names: Vec::new(),
            slot_values: Vec::new(),
            handlers: Vec::new(),
            foreign: Some(foreign),
        }
    }

    pub fn foreign(&self) -> Option<&ForeignData> {
        let HeapObject::General { foreign, .. } = self;
        foreign.as_ref()
    }

    /// The VM-internal prototype used for dispatch's chain walk. Not a
    /// slot — this is the language's delegation machinery.
    pub fn proto(&self) -> Value {
        let HeapObject::General { proto, .. } = self;
        *proto
    }

    pub fn set_proto(&mut self, p: Value) {
        if let HeapObject::General { proto, .. } = self {
            *proto = p;
        }
    }

    /// Look up a slot value by name (symbol ID). Note: this only
    /// walks the real slots vec — foreign virtual slots (e.g. a
    /// Pair's car/cdr) are handled by `Heap::slot_of`.
    pub fn slot_get(&self, name: u32) -> Option<Value> {
        let HeapObject::General { slot_names, slot_values, .. } = self;
        slot_names.iter().position(|n| *n == name).map(|i| slot_values[i])
    }

    /// Set a slot value by name. Grows the object if the slot doesn't exist.
    pub fn slot_set(&mut self, name: u32, val: Value) -> bool {
        let HeapObject::General { slot_names, slot_values, .. } = self;
        if let Some(i) = slot_names.iter().position(|n| *n == name) {
            slot_values[i] = val;
        } else {
            slot_names.push(name);
            slot_values.push(val);
        }
        true
    }

    /// Remove a slot by name. No-op for missing slots.
    pub fn slot_remove(&mut self, name: u32) {
        let HeapObject::General { slot_names, slot_values, .. } = self;
        if let Some(i) = slot_names.iter().position(|n| *n == name) {
            slot_names.remove(i);
            slot_values.remove(i);
        }
    }

    /// Get the explicit slot names (not including the proto — that's
    /// VM-internal, not a slot).
    pub fn slot_names(&self) -> Vec<u32> {
        let HeapObject::General { slot_names, .. } = self;
        slot_names.clone()
    }

    /// Look up a handler by selector (symbol ID).
    pub fn handler_get(&self, selector: u32) -> Option<Value> {
        let HeapObject::General { handlers, .. } = self;
        handlers.iter().find(|(s, _)| *s == selector).map(|(_, v)| *v)
    }

    /// Set (or add) a handler. Handlers are open — always succeeds.
    pub fn handler_set(&mut self, selector: u32, handler: Value) {
        let HeapObject::General { handlers, .. } = self;
        if let Some(entry) = handlers.iter_mut().find(|(s, _)| *s == selector) {
            entry.1 = handler;
        } else {
            handlers.push((selector, handler));
        }
    }

    /// Get all handler names (for introspection).
    pub fn handler_names(&self) -> Vec<u32> {
        let HeapObject::General { handlers, .. } = self;
        handlers.iter().map(|(s, _)| *s).collect()
    }
}
