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
pub mod dynload;
pub mod json;
pub mod vec3;

use crate::heap::Heap;
use crate::value::Value;
use crate::vat::Vat;

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

/// Deterministic FNV-1a 64-bit hash. Used for String/Bytes content
/// hashing — deterministic (no randomization) so content addressing
/// is stable across processes and images.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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

/// Look up a built-in type plugin by name.
pub fn builtin_type_plugin(name: &str) -> Option<Box<dyn Plugin>> {
    match name {
        "core" => Some(Box::new(core::CorePlugin)),
        "numeric" => Some(Box::new(numeric::NumericPlugin)),
        "collections" => Some(Box::new(collections::CollectionsPlugin)),
        "block" => Some(Box::new(block::BlockPlugin)),
        "effects" => Some(Box::new(effects::EffectsPlugin)),
        "json" => Some(Box::new(json::JsonPlugin)),
        "vec3" => Some(Box::new(vec3::Vec3Plugin)),
        _ => None,
    }
}

/// Look up a built-in capability plugin by name.
pub fn builtin_capability(name: &str) -> Option<Box<dyn CapabilityPlugin>> {
    match name {
        "console" => Some(Box::new(capabilities::ConsoleCapability)),
        "clock"   => Some(Box::new(capabilities::ClockCapability)),
        "file"    => Some(Box::new(capabilities::FileCapability)),
        "random"  => Some(Box::new(capabilities::RandomCapability)),
        _ => None,
    }
}

/// Register type plugins on a heap based on manifest [types].
pub fn register_from_manifest(heap: &mut Heap, types: &std::collections::HashMap<String, String>) {
    // ensure "core" loads first, then alphabetical
    let mut names: Vec<&String> = types.keys().collect();
    names.sort_by(|a, b| {
        if a.as_str() == "core" { std::cmp::Ordering::Less }
        else if b.as_str() == "core" { std::cmp::Ordering::Greater }
        else { a.cmp(b) }
    });
    for name in names {
        let spec = &types[name];
        if let Some(builtin_name) = crate::manifest::Manifest::is_builtin(spec) {
            if let Some(plugin) = builtin_type_plugin(builtin_name) {
                plugin.register(heap);
            } else {
                eprintln!("  ~ unknown builtin type: {builtin_name}");
            }
        } else {
            eprintln!("  ~ external type plugins not yet supported: {spec}");
        }
    }
}

/// Register all default type plugins (fallback when no manifest).
pub fn register_all(heap: &mut Heap) {
    for name in ["core", "numeric", "collections", "block", "effects", "json"] {
        if let Some(p) = builtin_type_plugin(name) { p.register(heap); }
    }
}
