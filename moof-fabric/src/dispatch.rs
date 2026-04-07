/// Message dispatch — the single privileged operation.
///
/// send(receiver, selector, args) → result
///
/// Dispatch order:
/// 1. Handler on the receiver itself (Object handler table + delegation chain)
/// 2. Handler on the type prototype (for immediates and non-Object heap types)
/// 3. Universal introspection (handlerNames, parent, handlerAt:, slotAt:, slotNames)
/// 4. doesNotUnderstand:
///
/// Handler invocation goes through the HandlerInvoker trait — the fabric
/// doesn't know how to execute handlers. Shells register invokers.

use crate::value::{Value, HeapObject};
use crate::heap::Heap;

/// Context passed to handler invokers. Provides access to the heap and
/// re-entrant message sending.
pub struct InvokeContext<'a> {
    pub heap: &'a mut Heap,
    pub type_protos: &'a TypeProtos,
    pub invokers: &'a [Box<dyn HandlerInvoker>],
    pub sym_does_not_understand: u32,
}

/// Trait for handler execution. Each shell registers one.
/// The native invoker (Rust closures) is always present.
/// The moof shell adds a bytecode invoker.
pub trait HandlerInvoker: Send {
    /// Can this invoker handle this handler value?
    fn can_invoke(&self, heap: &Heap, handler: Value) -> bool;

    /// Invoke the handler. receiver is args[0] by convention.
    fn invoke(
        &self,
        ctx: &mut InvokeContext,
        handler: Value,
        receiver: Value,
        args: &[Value],
    ) -> Result<Value, String>;
}

/// Type prototype registry. Maps value kinds to prototype object ids.
#[derive(Default)]
pub struct TypeProtos {
    pub integer: Option<u32>,
    pub float: Option<u32>,
    pub boolean: Option<u32>,
    pub string: Option<u32>,
    pub cons: Option<u32>,
    pub nil: Option<u32>,
    pub symbol: Option<u32>,
    pub bytes: Option<u32>,
}

impl TypeProtos {
    pub fn for_value(&self, heap: &Heap, val: Value) -> Option<u32> {
        match val {
            Value::Integer(_) => self.integer,
            Value::Float(_) => self.float.or(self.integer),
            Value::True | Value::False => self.boolean,
            Value::Nil => self.nil,
            Value::Symbol(_) => self.symbol,
            Value::Object(id) => match heap.get(id) {
                HeapObject::String(_) => self.string,
                HeapObject::Cons { .. } => self.cons,
                HeapObject::Bytes(_) => self.bytes,
                HeapObject::Object { .. } => None, // Objects use their own handler table
            },
        }
    }
}

/// Look up a handler in the delegation chain.
pub fn lookup_handler(heap: &Heap, obj_id: u32, selector: u32) -> Option<Value> {
    let mut current = Some(obj_id);
    while let Some(id) = current {
        match heap.get(id) {
            HeapObject::Object { parent, handlers, .. } => {
                for &(sel, handler) in handlers {
                    if sel == selector {
                        return Some(handler);
                    }
                }
                match parent {
                    Value::Object(pid) => current = Some(*pid),
                    _ => current = None,
                }
            }
            _ => return None,
        }
    }
    None
}

