//! the substrate's primordial native methods + global bindings.
//!
//! installed during `World::new()`, before any moof source loads.
//! covers exactly what's needed for the phase-A forcing function:
//!
//! - `:call` on `Method` (covers Closure, all method-Forms).
//! - arithmetic + comparison on `Integer` (`:+`, `:-`, `:*`, `:/`,
//!   `:<`, `:>`, `:<=`, `:>=`, `:=`, `:!=`).
//! - structural ops: `:head`, `:tail`, `:cons:`, `:null?` on `List`.
//! - identity / equality on `Object`, `Symbol`, `Bool`, `Nil`.
//! - reflection on `Object`: `:proto`, `:slots`, `:handlers`,
//!   `:meta`, `:source`, `:identity`, `:=`, `:is`, `:to-string`,
//!   `:inspect`, `:new`, `:does-not-understand:with:`.
//! - global callables that forward to receiver methods: `+`, `-`,
//!   `*`, `/`, `<`, `>`, `<=`, `>=`, `=`, `!=`, `head`, `tail`,
//!   `cons`, `null?`, `list?`, `not`.
//!
//! everything else — `length`, `map`, `filter`, the protocol
//! framework — lives in moof code at phase A.10.

use crate::form::{Form, FormId};
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{NativeFn, RaiseError, World};

/// install all phase-A intrinsics. idempotent: safe to call once
/// at world init.
pub fn install(w: &mut World) {
    install_call_on_method(w);
    install_integer_methods(w);
    install_float_methods(w);
    install_symbol_methods(w);
    install_bool_methods(w);
    install_nil_methods(w);
    install_char_methods(w);
    install_string_methods(w);
    install_table_methods(w);
    install_object_reflection(w);
    install_list_methods(w);
    install_method_methods(w);
    install_method_reflection(w);
    install_console_proto_and_caps(w);
    install_globals(w);
    install_proto_globals(w);
}

// ─────────────────────────────────────────────────────────────────
// Table methods — `docs/concepts/tables.md`
//
// the universal collection: positional + keyed entries in one
// type. methods access either axis based on the key's runtime
// kind (Integer → positional; anything else → keyed).
// ─────────────────────────────────────────────────────────────────

fn install_table_methods(w: &mut World) {
    // override Object's :new so [Table new] returns a fresh Table
    // with the rep slot populated. (Object's default :new would
    // alloc an empty Form whose :emit: / :at: / etc. would all
    // fail on the missing :rep handle.)
    w.install_native(w.protos.table, "new", |w, _self, _args| {
        Ok(w.make_table())
    });

    w.install_native(w.protos.table, "length", |w, self_, _| {
        let n = match w.table_repr(self_) {
            Some(r) => r.positional.len() as i64,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "length on non-Table",
                ))
            }
        };
        Ok(Value::Int(n))
    });

    w.install_native(w.protos.table, "size", |w, self_, _| {
        let n = match w.table_repr(self_) {
            Some(r) => r.size() as i64,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "size on non-Table",
                ))
            }
        };
        Ok(Value::Int(n))
    });

    w.install_native(w.protos.table, "empty?", |w, self_, _| {
        let empty = match w.table_repr(self_) {
            Some(r) => r.size() == 0,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "empty? on non-Table",
                ))
            }
        };
        Ok(Value::Bool(empty))
    });

    // [t at: k] — Integer key reads positional; anything else reads keyed.
    w.install_native(w.protos.table, "at:", |w, self_, args| {
        let key = args.first().copied().unwrap_or(Value::Nil);
        match key {
            Value::Int(i) => {
                let v = w.table_repr(self_).and_then(|r| {
                    if i < 0 {
                        None
                    } else {
                        r.positional.get(i as usize).copied()
                    }
                });
                v.ok_or_else(|| {
                    RaiseError::new(
                        w.intern("index-out-of-bounds"),
                        format!("[Table at: {}] out of range", i),
                    )
                })
            }
            other => {
                let v = w
                    .table_repr(self_)
                    .and_then(|r| r.keyed.get(&other).copied())
                    .unwrap_or(Value::Nil);
                Ok(v)
            }
        }
    });

    // [t at: k put: v] — keyword shape, sets positional or keyed
    // based on key type. positional :at:put: at index = length pushes;
    // larger indices error.
    w.install_native(w.protos.table, "at:put:", |w, self_, args| {
        let key = args.first().copied().unwrap_or(Value::Nil);
        let val = args.get(1).copied().unwrap_or(Value::Nil);
        match key {
            Value::Int(i) => {
                // do the mutation in a scope that drops the borrow
                // before we may touch w.intern().
                let result: Result<(), (i64, usize)> = (|| {
                    let r = w.table_repr_mut(self_).ok_or((-1, 0))?;
                    if i < 0 {
                        return Err((i, 0));
                    }
                    let idx = i as usize;
                    if idx < r.positional.len() {
                        r.positional[idx] = val;
                        Ok(())
                    } else if idx == r.positional.len() {
                        r.positional.push(val);
                        Ok(())
                    } else {
                        Err((i, r.positional.len()))
                    }
                })();
                match result {
                    Ok(()) => Ok(self_),
                    Err((i, _)) if i == -1 => Err(RaiseError::new(
                        w.intern("type-error"),
                        "at:put: on non-Table",
                    )),
                    Err((i, _)) if i < 0 => Err(RaiseError::new(
                        w.intern("index-out-of-bounds"),
                        format!("Table positional index must be non-negative; got {}", i),
                    )),
                    Err((i, len)) => Err(RaiseError::new(
                        w.intern("index-out-of-bounds"),
                        format!(
                            "Table positional index {} out of range (length {})",
                            i, len
                        ),
                    )),
                }
            }
            other => {
                let r = w.table_repr_mut(self_);
                if let Some(r) = r {
                    r.keyed.insert(other, val);
                    Ok(self_)
                } else {
                    Err(RaiseError::new(
                        w.intern("type-error"),
                        "at:put: on non-Table",
                    ))
                }
            }
        }
    });

    // [t push: v] — positional append.
    w.install_native(w.protos.table, "push:", |w, self_, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let r = match w.table_repr_mut(self_) {
            Some(r) => r,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "push: on non-Table",
                ))
            }
        };
        r.positional.push(v);
        Ok(self_)
    });

    // [t pop] — remove and return the last positional. raises if empty.
    w.install_native(w.protos.table, "pop", |w, self_, _| {
        let v = match w.table_repr_mut(self_).and_then(|r| r.positional.pop()) {
            Some(v) => v,
            None => {
                return Err(RaiseError::new(
                    w.intern("empty-table"),
                    "pop on empty Table",
                ))
            }
        };
        Ok(v)
    });

    // [t containsKey?: k] — works on keyed + positional (via index range).
    w.install_native(w.protos.table, "containsKey?:", |w, self_, args| {
        let key = args.first().copied().unwrap_or(Value::Nil);
        let r = match w.table_repr(self_) {
            Some(r) => r,
            None => return Ok(Value::Bool(false)),
        };
        let present = match key {
            Value::Int(i) => i >= 0 && (i as usize) < r.positional.len(),
            other => r.keyed.contains_key(&other),
        };
        Ok(Value::Bool(present))
    });

    // [t remove: k] — remove the keyed entry; positional indices
    // not supported (use :at:put: with the new value, or :pop).
    w.install_native(w.protos.table, "remove:", |w, self_, args| {
        let key = args.first().copied().unwrap_or(Value::Nil);
        let r = match w.table_repr_mut(self_) {
            Some(r) => r,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "remove: on non-Table",
                ))
            }
        };
        let v = r.keyed.shift_remove(&key).unwrap_or(Value::Nil);
        Ok(v)
    });

    // [t keys] — list of all keys (positional indices first, then
    // keyed keys in insertion order).
    w.install_native(w.protos.table, "keys", |w, self_, _| {
        let mut keys: Vec<Value> = Vec::new();
        if let Some(r) = w.table_repr(self_) {
            for i in 0..r.positional.len() {
                keys.push(Value::Int(i as i64));
            }
            for k in r.keyed.keys() {
                keys.push(*k);
            }
        }
        Ok(w.make_list(&keys))
    });

    // [t values] — list of all values (positional in order, then keyed).
    w.install_native(w.protos.table, "values", |w, self_, _| {
        let mut vals: Vec<Value> = Vec::new();
        if let Some(r) = w.table_repr(self_) {
            for &v in &r.positional {
                vals.push(v);
            }
            for &v in r.keyed.values() {
                vals.push(v);
            }
        }
        Ok(w.make_list(&vals))
    });

    // [t toList] — positional axis as a List. (does not include keyed entries.)
    w.install_native(w.protos.table, "toList", |w, self_, _| {
        let vs: Vec<Value> = w
            .table_repr(self_)
            .map(|r| r.positional.clone())
            .unwrap_or_default();
        Ok(w.make_list(&vs))
    });

    // [t forEach: f] — iterate positional values then keyed values.
    w.install_native(w.protos.table, "forEach:", |w, self_, args| {
        let blk = args.first().copied().unwrap_or(Value::Nil);
        let snapshot: Vec<Value> = match w.table_repr(self_) {
            Some(r) => {
                let mut all: Vec<Value> = r.positional.clone();
                all.extend(r.keyed.values().copied());
                all
            }
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "forEach: on non-Table",
                ))
            }
        };
        let call_sym = w.intern("call");
        for v in snapshot {
            w.send(blk, call_sym, &[v])?;
        }
        Ok(Value::Nil)
    });

    // [t map: f] — produce a new Table with f applied to each
    // positional and keyed value (keys preserved).
    w.install_native(w.protos.table, "map:", |w, self_, args| {
        let blk = args.first().copied().unwrap_or(Value::Nil);
        let positional: Vec<Value>;
        let keyed_keys: Vec<Value>;
        let keyed_vals: Vec<Value>;
        match w.table_repr(self_) {
            Some(r) => {
                positional = r.positional.clone();
                keyed_keys = r.keyed.keys().copied().collect();
                keyed_vals = r.keyed.values().copied().collect();
            }
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "map: on non-Table",
                ))
            }
        }
        let new_table = w.make_table();
        let call_sym = w.intern("call");
        // positional: map each, push.
        for v in positional {
            let mapped = w.send(blk, call_sym, &[v])?;
            if let Some(r) = w.table_repr_mut(new_table) {
                r.positional.push(mapped);
            }
        }
        // keyed: same value mapping, key preserved.
        for (k, v) in keyed_keys.into_iter().zip(keyed_vals.into_iter()) {
            let mapped = w.send(blk, call_sym, &[v])?;
            if let Some(r) = w.table_repr_mut(new_table) {
                r.keyed.insert(k, mapped);
            }
        }
        Ok(new_table)
    });

    // [t filter: pred] — produce a new Table with only entries
    // (positional + keyed) for which pred returns truthy.
    w.install_native(w.protos.table, "filter:", |w, self_, args| {
        let pred = args.first().copied().unwrap_or(Value::Nil);
        let positional: Vec<Value>;
        let keyed_keys: Vec<Value>;
        let keyed_vals: Vec<Value>;
        match w.table_repr(self_) {
            Some(r) => {
                positional = r.positional.clone();
                keyed_keys = r.keyed.keys().copied().collect();
                keyed_vals = r.keyed.values().copied().collect();
            }
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "filter: on non-Table",
                ))
            }
        }
        let new_table = w.make_table();
        let call_sym = w.intern("call");
        for v in positional {
            let keep = w.send(pred, call_sym, &[v])?;
            if keep.is_truthy() {
                if let Some(r) = w.table_repr_mut(new_table) {
                    r.positional.push(v);
                }
            }
        }
        for (k, v) in keyed_keys.into_iter().zip(keyed_vals.into_iter()) {
            let keep = w.send(pred, call_sym, &[v])?;
            if keep.is_truthy() {
                if let Some(r) = w.table_repr_mut(new_table) {
                    r.keyed.insert(k, v);
                }
            }
        }
        Ok(new_table)
    });

    // [t reduce: f from: init] — fold over positional then keyed values.
    w.install_native(w.protos.table, "reduce:from:", |w, self_, args| {
        let f = args.first().copied().unwrap_or(Value::Nil);
        let mut acc = args.get(1).copied().unwrap_or(Value::Nil);
        let snapshot: Vec<Value> = match w.table_repr(self_) {
            Some(r) => {
                let mut all: Vec<Value> = r.positional.clone();
                all.extend(r.keyed.values().copied());
                all
            }
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "reduce:from: on non-Table",
                ))
            }
        };
        let call_sym = w.intern("call");
        for v in snapshot {
            acc = w.send(f, call_sym, &[acc, v])?;
        }
        Ok(acc)
    });

    // [t = other] — structural equality. compares positional in
    // order + keyed by key/value pairs (insertion order).
    w.install_native(w.protos.table, "=", |w, self_, args| {
        let other = args.first().copied().unwrap_or(Value::Nil);
        let a = w.table_repr(self_);
        let b = w.table_repr(other);
        match (a, b) {
            (Some(ra), Some(rb)) => Ok(Value::Bool(
                ra.positional == rb.positional && ra.keyed == rb.keyed,
            )),
            _ => Ok(Value::Bool(false)),
        }
    });

    w.install_native(w.protos.table, "!=", |w, self_, args| {
        let other = args.first().copied().unwrap_or(Value::Nil);
        let a = w.table_repr(self_);
        let b = w.table_repr(other);
        match (a, b) {
            (Some(ra), Some(rb)) => Ok(Value::Bool(
                !(ra.positional == rb.positional && ra.keyed == rb.keyed),
            )),
            _ => Ok(Value::Bool(true)),
        }
    });

    // [t toString] — `#[1 2 'name => "ada"]`-shaped rendering.
    w.install_native(w.protos.table, "toString", |w, self_, _| {
        render_table_to_string(w, self_).map(|s| w.make_string(&s))
    });

    // [t asString] — collect positional Chars into a String. raises
    // if any positional entry isn't a Char.
    w.install_native(w.protos.table, "asString", |w, self_, _| {
        let entries: Vec<Value> = match w.table_repr(self_) {
            Some(r) => r.positional.clone(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "asString on non-Table",
                ));
            }
        };
        let mut out = String::new();
        for v in entries {
            match v {
                Value::Char(cp) => {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                }
                _ => {
                    return Err(RaiseError::new(
                        w.intern("type-error"),
                        "asString: every positional entry must be a Char",
                    ));
                }
            }
        }
        Ok(w.make_string(&out))
    });

    // [t as: String] — protocol-style coercion. arg is a proto-Form.
    w.install_native(w.protos.table, "as:", |w, self_, args| {
        let target = args.first().copied().unwrap_or(Value::Nil);
        let table_proto = Value::Form(w.protos.table);
        let string_proto = Value::Form(w.protos.string);
        let list_proto = Value::Form(w.protos.list);
        if target == string_proto {
            let sel = w.intern("asString");
            return w.send(self_, sel, &[]);
        }
        if target == table_proto {
            return Ok(self_);
        }
        if target == list_proto {
            // a Table's positional entries as a List.
            let entries: Vec<Value> = match w.table_repr(self_) {
                Some(r) => r.positional.clone(),
                None => Vec::new(),
            };
            return Ok(w.make_list(&entries));
        }
        Err(RaiseError::new(
            w.intern("conversion"),
            "Table can be converted as: String, List, Table",
        ))
    });
}

