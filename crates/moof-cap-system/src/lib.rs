// System capability — introspection handlers over System's state.
//
// This is the capability face of the rust-side `System` struct. Its
// handlers read from slots pushed by rust-side System on boot and on
// mutation. Nothing here is privileged; the handlers return what
// they find in slots, same as any other capability.
//
// Slot layout (written by crates/moof-cli/src/system.rs):
//   capability-names : list of symbols
//   user-vats        : list of integers  (vat ids)
//   grants-table     : list of pairs (name-sym . list of cap-syms)
//
// Any cross-vat send to this capability returns an Act<T> as usual;
// consumers `do`-notation or `then:` to drain.

use moof_core::{Heap, Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

/// Symbols for the slots rust-side System writes into. Kept here
/// (not re-interned on each handler call) because the capability's
/// vat is stable once setup runs.
pub const SLOT_CAPS: &str = "capability-names";
pub const SLOT_VATS: &str = "user-vats";
pub const SLOT_GRANTS: &str = "grants-table";

pub struct SystemCapability;

fn read_slot(heap: &Heap, recv: Value, name: &str) -> Result<Value, String> {
    let id = recv.as_any_object().ok_or("system: receiver is not an object")?;
    let Some(sym) = heap.find_symbol(name) else { return Ok(Value::NIL); };
    Ok(heap.get(id).slot_get(sym).unwrap_or(Value::NIL))
}

impl CapabilityPlugin for SystemCapability {
    fn name(&self) -> &str { "system" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        // [system capabilities] → list of capability-name symbols
        native(heap, obj, "capabilities", |heap, recv, _args| {
            read_slot(heap, recv, SLOT_CAPS)
        });

        // [system vats] → list of user-vat ids as integers
        native(heap, obj, "vats", |heap, recv, _args| {
            read_slot(heap, recv, SLOT_VATS)
        });

        // [system grants] → list of (interface-name . cap-list) pairs
        // representing the manifest's grants table plus any runtime
        // additions. today matches manifest exactly.
        native(heap, obj, "grants", |heap, recv, _args| {
            read_slot(heap, recv, SLOT_GRANTS)
        });

        // [system describe] → human-readable summary
        native(heap, obj, "describe", |heap, recv, _args| {
            let caps = read_slot(heap, recv, SLOT_CAPS)?;
            let vats = read_slot(heap, recv, SLOT_VATS)?;
            let ncaps = heap.list_to_vec(caps).len();
            let nvats = heap.list_to_vec(vats).len();
            Ok(heap.alloc_string(&format!("<System: {ncaps} caps, {nvats} user vats>")))
        });

        // [system typeName] → 'System for the describe protocol, etc.
        let type_sym = heap.intern("System");
        native(heap, obj, "typeName", move |_, _, _| Ok(Value::symbol(type_sym)));

        obj
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(SystemCapability)
}