/// The core message send.
///
/// "the vm's single privileged operation is send. everything reduces to it."
pub fn send(
    heap: &mut Heap,
    type_protos: &TypeProtos,
    invokers: &[Box<dyn HandlerInvoker>],
    sym_dnu: u32,
    receiver: Value,
    selector: u32,
    args: &[Value],
) -> Result<Value, String> {
    // 1. Object handler lookup (delegation chain)
    if let Value::Object(id) = receiver {
        if let HeapObject::Object { .. } = heap.get(id) {
            if let Some(handler) = lookup_handler(heap, id, selector) {
                return invoke_handler(heap, type_protos, invokers, sym_dnu, handler, receiver, args);
            }
        }
    }

    // 2. Type prototype handler lookup
    if let Some(proto_id) = type_protos.for_value(heap, receiver) {
        if let Some(handler) = lookup_handler(heap, proto_id, selector) {
            return invoke_handler(heap, type_protos, invokers, sym_dnu, handler, receiver, args);
        }
    }

    // 3. Universal introspection protocol
    let sel_name = heap.symbol_name(selector).to_string();
    match sel_name.as_str() {
        "handlerNames" => {
            if let Value::Object(id) = receiver {
                let names: Vec<Value> = heap.handler_names(id).into_iter()
                    .map(Value::Symbol).collect();
                return Ok(heap.list(&names));
            }
            if let Some(proto_id) = type_protos.for_value(heap, receiver) {
                let names: Vec<Value> = heap.handler_names(proto_id).into_iter()
                    .map(Value::Symbol).collect();
                return Ok(heap.list(&names));
            }
            return Ok(Value::Nil);
        }
        "parent" => {
            if let Value::Object(id) = receiver {
                return Ok(heap.parent(id));
            }
            if let Some(proto_id) = type_protos.for_value(heap, receiver) {
                return Ok(Value::Object(proto_id));
            }
            return Ok(Value::Nil);
        }
        "handlerAt:" => {
            let key = args.first().and_then(|v| v.as_symbol())
                .ok_or("handlerAt: expects a symbol")?;
            if let Value::Object(id) = receiver {
                if let Some(h) = lookup_handler(heap, id, key) {
                    return Ok(h);
                }
            }
            if let Some(proto_id) = type_protos.for_value(heap, receiver) {
                if let Some(h) = lookup_handler(heap, proto_id, key) {
                    return Ok(h);
                }
            }
            return Ok(Value::Nil);
        }
        "slotAt:" => {
            let key = args.first().and_then(|v| v.as_symbol())
                .ok_or("slotAt: expects a symbol")?;
            if let Value::Object(id) = receiver {
                return Ok(heap.slot_get(id, key));
            }
            return Ok(Value::Nil);
        }
        "slotAt:put:" => {
            let key = args.first().and_then(|v| v.as_symbol())
                .ok_or("slotAt:put: expects a symbol")?;
            let val = args.get(1).copied().unwrap_or(Value::Nil);
            if let Value::Object(id) = receiver {
                heap.slot_set(id, key, val);
                return Ok(val);
            }
            return Err("slotAt:put: receiver must be an object".into());
        }
        "slotNames" => {
            if let Value::Object(id) = receiver {
                let names: Vec<Value> = heap.slot_names(id).into_iter()
                    .map(Value::Symbol).collect();
                return Ok(heap.list(&names));
            }
            return Ok(Value::Nil);
        }
        _ => {}
    }

    // 4. doesNotUnderstand:
    if selector != sym_dnu {
        if let Value::Object(id) = receiver {
            if let Some(dnu_handler) = lookup_handler(heap, id, sym_dnu) {
                let sel_sym = Value::Symbol(selector);
                let args_list = heap.list(args);
                return invoke_handler(
                    heap, type_protos, invokers, sym_dnu,
                    dnu_handler, receiver, &[sel_sym, args_list],
                );
            }
        }
        if let Some(proto_id) = type_protos.for_value(heap, receiver) {
            if let Some(dnu_handler) = lookup_handler(heap, proto_id, sym_dnu) {
                let sel_sym = Value::Symbol(selector);
                let args_list = heap.list(args);
                return invoke_handler(
                    heap, type_protos, invokers, sym_dnu,
                    dnu_handler, receiver, &[sel_sym, args_list],
                );
            }
        }
    }

    Err(format!("doesNotUnderstand: {} on {:?}", sel_name, receiver))
}

/// Invoke a handler through the registered invokers.
fn invoke_handler(
    heap: &mut Heap,
    type_protos: &TypeProtos,
    invokers: &[Box<dyn HandlerInvoker>],
    sym_dnu: u32,
    handler: Value,
    receiver: Value,
    args: &[Value],
) -> Result<Value, String> {
    // Build full args: [receiver, ...args]
    let mut full_args = Vec::with_capacity(args.len() + 1);
    full_args.push(receiver);
    full_args.extend_from_slice(args);

    // Find an invoker that can handle this handler
    // We need to check without holding a mutable borrow on heap
    let invoker_idx = {
        let heap_ref: &Heap = heap;
        invokers.iter().position(|inv| inv.can_invoke(heap_ref, handler))
    };

    if let Some(idx) = invoker_idx {
        let mut ctx = InvokeContext { heap, type_protos, invokers, sym_does_not_understand: sym_dnu };
        invokers[idx].invoke(&mut ctx, handler, receiver, &full_args)
    } else {
        Err(format!("No invoker registered for handler {:?}", handler))
    }
}