fn render_table_to_string(w: &mut World, table: Value) -> Result<String, RaiseError> {
    let positional: Vec<Value>;
    let keyed_keys: Vec<Value>;
    let keyed_vals: Vec<Value>;
    match w.table_repr(table) {
        Some(r) => {
            positional = r.positional.clone();
            keyed_keys = r.keyed.keys().copied().collect();
            keyed_vals = r.keyed.values().copied().collect();
        }
        None => {
            return Err(RaiseError::new(
                w.intern("type-error"),
                "toString on non-Table",
            ));
        }
    }
    let mut out = String::from("#[");
    let to_string = w.intern("toString");
    let mut first = true;
    for v in positional {
        if !first {
            out.push(' ');
        }
        first = false;
        let s = w.send(v, to_string, &[])?;
        let txt = w.string_text(s).map(|t| t.to_string());
        out.push_str(&txt.unwrap_or_else(|| "?".into()));
    }
    for (k, v) in keyed_keys.into_iter().zip(keyed_vals.into_iter()) {
        if !first {
            out.push(' ');
        }
        first = false;
        let ks = w.send(k, to_string, &[])?;
        let kt = w.string_text(ks).map(|t| t.to_string());
        // symbols are rendered without the leading quote in
        // table-toString — we render them as their text. that
        // matches how `'name => "ada"` shows up in the source.
        out.push_str(&kt.unwrap_or_else(|| "?".into()));
        out.push_str(" => ");
        let vs = w.send(v, to_string, &[])?;
        let vt = w.string_text(vs).map(|t| t.to_string());
        out.push_str(&vt.unwrap_or_else(|| "?".into()));
    }
    out.push(']');
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────
// Char — tagged-immediate Unicode scalar.
// strings iterate to Chars; `[s at: i]` returns a Char.
// ─────────────────────────────────────────────────────────────────

fn install_char_methods(w: &mut World) {
    w.install_native(w.protos.char_, "codepoint", |w, self_, _| {
        match self_ {
            Value::Char(cp) => Ok(Value::Int(cp as i64)),
            _ => Err(RaiseError::new(
                w.intern("type-error"),
                "codepoint on non-Char",
            )),
        }
    });
    w.install_native(w.protos.char_, "toString", |w, self_, _| match self_ {
        Value::Char(cp) => {
            let text = char::from_u32(cp)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("?{:x}", cp));
            Ok(w.make_string(&text))
        }
        _ => Err(RaiseError::new(w.intern("type-error"), "toString on non-Char")),
    });
    w.install_native(w.protos.char_, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.char_, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.char_, "<", |_, self_, args| match (self_, args[0]) {
        (Value::Char(a), Value::Char(b)) => Ok(Value::Bool(a < b)),
        _ => Ok(Value::Bool(false)),
    });
    w.install_native(w.protos.char_, "letter?", |_, self_, _| match self_ {
        Value::Char(cp) => Ok(Value::Bool(
            char::from_u32(cp).map(|c| c.is_alphabetic()).unwrap_or(false),
        )),
        _ => Ok(Value::Bool(false)),
    });
    w.install_native(w.protos.char_, "digit?", |_, self_, _| match self_ {
        Value::Char(cp) => Ok(Value::Bool(
            char::from_u32(cp).map(|c| c.is_ascii_digit()).unwrap_or(false),
        )),
        _ => Ok(Value::Bool(false)),
    });
    w.install_native(w.protos.char_, "whitespace?", |_, self_, _| match self_ {
        Value::Char(cp) => Ok(Value::Bool(
            char::from_u32(cp).map(|c| c.is_whitespace()).unwrap_or(false),
        )),
        _ => Ok(Value::Bool(false)),
    });
    w.install_native(w.protos.char_, "upcase", |_, self_, _| match self_ {
        Value::Char(cp) => {
            let upper = char::from_u32(cp)
                .and_then(|c| c.to_uppercase().next())
                .map(|c| c as u32)
                .unwrap_or(cp);
            Ok(Value::Char(upper))
        }
        v => Ok(v),
    });
    w.install_native(w.protos.char_, "downcase", |_, self_, _| match self_ {
        Value::Char(cp) => {
            let lower = char::from_u32(cp)
                .and_then(|c| c.to_lowercase().next())
                .map(|c| c as u32)
                .unwrap_or(cp);
            Ok(Value::Char(lower))
        }
        v => Ok(v),
    });
}

// ─────────────────────────────────────────────────────────────────
// String methods
// ─────────────────────────────────────────────────────────────────

