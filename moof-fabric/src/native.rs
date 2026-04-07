/// Built-in native handler invoker.
///
/// Handlers are Objects with a "native-name" slot (a String).
/// The NativeInvoker looks up the name in a registry of Rust closures.
/// This is how type prototypes (Integer +, String length, etc.) work.

use std::collections::HashMap;
use crate::value::{Value, HeapObject};
use crate::heap::Heap;
use crate::dispatch::{HandlerInvoker, InvokeContext};

pub type NativeFn = Box<dyn Fn(&mut Heap, &[Value]) -> Result<Value, String> + Send>;

/// Registry of native functions + their invoker.
pub struct NativeInvoker {
    funcs: HashMap<String, NativeFn>,
    /// Symbol id for "native-name" (cached after first use)
    name_sym: Option<u32>,
}

impl NativeInvoker {
    pub fn new() -> Self {
        NativeInvoker {
            funcs: HashMap::new(),
            name_sym: None,
        }
    }

    /// Register a native function by name.
    pub fn register(&mut self, name: impl Into<String>, f: NativeFn) {
        self.funcs.insert(name.into(), f);
    }

    /// Create a handler object in the heap for a native function.
    /// Returns the object id.
    pub fn make_handler(heap: &mut Heap, name: &str) -> u32 {
        let name_sym = heap.intern("native-name");
        let name_val = heap.alloc_string(name);
        let obj = heap.alloc(HeapObject::Object {
            parent: Value::Nil,
            slots: vec![(name_sym, name_val)],
            handlers: Vec::new(),
        });
        obj
    }
}

impl HandlerInvoker for NativeInvoker {
    fn can_invoke(&self, heap: &Heap, handler: Value) -> bool {
        if let Value::Object(id) = handler {
            if let HeapObject::Object { slots, .. } = heap.get(id) {
                if let Some(sym) = heap.symbol_lookup_only("native-name") {
                    return slots.iter().any(|(k, _)| *k == sym);
                }
            }
        }
        false
    }

    fn invoke(
        &self,
        ctx: &mut InvokeContext,
        handler: Value,
        _receiver: Value,
        args: &[Value],
    ) -> Result<Value, String> {
        let handler_id = handler.as_object().ok_or("native handler must be object")?;

        // Read the native name
        let name_sym = ctx.heap.intern("native-name");
        let name_val = ctx.heap.slot_get(handler_id, name_sym);
        let name = match name_val {
            Value::Object(id) => match ctx.heap.get(id) {
                HeapObject::String(s) => s.clone(),
                _ => return Err("native handler: name is not a string".into()),
            },
            _ => return Err("native handler: missing native-name slot".into()),
        };

        match self.funcs.get(&name) {
            Some(f) => f(ctx.heap, args),
            None => Err(format!("native function '{}' not registered", name)),
        }
    }
}
