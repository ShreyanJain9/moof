use moof_fabric::*;
use moof_fabric::dispatch::InvokeContext;

/// A native invoker: handlers are Object ids pointing to Objects with
/// a "native-name" slot. The invoker looks up a closure by name.
struct NativeInvoker {
    funcs: std::collections::HashMap<String, Box<dyn Fn(&mut Heap, &[Value]) -> Result<Value, String> + Send>>,
}

impl NativeInvoker {
    fn new() -> Self {
        NativeInvoker { funcs: std::collections::HashMap::new() }
    }

    fn register(&mut self, name: &str, f: impl Fn(&mut Heap, &[Value]) -> Result<Value, String> + Send + 'static) {
        self.funcs.insert(name.to_string(), Box::new(f));
    }
}

impl HandlerInvoker for NativeInvoker {
    fn can_invoke(&self, heap: &Heap, handler: Value) -> bool {
        if let Value::Object(id) = handler {
            if let HeapObject::Object { slots, .. } = heap.get(id) {
                // Check for a "native-name" slot with a string value
                if let Some(sym) = heap.symbol_lookup_only("native-name") {
                    return slots.iter().any(|(k, _)| *k == sym);
                }
            }
        }
        false
    }

    fn invoke(&self, ctx: &mut InvokeContext, handler: Value, _receiver: Value, args: &[Value]) -> Result<Value, String> {
        let handler_id = handler.as_object().ok_or("native handler must be object")?;
        let name_sym = ctx.heap.intern("native-name");
        let name_val = ctx.heap.slot_get(handler_id, name_sym);
        let name = match name_val {
            Value::Object(id) => match ctx.heap.get(id) {
                HeapObject::String(s) => s.clone(),
                _ => return Err("native handler missing name".into()),
            },
            _ => return Err("native handler missing name".into()),
        };
        match self.funcs.get(&name) {
            Some(f) => f(ctx.heap, args),
            None => Err(format!("native function '{}' not found", name)),
        }
    }
}

#[test]
fn test_create_and_read_slots() {
    let mut fabric = Fabric::new();

    let obj = fabric.create_object(Value::Nil);
    fabric.set_slot(obj, "x", Value::Integer(42));
    fabric.set_slot(obj, "y", Value::Integer(7));

    assert_eq!(fabric.get_slot(obj, "x"), Value::Integer(42));
    assert_eq!(fabric.get_slot(obj, "y"), Value::Integer(7));
    assert_eq!(fabric.get_slot(obj, "z"), Value::Nil);
}

#[test]
fn test_send_with_native_invoker() {
    let mut fabric = Fabric::new();

    // Create a native invoker with a "double" function
    let mut invoker = NativeInvoker::new();
    invoker.register("double", |_heap, args| {
        // args[0] = self (receiver), args[1] = the actual argument
        let n = args.get(1).and_then(|v| v.as_integer()).ok_or("expected integer")?;
        Ok(Value::Integer(n * 2))
    });

    // Create a handler object (an Object with a native-name slot)
    let handler_id = fabric.create_object(Value::Nil);
    let name_val = fabric.alloc_string("double");
    fabric.set_slot(handler_id, "native-name", name_val);

    // Create a target object with the handler
    let obj = fabric.create_object(Value::Nil);
    fabric.add_handler(obj, "double", Value::Object(handler_id));

    // Register the invoker
    fabric.register_invoker(Box::new(invoker));

    // Send!
    let sel = fabric.intern("double");
    let result = fabric.send(Value::Object(obj), sel, &[Value::Integer(21)]);
    assert_eq!(result.unwrap(), Value::Integer(42));
}

#[test]
fn test_delegation() {
    let mut fabric = Fabric::new();

    // Parent with a handler
    let mut invoker = NativeInvoker::new();
    invoker.register("greet", |heap, _args| {
        Ok(heap.alloc_string("hello!"))
    });

    let handler_id = fabric.create_object(Value::Nil);
    let name_val = fabric.alloc_string("greet");
    fabric.set_slot(handler_id, "native-name", name_val);

    let parent = fabric.create_object(Value::Nil);
    fabric.add_handler(parent, "greet", Value::Object(handler_id));

    // Child delegates to parent
    let child = fabric.create_object(Value::Object(parent));

    fabric.register_invoker(Box::new(invoker));

    // Child finds parent's handler via delegation
    let sel = fabric.intern("greet");
    let result = fabric.send(Value::Object(child), sel, &[]);
    match result.unwrap() {
        Value::Object(id) => match fabric.heap.get(id) {
            HeapObject::String(s) => assert_eq!(s, "hello!"),
            _ => panic!("expected string"),
        },
        v => panic!("expected string object, got {:?}", v),
    }
}

#[test]
fn test_universal_introspection() {
    let mut fabric = Fabric::new();

    let obj = fabric.create_object(Value::Nil);
    fabric.set_slot(obj, "x", Value::Integer(10));
    fabric.set_slot(obj, "y", Value::Integer(20));

    // slotAt:
    let sym_x = fabric.intern("x");
    let sel_slot_at = fabric.intern("slotAt:");
    let sel_slot_names = fabric.intern("slotNames");
    let sel_parent = fabric.intern("parent");

    let result = fabric.send(Value::Object(obj), sel_slot_at, &[Value::Symbol(sym_x)]);
    assert_eq!(result.unwrap(), Value::Integer(10));

    // slotNames
    let result = fabric.send(Value::Object(obj), sel_slot_names, &[]);
    let names = fabric.heap.list_to_vec(result.unwrap());
    assert_eq!(names.len(), 2);

    // parent
    let result = fabric.send(Value::Object(obj), sel_parent, &[]);
    assert_eq!(result.unwrap(), Value::Nil);
}

#[test]
fn test_does_not_understand() {
    let mut fabric = Fabric::new();
    let obj = fabric.create_object(Value::Nil);

    let sel = fabric.intern("nonexistent");
    let result = fabric.send(Value::Object(obj), sel, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("doesNotUnderstand"));
}