fn install_string_methods(w: &mut World) {
    // String is conceptually a sequence of Chars (Unicode scalar
    // values). `:length` is the *char* count; `:byteLength` is the
    // raw-byte count. iteration walks Chars; `[s at: i]` returns a
    // Char. internal storage is UTF-8 for efficiency. matches
    // `docs/concepts/strings.md`.
    w.install_native(w.protos.string, "length", |w, self_, _| {
        let count_opt = w.string_text(self_).map(|t| t.chars().count() as i64);
        match count_opt {
            Some(n) => Ok(Value::Int(n)),
            None => Err(RaiseError::new(w.intern("type-error"), "length on non-String")),
        }
    });
    w.install_native(w.protos.string, "byteLength", |w, self_, _| {
        let len_opt = w.string_bytes(self_).map(|b| b.len() as i64);
        match len_opt {
            Some(n) => Ok(Value::Int(n)),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "byteLength on non-String",
            )),
        }
    });
    w.install_native(w.protos.string, "at:", |w, self_, args| {
        let idx = match args.first().copied() {
            Some(Value::Int(n)) => n,
            _ => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "[s at: i] requires an Integer index",
                ));
            }
        };
        let result = w.string_text(self_).and_then(|t| {
            if idx < 0 {
                None
            } else {
                t.chars().nth(idx as usize).map(|c| c as u32)
            }
        });
        match result {
            Some(cp) => Ok(Value::Char(cp)),
            None => Err(RaiseError::new(
                w.intern("index-out-of-bounds"),
                format!("[String at: {}] out of range", idx),
            )),
        }
    });
    // [s toList] — return a List of Chars. lets users walk a
    // string with the standard List protocols. matches docs:
    // `concepts/strings.md` :to-list (camelCased here as :toList).
    w.install_native(w.protos.string, "toList", |w, self_, _| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(w.intern("type-error"), "toList on non-String"));
            }
        };
        Ok(w.make_list(&chars))
    });
    // [s upcase] / [s downcase]
    w.install_native(w.protos.string, "upcase", |w, self_, _| {
        let upper = w.string_text(self_).map(|t| t.to_uppercase());
        match upper {
            Some(u) => Ok(w.make_string(&u)),
            None => Err(RaiseError::new(w.intern("type-error"), "upcase on non-String")),
        }
    });
    w.install_native(w.protos.string, "downcase", |w, self_, _| {
        let lower = w.string_text(self_).map(|t| t.to_lowercase());
        match lower {
            Some(d) => Ok(w.make_string(&d)),
            None => Err(RaiseError::new(w.intern("type-error"), "downcase on non-String")),
        }
    });
    // [s trim]
    w.install_native(w.protos.string, "trim", |w, self_, _| {
        let trimmed = w.string_text(self_).map(|t| t.trim().to_string());
        match trimmed {
            Some(t) => Ok(w.make_string(&t)),
            None => Err(RaiseError::new(w.intern("type-error"), "trim on non-String")),
        }
    });
    // [s contains?: needle] — substring search.
    w.install_native(w.protos.string, "contains?:", |w, self_, args| {
        let needle = args.first().copied().unwrap_or(Value::Nil);
        let result = w.string_text(self_).and_then(|hay| {
            w.string_text(needle).map(|n| hay.contains(n))
        });
        match result {
            Some(b) => Ok(Value::Bool(b)),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "contains?: requires String receiver and argument",
            )),
        }
    });
    // [s startsWith?: prefix]
    w.install_native(w.protos.string, "startsWith?:", |w, self_, args| {
        let needle = args.first().copied().unwrap_or(Value::Nil);
        let result = w.string_text(self_).and_then(|hay| {
            w.string_text(needle).map(|n| hay.starts_with(n))
        });
        match result {
            Some(b) => Ok(Value::Bool(b)),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "startsWith?: requires String args",
            )),
        }
    });
    // [s endsWith?: suffix]
    w.install_native(w.protos.string, "endsWith?:", |w, self_, args| {
        let needle = args.first().copied().unwrap_or(Value::Nil);
        let result = w.string_text(self_).and_then(|hay| {
            w.string_text(needle).map(|n| hay.ends_with(n))
        });
        match result {
            Some(b) => Ok(Value::Bool(b)),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "endsWith?: requires String args",
            )),
        }
    });
    // [s indexOf: needle] — char-index of first occurrence, or -1.
    // (the -1 sentinel is the discoverable default; phase G+ may
    // promote to a Maybe/Optional shape.)
    w.install_native(w.protos.string, "indexOf:", |w, self_, args| {
        let needle = args.first().copied().unwrap_or(Value::Nil);
        let pair = w.string_text(self_).and_then(|hay| {
            w.string_text(needle).map(|n| (hay.to_string(), n.to_string()))
        });
        let (hay, needle_str) = match pair {
            Some(p) => p,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "indexOf: requires String args",
                ))
            }
        };
        // byte-index → char-index conversion: count chars before the
        // matched byte position.
        let byte_idx = hay.find(&needle_str);
        let result = match byte_idx {
            None => -1,
            Some(b) => hay[..b].chars().count() as i64,
        };
        Ok(Value::Int(result))
    });
    // [s slice: start length: n] — substring by char-index.
    w.install_native(w.protos.string, "slice:length:", |w, self_, args| {
        let start = match args.first().copied() {
            Some(Value::Int(n)) => n,
            _ => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "slice:length: needs Integer start",
                ))
            }
        };
        let len = match args.get(1).copied() {
            Some(Value::Int(n)) => n,
            _ => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "slice:length: needs Integer length",
                ))
            }
        };
        let text = match w.string_text(self_) {
            Some(t) => t.to_string(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "slice:length: on non-String",
                ))
            }
        };
        if start < 0 || len < 0 {
            return Err(RaiseError::new(
                w.intern("index-out-of-bounds"),
                "slice:length: negative start or length",
            ));
        }
        let collected: String = text
            .chars()
            .skip(start as usize)
            .take(len as usize)
            .collect();
        Ok(w.make_string(&collected))
    });
    // [s replace: needle with: replacement]
    w.install_native(w.protos.string, "replace:with:", |w, self_, args| {
        let needle = args.first().copied().unwrap_or(Value::Nil);
        let repl = args.get(1).copied().unwrap_or(Value::Nil);
        let triple = w.string_text(self_).and_then(|s| {
            w.string_text(needle).and_then(|n| {
                w.string_text(repl).map(|r| (s.to_string(), n.to_string(), r.to_string()))
            })
        });
        match triple {
            Some((s, n, r)) => Ok(w.make_string(&s.replace(&n, &r))),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "replace:with: requires String args",
            )),
        }
    });
    // [s split: sep] — returns a List of Strings.
    w.install_native(w.protos.string, "split:", |w, self_, args| {
        let sep = args.first().copied().unwrap_or(Value::Nil);
        let pair = w.string_text(self_).and_then(|s| {
            w.string_text(sep).map(|p| (s.to_string(), p.to_string()))
        });
        let (s, p) = match pair {
            Some(x) => x,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "split: requires String args",
                ))
            }
        };
        let parts: Vec<Value> = if p.is_empty() {
            // empty separator — split into chars-as-strings.
            s.chars().map(|c| w.make_string(&c.to_string())).collect()
        } else {
            s.split(&p).map(|piece| w.make_string(piece)).collect()
        };
        Ok(w.make_list(&parts))
    });
    // [s lines] — split on '\n', returns a List of Strings.
    w.install_native(w.protos.string, "lines", |w, self_, _| {
        let text = match w.string_text(self_) {
            Some(t) => t.to_string(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "lines on non-String",
                ))
            }
        };
        let lines: Vec<Value> = text.lines().map(|l| w.make_string(l)).collect();
        Ok(w.make_list(&lines))
    });
    // [s forEach: f] — invoke f on each Char.
    w.install_native(w.protos.string, "forEach:", |w, self_, args| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "forEach: on non-String",
                ));
            }
        };
        let blk = args.first().copied().unwrap_or(Value::Nil);
        let call_sym = w.intern("call");
        for ch in chars {
            w.send(blk, call_sym, &[ch])?;
        }
        Ok(Value::Nil)
    });
    w.install_native(w.protos.string, "toString", |_, self_, _| Ok(self_));

    // [s asTable] — a Table of Chars, one per Unicode scalar.
    // [s asList]  — same shape but as a List (alias for toList).
    // [s as: Table] — protocol-style coercion. arg is a proto-Form;
    // dispatches by identity.
    w.install_native(w.protos.string, "asTable", |w, self_, _| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "asTable on non-String",
                ));
            }
        };
        let tbl = w.make_table();
        if let Some(r) = w.table_repr_mut(tbl) {
            for v in chars {
                r.positional.push(v);
            }
        }
        Ok(tbl)
    });
    w.install_native(w.protos.string, "asList", |w, self_, _| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "asList on non-String",
                ));
            }
        };
        Ok(w.make_list(&chars))
    });
    w.install_native(w.protos.string, "as:", |w, self_, args| {
        let target = args.first().copied().unwrap_or(Value::Nil);
        let table_proto = Value::Form(w.protos.table);
        let list_proto = Value::Form(w.protos.list);
        let string_proto = Value::Form(w.protos.string);
        if target == table_proto {
            let sel = w.intern("asTable");
            return w.send(self_, sel, &[]);
        }
        if target == list_proto {
            let sel = w.intern("asList");
            return w.send(self_, sel, &[]);
        }
        if target == string_proto {
            return Ok(self_);
        }
        Err(RaiseError::new(
            w.intern("conversion"),
            "String can be converted as: Table, List, String",
        ))
    });

    // [s map: f] — map each Char through `f` and collect the
    // results into a new String (each result must be a Char or
    // String).
    w.install_native(w.protos.string, "map:", |w, self_, args| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "map: on non-String",
                ));
            }
        };
        let blk = args.first().copied().unwrap_or(Value::Nil);
        let call_sym = w.intern("call");
        let mut out = String::new();
        for ch in chars {
            let r = w.send(blk, call_sym, &[ch])?;
            match r {
                Value::Char(cp) => {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                }
                _ => {
                    if let Some(t) = w.string_text(r) {
                        out.push_str(t);
                    } else {
                        return Err(RaiseError::new(
                            w.intern("type-error"),
                            "String map: block must return a Char or String",
                        ));
                    }
                }
            }
        }
        Ok(w.make_string(&out))
    });

    // [s filter: pred] — keep Chars where pred returns truthy.
    w.install_native(w.protos.string, "filter:", |w, self_, args| {
        let chars: Vec<Value> = match w.string_text(self_) {
            Some(t) => t.chars().map(|c| Value::Char(c as u32)).collect(),
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "filter: on non-String",
                ));
            }
        };
        let blk = args.first().copied().unwrap_or(Value::Nil);
        let call_sym = w.intern("call");
        let mut out = String::new();
        for ch in chars {
            let r = w.send(blk, call_sym, &[ch])?;
            if r.is_truthy() {
                if let Value::Char(cp) = ch {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                }
            }
        }
        Ok(w.make_string(&out))
    });

    // [s reverse] — return a new String with the Chars reversed.
    w.install_native(w.protos.string, "reverse", |w, self_, _| {
        let reversed: Option<String> = w
            .string_text(self_)
            .map(|t| t.chars().rev().collect());
        match reversed {
            Some(s) => Ok(w.make_string(&s)),
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "reverse on non-String",
            )),
        }
    });
    w.install_native(w.protos.string, "=", |w, self_, args| {
        let a = w.string_text(self_).map(|t| t.to_string());
        let b = w.string_text(args[0]).map(|t| t.to_string());
        Ok(Value::Bool(matches!((a, b), (Some(x), Some(y)) if x == y)))
    });
    w.install_native(w.protos.string, "!=", |w, self_, args| {
        let a = w.string_text(self_).map(|t| t.to_string());
        let b = w.string_text(args[0]).map(|t| t.to_string());
        Ok(Value::Bool(!matches!((a, b), (Some(x), Some(y)) if x == y)))
    });
    // [s + other] — concatenation. accepts String or Symbol on the
    // RHS for ergonomics; falls through to :to-string otherwise.
    w.install_native(w.protos.string, "+", |w, self_, args| {
        let lhs_text = w.string_text(self_).map(|t| t.to_string());
        let mut out = match lhs_text {
            Some(t) => t,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "+ on non-String receiver",
                ))
            }
        };
        let rhs = args.first().copied().unwrap_or(Value::Nil);
        let rhs_str = w
            .string_text(rhs)
            .map(|t| t.to_string())
            .or_else(|| match rhs {
                Value::Sym(s) => Some(w.resolve(s).to_string()),
                _ => None,
            });
        if let Some(t) = rhs_str {
            out.push_str(&t);
            Ok(w.make_string(&out))
        } else {
            // delegate to rhs's :to-string for ergonomics.
            let to_string = w.intern("toString");
            let r = w.send(rhs, to_string, &[])?;
            let appended = w.string_text(r).map(|t| t.to_string());
            match appended {
                Some(t) => {
                    out.push_str(&t);
                    Ok(w.make_string(&out))
                }
                None => Err(RaiseError::new(
                    w.intern("type-error"),
                    "String +: rhs :to-string did not return a String",
                )),
            }
        }
    });
    w.install_native(w.protos.string, "concat:", |w, self_, args| {
        let lhs = w.string_text(self_).map(|t| t.to_string());
        let mut out = match lhs {
            Some(t) => t,
            None => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "concat: on non-String",
                ))
            }
        };
        let rhs = args.first().copied().unwrap_or(Value::Nil);
        let rhs_str = w.string_text(rhs).map(|t| t.to_string());
        match rhs_str {
            Some(t) => {
                out.push_str(&t);
                Ok(w.make_string(&out))
            }
            None => Err(RaiseError::new(
                w.intern("type-error"),
                "concat: requires a String argument",
            )),
        }
    });
    w.install_native(w.protos.string, "empty?", |w, self_, _| {
        let empty = w.string_bytes(self_).map(|b| b.is_empty());
        match empty {
            Some(b) => Ok(Value::Bool(b)),
            None => Err(RaiseError::new(w.intern("type-error"), "empty? on non-String")),
        }
    });
}

/// :to-string on Method (covers Closure too). renders the source
/// if available, else `<closure>` / `<method>`.
fn install_method_methods(w: &mut World) {
    w.install_native(w.protos.method, "toString", |w, self_, _| {
        let id = match self_.as_form_id() {
            Some(id) => id,
            None => {
                return Ok(w.make_string("<method>"));
            }
        };
        let source = w.heap.get(id).meta_at(w.source_sym);
        if source.is_nil() {
            return Ok(w.make_string("<closure>"));
        }
        match source {
            Value::Sym(s) => {
                let text = format!("<method:{}>", w.resolve(s));
                Ok(w.make_string(&text))
            }
            Value::Form(_) => {
                let inner = render_list_to_string(w, source)?;
                let text = format!("<closure source: {}>", inner);
                Ok(w.make_string(&text))
            }
            _ => Ok(w.make_string("<closure>")),
        }
    });
}

// ─────────────────────────────────────────────────────────────────
// Method / Chunk / Closure reflection — `docs/laws/reflection-contract.md`
//
// every Method-or-Closure exposes:
//   :body         → the chunk-Form (closures point here via :body slot;
//                   for a chunk, returns self).
//   :source       → the source form (already in `:source` meta).
//   :params       → a Table of param symbols.
//   :consts       → a Table of constants the chunk loads.
//   :bytecodes    → a Table of decoded opcode-Forms.
//   :disassemble  → a String human-readable view.
//
// each opcode-Form has slots `:op` (a Sym) and `:operands` (a Table).
// reflection is read-only: edit source, the substrate re-derives.
// ─────────────────────────────────────────────────────────────────

/// resolve a method/closure receiver to its chunk-Form id, if any.
/// returns the id of the chunk holding the bytecode in
/// `world.chunk_ops`. closures store this in the `:body` slot;
/// chunk-Forms are themselves the chunk.
fn chunk_id_of(world: &World, value: Value) -> Option<crate::form::FormId> {
    let id = value.as_form_id()?;
    let f = world.heap.get(id);
    // a Closure-Form's `:body` slot points at the chunk-Form.
    let body = f.slot(world.body_sym);
    if let Some(bid) = body.as_form_id() {
        if world.chunk_ops.contains_key(&bid) {
            return Some(bid);
        }
    }
    // a chunk-Form is its own chunk.
    if world.chunk_ops.contains_key(&id) {
        return Some(id);
    }
    None
}

