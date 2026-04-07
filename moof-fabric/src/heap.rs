/// The heap: an arena of objects + a symbol table.
///
/// Objects are identified by u32 indices. Symbols are interned strings.
/// The heap is the persistent state of the fabric — save it, and you save
/// the entire objectspace.

use crate::value::{Value, HeapObject};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Heap {
    objects: Vec<HeapObject>,
    symbol_names: Vec<String>,
    #[serde(skip)]
    symbol_lookup: HashMap<String, u32>,
}

impl Heap {
    pub fn new() -> Self {
        let mut heap = Heap {
            objects: Vec::new(),
            symbol_names: Vec::new(),
            symbol_lookup: HashMap::new(),
        };
        // Reserve id 0 as a nil sentinel
        heap.objects.push(HeapObject::Object {
            parent: Value::Nil,
            slots: Vec::new(),
            handlers: Vec::new(),
        });
        heap
    }

    // ── Allocation ──

    pub fn alloc(&mut self, obj: HeapObject) -> u32 {
        let id = self.objects.len() as u32;
        self.objects.push(obj);
        id
    }

    pub fn alloc_object(&mut self, parent: Value) -> u32 {
        self.alloc(HeapObject::Object {
            parent,
            slots: Vec::new(),
            handlers: Vec::new(),
        })
    }

    pub fn alloc_string(&mut self, s: &str) -> Value {
        Value::Object(self.alloc(HeapObject::String(s.to_string())))
    }

    pub fn alloc_bytes(&mut self, data: Vec<u8>) -> u32 {
        self.alloc(HeapObject::Bytes(data))
    }

    pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
        Value::Object(self.alloc(HeapObject::Cons { car, cdr }))
    }

    pub fn list(&mut self, vals: &[Value]) -> Value {
        let mut result = Value::Nil;
        for &v in vals.iter().rev() {
            result = self.cons(v, result);
        }
        result
    }

    // ── Access ──

    pub fn get(&self, id: u32) -> &HeapObject {
        &self.objects[id as usize]
    }

    pub fn get_mut(&mut self, id: u32) -> &mut HeapObject {
        &mut self.objects[id as usize]
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn car(&self, val: Value) -> Value {
        match val {
            Value::Object(id) => match self.get(id) {
                HeapObject::Cons { car, .. } => *car,
                _ => Value::Nil,
            },
            _ => Value::Nil,
        }
    }

    pub fn cdr(&self, val: Value) -> Value {
        match val {
            Value::Object(id) => match self.get(id) {
                HeapObject::Cons { cdr, .. } => *cdr,
                _ => Value::Nil,
            },
            _ => Value::Nil,
        }
    }

    pub fn list_to_vec(&self, val: Value) -> Vec<Value> {
        let mut result = Vec::new();
        let mut current = val;
        while let Value::Object(id) = current {
            match self.get(id) {
                HeapObject::Cons { car, cdr } => {
                    result.push(*car);
                    current = *cdr;
                }
                _ => break,
            }
        }
        result
    }

    // ── Symbols ──

    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.symbol_lookup.get(name) {
            return id;
        }
        let id = self.symbol_names.len() as u32;
        self.symbol_names.push(name.to_string());
        self.symbol_lookup.insert(name.to_string(), id);
        id
    }

    pub fn symbol_name(&self, id: u32) -> &str {
        &self.symbol_names[id as usize]
    }

    pub fn symbol_lookup_only(&self, name: &str) -> Option<u32> {
        self.symbol_lookup.get(name).copied()
    }

    pub fn symbol_count(&self) -> usize {
        self.symbol_names.len()
    }

    /// Rebuild symbol_lookup from symbol_names (after deserialization).
    pub fn rebuild_symbol_lookup(&mut self) {
        self.symbol_lookup.clear();
        for (i, name) in self.symbol_names.iter().enumerate() {
            self.symbol_lookup.insert(name.clone(), i as u32);
        }
    }

    // ── Slots ──

    pub fn slot_get(&self, obj_id: u32, sym: u32) -> Value {
        match self.get(obj_id) {
            HeapObject::Object { slots, .. } => {
                slots.iter()
                    .find(|(k, _)| *k == sym)
                    .map(|(_, v)| *v)
                    .unwrap_or(Value::Nil)
            }
            _ => Value::Nil,
        }
    }

    pub fn slot_set(&mut self, obj_id: u32, sym: u32, val: Value) {
        if let HeapObject::Object { slots, .. } = self.get_mut(obj_id) {
            if let Some(entry) = slots.iter_mut().find(|(k, _)| *k == sym) {
                entry.1 = val;
            } else {
                slots.push((sym, val));
            }
        }
    }

    pub fn slot_names(&self, obj_id: u32) -> Vec<u32> {
        match self.get(obj_id) {
            HeapObject::Object { slots, .. } => slots.iter().map(|(k, _)| *k).collect(),
            _ => Vec::new(),
        }
    }

    // ── Handlers ──

    pub fn add_handler(&mut self, obj_id: u32, selector: u32, handler: Value) {
        if let HeapObject::Object { handlers, .. } = self.get_mut(obj_id) {
            if let Some(entry) = handlers.iter_mut().find(|(k, _)| *k == selector) {
                entry.1 = handler;
            } else {
                handlers.push((selector, handler));
            }
        }
    }

    pub fn handler_names(&self, obj_id: u32) -> Vec<u32> {
        match self.get(obj_id) {
            HeapObject::Object { handlers, .. } => handlers.iter().map(|(k, _)| *k).collect(),
            _ => Vec::new(),
        }
    }

    pub fn parent(&self, obj_id: u32) -> Value {
        match self.get(obj_id) {
            HeapObject::Object { parent, .. } => *parent,
            _ => Value::Nil,
        }
    }

    pub fn set_parent(&mut self, obj_id: u32, new_parent: Value) {
        if let HeapObject::Object { parent, .. } = self.get_mut(obj_id) {
            *parent = new_parent;
        }
    }

    // ── Environments ──

    pub fn alloc_env(&mut self, parent: Option<u32>) -> u32 {
        self.alloc(HeapObject::Environment {
            parent,
            bindings: HashMap::new(),
        })
    }

    pub fn env_define(&mut self, env_id: u32, sym: u32, val: Value) {
        if let HeapObject::Environment { bindings, .. } = self.get_mut(env_id) {
            bindings.insert(sym, val);
        }
    }

    pub fn env_lookup(&self, env_id: u32, sym: u32) -> Option<Value> {
        let mut current = Some(env_id);
        while let Some(eid) = current {
            match self.get(eid) {
                HeapObject::Environment { parent, bindings } => {
                    if let Some(val) = bindings.get(&sym) {
                        return Some(*val);
                    }
                    current = *parent;
                }
                _ => return None,
            }
        }
        None
    }

    pub fn env_set(&mut self, env_id: u32, sym: u32, val: Value) -> bool {
        // Walk the chain to find the binding, then mutate
        let mut current = Some(env_id);
        while let Some(eid) = current {
            match self.get(eid) {
                HeapObject::Environment { parent, bindings } => {
                    if bindings.contains_key(&sym) {
                        // Found it — mutate
                        if let HeapObject::Environment { bindings, .. } = self.get_mut(eid) {
                            bindings.insert(sym, val);
                        }
                        return true;
                    }
                    current = *parent;
                }
                _ => return false,
            }
        }
        false
    }

    pub fn env_remove(&mut self, env_id: u32, sym: u32) {
        if let HeapObject::Environment { bindings, .. } = self.get_mut(env_id) {
            bindings.remove(&sym);
        }
    }

    // ── Mutation helper ──

    pub fn mutate<F: FnOnce(&mut HeapObject)>(&mut self, id: u32, f: F) {
        f(&mut self.objects[id as usize]);
    }

    // ── Serialization helpers ──

    pub fn objects_clone(&self) -> Vec<HeapObject> { self.objects.clone() }
    pub fn symbol_names_clone(&self) -> Vec<String> { self.symbol_names.clone() }

    pub fn from_parts(objects: Vec<HeapObject>, symbol_names: Vec<String>) -> Self {
        let mut heap = Heap {
            objects,
            symbol_names,
            symbol_lookup: HashMap::new(),
        };
        heap.rebuild_symbol_lookup();
        heap
    }
}
