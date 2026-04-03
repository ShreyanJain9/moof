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

    /// Get a mutable reference to a heap object.
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

    // ── Convenience constructors ──

    /// Allocate a cons cell.
    pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
        Value::Object(self.alloc(HeapObject::Cons { car, cdr }))
    }

    /// Allocate a string.
    pub fn alloc_string(&mut self, s: &str) -> Value {
        Value::Object(self.alloc(HeapObject::MoofString(s.to_string())))
    }

    /// Build a proper list from a slice of values. (a b c) → cons(a, cons(b, cons(c, nil)))
    pub fn list(&mut self, vals: &[Value]) -> Value {
        let mut result = Value::Nil;
        for &v in vals.iter().rev() {
            result = self.cons(v, result);
        }
        result
    }

    /// Allocate a new empty environment with optional parent.
    pub fn alloc_env(&mut self, parent: Option<u32>) -> u32 {
        self.alloc(HeapObject::Environment(Environment::new(parent)))
    }

    /// Allocate a bytecode chunk.
    pub fn alloc_chunk(&mut self, chunk: BytecodeChunk) -> u32 {
        self.alloc(HeapObject::BytecodeChunk(chunk))
    }

    /// Walk a cons-list and collect values into a Vec.
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
                            // dotted pair tail — push it and stop
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

    /// Get the car of a cons cell.
    pub fn car(&self, val: Value) -> Value {
        match val {
            Value::Object(id) => match self.get(id) {
                HeapObject::Cons { car, .. } => *car,
                _ => Value::Nil,
            },
            _ => Value::Nil,
        }
    }

    /// Get the cdr of a cons cell.
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