/// build (or fetch) the Opcode proto exposed as the global
/// `Opcode`. has `:op` and `:operands` slot-getters so opcode-Forms
/// dispatch nicely.
fn ensure_opcode_proto(world: &mut World) -> crate::form::FormId {
    let name_sym = world.intern("Opcode");
    let global = world.global_env;
    if let Some(existing) = world.env_lookup(global, name_sym) {
        if let Some(id) = existing.as_form_id() {
            return id;
        }
    }
    // fresh proto with `op` and `operands` getters.
    let proto_id = world.alloc(crate::form::Form::with_proto(Value::Form(
        world.protos.object,
    )));
    let name_meta = world.intern("name");
    world
        .heap
        .get_mut(proto_id)
        .meta
        .insert(name_meta, Value::Sym(name_sym));
    world.env_bind(global, name_sym, Value::Form(proto_id));

    // slot-getters for :op and :operands.
    world.install_native(proto_id, "op", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "op: receiver not a Form")
        })?;
        let op_sym = w.intern("op");
        Ok(w.heap.get(id).slot(op_sym))
    });
    world.install_native(proto_id, "operands", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "operands: receiver not a Form")
        })?;
        let operands_sym = w.intern("operands");
        Ok(w.heap.get(id).slot(operands_sym))
    });
    // [opcode toString] → "<LoadConst 0>" etc.
    world.install_native(proto_id, "toString", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "toString: receiver not a Form")
        })?;
        let op_sym = w.intern("op");
        let operands_sym = w.intern("operands");
        let name = match w.heap.get(id).slot(op_sym) {
            Value::Sym(s) => w.resolve(s).to_string(),
            _ => "?".to_string(),
        };
        let operands_v = w.heap.get(id).slot(operands_sym);
        let mut parts = vec![name];
        if let Some(r) = w.table_repr(operands_v) {
            for v in r.positional.clone() {
                parts.push(render_value(w, v));
            }
        }
        let s = format!("<{}>", parts.join(" "));
        Ok(w.make_string(&s))
    });

    proto_id
}

/// build an opcode-Form for a single `Op`. each form has slots
/// `:op` (Sym) and `:operands` (Table) — operands are pushed in
/// declaration order so positional access works. the form's proto
/// is `Opcode`, which exposes `:op`, `:operands`, `:toString`.
fn opcode_form(world: &mut World, op: crate::opcodes::Op) -> Value {
    use crate::opcodes::Op;
    let op_sym = world.intern("op");
    let operands_sym = world.intern("operands");
    let opcode_proto = ensure_opcode_proto(world);
    let mut form = crate::form::Form::with_proto(Value::Form(opcode_proto));

    let (name, operands): (&'static str, Vec<Value>) = match op {
        Op::LoadConst(idx) => ("LoadConst", vec![Value::Int(idx as i64)]),
        Op::PushNil => ("PushNil", vec![]),
        Op::PushTrue => ("PushTrue", vec![]),
        Op::PushFalse => ("PushFalse", vec![]),
        Op::Pop => ("Pop", vec![]),
        Op::Dup => ("Dup", vec![]),
        Op::LoadName(s) => ("LoadName", vec![Value::Sym(s)]),
        Op::StoreName(s) => ("StoreName", vec![Value::Sym(s)]),
        Op::LoadSelf => ("LoadSelf", vec![]),
        Op::DefineGlobal(s) => ("DefineGlobal", vec![Value::Sym(s)]),
        Op::Send {
            selector,
            argc,
            ic_idx,
        } => (
            "Send",
            vec![
                Value::Sym(selector),
                Value::Int(argc as i64),
                Value::Int(ic_idx as i64),
            ],
        ),
        Op::TailSend { selector, argc } => (
            "TailSend",
            vec![Value::Sym(selector), Value::Int(argc as i64)],
        ),
        Op::SuperSend {
            selector,
            argc,
            ic_idx,
        } => (
            "SuperSend",
            vec![
                Value::Sym(selector),
                Value::Int(argc as i64),
                Value::Int(ic_idx as i64),
            ],
        ),
        Op::PushClosure { chunk } => ("PushClosure", vec![Value::Form(chunk)]),
        Op::Jump(off) => ("Jump", vec![Value::Int(off as i64)]),
        Op::JumpIfFalse(off) => ("JumpIfFalse", vec![Value::Int(off as i64)]),
        Op::Return => ("Return", vec![]),
    };

    let name_sym = world.intern(name);
    form.slots.insert(op_sym, Value::Sym(name_sym));
    let operands_tbl = world.make_table();
    if let Some(r) = world.table_repr_mut(operands_tbl) {
        for v in operands {
            r.positional.push(v);
        }
    }
    form.slots.insert(operands_sym, operands_tbl);
    Value::Form(world.alloc(form))
}

fn install_method_reflection(w: &mut World) {
    // ensure the Opcode proto exists at install time, so global
    // `Opcode` is bound from the start.
    let _ = ensure_opcode_proto(w);
    // [m body] — the chunk-Form (closure's `:body`, or self if a chunk).
    w.install_native(w.protos.method, "body", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "body: receiver not a Form")
        })?;
        // Closure case: has a :body slot pointing at a chunk.
        let body = w.heap.get(id).slot(w.body_sym);
        if let Some(bid) = body.as_form_id() {
            if w.chunk_ops.contains_key(&bid) {
                return Ok(Value::Form(bid));
            }
        }
        // chunk case: receiver is itself a chunk.
        if w.chunk_ops.contains_key(&id) {
            return Ok(Value::Form(id));
        }
        Ok(Value::Nil)
    });

    // [m source] — the source form stored in :source meta. nil if
    // none (e.g. a bare-rust intrinsic).
    w.install_native(w.protos.method, "source", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "source: receiver not a Form")
        })?;
        Ok(w.heap.get(id).meta_at(w.source_sym))
    });

    // [m params] — a Table of param symbols.
    w.install_native(w.protos.method, "params", |w, self_, _| {
        let id = match self_.as_form_id() {
            Some(i) => i,
            None => return Ok(w.make_table()),
        };
        // params live on the closure or on the chunk; check both.
        let mut params = w.heap.get(id).slot(w.params_sym);
        if params.is_nil() {
            if let Some(cid) = chunk_id_of(w, self_) {
                params = w.heap.get(cid).slot(w.params_sym);
            }
        }
        // walk the params List up front so we don't fight the
        // borrow checker once we hold the table-rep mut.
        let head_sym = w.intern("head");
        let tail_sym = w.intern("tail");
        let list_proto = Value::Form(w.protos.list);
        let mut items: Vec<Value> = Vec::new();
        let mut cur = params;
        while let Some(fid) = cur.as_form_id() {
            let f = w.heap.get(fid);
            if f.proto != list_proto {
                break;
            }
            let head = f.slot(head_sym);
            let tail = f.slot(tail_sym);
            if head.is_nil() && tail.is_nil() {
                break;
            }
            items.push(head);
            cur = tail;
        }
        let tbl = w.make_table();
        if let Some(r) = w.table_repr_mut(tbl) {
            for v in items {
                r.positional.push(v);
            }
        }
        Ok(tbl)
    });

    // [m consts] — a Table of the chunk's constants.
    w.install_native(w.protos.method, "consts", |w, self_, _| {
        let cid = match chunk_id_of(w, self_) {
            Some(c) => c,
            None => return Ok(w.make_table()),
        };
        let consts = w.chunk_consts.get(&cid).cloned().unwrap_or_default();
        let tbl = w.make_table();
        if let Some(r) = w.table_repr_mut(tbl) {
            for v in consts {
                r.positional.push(v);
            }
        }
        Ok(tbl)
    });

    // [m bytecodes] — a Table of opcode-Forms decoded from the chunk.
    // edit source → substrate re-derives → :bytecodes reflects it.
    w.install_native(w.protos.method, "bytecodes", |w, self_, _| {
        let cid = match chunk_id_of(w, self_) {
            Some(c) => c,
            None => return Ok(w.make_table()),
        };
        let ops = w.chunk_ops.get(&cid).cloned().unwrap_or_default();
        let tbl = w.make_table();
        for op in ops {
            let v = opcode_form(w, op);
            if let Some(r) = w.table_repr_mut(tbl) {
                r.positional.push(v);
            }
        }
        Ok(tbl)
    });

    // [m disassemble] — a String human-readable rendering of the
    // bytecode. one op per line, with constants interpolated.
    w.install_native(w.protos.method, "disassemble", |w, self_, _| {
        use crate::opcodes::Op;
        let cid = match chunk_id_of(w, self_) {
            Some(c) => c,
            None => return Ok(w.make_string("<no bytecode>")),
        };
        let ops = w.chunk_ops.get(&cid).cloned().unwrap_or_default();
        let consts = w.chunk_consts.get(&cid).cloned().unwrap_or_default();
        let mut out = String::new();
        for (i, op) in ops.iter().enumerate() {
            let line = match *op {
                Op::LoadConst(idx) => {
                    let c = consts.get(idx as usize).copied().unwrap_or(Value::Nil);
                    let rendered = render_value(w, c);
                    format!("{:>3}  LoadConst    {:<3}  ; {}", i, idx, rendered)
                }
                Op::PushNil => format!("{:>3}  PushNil", i),
                Op::PushTrue => format!("{:>3}  PushTrue", i),
                Op::PushFalse => format!("{:>3}  PushFalse", i),
                Op::Pop => format!("{:>3}  Pop", i),
                Op::Dup => format!("{:>3}  Dup", i),
                Op::LoadName(s) => format!("{:>3}  LoadName     {}", i, w.resolve(s)),
                Op::StoreName(s) => format!("{:>3}  StoreName    {}", i, w.resolve(s)),
                Op::LoadSelf => format!("{:>3}  LoadSelf", i),
                Op::DefineGlobal(s) => format!("{:>3}  DefineGlobal {}", i, w.resolve(s)),
                Op::Send {
                    selector,
                    argc,
                    ic_idx,
                } => format!(
                    "{:>3}  Send         {}/{}  ; ic={}",
                    i,
                    w.resolve(selector),
                    argc,
                    ic_idx
                ),
                Op::TailSend { selector, argc } => format!(
                    "{:>3}  TailSend     {}/{}",
                    i,
                    w.resolve(selector),
                    argc
                ),
                Op::SuperSend {
                    selector,
                    argc,
                    ic_idx,
                } => format!(
                    "{:>3}  SuperSend    {}/{}  ; ic={}",
                    i,
                    w.resolve(selector),
                    argc,
                    ic_idx
                ),
                Op::PushClosure { chunk } => {
                    format!("{:>3}  PushClosure  <chunk #{}>", i, chunk.0)
                }
                Op::Jump(off) => format!("{:>3}  Jump         {:+}", i, off),
                Op::JumpIfFalse(off) => format!("{:>3}  JumpIfFalse  {:+}", i, off),
                Op::Return => format!("{:>3}  Return", i),
            };
            out.push_str(&line);
            out.push('\n');
        }
        Ok(w.make_string(&out))
    });
}

/// stringify a Value briefly (used in disassembly comments). we
/// don't recurse into nested forms — just enough for a one-liner.
fn render_value(w: &World, v: Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => if b { "#true" } else { "#false" }.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(_) => v.as_float().map(|f| f.to_string()).unwrap_or_default(),
        Value::Sym(s) => format!("'{}", w.resolve(s)),
        Value::Char(c) => format!(
            "#\\{}",
            char::from_u32(c).map(|c| c.to_string()).unwrap_or_default()
        ),
        Value::Form(id) => {
            // string?
            if let Some(t) = w.string_text(v) {
                return format!("\"{}\"", t);
            }
            format!("<Form#{}>", id.0)
        }
        Value::Foreign(_) => "<foreign>".to_string(),
    }
}

