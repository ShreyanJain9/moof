/// moof-fabric: the objectspace kernel.
///
/// Objects, messaging, scheduling, persistence.
/// No language. No syntax. No compiler.
///
/// The fabric is a persistent, shared heap of objects that respond to messages.
/// Shells (languages) plug in by registering HandlerInvokers.
/// Extensions plug in for I/O.

pub mod value;
pub mod heap;
pub mod dispatch;
pub mod vat;
pub mod native;

pub use value::{Value, HeapObject};
pub use heap::Heap;
pub use dispatch::{HandlerInvoker, InvokeContext, TypeProtos, lookup_handler};
pub use vat::{Vat, VatId, VatStatus, Message, Scheduler};
pub use native::{NativeInvoker, NativeFn};

/// The fabric — a running objectspace.
pub struct Fabric {
    pub heap: Heap,
    pub type_protos: TypeProtos,
    pub invokers: Vec<Box<dyn HandlerInvoker>>,
    pub scheduler: Scheduler,
    sym_dnu: u32,
    sym_call: u32,
}

impl Fabric {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let sym_dnu = heap.intern("doesNotUnderstand:");
        let sym_call = heap.intern("call:");
        Fabric {
            heap,
            type_protos: TypeProtos::default(),
            invokers: Vec::new(),
            scheduler: Scheduler::new(),
            sym_dnu,
            sym_call,
        }
    }

    // ── The one operation ──

    pub fn send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        dispatch::send(
            &mut self.heap,
            &self.type_protos,
            &self.invokers,
            self.sym_dnu,
            receiver,
            selector,
            args,
        )
    }

    // ── Object creation ──

    pub fn create_object(&mut self, parent: Value) -> u32 {
        self.heap.alloc_object(parent)
    }

    pub fn set_slot(&mut self, obj: u32, name: &str, val: Value) {
        let sym = self.heap.intern(name);
        self.heap.slot_set(obj, sym, val);
    }

    pub fn get_slot(&self, obj: u32, name: &str) -> Value {
        if let Some(sym) = self.heap.symbol_lookup_only(name) {
            self.heap.slot_get(obj, sym)
        } else {
            Value::Nil
        }
    }

    pub fn add_handler(&mut self, obj: u32, selector: &str, handler: Value) {
        let sym = self.heap.intern(selector);
        self.heap.add_handler(obj, sym, handler);
    }

    /// Register a native function and create a handler object for it.
    /// Returns the handler Value (an Object with a native-name slot).
    pub fn register_native(&mut self, name: &str, f: NativeFn) -> Value {
        // Register the closure in the first NativeInvoker we find
        // (or create one if none exists)
        let handler_id = NativeInvoker::make_handler(&mut self.heap, name);
        Value::Object(handler_id)
    }

    /// Add a native handler to an object.
    pub fn add_native_handler(&mut self, obj: u32, selector: &str, native_name: &str) {
        let handler_id = NativeInvoker::make_handler(&mut self.heap, native_name);
        let sym = self.heap.intern(selector);
        self.heap.add_handler(obj, sym, Value::Object(handler_id));
    }

    // ── Symbols ──

    pub fn intern(&mut self, name: &str) -> u32 { self.heap.intern(name) }
    pub fn symbol_name(&self, id: u32) -> &str { self.heap.symbol_name(id) }
    pub fn sym_call(&self) -> u32 { self.sym_call }
    pub fn sym_dnu(&self) -> u32 { self.sym_dnu }

    // ── Shell registration ──

    pub fn register_invoker(&mut self, invoker: Box<dyn HandlerInvoker>) {
        self.invokers.push(invoker);
    }

    // ── Vats ──

    pub fn create_vat(&mut self) -> VatId {
        self.scheduler.create_vat()
    }

    pub fn enqueue_message(&mut self, vat_id: VatId, msg: Message) -> bool {
        self.scheduler.enqueue(vat_id, msg)
    }

    /// Run one scheduler tick: deliver one message per ready vat.
    pub fn tick(&mut self) {
        let deliveries = self.scheduler.collect_ready_messages();
        for (vat_id, msg) in deliveries {
            let result = self.send(
                Value::Object(msg.receiver),
                msg.selector,
                &msg.args,
            );

            match result {
                Ok(val) => {
                    if let Some(resolver_id) = msg.resolver {
                        let val_sym = self.heap.intern("value");
                        let resolved_sym = self.heap.intern("resolved");
                        self.heap.slot_set(resolver_id, val_sym, val);
                        self.heap.slot_set(resolver_id, resolved_sym, Value::True);
                    }
                }
                Err(e) => {
                    self.scheduler.set_status(vat_id, VatStatus::Suspended { error: e });
                }
            }
        }
    }

    // ── Convenience ──

    pub fn alloc_string(&mut self, s: &str) -> Value { self.heap.alloc_string(s) }
    pub fn cons(&mut self, car: Value, cdr: Value) -> Value { self.heap.cons(car, cdr) }
    pub fn list(&mut self, vals: &[Value]) -> Value { self.heap.list(vals) }
}
