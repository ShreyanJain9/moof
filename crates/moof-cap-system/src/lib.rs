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
/// Resolution table: list of `(url-string . (vat-id obj-id))` pairs.
/// System pushes this on boot / mutation; `[system resolve: url]`
/// walks it and returns a fresh FarRef (in the cap's vat) for the
/// matching entry. Cross-vat copy carries the FarRef to the caller.
pub const SLOT_RESOLVE_TABLE: &str = "resolve-table";

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

        // [system resolve: url] → a FarRef (or nil) for the URL.
        // Walks the resolve-table slot, which rust-side System keeps
        // in sync with the live cap / vat registries.
        //
        // The FarRef is created fresh in this cap's vat with the
        // right (target_vat, target_obj, url) slots; cross-vat copy
        // carries it to the caller. The url argument may be a URL
        // value (with a `path` slot) or a string.
        native(heap, obj, "resolve:", |heap, recv, args| {
            let url_arg = args.first().copied().unwrap_or(Value::NIL);
            // accept both String and URL-with-.path
            let url_str = url_arg.as_any_object()
                .and_then(|id| heap.get_string(id).map(|s| s.to_string()))
                .or_else(|| {
                    // URL value: has scheme + path slots
                    let uid = url_arg.as_any_object()?;
                    let scheme_sym = heap.find_symbol("scheme")?;
                    let path_sym = heap.find_symbol("path")?;
                    let scheme_v = heap.get(uid).slot_get(scheme_sym)?;
                    let path_v = heap.get(uid).slot_get(path_sym)?;
                    let scheme = heap.get_string(scheme_v.as_any_object()?)?.to_string();
                    let path = heap.get_string(path_v.as_any_object()?)?.to_string();
                    Some(format!("{scheme}:{path}"))
                })
                .ok_or("resolve: argument must be a URL or String")?;

            let table = read_slot(heap, recv, SLOT_RESOLVE_TABLE)?;
            // table is a list of cons-pairs (url . (vat-id obj-id))
            for entry in heap.list_to_vec(table) {
                let (k, v) = heap.pair_of(entry.as_any_object().unwrap_or(0))
                    .unwrap_or((Value::NIL, Value::NIL));
                let entry_url = k.as_any_object()
                    .and_then(|id| heap.get_string(id)).unwrap_or("").to_string();
                if entry_url == url_str {
                    // v is a 2-list (vat-id obj-id)
                    let items = heap.list_to_vec(v);
                    let vat_id = items.first().and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                    let obj_id = items.get(1).and_then(|v| v.as_integer()).unwrap_or(0) as u32;
                    // build a FarRef in this cap's vat
                    let farref_proto = heap.lookup_type("FarRef");
                    let tv = heap.intern("__target_vat");
                    let to = heap.intern("__target_obj");
                    let us = heap.intern("url");
                    let url_val = heap.alloc_string(&url_str);
                    return Ok(heap.make_object_with_slots(
                        farref_proto,
                        vec![tv, to, us],
                        vec![Value::integer(vat_id as i64), Value::integer(obj_id as i64), url_val],
                    ));
                }
            }
            Ok(Value::NIL)
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