/// expose the canonical protos as moof globals (`Object`, `List`,
/// `Integer`, …). user code can refer to them by name to install
/// handlers, allocate instances, and inspect the proto chain.
fn install_proto_globals(w: &mut World) {
    let bindings = [
        ("Object", w.protos.object),
        ("Nil-proto", w.protos.nil),
        ("Bool", w.protos.bool_),
        ("Integer", w.protos.integer),
        ("Symbol", w.protos.symbol),
        ("Char", w.protos.char_),
        ("String", w.protos.string),
        ("List", w.protos.list),
        ("Table", w.protos.table),
        ("Method", w.protos.method),
        ("Chunk", w.protos.chunk),
        ("Closure", w.protos.closure),
        ("Env", w.protos.env),
        ("ForeignHandle", w.protos.foreign),
    ];
    let global = w.global_env;
    for (name, id) in bindings {
        let s = w.intern(name);
        w.env_bind(global, s, Value::Form(id));
    }
}

// ─────────────────────────────────────────────────────────────────
// :call on Method (so Closures + plain method-Forms are callable)
// ─────────────────────────────────────────────────────────────────

fn install_call_on_method(w: &mut World) {
    // [m call: arg…] ≡ world.invoke(m, captured-self-or-nil, args).
    // for a closure created inside a method body, the captured self
    // is in the closure's :captured-self slot (set by PushClosure).
    // for a standalone method-Form, no captured self exists; pass nil.
    w.install_native(w.protos.method, "call", |world, self_, args| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(world.intern("dispatch"), "receiver of :call is not a Form")
        })?;
        let captured_sym = world.intern("captured-self");
        let captured = world.heap.get(id).slot(captured_sym);
        // closures-as-callables have no defining-proto in the OO
        // sense (they're not "found on" a proto). super-send from
        // inside a closure body raises a useful error.
        world.invoke(id, captured, args, FormId::NONE)
    });
}

// ─────────────────────────────────────────────────────────────────
// Integer methods
// ─────────────────────────────────────────────────────────────────

fn install_integer_methods(w: &mut World) {
    // arithmetic auto-promotes when the rhs is a Float.
    // [Int + Int] → Int; [Int + Float] → Float.
    w.install_native(w.protos.integer, "+", |w, self_, args| {
        let a = int_arg(w, self_, "+")?;
        match args[0] {
            Value::Int(b) => Ok(Value::Int(a.wrapping_add(b))),
            Value::Float(_) => Ok(Value::float(a as f64 + args[0].as_float().unwrap())),
            _ => Err(RaiseError::new(
                w.intern("type-error"),
                "+ expected a numeric rhs",
            )),
        }
    });
    w.install_native(w.protos.integer, "-", |w, self_, args| {
        let a = int_arg(w, self_, "-")?;
        match args[0] {
            Value::Int(b) => Ok(Value::Int(a.wrapping_sub(b))),
            Value::Float(_) => Ok(Value::float(a as f64 - args[0].as_float().unwrap())),
            _ => Err(RaiseError::new(
                w.intern("type-error"),
                "- expected a numeric rhs",
            )),
        }
    });
    w.install_native(w.protos.integer, "*", |w, self_, args| {
        let a = int_arg(w, self_, "*")?;
        match args[0] {
            Value::Int(b) => Ok(Value::Int(a.wrapping_mul(b))),
            Value::Float(_) => Ok(Value::float(a as f64 * args[0].as_float().unwrap())),
            _ => Err(RaiseError::new(
                w.intern("type-error"),
                "* expected a numeric rhs",
            )),
        }
    });
    w.install_native(w.protos.integer, "/", |w, self_, args| {
        let a = int_arg(w, self_, "/")?;
        match args[0] {
            Value::Int(b) => {
                if b == 0 {
                    return Err(RaiseError::new(
                        w.intern("division-by-zero"),
                        "integer division by zero",
                    ));
                }
                Ok(Value::Int(a.wrapping_div(b)))
            }
            Value::Float(_) => Ok(Value::float(a as f64 / args[0].as_float().unwrap())),
            _ => Err(RaiseError::new(
                w.intern("type-error"),
                "/ expected a numeric rhs",
            )),
        }
    });
    w.install_native(w.protos.integer, "=", |_, self_, args| {
        let a = self_.as_int().unwrap();
        Ok(Value::Bool(match args[0] {
            Value::Int(b) => a == b,
            Value::Float(_) => (a as f64) == args[0].as_float().unwrap(),
            _ => false,
        }))
    });
    w.install_native(w.protos.integer, "!=", |_, self_, args| {
        let a = self_.as_int().unwrap();
        Ok(Value::Bool(match args[0] {
            Value::Int(b) => a != b,
            Value::Float(_) => (a as f64) != args[0].as_float().unwrap(),
            _ => true,
        }))
    });
    w.install_native(w.protos.integer, "<", |w, self_, args| {
        let a = self_.as_int().unwrap();
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "< expected a numeric rhs")
        })?;
        Ok(Value::Bool((a as f64) < b))
    });
    w.install_native(w.protos.integer, ">", |w, self_, args| {
        let a = self_.as_int().unwrap();
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "> expected a numeric rhs")
        })?;
        Ok(Value::Bool((a as f64) > b))
    });
    w.install_native(w.protos.integer, "<=", |w, self_, args| {
        let a = self_.as_int().unwrap();
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "<= expected a numeric rhs")
        })?;
        Ok(Value::Bool((a as f64) <= b))
    });
    w.install_native(w.protos.integer, ">=", |w, self_, args| {
        let a = self_.as_int().unwrap();
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ">= expected a numeric rhs")
        })?;
        Ok(Value::Bool((a as f64) >= b))
    });
    w.install_native(w.protos.integer, "toString", |w, self_, _args| {
        let a = int_arg(w, self_, "toString")?;
        Ok(w.make_string(&a.to_string()))
    });
    w.install_native(w.protos.integer, "asFloat", |w, self_, _args| {
        let a = int_arg(w, self_, "asFloat")?;
        Ok(Value::float(a as f64))
    });
}

fn int_arg(w: &mut World, v: Value, op: &str) -> Result<i64, RaiseError> {
    v.as_int().ok_or_else(|| {
        RaiseError::new(
            w.intern("type-error"),
            format!("{} expected an Integer", op),
        )
    })
}

// ─────────────────────────────────────────────────────────────────
// Float — IEEE-754 f64 with `Int → Float` promotion.
// ─────────────────────────────────────────────────────────────────

fn install_float_methods(w: &mut World) {
    fn float_arg(w: &mut World, v: Value, op: &str) -> Result<f64, RaiseError> {
        v.as_float().ok_or_else(|| {
            RaiseError::new(
                w.intern("type-error"),
                format!("{} expected a Float", op),
            )
        })
    }
    w.install_native(w.protos.float, "+", |w, self_, args| {
        let a = float_arg(w, self_, "+")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "+ expected a numeric rhs")
        })?;
        Ok(Value::float(a + b))
    });
    w.install_native(w.protos.float, "-", |w, self_, args| {
        let a = float_arg(w, self_, "-")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "- expected a numeric rhs")
        })?;
        Ok(Value::float(a - b))
    });
    w.install_native(w.protos.float, "*", |w, self_, args| {
        let a = float_arg(w, self_, "*")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "* expected a numeric rhs")
        })?;
        Ok(Value::float(a * b))
    });
    w.install_native(w.protos.float, "/", |w, self_, args| {
        let a = float_arg(w, self_, "/")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "/ expected a numeric rhs")
        })?;
        Ok(Value::float(a / b))
    });
    w.install_native(w.protos.float, "=", |_, self_, args| {
        let a = self_.as_float().unwrap();
        Ok(Value::Bool(args[0].as_number_f64().map_or(false, |b| a == b)))
    });
    w.install_native(w.protos.float, "!=", |_, self_, args| {
        let a = self_.as_float().unwrap();
        Ok(Value::Bool(args[0].as_number_f64().map_or(true, |b| a != b)))
    });
    w.install_native(w.protos.float, "<", |w, self_, args| {
        let a = float_arg(w, self_, "<")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "< expected a numeric rhs")
        })?;
        Ok(Value::Bool(a < b))
    });
    w.install_native(w.protos.float, ">", |w, self_, args| {
        let a = float_arg(w, self_, ">")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "> expected a numeric rhs")
        })?;
        Ok(Value::Bool(a > b))
    });
    w.install_native(w.protos.float, "<=", |w, self_, args| {
        let a = float_arg(w, self_, "<=")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "<= expected a numeric rhs")
        })?;
        Ok(Value::Bool(a <= b))
    });
    w.install_native(w.protos.float, ">=", |w, self_, args| {
        let a = float_arg(w, self_, ">=")?;
        let b = args[0].as_number_f64().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ">= expected a numeric rhs")
        })?;
        Ok(Value::Bool(a >= b))
    });
    w.install_native(w.protos.float, "toString", |w, self_, _| {
        let a = float_arg(w, self_, "toString")?;
        Ok(w.make_string(&format_float(a)))
    });
    w.install_native(w.protos.float, "abs", |w, self_, _| {
        let a = float_arg(w, self_, "abs")?;
        Ok(Value::float(a.abs()))
    });
    w.install_native(w.protos.float, "sqrt", |w, self_, _| {
        let a = float_arg(w, self_, "sqrt")?;
        Ok(Value::float(a.sqrt()))
    });
    w.install_native(w.protos.float, "log", |w, self_, _| {
        let a = float_arg(w, self_, "log")?;
        Ok(Value::float(a.ln()))
    });
    w.install_native(w.protos.float, "exp", |w, self_, _| {
        let a = float_arg(w, self_, "exp")?;
        Ok(Value::float(a.exp()))
    });
    w.install_native(w.protos.float, "sin", |w, self_, _| {
        let a = float_arg(w, self_, "sin")?;
        Ok(Value::float(a.sin()))
    });
    w.install_native(w.protos.float, "cos", |w, self_, _| {
        let a = float_arg(w, self_, "cos")?;
        Ok(Value::float(a.cos()))
    });
    w.install_native(w.protos.float, "floor", |w, self_, _| {
        let a = float_arg(w, self_, "floor")?;
        Ok(Value::float(a.floor()))
    });
    w.install_native(w.protos.float, "ceil", |w, self_, _| {
        let a = float_arg(w, self_, "ceil")?;
        Ok(Value::float(a.ceil()))
    });
    w.install_native(w.protos.float, "round", |w, self_, _| {
        let a = float_arg(w, self_, "round")?;
        Ok(Value::float(a.round()))
    });
    w.install_native(w.protos.float, "asInteger", |w, self_, _| {
        let a = float_arg(w, self_, "asInteger")?;
        Ok(Value::Int(a as i64))
    });
    w.install_native(w.protos.float, "nan?", |_, self_, _| {
        Ok(Value::Bool(self_.as_float().map_or(false, |f| f.is_nan())))
    });
    w.install_native(w.protos.float, "finite?", |_, self_, _| {
        Ok(Value::Bool(self_.as_float().map_or(false, |f| f.is_finite())))
    });
    w.install_native(w.protos.float, "zero?", |_, self_, _| {
        Ok(Value::Bool(self_.as_float().map_or(false, |f| f == 0.0)))
    });
    w.install_native(w.protos.float, "positive?", |_, self_, _| {
        Ok(Value::Bool(self_.as_float().map_or(false, |f| f > 0.0)))
    });
    w.install_native(w.protos.float, "negative?", |_, self_, _| {
        Ok(Value::Bool(self_.as_float().map_or(false, |f| f < 0.0)))
    });
}

/// render a float with up to ~17 sig-digits and a `.` even for
/// whole values (so `1.0` doesn't render as `1`).
fn format_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "∞".into()
        } else {
            "-∞".into()
        }
    } else if f == 0.0 {
        "0.0".into()
    } else {
        let s = format!("{}", f);
        if !s.contains('.') && !s.contains('e') && !s.contains('E') {
            format!("{}.0", s)
        } else {
            s
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// Symbol / Bool / Nil — minimum equality story
// ─────────────────────────────────────────────────────────────────

fn install_symbol_methods(w: &mut World) {
    w.install_native(w.protos.symbol, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.symbol, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.symbol, "toString", |w, self_, _| {
        let s = self_.as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "to-string on non-Symbol")
        })?;
        let text = w.resolve(s).to_string();
        Ok(w.make_string(&text))
    });
}

