//! Message dispatch: the only operation in the fabric.
//!
//! send(receiver, selector, args) →
//!   1. if receiver is an object: look in its handler table
//!   2. if not found: walk the parent chain (delegation)
//!   3. if still not found: look in the type prototype for receiver's type
//!   4. if still not found: fire doesNotUnderstand: on the receiver
//!   5. when a handler is found: ask registered HandlerInvokers to execute it

use crate::store::Store;
use crate::value::Value;

/// Context passed to handler invokers during dispatch.
pub struct InvokeContext<'a> {
    pub store: &'a mut Store,
    pub invokers: &'a [Box<dyn HandlerInvoker>],
    /// Type prototype table: maps type tag → object ID of the prototype.
    pub type_protos: &'a [Option<u32>; 8],
}

/// Trait for pluggable handler execution.
/// The fabric doesn't know how to run handlers — invokers do.
pub trait HandlerInvoker: Send {
    /// Can this invoker handle this handler value?
    fn can_invoke(&self, store: &Store, handler: Value) -> bool;

    /// Invoke the handler. Returns the result value.
    fn invoke(
        &self,
        ctx: &mut InvokeContext,
        handler: Value,
        receiver: Value,
        args: &[Value],
    ) -> Result<Value, String>;
}

/// The native invoker: handles handlers that are symbol references to
/// registered Rust closures.
pub struct NativeInvoker {
    /// (name_symbol_id, closure)
    registry: Vec<(u32, Box<dyn Fn(&mut InvokeContext, Value, &[Value]) -> Result<Value, String> + Send>)>,
}

impl NativeInvoker {
    pub fn new() -> Self {
        NativeInvoker {
            registry: Vec::new(),
        }
    }

    /// Register a native handler. Returns a symbol value that can be used as a handler.
    pub fn register(
        &mut self,
        name_sym: u32,
        f: impl Fn(&mut InvokeContext, Value, &[Value]) -> Result<Value, String> + Send + 'static,
    ) -> Value {
        self.registry.push((name_sym, Box::new(f)));
        Value::symbol(name_sym)
    }

    /// Look up and call a native by symbol ID.
    fn call(
        &self,
        name_sym: u32,
        ctx: &mut InvokeContext,
        receiver: Value,
        args: &[Value],
    ) -> Result<Value, String> {
        for (id, f) in &self.registry {
            if *id == name_sym {
                return f(ctx, receiver, args);
            }
        }
        Err(format!("no native handler for symbol {name_sym}"))
    }
}

impl HandlerInvoker for NativeInvoker {
    fn can_invoke(&self, _store: &Store, handler: Value) -> bool {
        if let Some(sym) = handler.as_symbol() {
            self.registry.iter().any(|(id, _)| *id == sym)
        } else {
            false
        }
    }

    fn invoke(
        &self,
        ctx: &mut InvokeContext,
        handler: Value,
        receiver: Value,
        args: &[Value],
    ) -> Result<Value, String> {
        let sym = handler
            .as_symbol()
            .ok_or_else(|| "native invoker: handler is not a symbol".to_string())?;
        self.call(sym, ctx, receiver, args)
    }
}

/// Perform message dispatch: the heart of the fabric.
///
/// Type tags for prototype lookup:
///   0=nil, 1=true, 2=false, 3=int, 4=sym, 5=obj
pub fn send(
    store: &mut Store,
    invokers: &[Box<dyn HandlerInvoker>],
    type_protos: &[Option<u32>; 8],
    receiver: Value,
    selector: u32,
    args: &[Value],
) -> Result<Value, String> {
    // 1. if receiver is an object, check its handler table + delegation chain
    if let Some(obj_id) = receiver.as_object() {
        if let Some(result) = lookup_handler_chain(store, obj_id, selector)? {
            let mut ctx = InvokeContext {
                store,
                invokers,
                type_protos,
            };
            return invoke_handler(&mut ctx, result, receiver, args);
        }
    }

    // 2. look in the type prototype for this value's type
    let type_tag = type_tag_of(receiver);
    if let Some(Some(proto_id)) = type_protos.get(type_tag as usize) {
        if let Some(handler) = lookup_handler_chain(store, *proto_id, selector)? {
            let mut ctx = InvokeContext {
                store,
                invokers,
                type_protos,
            };
            return invoke_handler(&mut ctx, handler, receiver, args);
        }
    }

    // 3. doesNotUnderstand:
    // for now, just error. a real implementation would send doesNotUnderstand:
    // to the receiver with the selector and args.
    let sel_name = store
        .symbol_name(selector)
        .unwrap_or_else(|_| format!("#{selector}"));
    Err(format!("{receiver:?} does not understand '{sel_name}'"))
}

/// Walk an object's handler table, then its parent chain.
fn lookup_handler_chain(
    store: &Store,
    obj_id: u32,
    selector: u32,
) -> Result<Option<Value>, String> {
    let mut current = obj_id;
    for _ in 0..256 {
        // depth limit to prevent infinite loops
        if let Some(handler) = store.handler_get(current, selector)? {
            return Ok(Some(handler));
        }
        let parent = store.parent(current)?;
        match parent.as_object() {
            Some(pid) => current = pid,
            None => return Ok(None),
        }
    }
    Err("delegation chain too deep (>256)".into())
}

/// Find the right invoker for a handler value and call it.
fn invoke_handler(
    ctx: &mut InvokeContext,
    handler: Value,
    receiver: Value,
    args: &[Value],
) -> Result<Value, String> {
    for invoker in ctx.invokers.iter() {
        if invoker.can_invoke(ctx.store, handler) {
            return invoker.invoke(ctx, handler, receiver, args);
        }
    }
    Err(format!(
        "no invoker can handle handler value {handler:?}"
    ))
}

/// Map a value to its type tag index (for type_protos lookup).
fn type_tag_of(v: Value) -> u8 {
    if v.is_nil() {
        0
    } else if v.is_bool() {
        1
    } else if v.is_integer() {
        2
    } else if v.is_float() {
        3
    } else if v.is_symbol() {
        4
    } else if v.is_object() {
        5
    } else {
        7 // shouldn't happen
    }
}
