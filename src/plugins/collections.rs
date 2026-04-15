use crate::plugins::native;
use crate::heap::*;
use crate::object::HeapObject;
use crate::value::Value;

use super::Plugin;

pub struct CollectionsPlugin;

impl Plugin for CollectionsPlugin {
    fn name(&self) -> &str { "collections" }

    fn register(&self, heap: &mut Heap) {
        let object_proto = heap.type_protos[PROTO_OBJ];

        // -- Cons prototype --
        let cons_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_CONS] = cons_proto;
        let cons_id = cons_proto.as_any_object().unwrap();

        native(heap, cons_id, "car", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("car: not a cons")?;
            Ok(heap.car(id))
        });
        native(heap, cons_id, "cdr", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("cdr: not a cons")?;
            Ok(heap.cdr(id))
        });
        native(heap, cons_id, "length", |heap, receiver, _args| {
            let mut count = 0i64;
            let mut cur = receiver;
            while let Some(id) = cur.as_any_object() {
                match heap.get(id) {
                    HeapObject::Pair(_, cdr) => { count += 1; cur = *cdr; }
                    _ => break,
                }
            }
            Ok(Value::integer(count))
        });
        native(heap, cons_id, "describe", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });

        // -- String prototype --
        let str_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_STR] = str_proto;
        let str_id = str_proto.as_any_object().unwrap();

        native(heap, str_id, "length", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("length: not a string")?;
            let s = heap.get_string(id).ok_or("length: not a Text object")?;
            Ok(Value::integer(s.len() as i64))
        });
        native(heap, str_id, "at:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("at: not a string")?;
            let s = heap.get_string(id).ok_or("at: not a Text object")?;
            let idx = args.first().and_then(|v| v.as_integer()).ok_or("at: arg not an integer")? as usize;
            let ch = s.chars().nth(idx).map(|c| c.to_string()).unwrap_or_default();
            Ok(heap.alloc_string(&ch))
        });
        native(heap, str_id, "++", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("++: not a string")?;
            let a = heap.get_string(id).ok_or("++: not a Text object")?.to_string();
            let arg = args.first().copied().unwrap_or(Value::NIL);
            let b = if let Some(bid) = arg.as_any_object() {
                heap.get_string(bid).map(|s| s.to_string()).unwrap_or_else(|| heap.format_value(arg))
            } else {
                heap.format_value(arg)
            };
            Ok(heap.alloc_string(&format!("{}{}", a, b)))
        });
        native(heap, str_id, "substring:to:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("substring:to: not a string")?;
            let s = heap.get_string(id).ok_or("substring:to: not a Text object")?;
            let from = args.first().and_then(|v| v.as_integer()).ok_or("substring:to: arg0 not int")? as usize;
            let to = args.get(1).and_then(|v| v.as_integer()).ok_or("substring:to: arg1 not int")? as usize;
            let chars: Vec<char> = s.chars().collect();
            let end = to.min(chars.len());
            let start = from.min(end);
            let sub: String = chars[start..end].iter().collect();
            Ok(heap.alloc_string(&sub))
        });
        native(heap, str_id, "split:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("split: not a string")?;
            let s = heap.get_string(id).ok_or("split: not a Text object")?.to_string();
            let delim_arg = args.first().copied().unwrap_or(Value::NIL);
            let did = delim_arg.as_any_object().ok_or("split: arg not a string")?;
            let delim = heap.get_string(did).ok_or("split: arg not a Text object")?.to_string();
            let parts: Vec<Value> = s.split(&delim).map(|p| heap.alloc_string(p)).collect();
            Ok(heap.list(&parts))
        });
        native(heap, str_id, "trim", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("trim: not a string")?;
            let s = heap.get_string(id).ok_or("trim: not a Text object")?.trim().to_string();
            Ok(heap.alloc_string(&s))
        });
        native(heap, str_id, "contains:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("contains: not a string")?;
            let s = heap.get_string(id).ok_or("contains: not a Text object")?.to_string();
            let arg = args.first().copied().unwrap_or(Value::NIL);
            let nid = arg.as_any_object().ok_or("contains: arg not a string")?;
            let needle = heap.get_string(nid).ok_or("contains: arg not a Text object")?;
            Ok(Value::boolean(s.contains(needle)))
        });
        native(heap, str_id, "startsWith:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("startsWith: not a string")?;
            let s = heap.get_string(id).ok_or("startsWith: not a Text object")?.to_string();
            let arg = args.first().copied().unwrap_or(Value::NIL);
            let pid = arg.as_any_object().ok_or("startsWith: arg not a string")?;
            let prefix = heap.get_string(pid).ok_or("startsWith: arg not a Text object")?;
            Ok(Value::boolean(s.starts_with(prefix)))
        });
        native(heap, str_id, "endsWith:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("endsWith: not a string")?;
            let s = heap.get_string(id).ok_or("endsWith: not a Text object")?.to_string();
            let arg = args.first().copied().unwrap_or(Value::NIL);
            let sid = arg.as_any_object().ok_or("endsWith: arg not a string")?;
            let suffix = heap.get_string(sid).ok_or("endsWith: arg not a Text object")?;
            Ok(Value::boolean(s.ends_with(suffix)))
        });
        native(heap, str_id, "toUpper", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("toUpper: not a string")?;
            let s = heap.get_string(id).ok_or("toUpper: not a Text object")?;
            Ok(heap.alloc_string(&s.to_uppercase()))
        });
        native(heap, str_id, "toLower", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("toLower: not a string")?;
            let s = heap.get_string(id).ok_or("toLower: not a Text object")?;
            Ok(heap.alloc_string(&s.to_lowercase()))
        });
        native(heap, str_id, "toInteger", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("toInteger: not a string")?;
            let s = heap.get_string(id).ok_or("toInteger: not a Text object")?;
            let n: i64 = s.trim().parse().map_err(|_| format!("toInteger: cannot parse '{}'", s))?;
            Ok(Value::integer(n))
        });
        native(heap, str_id, "describe", |_heap, receiver, _args| {
            Ok(receiver) // strings describe as themselves
        });
        native(heap, str_id, "indexOf:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("indexOf: not a string")?;
            let sub_id = args.first().and_then(|v| v.as_any_object()).ok_or("indexOf: arg not a string")?;
            match (heap.get(id), heap.get(sub_id)) {
                (HeapObject::Text(s), HeapObject::Text(sub)) => {
                    match s.find(sub.as_str()) {
                        Some(pos) => Ok(Value::integer(pos as i64)),
                        None => Ok(Value::NIL),
                    }
                }
                _ => Err("indexOf: not strings".into()),
            }
        });
        native(heap, str_id, "replace:with:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("replace:with: not a string")?;
            let old_id = args.get(0).and_then(|v| v.as_any_object()).ok_or("replace:with: first arg not a string")?;
            let new_id = args.get(1).and_then(|v| v.as_any_object()).ok_or("replace:with: second arg not a string")?;
            match (heap.get(id), heap.get(old_id), heap.get(new_id)) {
                (HeapObject::Text(s), HeapObject::Text(old), HeapObject::Text(new)) => {
                    let result = s.replacen(old.as_str(), new.as_str(), 1);
                    Ok(heap.alloc_string(&result))
                }
                _ => Err("replace:with: not strings".into()),
            }
        });
        native(heap, str_id, "replaceAll:with:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("replaceAll:with: not a string")?;
            let old_id = args.get(0).and_then(|v| v.as_any_object()).ok_or("replaceAll:with: first arg not a string")?;
            let new_id = args.get(1).and_then(|v| v.as_any_object()).ok_or("replaceAll:with: second arg not a string")?;
            match (heap.get(id), heap.get(old_id), heap.get(new_id)) {
                (HeapObject::Text(s), HeapObject::Text(old), HeapObject::Text(new)) => {
                    let result = s.replace(old.as_str(), new.as_str());
                    Ok(heap.alloc_string(&result))
                }
                _ => Err("replaceAll:with: not strings".into()),
            }
        });
        native(heap, str_id, "toFloat", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("toFloat: not a string")?;
            match heap.get(id) {
                HeapObject::Text(s) => match s.parse::<f64>() {
                    Ok(n) => Ok(Value::float(n)),
                    Err(_) => Err(format!("toFloat: cannot parse '{s}'")),
                },
                _ => Err("toFloat: not a string".into()),
            }
        });
        native(heap, str_id, "<", |heap, receiver, args| {
            let a_id = receiver.as_any_object().ok_or("< : not a string")?;
            let b_id = args.first().and_then(|v| v.as_any_object()).ok_or("< : arg not a string")?;
            match (heap.get(a_id), heap.get(b_id)) {
                (HeapObject::Text(a), HeapObject::Text(b)) => Ok(Value::boolean(a < b)),
                _ => Err("< : not strings".into()),
            }
        });

        // -- Table prototype --
        let table_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_TABLE] = table_proto;
        let table_id = table_proto.as_any_object().unwrap();

        native(heap, table_id, "at:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("at: not a table")?;
            let key = args.first().copied().unwrap_or(Value::NIL);
            match heap.get(id) {
                HeapObject::Table { seq, map } => {
                    // try integer index into seq first
                    if let Some(idx) = key.as_integer() {
                        if idx >= 0 && (idx as usize) < seq.len() {
                            return Ok(seq[idx as usize]);
                        }
                    }
                    // then check map (content equality for strings)
                    for (k, v) in map {
                        if heap.values_equal(*k, key) { return Ok(*v); }
                    }
                    Ok(Value::NIL)
                }
                _ => Err("at: not a Table".into()),
            }
        });
        // at:put: — returns a NEW table (non-destructive)
        native(heap, table_id, "at:put:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("at:put: not a table")?;
            let key = args.first().copied().unwrap_or(Value::NIL);
            let val = args.get(1).copied().unwrap_or(Value::NIL);
            match heap.get(id) {
                HeapObject::Table { seq, map } => {
                    let mut new_seq = seq.clone();
                    let mut new_map = map.clone();
                    if let Some(idx) = key.as_integer() {
                        if idx >= 0 && (idx as usize) < new_seq.len() {
                            new_seq[idx as usize] = val;
                            return Ok(heap.alloc_val(HeapObject::Table { seq: new_seq, map: new_map }));
                        }
                    }
                    // map entry: update existing or append
                    let existing = new_map.iter().position(|(k, _)| heap.values_equal(*k, key));
                    if let Some(pos) = existing {
                        new_map[pos].1 = val;
                    } else {
                        new_map.push((key, val));
                    }
                    Ok(heap.alloc_val(HeapObject::Table { seq: new_seq, map: new_map }))
                }
                _ => Err("at:put: not a Table".into()),
            }
        });
        // push: — returns a NEW table with element appended (non-destructive)
        native(heap, table_id, "push:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("push: not a table")?;
            let val = args.first().copied().unwrap_or(Value::NIL);
            match heap.get(id) {
                HeapObject::Table { seq, map } => {
                    let mut new_seq = seq.clone();
                    new_seq.push(val);
                    Ok(heap.alloc_val(HeapObject::Table { seq: new_seq, map: map.clone() }))
                }
                _ => Err("push: not a Table".into()),
            }
        });
        native(heap, table_id, "length", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("length: not a table")?;
            match heap.get(id) {
                HeapObject::Table { seq, .. } => Ok(Value::integer(seq.len() as i64)),
                _ => Err("length: not a Table".into()),
            }
        });
        native(heap, table_id, "keys", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("keys: not a table")?;
            let keys: Vec<Value> = match heap.get(id) {
                HeapObject::Table { map, .. } => map.iter().map(|(k, _)| *k).collect(),
                _ => return Err("keys: not a Table".into()),
            };
            Ok(heap.list(&keys))
        });
        native(heap, table_id, "values", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("values: not a table")?;
            let vals: Vec<Value> = match heap.get(id) {
                HeapObject::Table { map, .. } => map.iter().map(|(_, v)| *v).collect(),
                _ => return Err("values: not a Table".into()),
            };
            Ok(heap.list(&vals))
        });
        native(heap, table_id, "describe", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });
        native(heap, table_id, "contains:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("contains: not a table")?;
            let key = args.first().copied().unwrap_or(Value::NIL);
            match heap.get(id) {
                HeapObject::Table { seq, map } => {
                    for v in seq {
                        if heap.values_equal(*v, key) { return Ok(Value::TRUE); }
                    }
                    for (k, _) in map {
                        if heap.values_equal(*k, key) { return Ok(Value::TRUE); }
                    }
                    Ok(Value::FALSE)
                }
                _ => Err("contains: not a Table".into()),
            }
        });
        // remove: — returns a NEW table with key removed (non-destructive)
        native(heap, table_id, "remove:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("remove: not a table")?;
            let key = args.first().copied().unwrap_or(Value::NIL);
            match heap.get(id) {
                HeapObject::Table { seq, map } => {
                    let new_map: Vec<(Value, Value)> = map.iter()
                        .filter(|(k, _)| !heap.values_equal(*k, key))
                        .cloned()
                        .collect();
                    Ok(heap.alloc_val(HeapObject::Table { seq: seq.clone(), map: new_map }))
                }
                _ => Err("remove: not a Table".into()),
            }
        });

        // -- register globals --
        let cons_sym = heap.intern("Cons");
        heap.env_def(cons_sym, cons_proto);
        let string_sym = heap.intern("String");
        heap.env_def(string_sym, str_proto);
        let table_sym = heap.intern("Table");
        heap.env_def(table_sym, table_proto);
    }
}