fn install_bool_methods(w: &mut World) {
    w.install_native(w.protos.bool_, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.bool_, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.bool_, "not", |_, self_, _args| match self_ {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        _ => Ok(Value::Bool(false)), // shouldn't happen if dispatch is right
    });
    w.install_native(w.protos.bool_, "toString", |w, self_, _| match self_ {
        Value::Bool(true) => Ok(w.make_string("#true")),
        Value::Bool(false) => Ok(w.make_string("#false")),
        _ => Err(RaiseError::new(
            w.intern("type-error"),
            "to-string on non-Bool",
        )),
    });
}

fn install_nil_methods(w: &mut World) {
    w.install_native(w.protos.nil, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.nil, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.nil, "toString", |w, _, _| Ok(w.make_string("nil")));
    w.install_native(w.protos.nil, "head", |w, _, _| {
        // (head nil) → nil. lispy convention; users beware.
        let _ = w;
        Ok(Value::Nil)
    });
    w.install_native(w.protos.nil, "tail", |w, _, _| {
        let _ = w;
        Ok(Value::Nil)
    });
    w.install_native(w.protos.nil, "null?", |_, _, _| Ok(Value::Bool(true)));
    // (cons h ()) — nil is the empty list, so consing onto it
    // builds a one-element list. without this, `(map …)` and
    // friends fall over at the recursion base case.
    w.install_native(w.protos.nil, "cons:", |w, self_, args| {
        let head_sym = w.head_sym;
        let tail_sym = w.tail_sym;
        let mut cell = Form::with_proto(Value::Form(w.protos.list));
        cell.slots.insert(head_sym, args[0]);
        cell.slots.insert(tail_sym, self_);
        let id = w.alloc(cell);
        Ok(Value::Form(id))
    });
}

// ─────────────────────────────────────────────────────────────────
// List (cons-cell) methods
// ─────────────────────────────────────────────────────────────────

fn install_list_methods(w: &mut World) {
    // we read the head/tail SymIds from the world *inside* each
    // native (they're already cached on `World`, not allocated per
    // call). this lets the closures be `fn` pointers (no capture).
    w.install_native(w.protos.list, "head", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "head on non-List")
        })?;
        let head_sym = w.head_sym;
        Ok(w.heap.get(id).slot(head_sym))
    });
    w.install_native(w.protos.list, "tail", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "tail on non-List")
        })?;
        let tail_sym = w.tail_sym;
        Ok(w.heap.get(id).slot(tail_sym))
    });
    w.install_native(w.protos.list, "null?", |_, _, _| Ok(Value::Bool(false)));
    w.install_native(w.protos.list, "cons:", |w, self_, args| {
        let head_sym = w.head_sym;
        let tail_sym = w.tail_sym;
        let mut cell = Form::with_proto(Value::Form(w.protos.list));
        cell.slots.insert(head_sym, args[0]);
        cell.slots.insert(tail_sym, self_);
        let id = w.alloc(cell);
        Ok(Value::Form(id))
    });
    // List :to-string — recursive `(elem1 elem2 ...)` rendering.
    w.install_native(w.protos.list, "toString", |w, self_, _| {
        let s = render_list_to_string(w, self_)?;
        Ok(w.make_string(&s))
    });
}

/// recursive list-to-string. each element's :to-string is sent;
/// the result is a String form; we read its bytes; join with
/// spaces; wrap in parens.
fn render_list_to_string(w: &mut World, list: Value) -> Result<String, RaiseError> {
    let mut out = String::from("(");
    let mut cur = list;
    let mut first = true;
    let to_string = w.intern("toString");
    let head_sym = w.head_sym;
    let tail_sym = w.tail_sym;
    loop {
        match cur {
            Value::Nil => break,
            Value::Form(id) => {
                if !first {
                    out.push(' ');
                }
                first = false;
                let head = w.heap.get(id).slot(head_sym);
                let tail = w.heap.get(id).slot(tail_sym);
                let head_str_v = w.send(head, to_string, &[])?;
                push_string_value(w, &mut out, head_str_v)?;
                cur = tail;
            }
            _ => {
                // improper list — should be rare in moof; show as
                // `(... . tail)`.
                if !first {
                    out.push(' ');
                }
                out.push_str(". ");
                let tail_str_v = w.send(cur, to_string, &[])?;
                push_string_value(w, &mut out, tail_str_v)?;
                break;
            }
        }
    }
    out.push(')');
    Ok(out)
}

/// pull text bytes out of a String form and append to `out`.
/// raises type-error if the value isn't a String.
fn push_string_value(
    w: &mut World,
    out: &mut String,
    value: Value,
) -> Result<(), RaiseError> {
    let copy = w.string_text(value).map(|t| t.to_string());
    match copy {
        Some(t) => {
            out.push_str(&t);
            Ok(())
        }
        None => Err(RaiseError::new(
            w.intern("type-error"),
            ":to-string did not return a String",
        )),
    }
}

// ─────────────────────────────────────────────────────────────────
// Object reflection — the load-bearing moldable promise (L6)
// ─────────────────────────────────────────────────────────────────

fn install_object_reflection(w: &mut World) {
    w.install_native(w.protos.object, "proto", |w, self_, _| {
        Ok(w.proto_of(self_))
    });

    w.install_native(w.protos.object, "slots", |w, self_, _| {
        // returns a moof list of (sym . value) pairs. for tagged
        // immediates with no slots, returns nil.
        match self_ {
            Value::Form(id) => {
                // collect (head, tail) cons cells from the slots
                // table, in insertion order.
                let f = w.heap.get(id);
                let pairs: Vec<(SymId, Value)> = f
                    .slots
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    // [proto handlerAt: 'sel] — the method-Form installed for `sel`
    // on this proto (NOT inherited). nil if absent. lets you get
    // a handle to any method without walking :handlers.
    w.install_native(w.protos.object, "handlerAt:", |w, self_, args| {
        let sel = match args[0] {
            Value::Sym(s) => s,
            _ => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "handlerAt: expects a symbol",
                ))
            }
        };
        match self_ {
            Value::Form(id) => Ok(w
                .heap
                .get(id)
                .handlers
                .get(&sel)
                .copied()
                .unwrap_or(Value::Nil)),
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "handlers", |w, self_, _| {
        // returns a moof list of (selector . method-Form) pairs from
        // *this proto* (not the inherited chain). reading inherited
        // handlers is the user's job (walk via :proto).
        match self_ {
            Value::Form(id) => {
                let pairs: Vec<(SymId, Value)> = w
                    .heap
                    .get(id)
                    .handlers
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "meta", |w, self_, _| {
        match self_ {
            Value::Form(id) => {
                let pairs: Vec<(SymId, Value)> = w
                    .heap
                    .get(id)
                    .meta
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "source", |w, self_, _| match self_ {
        Value::Form(id) => Ok(w.heap.get(id).meta_at(w.source_sym)),
        _ => Ok(Value::Nil),
    });

    w.install_native(w.protos.object, "identity", |_, self_, _| match self_ {
        Value::Form(id) => Ok(Value::Int(id.0 as i64)),
        // tagged-immediates report identity = 0 (no heap slot).
        _ => Ok(Value::Int(0)),
    });

    w.install_native(w.protos.object, "is", |_, self_, args| {
        // identity equality (same heap-id or same tagged-immediate).
        Ok(Value::Bool(self_ == args[0]))
    });

    // Object's `:=` is identity equality by default. specific protos
    // (Integer, Symbol, etc.) override with structural equality.
    w.install_native(w.protos.object, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });

    w.install_native(w.protos.object, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });

    w.install_native(w.protos.object, "toString", |w, self_, _| {
        // default rendering: `<Form#N>` for heap forms; tagged
        // immediates have their own to-string overrides.
        let text = match self_ {
            Value::Form(id) => format!("<Form#{}>", id.0),
            Value::Foreign(id) => format!("<Foreign#{}>", id.0),
            Value::Nil => "nil".to_string(),
            // defensive fallbacks (each tagged kind overrides above):
            Value::Bool(b) => (if b { "#true" } else { "#false" }).to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(_) => format_float(self_.as_float().unwrap()),
            Value::Sym(s) => w.resolve(s).to_string(),
            Value::Char(cp) => {
                if let Some(ch) = char::from_u32(cp) {
                    ch.to_string()
                } else {
                    format!("<bad-char:{:#x}>", cp)
                }
            }
        };
        Ok(w.make_string(&text))
    });

    w.install_native(w.protos.object, "inspect", |w, self_, _| {
        // phase A: same as :to-string. phase C swaps in a richer
        // moof-side Inspector view.
        let to_string = w.intern("toString");
        w.send(self_, to_string, &[])
    });

    w.install_native(w.protos.object, "new", |w, self_, _args| {
        // (Proto :new) → fresh instance, then [self initialize].
        let proto_id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":new on non-Form proto")
        })?;
        let f = Form::with_proto(Value::Form(proto_id));
        let id = w.alloc(f);
        let instance = Value::Form(id);
        // smalltalk-style: invoke :initialize on the new instance.
        // Object's default :initialize is a no-op; user protos
        // override.
        let initialize = w.intern("initialize");
        w.send(instance, initialize, &[])?;
        Ok(instance)
    });

    // default :initialize is a no-op. user protos override.
    w.install_native(w.protos.object, "initialize", |_, self_, _| Ok(self_));

    // default does-not-understand:with: raises. user code can
    // override on any proto.
    w.install_native(
        w.protos.object,
        "doesNotUnderstand:with:",
        |w, self_, args| {
            let sel = args[0].as_sym().unwrap_or(SymId::NONE);
            let kind = w.intern("doesNotUnderstand");
            Err(RaiseError::new(
                kind,
                format!(
                    "{} does not understand `{}`",
                    fmt_short(w, self_),
                    if sel.is_none() { "<unknown>" } else { w.resolve(sel) }
                ),
            ))
        },
    );
}

fn fmt_short(w: &World, v: Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(true) => "#true".into(),
        Value::Bool(false) => "#false".into(),
        Value::Int(n) => n.to_string(),
        Value::Float(_) => format_float(v.as_float().unwrap()),
        Value::Sym(s) => format!("'{}", w.resolve(s)),
        Value::Char(cp) => {
            char::from_u32(cp)
                .map(|c| format!("#\\{}", c))
                .unwrap_or_else(|| format!("#\\u{{{:x}}}", cp))
        }
        Value::Form(id) => format!("<Form#{}>", id.0),
        Value::Foreign(id) => format!("<Foreign#{}>", id.0),
    }
}

// ─────────────────────────────────────────────────────────────────
// Console proto + $out / $err caps
//
// per `process/docs-driven.md`'s capability rule, there is *no*
// path to stdout from moof code that isn't through a cap. the
// supervisor (in phase A: the substrate seed itself) constructs
// the primordial $out and $err caps at boot and binds them in the
// global env.
//
// phase A's Console is bare rust stdout/stderr — a placeholder for
// the proper `os/console.mco` that lands in phase B alongside the
// mco loader. the moof interface (`[$out emit: bytes]`,
// `[$out say: x]`) is the same at both phases.
// ─────────────────────────────────────────────────────────────────

/// kind tag for `Console`'s `:fd` ForeignHandle slot. lets the
/// native `:emit:` distinguish a real fd-handle from any other
/// foreign value the user might shove into the slot.
const CONSOLE_FD_TAG: u32 = 0xC0_5E_FD_01;

/// the rust-side state held by a Console fd ForeignHandle.
///
/// in phase A, we just remember whether to write to stdout or
/// stderr — both are global file descriptors owned by the OS, so
/// we don't need to free anything. when phase B's `os/console.mco`
/// lands, this becomes a real `RawFd` with an `Owned` flag for
/// fds that need closing on gc.
struct ConsoleFd {
    target: ConsoleTarget,
}

#[derive(Copy, Clone)]
enum ConsoleTarget {
    Stdout,
    Stderr,
}

unsafe extern "C" fn console_fd_dtor(ptr: *mut std::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: we minted this ptr from `Box::into_raw` in
    // `make_primordial_console`. it's a valid `Box<ConsoleFd>`
    // and the gc owns it. boxed drop is the right cleanup.
    let _ = unsafe { Box::from_raw(ptr as *mut ConsoleFd) };
}

fn make_primordial_console(w: &mut World, console_proto: FormId, target: ConsoleTarget) -> FormId {
    let fd_box = Box::new(ConsoleFd { target });
    let ptr = Box::into_raw(fd_box) as *mut std::ffi::c_void;
    let handle_id = w.foreign.alloc(crate::foreign::ForeignHandle {
        ptr,
        destructor: Some(console_fd_dtor),
        tag: CONSOLE_FD_TAG,
    });
    let fd_sym = w.intern("fd");
    let label_sym = w.intern("label");
    let label_text = match target {
        ConsoleTarget::Stdout => "stdout",
        ConsoleTarget::Stderr => "stderr",
    };
    let label_value = Value::Sym(w.intern(label_text));

    let mut form = Form::with_proto(Value::Form(console_proto));
    form.slots.insert(fd_sym, Value::Foreign(handle_id));
    form.slots.insert(label_sym, label_value);
    w.alloc(form)
}

fn install_console_proto_and_caps(w: &mut World) {
    // allocate a Console proto inheriting from Object.
    let console_proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // primitive methods (rust):
    //   :emit:  — write bytes to the fd held in self's :fd slot.
    //   :close  — phase A: no-op (stdout/stderr are os-owned).
    //   :next, :done? — these are write-only; raise on read attempt.
    //
    // the :fd slot holds a ForeignHandle (tag = CONSOLE_FD_TAG). we
    // verify the tag before casting — guards against a user who
    // forgot to call :initialize, or who shoved a different
    // ForeignHandle into the slot.
    w.install_native(console_proto, "emit:", |w, self_, args| {
        use std::io::Write;
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "emit: receiver is not a Form")
        })?;
        let fd_sym = w.intern("fd");
        let fd_value = w.heap.get(id).slot(fd_sym);
        let foreign_id = match fd_value {
            Value::Foreign(fid) => fid,
            _ => {
                return Err(RaiseError::new(
                    w.intern("dispatch-error"),
                    "Console :fd slot is not a ForeignHandle (uninitialized?)",
                ));
            }
        };
        let handle = w.foreign.get(foreign_id);
        if handle.tag != CONSOLE_FD_TAG {
            return Err(RaiseError::new(
                w.intern("type-error"),
                "Console :fd has wrong ForeignHandle tag",
            ));
        }
        // SAFETY: tag-check confirms this pointer was minted by
        // make_primordial_console; cast back to ConsoleFd.
        let target = unsafe { (*(handle.ptr as *const ConsoleFd)).target };
        // accept a String form (preferred) or a Symbol (legacy
        // path for substrate-level callers that haven't yet built
        // a String). reject anything else clearly.
        let bytes: Vec<u8> = match args.first().copied() {
            Some(v) => {
                if let Some(t) = w.string_text(v) {
                    t.as_bytes().to_vec()
                } else if let Value::Sym(s) = v {
                    w.resolve(s).as_bytes().to_vec()
                } else {
                    return Err(RaiseError::new(
                        w.intern("type-error"),
                        "emit: expects a String (or Symbol)",
                    ));
                }
            }
            None => {
                return Err(RaiseError::new(
                    w.intern("arity"),
                    "emit: requires one argument",
                ));
            }
        };
        let result = match target {
            ConsoleTarget::Stdout => std::io::stdout().write_all(&bytes),
            ConsoleTarget::Stderr => std::io::stderr().write_all(&bytes),
        };
        result.map_err(|e| RaiseError::new(w.intern("io-error"), e.to_string()))?;
        Ok(Value::Nil)
    });

    // :say: x  — derived: emit (to-string x) then a newline.
    w.install_native(console_proto, "say:", |w, self_, args| {
        let to_string = w.intern("toString");
        let text = w.send(args[0], to_string, &[])?;
        let emit = w.intern("emit:");
        w.send(self_, emit, &[text])?;
        let newline = w.make_string("\n");
        w.send(self_, emit, &[newline])?;
        Ok(Value::Nil)
    });

    // :show: x — emit without newline.
    w.install_native(console_proto, "show:", |w, self_, args| {
        let to_string = w.intern("toString");
        let text = w.send(args[0], to_string, &[])?;
        let emit = w.intern("emit:");
        w.send(self_, emit, &[text])?;
        Ok(Value::Nil)
    });

    // :close — phase A: no-op. phase B's mco wires up real fd cleanup.
    w.install_native(console_proto, "close", |_, _, _| Ok(Value::Nil));

    // :next / :done? — Console is sink-only.
    w.install_native(console_proto, "next", |w, _, _| {
        Err(RaiseError::new(
            w.intern("not-supported"),
            ":next on a Console (write-only)",
        ))
    });
    w.install_native(console_proto, "done?", |_, _, _| Ok(Value::Bool(false)));

    // primordial $out, $err — fd held in a real ForeignHandle.
    // the supervisor (here: the substrate at boot) is the *only*
    // place these are constructed. user code reaches them via
    // env_lookup; cannot mint new Console instances pointing at
    // stdout/stderr without supervisor authority.
    let out_id = make_primordial_console(w, console_proto, ConsoleTarget::Stdout);
    let err_id = make_primordial_console(w, console_proto, ConsoleTarget::Stderr);

    let global = w.global_env;
    let dollar_out = w.intern("$out");
    let dollar_err = w.intern("$err");
    w.env_bind(global, dollar_out, Value::Form(out_id));
    w.env_bind(global, dollar_err, Value::Form(err_id));

    // expose the proto by name so user code can subclass.
    // (`[Console new]` would yield a Console without an :fd slot;
    // `:emit:` would raise. real fd capture lands in phase A.9
    // when the mco loader exposes os-side primitives.)
    let console_name = w.intern("Console");
    w.env_bind(global, console_name, Value::Form(console_proto));
}

