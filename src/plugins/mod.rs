// Plugin system: native modules that extend the objectspace.
//
// Two kinds of plugins:
//
// 1. Plugin — registers type prototypes and native handlers on a Heap.
//    Used for core language types (Integer, String, etc.). Every vat
//    gets these automatically.
//
// 2. CapabilityPlugin — creates a capability vat with native handlers.
//    Used for IO (Console, Clock, Store, etc.). Each capability is its
//    own vat. Sends to it go through FarRef → Act. The capability's
//    native code is the only thing that touches the outside world.
//
// External code adds plugins by creating a Runtime with custom plugin
// lists — no modification to moof source needed.

pub mod core;
pub mod numeric;
pub mod collections;
pub mod effects;
pub mod block;
pub mod capabilities;

use crate::heap::Heap;
use crate::object::HeapObject;
use crate::value::Value;
use crate::scheduler::Vat;

/// A native module that extends every vat's objectspace.
/// Registers type prototypes and handlers on a Heap.
pub trait Plugin {
    fn name(&self) -> &str;
    fn register(&self, heap: &mut Heap);
}

/// A native capability that lives in its own vat.
/// Creates a root object with native handlers. Sends to the
/// capability go through FarRef → outbox → scheduler → native
/// handler → Act resolution. All effects are mediated.
pub trait CapabilityPlugin {
    /// The name used to bind the FarRef in the REPL (e.g. "console").
    fn name(&self) -> &str;

    /// Set up native handlers on a root object in the given vat.
    /// Returns the root object ID (for FarRef creation).
    fn setup(&self, vat: &mut Vat) -> u32;
}

/// Register a native handler on a prototype.
pub fn native(
    heap: &mut Heap,
    proto_id: u32,
    selector: &str,
    f: impl Fn(&mut Heap, Value, &[Value]) -> Result<Value, String> + 'static,
) {
    let sym = heap.intern(selector);
    let h = heap.register_native(selector, f);
    heap.get_mut(proto_id).handler_set(sym, h);
}

/// Register an integer binary op.
pub fn int_binop(heap: &mut Heap, proto_id: u32, sel: &str, f: fn(i64, i64) -> Value) {
    let name = sel.to_string();
    native(heap, proto_id, sel, move |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or_else(|| format!("{name}: not int"))?;
        let b = args.first().and_then(|v| v.as_integer())
            .ok_or_else(|| format!("{name}: arg not int"))?;
        Ok(f(a, b))
    });
}

/// Register a float binary op.
pub fn float_binop(heap: &mut Heap, proto_id: u32, sel: &str, f: fn(f64, f64) -> Value) {
    let name = sel.to_string();
    native(heap, proto_id, sel, move |_heap, receiver, args| {
        let a = receiver.as_float().ok_or_else(|| format!("{name}: not float"))?;
        let b = args.first().and_then(|v| v.as_float())
            .ok_or_else(|| format!("{name}: arg not numeric"))?;
        Ok(f(a, b))
    });
}

/// Register a float unary op.
pub fn float_unary(heap: &mut Heap, proto_id: u32, sel: &str, f: fn(f64) -> Value) {
    let name = sel.to_string();
    native(heap, proto_id, sel, move |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or_else(|| format!("{name}: not float"))?;
        Ok(f(a))
    });
}

/// The default type plugins: registered on every vat's heap.
pub fn default_plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(core::CorePlugin),
        Box::new(numeric::NumericPlugin),
        Box::new(collections::CollectionsPlugin),
        Box::new(block::BlockPlugin),
        Box::new(effects::EffectsPlugin),
    ]
}

/// The default capability plugins: each becomes its own vat.
pub fn default_capabilities() -> Vec<Box<dyn CapabilityPlugin>> {
    vec![
        Box::new(capabilities::ConsoleCapability),
        Box::new(capabilities::ClockCapability),
    ]
}

/// Register all type plugins on a heap.
pub fn register_all(heap: &mut Heap) {
    for plugin in default_plugins() {
        plugin.register(heap);
    }
}
