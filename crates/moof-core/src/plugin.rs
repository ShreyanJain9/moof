// Plugin trait + handler-registration helpers.
//
// A `Plugin` is a native module that registers type prototypes
// and handlers on a `Heap`. Every vat in a moof process gets the
// same set of type plugins via `Heap::register_foreign_type` +
// `Plugin::register` being called once per vat's heap at init.
//
// This is the minimum external type-plugin authors need — the
// counterpart `CapabilityPlugin` (capability vats, which own
// mutable state and need a `Vat`) lives in moof-runtime since
// it depends on the VM / scheduler layer.

use crate::heap::{Heap, PROTO_OBJ};
use crate::value::Value;
use crate::foreign::ForeignType;

/// A native module that extends every vat's objectspace. Registers
/// foreign types + type prototypes + handlers on a Heap.
pub trait Plugin {
    fn name(&self) -> &str;
    fn register(&self, heap: &mut Heap);
}

/// One-step proto registration for a `ForeignType`.
///
/// Does three things:
///   1. Registers `T` in the heap's foreign-type registry.
///   2. Creates a plain prototype object under the root Object proto.
///   3. Auto-installs a `typeName` handler on that proto returning
///      `Value::symbol(T::prototype_name())`, so
///      `[myfoo typeName]` returns the correct symbol for free
///      (without plugin authors having to remember).
///   4. Binds the prototype in the root env under `T::prototype_name()`.
///
/// Returns the prototype `Value`. Plugin bodies typically call this
/// once, then install instance handlers on the returned proto.
pub fn register_foreign_proto<T: ForeignType>(heap: &mut Heap) -> Value {
    heap.register_foreign_type::<T>().expect("register foreign type");

    let object_proto = heap.type_protos[PROTO_OBJ];
    let proto = heap.make_object(object_proto);
    let proto_id = proto.as_any_object().expect("proto is an object");

    // typeName — returns the type's prototype name as a symbol.
    let type_sym = heap.intern(T::prototype_name());
    native(heap, proto_id, "typeName", move |_, _, _| Ok(Value::symbol(type_sym)));

    // bind `<TypeName> = proto` in the root env for moof-level lookup.
    let bind_sym = heap.intern(T::prototype_name());
    heap.env_def(bind_sym, proto);

    proto
}

/// Register a native handler on a prototype by selector name.
///
/// Handler bodies receive `(&mut Heap, receiver, &[args])` and
/// return `Result<Value, String>`. The closure is stored in the
/// heap's native table and bound as the proto's `selector` handler.
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

/// Register an integer binary op: extracts `i64` from receiver and
/// first arg, invokes `f`, wraps result in a `Value`.
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

/// Deterministic FNV-1a 64-bit hash. Used by the String/Bytes
/// content-hash implementations — deterministic (no randomization)
/// so content addressing is stable across processes and images.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