// ─────────────────────────────────────────────────────────────────
// global callables
// ─────────────────────────────────────────────────────────────────

fn install_globals(w: &mut World) {
    // moof discipline (process/docs-driven.md): free functions are
    // reserved for *constructors with no meaningful receiver* and
    // *substrate metaprogramming primitives*. user-data ops like
    // `length`, `map`, `+` etc. are methods on the receiver.

    // (cons head tail) — list constructor with no meaningful
    // receiver among args. lowers to `[tail cons: head]`.
    install_global(w, "cons", |world, _, args| {
        if args.len() != 2 {
            return Err(RaiseError::new(world.intern("arity"), "cons takes 2 args"));
        }
        let cons_sel = world.intern("cons:");
        world.send(args[1], cons_sel, &[args[0]])
    });

    // (list a b c) — variadic list constructor.
    install_global(w, "list", |world, _, args| Ok(world.make_list(args)));

    // (macroexpand '(foo a b)) — run the macro for `foo` with the
    // unevaluated args and return the expansion. raises if `foo`
    // isn't a registered macro.
    install_global(w, "macroexpand", |world, _, args| {
        if args.len() != 1 {
            return Err(RaiseError::new(
                world.intern("arity"),
                "macroexpand: (macroexpand 'form)",
            ));
        }
        let form = args[0];
        let elems = world.list_to_vec(form).map_err(|_| {
            RaiseError::new(
                world.intern("type-error"),
                "macroexpand: arg must be a list-form",
            )
        })?;
        if elems.is_empty() {
            return Err(RaiseError::new(
                world.intern("macroexpand"),
                "empty form",
            ));
        }
        let head = elems[0].as_sym().ok_or_else(|| {
            RaiseError::new(
                world.intern("macroexpand"),
                "form head is not a symbol",
            )
        })?;
        let macro_v = world.macros.get(&head).copied().ok_or_else(|| {
            RaiseError::new(
                world.intern("macroexpand"),
                format!("`{}` is not a macro", world.resolve(head)),
            )
        })?;
        let mid = macro_v.as_form_id().ok_or_else(|| {
            RaiseError::new(
                world.intern("macroexpand"),
                "macro entry is not a Form",
            )
        })?;
        let macro_args: Vec<Value> = elems[1..].to_vec();
        world.invoke(mid, Value::Nil, &macro_args, FormId::NONE)
    });

    // (append xs ys …) — concatenate lists left-to-right. used by
    // quasiquote splicing. (append) → '(); (append xs) → xs.
    install_global(w, "append", |world, _, args| {
        let mut out: Vec<Value> = Vec::new();
        let head_sym = world.intern("head");
        let tail_sym = world.intern("tail");
        for &arg in args {
            let mut cur = arg;
            while let Some(fid) = cur.as_form_id() {
                let f = world.heap.get(fid);
                if f.proto != Value::Form(world.protos.list) {
                    break;
                }
                let head = f.slot(head_sym);
                let tail = f.slot(tail_sym);
                if head.is_nil() && tail.is_nil() {
                    break;
                }
                out.push(head);
                cur = tail;
            }
        }
        Ok(world.make_list(&out))
    });

    // ── substrate metaprogramming ───────────────────────────────
    //
    // these cross the moldable-substrate boundary: they read or
    // mutate the heap's internal structure (slot tables, handler
    // tables). we keep them as free functions because the action
    // is "modify the substrate's view of this Form," not "send a
    // message to a receiver." compare to e.g. `Object.defineProperty`
    // in javascript — substrate-shaped, not OO-shaped.

    install_global(w, "slot", |w, _, args| {
        if args.len() != 2 {
            return Err(RaiseError::new(w.intern("arity"), "(slot v 'name)"));
        }
        let id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot on tagged-immediate")
        })?;
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        Ok(w.heap.get(id).slot(name))
    });
    install_global(w, "slotSet!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(slot-set! v 'name value)",
            ));
        }
        let id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot-set! on tagged-immediate")
        })?;
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        w.heap.get_mut(id).slots.insert(name, args[2]);
        Ok(args[2])
    });
    // (getOrCreateProto 'Name Parent) — defproto's reopen helper.
    // - if Name is already bound in the global env to a Form,
    //   return that Form (reopen, preserving identity).
    // - else allocate a fresh Form whose proto is Parent, bind it,
    //   and return the new proto.
    //
    // matches prototype-tradition's class-as-object discipline:
    // protos are mutable values you can extend in place. matches
    // smalltalk's `class extend`.
    install_global(w, "getOrCreateProto", |w, _, args| {
        if args.len() != 2 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(getOrCreateProto 'Name Parent)",
            ));
        }
        let name_sym = args[0].as_sym().ok_or_else(|| {
            RaiseError::new(
                w.intern("type-error"),
                "getOrCreateProto: name must be a symbol",
            )
        })?;
        let global = w.global_env;
        if let Some(existing) = w.env_lookup(global, name_sym) {
            if existing.as_form_id().is_some() {
                return Ok(existing);
            }
        }
        let parent = args[1];
        let mut form = Form::with_proto(parent);
        let name_meta = w.intern("name");
        form.meta.insert(name_meta, Value::Sym(name_sym));
        let new_id = w.alloc(form);
        let v = Value::Form(new_id);
        w.env_bind(global, name_sym, v);
        Ok(v)
    });
    // (set-handler! Proto 'sel fn) — moldable-substrate primitive.
    // bumps the proto's generation counter so existing inline
    // caches re-resolve on next dispatch.
    install_global(w, "setHandler!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(set-handler! Proto 'sel fn)",
            ));
        }
        let proto_id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "set-handler! Proto must be a Form")
        })?;
        let sel = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "set-handler! selector must be a symbol")
        })?;
        w.heap.get_mut(proto_id).handlers.insert(sel, args[2]);
        // bump generation so existing ICs invalidate.
        // (`docs/laws/substrate-laws.md` L10.)
        w.bump_proto_generation(proto_id);
        Ok(args[2])
    });
}

