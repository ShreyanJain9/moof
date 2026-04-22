// send(receiver, selector, args) → result
//
// The one operation. The heart of the runtime.
//
// 1. Look in receiver's handler table (or type prototype for primitives)
// 2. Walk the parent chain (delegation, depth limit 256)
// 3. If found: execute (bytecode → VM, native → rust closure)
// 4. If not found: send doesNotUnderstand: to receiver

use crate::heap::Heap;
use crate::value::Value;

const MAX_DELEGATION_DEPTH: usize = 256;

/// Perform message dispatch. This is called by the VM for every SEND.
///
/// Returns (handler_value, receiver). The VM is responsible for actually
/// executing the handler (bytecode or native).
///
/// Uses a monomorphic inline-ish cache keyed by (prototype, selector).
/// Most sends find their handler on a type prototype (e.g. Cons#each:
/// lives on the Cons proto, hit by every list operation), so caching
/// lookup_in_chain results keyed by the starting proto handles the
/// bulk of the load. Instance-local handlers are handled on a fast
/// path before the cache is consulted — they're rare and we don't
/// want to pollute the cache with per-instance keys.
pub fn lookup_handler(heap: &mut Heap, receiver: Value, selector: u32) -> Result<(Value, Value), String> {
    // Fast path: instance has its own handler installed directly (object
    // literals with [sel] handlers, closures, etc.). Never cached — each
    // instance would be its own key.
    if let Some(id) = receiver.as_any_object() {
        if let Some(handler) = heap.get(id).handler_get(selector) {
            return Ok((handler, receiver));
        }
    }

    // Determine the prototype we'd walk. For heap variants this is the
    // type-prototype (Cons/String/Table/...). For primitives it's
    // type_protos[tag]. For general objects it's their parent slot.
    let proto = heap.prototype_of(receiver);
    if let Some(proto_id) = proto.as_any_object() {
        // Cache check. (proto_id, selector) → handler.
        if let Some(&cached) = heap.send_cache.get(&(proto_id, selector)) {
            return Ok((cached, receiver));
        }
        // Miss: walk from the proto.
        if let Some(handler) = lookup_in_chain(heap, proto_id, selector)? {
            heap.send_cache.insert((proto_id, selector), handler);
            return Ok((handler, receiver));
        }
    }

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

        // walk the prototype chain (VM-internal)
        let proto = obj.proto();
        match proto.as_any_object() {
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
