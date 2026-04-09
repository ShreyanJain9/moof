// send(receiver, selector, args) → result
//
// The one operation. The heart of the runtime.
//
// 1. Look in receiver's handler table (or type prototype for primitives)
// 2. Walk the parent chain (delegation, depth limit 256)
// 3. If found: execute (bytecode → VM, native → rust closure)
// 4. If not found: send doesNotUnderstand: to receiver

use crate::heap::Heap;
use crate::object::HeapObject;
use crate::value::Value;

const MAX_DELEGATION_DEPTH: usize = 256;

/// Perform message dispatch. This is called by the VM for every SEND instruction.
///
/// Returns the handler value and the object ID where it was found, or an error.
/// The VM is responsible for actually executing the handler (bytecode or native).
pub fn lookup_handler(heap: &Heap, receiver: Value, selector: u32) -> Result<(Value, Value), String> {
    // 1. if receiver is a heap object, check its handler table + delegation chain
    if let Some(id) = receiver.as_any_object() {
        if let Some(handler) = lookup_in_chain(heap, id, selector)? {
            return Ok((handler, receiver));
        }
    }

    // 2. look in the type prototype
    let tag = receiver.type_tag() as usize;
    let proto = if let Some(id) = receiver.as_any_object() {
        // for heap objects, also check variant-specific protos
        let variant_proto = match heap.get(id) {
            HeapObject::Pair(_, _) => heap.type_protos.get(6),
            HeapObject::Text(_) => heap.type_protos.get(7),
            HeapObject::Buffer(_) => heap.type_protos.get(8),
            HeapObject::Table { .. } => heap.type_protos.get(9),
            HeapObject::Closure { .. } => heap.type_protos.get(11),
            HeapObject::General { .. } => None,
        };
        // try variant proto first, then generic object proto
        if let Some(Some(vp)) = variant_proto.map(|p| p.as_any_object()) {
            if let Some(handler) = lookup_in_chain(heap, vp, selector)? {
                return Ok((handler, receiver));
            }
        }
        heap.type_protos.get(tag).copied().unwrap_or(Value::NIL)
    } else {
        heap.type_protos.get(tag).copied().unwrap_or(Value::NIL)
    };

    if let Some(proto_id) = proto.as_any_object() {
        if let Some(handler) = lookup_in_chain(heap, proto_id, selector)? {
            return Ok((handler, receiver));
        }
    }

    // 3. not found — return error (caller handles doesNotUnderstand: dispatch)
    let sel_name = heap.symbol_name(selector);
    Err(format!("{} does not understand '{}'", heap.format_value(receiver), sel_name))
}

/// Walk the delegation chain looking for a handler.
fn lookup_in_chain(heap: &Heap, start_id: u32, selector: u32) -> Result<Option<Value>, String> {
    let mut current_id = start_id;
    for _ in 0..MAX_DELEGATION_DEPTH {
        let obj = heap.get(current_id);

        // check this object's handlers
        if let Some(handler) = obj.handler_get(selector) {
            return Ok(Some(handler));
        }

        // walk to parent
        let parent = obj.parent();
        match parent.as_any_object() {
            Some(pid) => current_id = pid,
            None => return Ok(None),
        }
    }
    Err("delegation chain too deep (>256)".into())
}

/// Check if a value is a native handler (a symbol pointing to a registered native).
pub fn is_native(heap: &Heap, handler: Value) -> bool {
    if let Some(sym) = handler.as_symbol() {
        heap.find_native(sym).is_some()
    } else {
        false
    }
}

/// Call a native handler. The handler value must be a symbol registered in heap.natives.
pub fn call_native(heap: &mut Heap, handler: Value, receiver: Value, args: &[Value]) -> Result<Value, String> {
    let sym = handler.as_symbol().ok_or("not a native handler")?;
    let idx = heap.find_native(sym).ok_or_else(|| {
        format!("native '{}' not found", heap.symbol_name(sym))
    })?;

    // we need to pull the closure out to avoid borrow issues
    // safety: we're taking a reference to the boxed closure, not moving it
    let native = &heap.natives[idx].1 as *const crate::heap::NativeFn;
    unsafe { (*native)(heap, receiver, args) }
}