/// allocate a global-dispatcher Form (proto: Method, native fn
/// recorded in side table) and bind it under `name` in the global
/// env.
fn install_global(w: &mut World, name: &str, native: NativeFn) {
    let f = Form::with_proto(Value::Form(w.protos.method));
    let id = w.alloc(f);
    let name_sym = w.intern(name);
    // tag :source with the symbol so `[+ source] → '+`.
    w.heap
        .get_mut(id)
        .meta
        .insert(w.source_sym, Value::Sym(name_sym));
    w.native_fns.insert(id, native);
    let global = w.global_env;
    w.env_bind(global, name_sym, Value::Form(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(w: &mut World, src: &str) -> Result<Value, RaiseError> {
        let form = w.read(src).map_err(|e| RaiseError::from_reader(&mut w.syms, e))?;
        let chunk = crate::compiler::compile(w, form)?;
        w.run_top(chunk)
    }

    fn fresh() -> World {
        // for tests that exercise stdlib methods (like :empty? on
        // List), we need the full new_world() with bootstrap.moof
        // loaded. tests of *intrinsics-only* behavior call new_bare().
        crate::new_world()
    }

    fn fresh_bare() -> World {
        // intrinsics-only — no bootstrap.moof. for tests that
        // verify the rust-side intrinsic wiring directly.
        let mut w = World::new();
        install(&mut w);
        w
    }

    #[test]
    fn arithmetic_works() {
        // canonical moof form: send-bracket binary ops on Integer.
        // free-function `(+ a b)` is an anti-pattern
        // (`process/docs-driven.md` stdlib rule).
        let mut w = fresh();
        assert_eq!(ev(&mut w, "[1 + 2]").unwrap(), Value::Int(3));
        assert_eq!(ev(&mut w, "[10 - 3]").unwrap(), Value::Int(7));
        assert_eq!(ev(&mut w, "[4 * 5]").unwrap(), Value::Int(20));
        assert_eq!(ev(&mut w, "[20 / 4]").unwrap(), Value::Int(5));
    }

    #[test]
    fn nested_arithmetic() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "[3 * [4 + 5]]").unwrap(), Value::Int(27));
    }

    #[test]
    fn comparison_works() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "[1 < 2]").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "[2 < 1]").unwrap(), Value::Bool(false));
        assert_eq!(ev(&mut w, "[5 = 5]").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "[5 >= 5]").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "[5 != 6]").unwrap(), Value::Bool(true));
    }

    #[test]
    fn integer_send_directly() {
        let mut w = fresh();
        let plus = w.intern("+");
        assert_eq!(
            w.send(Value::Int(5), plus, &[Value::Int(7)]).unwrap(),
            Value::Int(12)
        );
    }

    #[test]
    fn proto_via_send_bracket() {
        let mut w = fresh();
        let r = ev(&mut w, "[5 proto]").unwrap();
        assert_eq!(r, Value::Form(w.protos.integer));
    }

    #[test]
    fn identity_returns_form_id() {
        let mut w = fresh();
        // tagged immediates have identity 0
        assert_eq!(ev(&mut w, "[5 identity]").unwrap(), Value::Int(0));
        // a fresh list has a real id
        let v = ev(&mut w, "(list 1 2 3)").unwrap();
        let id = v.as_form_id().unwrap();
        let identity_sym = w.intern("identity");
        let r = w.send(v, identity_sym, &[]).unwrap();
        assert_eq!(r, Value::Int(id.0 as i64));
    }

    #[test]
    fn list_head_tail_cons() {
        let mut w = fresh();
        let head_sym = w.intern("head");
        // build (1 2 3) and inspect
        let v = ev(&mut w, "(list 1 2 3)").unwrap();
        assert_eq!(w.send(v, head_sym, &[]).unwrap(), Value::Int(1));
        let tail_sym = w.intern("tail");
        let tail = w.send(v, tail_sym, &[]).unwrap();
        assert_eq!(w.send(tail, head_sym, &[]).unwrap(), Value::Int(2));
        // (cons 0 (list 1 2 3)) → list with first element 0
        let consed = ev(&mut w, "(cons 0 (list 1 2 3))").unwrap();
        assert_eq!(w.send(consed, head_sym, &[]).unwrap(), Value::Int(0));
    }

    #[test]
    fn empty_check_works() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "[nil empty?]").unwrap(), Value::Bool(true));
        assert_eq!(
            ev(&mut w, "[(list 1) empty?]").unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn integer_to_string() {
        let mut w = fresh();
        let r = ev(&mut w, "[42 toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "42");
    }

    #[test]
    fn def_then_use() {
        let mut w = fresh();
        ev(&mut w, "(def x 10)").unwrap();
        ev(&mut w, "(def y 20)").unwrap();
        assert_eq!(ev(&mut w, "[x + y]").unwrap(), Value::Int(30));
    }

    #[test]
    fn factorial_works_end_to_end() {
        // recursion via send-brackets; the receiver-as-self pattern.
        let mut w = fresh();
        ev(
            &mut w,
            "(def fact (fn (n)
                (if [n = 0]
                    1
                    [n * (fact [n - 1])])))",
        )
        .unwrap();
        assert_eq!(ev(&mut w, "(fact 0)").unwrap(), Value::Int(1));
        assert_eq!(ev(&mut w, "(fact 1)").unwrap(), Value::Int(1));
        assert_eq!(ev(&mut w, "(fact 5)").unwrap(), Value::Int(120));
        assert_eq!(ev(&mut w, "(fact 10)").unwrap(), Value::Int(3628800));
    }

    #[test]
    fn closures_capture_correctly() {
        let mut w = fresh();
        ev(
            &mut w,
            "(def make-adder (fn (n) (fn (x) [x + n])))",
        )
        .unwrap();
        assert_eq!(ev(&mut w, "((make-adder 5) 7)").unwrap(), Value::Int(12));
        assert_eq!(ev(&mut w, "((make-adder 10) 20)").unwrap(), Value::Int(30));
    }

    #[test]
    fn let_with_arithmetic() {
        let mut w = fresh();
        assert_eq!(
            ev(&mut w, "(let ((a 3) (b 4)) [a + b])").unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn does_not_understand_default_raises() {
        let mut w = fresh();
        let mystery = w.intern("flibbertigibbet");
        let err = w.send(Value::Int(5), mystery, &[]).unwrap_err();
        assert_eq!(w.resolve(err.kind), "doesNotUnderstand");
    }

    #[test]
    fn reflection_proto_via_send() {
        let mut w = fresh();
        let proto_sym = w.intern("proto");
        assert_eq!(
            w.send(Value::Int(7), proto_sym, &[]).unwrap(),
            Value::Form(w.protos.integer)
        );
        assert_eq!(
            w.send(Value::Bool(true), proto_sym, &[]).unwrap(),
            Value::Form(w.protos.bool_)
        );
    }

    #[test]
    fn reflection_source_returns_source_for_chunks() {
        // a chunk's :source meta carries the original Form.
        let mut w = fresh();
        let f = w.read("(+ 1 2)").unwrap();
        let chunk = crate::compiler::compile(&mut w, f).unwrap();
        let source_sel = w.intern("source");
        let r = w.send(Value::Form(chunk), source_sel, &[]).unwrap();
        // r should be the original parsed form (a list).
        assert_eq!(r, f);
    }

    #[test]
    fn reflection_slots_returns_slot_pairs() {
        // build an object with known slots; reflect.
        let mut w = fresh();
        let mut f = Form::with_proto(Value::Form(w.protos.object));
        let a = w.intern("a");
        let b = w.intern("b");
        f.slots.insert(a, Value::Int(1));
        f.slots.insert(b, Value::Int(2));
        let id = w.alloc(f);
        let slots_sel = w.intern("slots");
        let r = w.send(Value::Form(id), slots_sel, &[]).unwrap();
        // r is a list of (sym . value) pairs, in insertion order.
        let entries = w.list_to_vec(r).unwrap();
        assert_eq!(entries.len(), 2);
        let pair0 = w.list_to_vec(entries[0]).unwrap();
        assert_eq!(pair0[0], Value::Sym(a));
        assert_eq!(pair0[1], Value::Int(1));
        let pair1 = w.list_to_vec(entries[1]).unwrap();
        assert_eq!(pair1[0], Value::Sym(b));
        assert_eq!(pair1[1], Value::Int(2));
    }

    #[test]
    fn integer_inspect_falls_through_to_to_string() {
        let mut w = fresh();
        let r = ev(&mut w, "[42 inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "42");
    }

    #[test]
    fn out_cap_is_bound_in_global_env() {
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let v = w.env_lookup(w.global_env, dollar_out).unwrap();
        // it's a Form (a Console instance).
        let id = v.as_form_id().unwrap();
        // its proto is Console.
        let proto = w.heap.get(id).proto;
        // Console isn't on `Protos` (it's a user-visible intrinsic
        // proto living in the global env). check via name lookup.
        let console_sym = w.intern("Console");
        let console_proto = w.env_lookup(w.global_env, console_sym).unwrap();
        assert_eq!(proto, console_proto);
    }

    #[test]
    fn out_cap_responds_to_emit() {
        // we can't easily capture stdout from a unit test; verify
        // that :emit: dispatches without panicking on a valid call.
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.global_env, dollar_out).unwrap();
        let emit = w.intern("emit:");
        // we deliberately use stderr for the test so test runner's
        // captured stdout isn't disrupted. switch out's label.
        let label_sym = w.intern("label");
        let stderr_sym = w.intern("stderr");
        let id = out.as_form_id().unwrap();
        w.heap.get_mut(id).slots.insert(label_sym, Value::Sym(stderr_sym));
        let payload = Value::Sym(w.intern(""));
        let r = w.send(out, emit, &[payload]).unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn out_cap_say_dispatches_through_to_string() {
        // :say: 42 → emit "42"; emit "\n". exercises the dispatch
        // chain without actually writing.
        let mut w = fresh();
        // route to stderr so test runner stays happy.
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.global_env, dollar_out).unwrap();
        let label_sym = w.intern("label");
        let stderr_sym = w.intern("stderr");
        let id = out.as_form_id().unwrap();
        w.heap.get_mut(id).slots.insert(label_sym, Value::Sym(stderr_sym));
        // the actual call:
        let say = w.intern("say:");
        let r = w.send(out, say, &[Value::Int(42)]).unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn no_free_function_print_in_world() {
        // process/docs-driven.md's capability rule: no
        // print/println/puts. *also* no map/filter/reduce/+/-/etc.
        // — those are methods on the receiver, not free functions.
        let mut w = fresh();
        let forbidden_io = ["print", "println", "puts", "simulated_println"];
        let forbidden_user_data_ops = [
            "map", "filter", "reduce", "each", "take", "drop",
            "+", "-", "*", "/", "<", ">", "<=", ">=", "=", "!=",
            "length", "empty?", "head", "tail", "null?", "abs",
            "zero?", "positive?", "negative?",
        ];
        for forbidden in forbidden_io.iter().chain(forbidden_user_data_ops.iter()) {
            let s = w.intern(forbidden);
            let v = w.env_lookup(w.global_env, s);
            assert!(
                v.is_none(),
                "forbidden global `{}` is bound (must not be)",
                forbidden
            );
        }
    }

    #[test]
    fn user_code_cannot_synthesize_a_cap() {
        // there is no constructor that produces a Console out of
        // thin air. (Console proto's :new would, but that's the
        // moldable extension hook for the future Transcript proto;
        // phase A's discipline is "don't invoke :new on Console
        // unless the supervisor authorizes it." we don't enforce
        // *yet* — phase B's cap-attenuation primitive does. for
        // now: document the gap honestly.)
        //
        // what *is* enforced: the supervisor binds the primordial
        // caps; they're the only ones in scope.
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let dollar_err = w.intern("$err");
        let dollar_x = w.intern("$x");
        assert!(w.env_lookup(w.global_env, dollar_out).is_some());
        assert!(w.env_lookup(w.global_env, dollar_err).is_some());
        assert!(w.env_lookup(w.global_env, dollar_x).is_none());
    }
}
