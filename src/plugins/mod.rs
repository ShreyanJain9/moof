// Plugin system: native modules that extend the objectspace.
//
// A Plugin registers type prototypes and native handlers on a Heap.
// The runtime loads plugins in order — later plugins can reference
// prototypes created by earlier ones.

pub mod core;
pub mod numeric;
pub mod collections;
pub mod effects;
pub mod block;

use crate::heap::Heap;
use crate::object::HeapObject;
use crate::value::Value;

/// A native module that extends the objectspace.
pub trait Plugin {
    fn name(&self) -> &str;
    fn register(&self, heap: &mut Heap);
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

/// The default plugin set: everything needed for a working moof runtime.
pub fn default_plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(core::CorePlugin),
        Box::new(numeric::NumericPlugin),
        Box::new(collections::CollectionsPlugin),
        Box::new(block::BlockPlugin),
        Box::new(effects::EffectsPlugin),
    ]
}

/// Register all plugins on a heap.
pub fn register_all(heap: &mut Heap) {
    for plugin in default_plugins() {
        plugin.register(heap);
    }
}
