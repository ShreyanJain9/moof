/// The heap: an arena of HeapObjects indexed by u32.
///
/// "Objects are indices into a typed arena slab, not raw pointers.
///  Image serialization is 'serialize the slab.'" (§9.2)

use super::value::{Value, HeapObject, BytecodeChunk};
use super::env::Environment;
use std::collections::HashMap;

pub struct Heap {
    /// The arena: all heap-allocated objects live here.
    objects: Vec<HeapObject>,
    /// Symbol interning table: string → symbol id.
    symbol_names: Vec<String>,
    symbol_lookup: HashMap<String, u32>,
}

impl Heap {
    pub fn new() -> Self {
        Heap {
            objects: Vec::new(),
            symbol_names: Vec::new(),
            symbol_lookup: HashMap::new(),
        }
    }

    /// Allocate a new heap object, returns its index.
    pub fn alloc(&mut self, obj: HeapObject) -> u32 {
        let id = self.objects.len() as u32;
        self.objects.push(obj);
        id
    }

    /// Get a reference to a heap object.
    pub fn get(&self, id: u32) -> &HeapObject {
        &self.objects[id as usize]
    }

    /// Mutate a heap object via closure.
    pub fn mutate<F>(&mut self, id: u32, f: F)
    where F: FnOnce(&mut HeapObject)
    {
        f(&mut self.objects[id as usize]);
    }

    /// Get a mutable reference.
    pub fn get_mut(&mut self, id: u32) -> &mut HeapObject {
        &mut self.objects[id as usize]
    }

    /// Intern a symbol. Returns the symbol id.
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.symbol_lookup.get(name) {
            return id;
        }
        let id = self.symbol_names.len() as u32;
        self.symbol_names.push(name.to_string());
        self.symbol_lookup.insert(name.to_string(), id);
        id
    }

    /// Look up a symbol name by id.
    pub fn symbol_name(&self, id: u32) -> &str {
        &self.symbol_names[id as usize]
    }

    /// Number of objects on the heap.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Number of interned symbols.
    pub fn symbol_count(&self) -> usize {
        self.symbol_names.len()
    }

    /// Clone all heap objects (for serialization).
    pub fn objects_clone(&self) -> Vec<HeapObject> {
        self.objects.clone()
    }

    /// Clone all symbol names (for serialization).
    pub fn symbol_names_clone(&self) -> Vec<String> {
        self.symbol_names.clone()
    }

    /// Reconstruct a heap from a binary image.
    pub fn from_image(objects: Vec<HeapObject>, symbol_names: Vec<String>) -> Self {
        let mut symbol_lookup = HashMap::new();
        for (id, name) in symbol_names.iter().enumerate() {
            symbol_lookup.insert(name.clone(), id as u32);
        }
        Heap {
            objects,
            symbol_names,
            symbol_lookup,
        }
    }

    // ── Specific mutation methods ──

    /// Define a binding in an environment.
    pub fn env_define(&mut self, env_id: u32, sym: u32, val: Value) {
        self.mutate(env_id, |obj| {
            if let HeapObject::Environment(env) = obj {
                env.define(sym, val);
            }
        });
    }

    /// Add or replace a handler on a GeneralObject.
    pub fn add_handler(&mut self, obj_id: u32, sel: u32, handler: Value) {
        self.mutate(obj_id, |obj| {
            if let HeapObject::GeneralObject { handlers, .. } = obj {
                if let Some(entry) = handlers.iter_mut().find(|(k, _)| *k == sel) {
                    entry.1 = handler;
                } else {
                    handlers.push((sel, handler));
                }
            }
        });
    }

    /// Set or add a slot on a GeneralObject.
    pub fn set_slot(&mut self, obj_id: u32, sym: u32, val: Value) {
        self.mutate(obj_id, |obj| {
            if let HeapObject::GeneralObject { slots, .. } = obj {
                if let Some(entry) = slots.iter_mut().find(|(k, _)| *k == sym) {
                    entry.1 = val;
                } else {
                    slots.push((sym, val));
                }
            }
        });
    }

    // ── Convenience constructors ──

    pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
        Value::Object(self.alloc(HeapObject::Cons { car, cdr }))
    }

    pub fn alloc_string(&mut self, s: &str) -> Value {
        Value::Object(self.alloc(HeapObject::MoofString(s.to_string())))
    }

    pub fn list(&mut self, vals: &[Value]) -> Value {
        let mut result = Value::Nil;
        for &v in vals.iter().rev() {
            result = self.cons(v, result);
        }
        result
    }

    pub fn alloc_env(&mut self, parent: Option<u32>) -> u32 {
        self.alloc(HeapObject::Environment(Environment::new(parent)))
    }

    pub fn alloc_chunk(&mut self, chunk: BytecodeChunk) -> u32 {
        self.alloc(HeapObject::BytecodeChunk(chunk))
    }

    pub fn list_to_vec(&self, mut val: Value) -> Vec<Value> {
        let mut result = Vec::new();
        loop {
            match val {
                Value::Nil => return result,
                Value::Object(id) => {
                    match self.get(id) {
                        HeapObject::Cons { car, cdr } => {
                            result.push(*car);
                            val = *cdr;
                        }
                        _ => {
                            result.push(val);
                            return result;
                        }
                    }
                }
                other => {
                    result.push(other);
                    return result;
                }
            }
        }
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
}
