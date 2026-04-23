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
/// Root namespace: a nested Table forming the plan-9-shaped
/// directory of everything the system owns. `[system root]`
/// exposes it; moof code walks via `walk:` / `at:`. See
/// `lib/kernel/namespace.moof`.
pub const SLOT_ROOT: &str = "root";

/// Parse a URL Value. Returns (scheme, path). Accepts both URL
/// objects (with scheme + path slots) and plain strings in the
/// form "scheme:path".
fn parse_url(heap: &Heap, v: Value) -> Option<(String, String)> {
    let id = v.as_any_object()?;
    if let Some(s) = heap.get_string(id) {
        let (scheme, rest) = s.split_once(':')?;
        return Some((scheme.to_string(), rest.to_string()));
    }
    let scheme_sym = heap.find_symbol("scheme")?;
    let path_sym = heap.find_symbol("path")?;
    let scheme_v = heap.get(id).slot_get(scheme_sym)?;
    let path_v = heap.get(id).slot_get(path_sym)?;
    let scheme = heap.get_string(scheme_v.as_any_object()?)?.to_string();
    let path = heap.get_string(path_v.as_any_object()?)?.to_string();
    Some((scheme, path))
}

/// Walk a '/'-separated path through a namespace (a Table, nested).
/// Empty segments are skipped. Returns the leaf value, or NIL if
/// any segment is missing.
fn walk_namespace(heap: &Heap, root: Value, path: &str) -> Value {
    let stripped = path.strip_prefix('/').unwrap_or(path);
    let mut cur = root;
    for seg in stripped.split('/') {
        if seg.is_empty() { continue; }
        cur = at_lookup(heap, cur, seg);
        if cur.is_nil() { return Value::NIL; }
    }
    cur
}

/// One-level namespace lookup: read the entry named `name` from a
/// Table. Canonicalizes the key by interning as a symbol (mirrors
/// moof's Table.at: semantics).
fn at_lookup(heap: &Heap, ns: Value, name: &str) -> Value {
    use moof_core::heap::Table;
    let Some(id) = ns.as_any_object() else { return Value::NIL; };
    let Some(table) = heap.foreign_ref::<Table>(Value::nursery(id)) else { return Value::NIL; };
    // try symbol lookup first (how most keys are stored)
    if let Some(sym_id) = heap.find_symbol(name) {
        if let Some(v) = table.map.get(&Value::symbol(sym_id)) {
            return *v;
        }
    }
    // try integer lookup (vats are keyed by int)
    if let Ok(n) = name.parse::<i64>() {
        if let Some(v) = table.map.get(&Value::integer(n)) {
            return *v;
        }
    }
    Value::NIL
}

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

        // [system root] → the root namespace as a nested Table.
        // moof code walks via `[root walk: "/caps/console"]` or
        // `[root at: "caps" at: "console"]`. leaves at caps/* are
        // (vat-id obj-id) pairs; pass through `[system resolve:]`
        // (or the `fromProxy:` helper in lib/system) to get a
        // live FarRef. See lib/kernel/namespace.moof.
        native(heap, obj, "root", |heap, recv, _args| {
            read_slot(heap, recv, SLOT_ROOT)
        });

        // [system resolve: url] → a FarRef (or nil) for the URL.
        //
        // Internally: walk the root namespace. The path `/caps/console`
        // navigates through the tree to a leaf (vat-id obj-id) pair;
        // we wrap that into a fresh FarRef in this cap's vat.
        // Cross-vat copy carries it to the caller.
        //
        // Accepts both URL values (with `path` slot) and plain strings.
        native(heap, obj, "resolve:", |heap, recv, args| {
            let url_arg = args.first().copied().unwrap_or(Value::NIL);
            let (scheme, path) = parse_url(heap, url_arg)
                .ok_or("resolve: argument must be a URL or 'scheme:path' String")?;
            if scheme != "moof" { return Ok(Value::NIL); }

            // walk the root namespace to find the leaf.
            let root = read_slot(heap, recv, SLOT_ROOT)?;
            let leaf = walk_namespace(heap, root, &path);

            // leaf should be a (vat-id obj-id) cons list for a valid
            // capability/vat entry. otherwise nil.
            let items = heap.list_to_vec(leaf);
            if items.len() < 2 { return Ok(Value::NIL); }
            let vat_id = items[0].as_integer().unwrap_or(0) as u32;
            let obj_id = items[1].as_integer().unwrap_or(0) as u32;
            if vat_id == 0 && obj_id == 0 { return Ok(Value::NIL); }

            // build a FarRef in this cap's vat with the url slot.
            let url_str = format!("{scheme}:{path}");
            let farref_proto = heap.lookup_type("FarRef");
            let tv = heap.intern("__target_vat");
            let to = heap.intern("__target_obj");
            let us = heap.intern("url");
            let url_val = heap.alloc_string(&url_str);
            Ok(heap.make_object_with_slots(
                farref_proto,
                vec![tv, to, us],
                vec![Value::integer(vat_id as i64), Value::integer(obj_id as i64), url_val],
            ))
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
