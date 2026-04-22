// JSON — parse and serialize. pure (no I/O), so exposed as a
// regular global value with `parse:` and `serialize:` handlers
// rather than a capability vat.
//
// Mapping:
//   JSON null          → nil
//   JSON true / false  → Boolean
//   JSON number        → Integer if representable, else Float
//   JSON string        → String
//   JSON array         → Cons list
//   JSON object        → Table (keyed part)
//
// fallible ops return the raw value on success, an Err on failure.
// same convention as the File / Random / HTTP capabilities.

use moof_core::{Plugin, native};
use moof_core::heap::*;
use moof_core::value::Value;
use serde_json::Value as JV;

pub struct JsonPlugin;

impl Plugin for JsonPlugin {
    fn name(&self) -> &str { "json" }

    fn register(&self, heap: &mut Heap) {
        let object_proto = heap.type_protos[PROTO_OBJ];
        let json_obj = heap.make_object(object_proto);
        let json_obj_id = json_obj.as_any_object().unwrap();

        // parse: str → moof-value (on success) or Err (on parse error)
        native(heap, json_obj_id, "parse:", |heap, _recv, args| {
            let v = args.first().copied().unwrap_or(Value::NIL);
            let s = match v.as_any_object().and_then(|id| heap.get_string(id)) {
                Some(s) => s.to_string(),
                None => return Err("json parse:: arg must be a String".into()),
            };
            match serde_json::from_str::<JV>(&s) {
                Ok(jv) => Ok(from_json(heap, &jv)),
                Err(e) => Ok(heap.make_error(&format!("json parse: {e}"))),
            }
        });

        // serialize: moof-value → String (JSON text)
        native(heap, json_obj_id, "serialize:", |heap, _recv, args| {
            let v = args.first().copied().unwrap_or(Value::NIL);
            match to_json(heap, v) {
                Ok(jv) => match serde_json::to_string(&jv) {
                    Ok(s) => Ok(heap.alloc_string(&s)),
                    Err(e) => Ok(heap.make_error(&format!("json serialize: {e}"))),
                },
                Err(e) => Ok(heap.make_error(&format!("json serialize: {e}"))),
            }
        });

        // pretty: val → pretty-printed JSON
        native(heap, json_obj_id, "pretty:", |heap, _recv, args| {
            let v = args.first().copied().unwrap_or(Value::NIL);
            match to_json(heap, v) {
                Ok(jv) => match serde_json::to_string_pretty(&jv) {
                    Ok(s) => Ok(heap.alloc_string(&s)),
                    Err(e) => Ok(heap.make_error(&format!("json pretty: {e}"))),
                },
                Err(e) => Ok(heap.make_error(&format!("json pretty: {e}"))),
            }
        });

        native(heap, json_obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<json>"))
        });

        let json_sym = heap.intern("json");
        heap.env_def(json_sym, json_obj);
    }
}

/// serde_json::Value → moof Value
fn from_json(heap: &mut Heap, jv: &JV) -> Value {
    match jv {
        JV::Null => Value::NIL,
        JV::Bool(b) => Value::boolean(*b),
        JV::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::integer(i)
            } else if let Some(f) = n.as_f64() {
                Value::float(f)
            } else {
                Value::NIL
            }
        }
        JV::String(s) => heap.alloc_string(s),
        JV::Array(arr) => {
            let items: Vec<Value> = arr.iter().map(|v| from_json(heap, v)).collect();
            heap.list(&items)
        }
        JV::Object(obj) => {
            // JSON object → Table with keyed part.
            let mut map: indexmap::IndexMap<Value, Value> = indexmap::IndexMap::with_capacity(obj.len());
            for (k, v) in obj {
                let key = heap.alloc_string(k);
                let val = from_json(heap, v);
                map.insert(key, val);
            }
            heap.alloc_table(Vec::new(), map)
        }
    }
}

/// moof Value → serde_json::Value
fn to_json(heap: &Heap, v: Value) -> Result<JV, String> {
    if v.is_nil() { return Ok(JV::Null); }
    if v.is_true() { return Ok(JV::Bool(true)); }
    if v.is_false() { return Ok(JV::Bool(false)); }
    if let Some(n) = v.as_integer() {
        return Ok(JV::Number(n.into()));
    }
    if v.is_float() {
        let f = f64::from_bits(v.to_bits());
        return serde_json::Number::from_f64(f)
            .map(JV::Number)
            .ok_or_else(|| format!("cannot serialize non-finite float {f}"));
    }
    if let Some(sym) = v.as_symbol() {
        return Ok(JV::String(heap.symbol_name(sym).into()));
    }
    let id = v.as_any_object().ok_or("unknown value type")?;
    if heap.is_pair(v) {
        let items = heap.list_to_vec(v);
        let mut arr: Vec<JV> = Vec::with_capacity(items.len());
        for item in items {
            arr.push(to_json(heap, item)?);
        }
        return Ok(JV::Array(arr));
    }
    if let Some(s) = heap.get_string(id) {
        return Ok(JV::String(s.to_string()));
    }
    if heap.is_bytes(v) {
        return Err("cannot serialize bytes (use base64 first)".into());
    }
    if let Some(t) = heap.get_table(id) {
        if !t.map.is_empty() {
            let mut obj = serde_json::Map::new();
            for (k, val) in &t.map {
                let key_str = match k.as_any_object().and_then(|id| heap.get_string(id)) {
                    Some(s) => s.to_string(),
                    None => heap.format_value(*k),
                };
                obj.insert(key_str, to_json(heap, *val)?);
            }
            return Ok(JV::Object(obj));
        } else {
            let mut arr: Vec<JV> = Vec::with_capacity(t.seq.len());
            for item in &t.seq {
                arr.push(to_json(heap, *item)?);
            }
            return Ok(JV::Array(arr));
        }
    }
    // General with no foreign payload: treat as a plain record
    let obj_ref = heap.get(id);
    let mut obj = serde_json::Map::new();
    for (n, v) in obj_ref.slot_names.iter().zip(obj_ref.slot_values.iter()) {
        obj.insert(heap.symbol_name(*n).to_string(), to_json(heap, *v)?);
    }
    Ok(JV::Object(obj))
}

/// Entry point for dylib loading. moof-cli's manifest loader
/// calls this via `libloading` when a `[types]` entry points
/// at this crate's cdylib.
#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn moof_core::Plugin> {
    Box::new(JsonPlugin)
}
