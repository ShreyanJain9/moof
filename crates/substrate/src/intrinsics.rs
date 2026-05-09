//! the substrate's primordial native methods + global bindings.
//!
//! installed during `World::new()`, before any moof source loads.
//! the rule per the v4-take-2 docs (vision/one-page.md, `audit-
//! 2026-04-29.md`): *only* what cannot be expressed in moof lives
//! here. everything derivable lives in `lib/bootstrap.moof`.
//!
//! what's here, by category:
//!
//! - **heap-touching primitives** the gc owns: alloc shapes for
//!   String / Table, `:cons:` cell builder, `Object :new`.
//! - **substrate metaprogramming**: `slot`, `slotSet!`,
//!   `setHandler!`, `getOrCreateProto`, `macroexpand`, `append`.
//! - **arithmetic / comparison primitives** on Integer / Float:
//!   `:+ :- :* :/ :< :> :=` and the few unary float math funcs
//!   that wrap `f64` methods.
//! - **String** UTF-8 / unicode primitives: `:length :byteLength
//!   :at: :toList :upcase :downcase :trim :contains?:
//!   :startsWith?: :indexOf: :slice:length: :replace:with: :split:
//!   :lines :+ := :asTable :as: :toString`.
//! - **Table** rep primitives: `:new :length :at: :at:put: :push:
//!   :pop :containsKey?: :remove: :keys :values := :toString
//!   :asString :as:`.
//! - **Char** primitives: `:codepoint :toString :< :letter? :digit?
//!   :whitespace? :upcase :downcase`.
//! - **Object reflection** through the heap: `:proto :slots
//!   :handlers :handlerAt: :meta :source :identity :is := :!=
//!   :toString :new :doesNotUnderstand:with:`.
//! - **Method / Closure reflection**: `:call :body :source :params
//!   :consts :bytecodes`.
//! - **Console** capability primitives: `:emit: :close :next` plus
//!   the `$out` / `$err` primordial caps.
//! - **Global ctors / metaprog**: `cons`, `list`, `slot`,
//!   `slotSet!`, `setHandler!`, `getOrCreateProto`, `macroexpand`,
//!   `append`.
//!
//! everything *derived* from those — `:length :map: :filter:
//!   :reduce:from: :forEach: :reverse :empty? :!= :<= :>= :zero?
//!   :positive? :negative? :abs :inspect :initialize :say: :show:
//!   :disassemble :asList :concat:` etc — lives in
//!   `lib/bootstrap.moof`. moof code is the canonical artifact;
//!   when the docs say a method exists, the bootstrap defines it.

use crate::form::{Form, FormId};
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{NativeFn, RaiseError, World};

// ─────────────────────────────────────────────────────────────────
// shared error helpers — DRY-out for the very common patterns of
// raising type-errors, arity-errors, and the like. they swallow
// the `w.intern(...)` + `RaiseError::new` boilerplate.
// ─────────────────────────────────────────────────────────────────

/// `RaiseError { kind: 'type-error, message: msg }`.
fn type_error(w: &mut World, msg: impl Into<String>) -> RaiseError {
    RaiseError::new(w.intern("type-error"), msg)
}

/// `RaiseError { kind, message: msg }` — for non-`type-error` kinds.
fn raise(w: &mut World, kind: &str, msg: impl Into<String>) -> RaiseError {
    RaiseError::new(w.intern(kind), msg)
}

/// extract a String form's text, or raise type-error tagged with
/// the operator name. shared by every String primitive.
fn str_arg<'a>(w: &'a mut World, v: Value, op: &str) -> Result<String, RaiseError> {
    match w.string_text(v) {
        Some(t) => Ok(t.to_string()),
        None => Err(type_error(w, format!("{} on non-String", op))),
    }
}

/// extract a numeric argument as f64, type-errored against `op`.
/// accepts Int (lossless promotion) and Float.
fn num_f64(w: &mut World, v: Value, op: &str) -> Result<f64, RaiseError> {
    v.as_number_f64()
        .ok_or_else(|| type_error(w, format!("{} expected a numeric rhs", op)))
}


/// install all phase-A intrinsics. idempotent: safe to call once
/// at world init.
pub fn install(w: &mut World) {
    crate::transporter::install(w);
    install_call_on_method(w);
    install_integer_methods(w);
    install_float_methods(w);
    install_char_methods(w);
    install_string_methods(w);
    install_bytes_methods(w);
    install_table_methods(w);
    install_object_reflection(w);
    install_cons_and_nil_primitives(w);
    install_heap_singleton(w);
    install_chunks_singleton(w);
    install_console_proto_and_caps(w);
    install_compiler_cap(w);
    install_globals(w);
    install_compiler_primitives(w);
    install_proto_globals(w);
    install_env_proto_methods(w);
    install_closure_proto_methods(w);
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
        let inst = w.make_table();
        // V2 task-9 — seal-after-initialize. Tables don't run a user
        // :initialize, but the seal still applies in FrozenByDefault.
        if w.vat_mode == crate::VatMode::FrozenByDefault {
            if let Value::Form(id) = inst {
                w.freeze(id)?;
            }
        }
        Ok(inst)
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

    // :size, :empty? are derived in lib/bootstrap.moof from :keys
    // and :length.

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
                // fast path: exact reference equality (works for
                // symbols, booleans, integers, nil, form-ids).
                if let Some(v) = w.table_repr(self_).and_then(|r| r.keyed.get(&other).copied()) {
                    return Ok(v);
                }
                // slow path: if the query key is a String form, do a
                // content-based linear scan. two different String
                // allocations with the same text are semantically
                // equal as table keys (the fast path misses them).
                if let Some(query_text) = w.string_text(other).map(|s| s.to_string()) {
                    let found = w.table_repr(self_).and_then(|r| {
                        r.keyed.iter().find_map(|(k, v)| {
                            // string_text on w returns Option<&str> tied to
                            // the World borrow, which conflicts with &r — so
                            // we inline the string-bytes check here.
                            if let Value::Form(k_id) = k {
                                if let Some(k_text) = w.string_text(Value::Form(*k_id)) {
                                    if k_text == query_text.as_str() {
                                        return Some(*v);
                                    }
                                }
                            }
                            None
                        })
                    });
                    return Ok(found.unwrap_or(Value::Nil));
                }
                Ok(Value::Nil)
            }
        }
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

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
    }).expect("install_native at boot — substrate bug");

    // :toList, :forEach:, :map:, :filter:, :reduce:from: are all
    // derived in lib/bootstrap.moof from :keys, :values, :at: and
    // :at:put: / :push:.

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
    }).expect("install_native at boot — substrate bug");

    // :!= is derived in lib/bootstrap.moof from :=.

    // :toString / :inspect — moof, in stdlib/table.moof. iterate
    // positional via [t at: i], keyed via [keys drop: length],
    // join via the closure passed in.

    // :asString and :as: live in stdlib/table.moof — moof code uses
    // [self toList] + Cons:reduce: + [c toString] for concatenation.
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
    }).expect("install_native at boot — substrate bug");
    // :toString lives on Object; its default handles `Value::Char`
    // (printing the single character). [Char toString] hits the
    // Object handler with the proto-Form receiver and returns
    // "Char" via :name.
    // [c inspect] — REPL-readable form: `#\a`, `#\space`, `#\u{1f496}`.
    // matches the reader's char-literal grammar so the inspect
    // output is parseable input. for `say:` / interpolation, use
    // :toString (which yields just the character).
    // :inspect — moof, in stdlib/char.moof. backed by the existing
    // :codepoint primitive plus `__char-from-codepoint` for the hex
    // escape's digit construction.
    // `=`, `!=` flow through Object's identity (Char(a) == Char(b)
    // iff their codepoints match).
    w.install_native(w.protos.char_, "<", |_, self_, args| match (self_, args[0]) {
        (Value::Char(a), Value::Char(b)) => Ok(Value::Bool(a < b)),
        _ => Ok(Value::Bool(false)),
    }).expect("install_native at boot — substrate bug");
    // unicode predicates: each one is "decode the codepoint to a
    // char, ask `char::is_X()`, fall back to false". macroized.
    macro_rules! char_predicate {
        ($w:expr, $sel:expr, $pred:expr) => {
            $w.install_native($w.protos.char_, $sel, |_, self_, _| match self_ {
                Value::Char(cp) => Ok(Value::Bool(
                    char::from_u32(cp).map($pred).unwrap_or(false),
                )),
                _ => Ok(Value::Bool(false)),
            }).expect("install_native at boot — substrate bug");
        };
    }
    char_predicate!(w, "letter?",     |c: char| c.is_alphabetic());
    char_predicate!(w, "digit?",      |c: char| c.is_ascii_digit());
    char_predicate!(w, "whitespace?", |c: char| c.is_whitespace());

    // unicode case mappings: same pattern — decode codepoint, take
    // first char of the iterator returned by `to_uppercase`/
    // `to_lowercase`, fall back to the original codepoint.
    macro_rules! char_case {
        ($w:expr, $sel:expr, $method:ident) => {
            $w.install_native($w.protos.char_, $sel, |_, self_, _| match self_ {
                Value::Char(cp) => {
                    let mapped = char::from_u32(cp)
                        .and_then(|c| c.$method().next())
                        .map(|c| c as u32)
                        .unwrap_or(cp);
                    Ok(Value::Char(mapped))
                }
                v => Ok(v),
            }).expect("install_native at boot — substrate bug");
        };
    }
    char_case!(w, "upcase",   to_uppercase);
    char_case!(w, "downcase", to_lowercase);
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
        let n = str_arg(w, self_, "length")?.chars().count() as i64;
        Ok(Value::Int(n))
    }).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.string, "byteLength", |w, self_, _| {
        let n = w
            .string_bytes(self_)
            .map(|b| b.len() as i64)
            .ok_or_else(|| type_error(w, "byteLength on non-String"))?;
        Ok(Value::Int(n))
    }).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.string, "at:", |w, self_, args| {
        let idx = match args.first().copied() {
            Some(Value::Int(n)) => n,
            _ => return Err(type_error(w, "[s at: i] requires an Integer index")),
        };
        let text = str_arg(w, self_, "at:")?;
        if idx < 0 {
            return Err(raise(
                w,
                "index-out-of-bounds",
                format!("[String at: {}] out of range", idx),
            ));
        }
        text.chars()
            .nth(idx as usize)
            .map(|c| Value::Char(c as u32))
            .ok_or_else(|| {
                raise(
                    w,
                    "index-out-of-bounds",
                    format!("[String at: {}] out of range", idx),
                )
            })
    }).expect("install_native at boot — substrate bug");

    // [s toList] — walk the string into a List of Chars. lets
    // users compose with List's protocol. (`concepts/strings.md`
    // :to-list, camelCased here.)
    w.install_native(w.protos.string, "toList", |w, self_, _| {
        let chars: Vec<Value> = str_arg(w, self_, "toList")?
            .chars()
            .map(|c| Value::Char(c as u32))
            .collect();
        Ok(w.make_list(&chars))
    }).expect("install_native at boot — substrate bug");

    // unicode case mapping stays rust — needs Char::to_uppercase /
    // to_lowercase tables that aren't reasonable to write in moof.
    macro_rules! str_unary_string {
        ($w:expr, $sel:literal, $method:ident) => {
            $w.install_native($w.protos.string, $sel, |w, self_, _| {
                let s = str_arg(w, self_, $sel)?.$method();
                Ok(w.make_string(&s))
            }).expect("install_native at boot — substrate bug");
        };
    }
    str_unary_string!(w, "upcase",   to_uppercase);
    str_unary_string!(w, "downcase", to_lowercase);

    // :trim, :indexOf:, :contains?:, :startsWith?:, :endsWith?:,
    // :replace:with:, :split:, :lines, :asTable, :as: — moof, in
    // stdlib/string.moof (and :endsWith?: in early/03-string-essentials.moof
    // because Symbol:endsWithColon? needs it before defmethod runs).
    // [s slice: start length: n] — substring by char-index.
    w.install_native(w.protos.string, "slice:length:", |w, self_, args| {
        let start = args.first().copied().and_then(|v| v.as_int()).ok_or_else(
            || type_error(w, "slice:length: needs Integer start"),
        )?;
        let len = args.get(1).copied().and_then(|v| v.as_int()).ok_or_else(
            || type_error(w, "slice:length: needs Integer length"),
        )?;
        if start < 0 || len < 0 {
            return Err(raise(
                w,
                "index-out-of-bounds",
                "slice:length: negative start or length",
            ));
        }
        let text = str_arg(w, self_, "slice:length:")?;
        let collected: String = text
            .chars()
            .skip(start as usize)
            .take(len as usize)
            .collect();
        Ok(w.make_string(&collected))
    }).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.string, "=", |w, self_, args| {
        // structural equality. mismatched proto → false; never
        // raises (so `[Symbol = String]` is well-defined).
        let eq = match (w.string_text(self_), w.string_text(args[0])) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        Ok(Value::Bool(eq))
    }).expect("install_native at boot — substrate bug");

    // [s + other] — concatenation. accepts String or Symbol on the
    // RHS for ergonomics; falls through to :toString otherwise.
    w.install_native(w.protos.string, "+", |w, self_, args| {
        let mut out = str_arg(w, self_, "+")?;
        let rhs = args.first().copied().unwrap_or(Value::Nil);
        // fast path: rhs is already a String or Symbol.
        let rhs_str = w
            .string_text(rhs)
            .map(|t| t.to_string())
            .or_else(|| match rhs {
                Value::Sym(s) => Some(w.resolve(s).to_string()),
                _ => None,
            });
        let appended = match rhs_str {
            Some(t) => t,
            None => {
                // ergonomic path: dispatch to rhs's :toString and
                // hope it returns a String.
                let to_string = w.intern("toString");
                let r = w.send(rhs, to_string, &[])?;
                str_arg(w, r, "+ (rhs :toString)")?
            }
        };
        out.push_str(&appended);
        Ok(w.make_string(&out))
    }).expect("install_native at boot — substrate bug");
    // [s concat: t] is :+ in lib/bootstrap.moof; [s empty?] is
    // [[s byteLength] = 0] there.

    // [s asUtf8Bytes] — return the raw UTF-8 encoding of s as a Bytes value.
    // useful for passing String data to wasm mcos that expect Bytes.
    w.install_native(w.protos.string, "asUtf8Bytes", |w, self_, _| {
        let raw = w
            .string_bytes(self_)
            .map(|b| b.to_vec())
            .ok_or_else(|| type_error(w, "asUtf8Bytes on non-String"))?;
        Ok(w.make_bytes(&raw))
    }).expect("install_native at boot — substrate bug");
}

// install_method_methods is gone — Method's :toString and :inspect
// are moof, in stdlib/method.moof. they read :source / :params /
// :name metas via existing primitives (`slot`, `__form-meta-at`)
// and dispatch on the source's shape (Symbol → native, Cons →
// closure, nil → bare).

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
    let global = world.here_form;
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
    world.form_meta_set(proto_id, name_meta, Value::Sym(name_sym))
        .expect("form_meta_set at boot — substrate bug");
    world.env_bind(global, name_sym, Value::Form(proto_id))
        .expect("env_bind at boot — substrate bug");

    // slot-getters for :op and :operands.
    world.install_native(proto_id, "op", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "op: receiver not a Form")
        })?;
        let op_sym = w.intern("op");
        Ok(w.form_slot(id, op_sym))
    }).expect("install_native at boot — substrate bug");
    world.install_native(proto_id, "operands", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "operands: receiver not a Form")
        })?;
        let operands_sym = w.intern("operands");
        Ok(w.form_slot(id, operands_sym))
    }).expect("install_native at boot — substrate bug");
    // [opcode toString] → "<LoadConst 0>" etc.
    world.install_native(proto_id, "toString", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "toString: receiver not a Form")
        })?;
        let op_sym = w.intern("op");
        let operands_sym = w.intern("operands");
        let name = match w.form_slot(id, op_sym) {
            Value::Sym(s) => w.resolve(s).to_string(),
            _ => "?".to_string(),
        };
        let operands_v = w.form_slot(id, operands_sym);
        let mut parts = vec![name];
        if let Some(r) = w.table_repr(operands_v) {
            for v in r.positional.clone() {
                parts.push(render_value(w, v));
            }
        }
        let s = format!("<{}>", parts.join(" "));
        Ok(w.make_string(&s))
    }).expect("install_native at boot — substrate bug");

    proto_id
}

/// build an opcode-Form `{Opcode :op 'name :operands [args…]}`.
/// shared by `opcode_form` (Op → Form, used by `[m bytecodes]`) and
/// the per-variant moof constructors (`(op-load-const idx)` etc.,
/// used by `compiler.moof`). encode/decode go through this so the
/// shape stays canonical in both directions.
fn mk_op_form(world: &mut World, name: &str, operands: &[Value]) -> Value {
    let op_sym = world.intern("op");
    let operands_sym = world.intern("operands");
    let opcode_proto = ensure_opcode_proto(world);
    let mut form = crate::form::Form::with_proto(Value::Form(opcode_proto));
    let name_sym = world.intern(name);
    form.slots.insert(op_sym, Value::Sym(name_sym));
    let operands_tbl = world.make_table();
    if let Some(r) = world.table_repr_mut(operands_tbl) {
        r.positional.extend_from_slice(operands);
    }
    form.slots.insert(operands_sym, operands_tbl);
    Value::Form(world.alloc(form))
}

/// build an opcode-Form for a single `Op`. each form has slots
/// `:op` (Sym) and `:operands` (Table) — operands are pushed in
/// declaration order so positional access works. the form's proto
/// is `Opcode`, which exposes `:op`, `:operands`, `:toString`.
fn opcode_form(world: &mut World, op: crate::opcodes::Op) -> Value {
    use crate::opcodes::Op;
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
    mk_op_form(world, name, &operands)
}

/// the inverse of `opcode_form`: read an opcode-Form's `:op` and
/// `:operands` and rebuild an `Op` variant. range-checks the
/// numeric operands against their bytecode bounds.
///
/// raises `'compile-error` for malformed forms, `'range-error` when
/// an operand exceeds the variant's bound (e.g. `argc > 255` in a
/// `Send`).
fn decode_op_form(
    world: &mut World,
    v: Value,
) -> Result<crate::opcodes::Op, RaiseError> {
    use crate::opcodes::Op;
    let id = v.as_form_id().ok_or_else(|| {
        type_error(world, "chunk-emit: opcode must be a Form")
    })?;
    let op_sym = world.intern("op");
    let operands_sym = world.intern("operands");
    let name = match world.form_slot(id, op_sym) {
        Value::Sym(s) => s,
        _ => return Err(raise(world, "compile-error", "opcode :op must be a Symbol")),
    };
    let operands_v = world.form_slot(id, operands_sym);
    let operands: Vec<Value> = world
        .table_repr(operands_v)
        .map(|r| r.positional.clone())
        .unwrap_or_default();
    let name_text = world.resolve(name).to_string();

    fn need_int(
        w: &mut World,
        op: &str,
        ops: &[Value],
        i: usize,
    ) -> Result<i64, RaiseError> {
        match ops.get(i).and_then(|v| v.as_int()) {
            Some(n) => Ok(n),
            None => Err(raise(
                w,
                "compile-error",
                format!("{}: expected Integer at operand {}", op, i),
            )),
        }
    }
    fn need_sym(
        w: &mut World,
        op: &str,
        ops: &[Value],
        i: usize,
    ) -> Result<SymId, RaiseError> {
        match ops.get(i).and_then(|v| v.as_sym()) {
            Some(s) => Ok(s),
            None => Err(raise(
                w,
                "compile-error",
                format!("{}: expected Symbol at operand {}", op, i),
            )),
        }
    }
    fn need_form(
        w: &mut World,
        op: &str,
        ops: &[Value],
        i: usize,
    ) -> Result<crate::form::FormId, RaiseError> {
        match ops.get(i).and_then(|v| v.as_form_id()) {
            Some(f) => Ok(f),
            None => Err(raise(
                w,
                "compile-error",
                format!("{}: expected Form at operand {}", op, i),
            )),
        }
    }
    fn fit_u16(w: &mut World, op: &str, n: i64) -> Result<u16, RaiseError> {
        u16::try_from(n).map_err(|_| {
            raise(
                w,
                "range-error",
                format!("{}: operand {} doesn't fit u16 (max 65535)", op, n),
            )
        })
    }
    fn fit_u8(w: &mut World, op: &str, n: i64) -> Result<u8, RaiseError> {
        u8::try_from(n).map_err(|_| {
            raise(
                w,
                "range-error",
                format!("{}: argc {} doesn't fit u8 (max 255)", op, n),
            )
        })
    }
    fn fit_i16(w: &mut World, op: &str, n: i64) -> Result<i16, RaiseError> {
        i16::try_from(n).map_err(|_| {
            raise(
                w,
                "range-error",
                format!("{}: jump offset {} doesn't fit i16", op, n),
            )
        })
    }

    Ok(match name_text.as_str() {
        "LoadConst" => {
            let n = need_int(world, "LoadConst", &operands, 0)?;
            Op::LoadConst(fit_u16(world, "LoadConst", n)?)
        }
        "PushNil" => Op::PushNil,
        "PushTrue" => Op::PushTrue,
        "PushFalse" => Op::PushFalse,
        "Pop" => Op::Pop,
        "Dup" => Op::Dup,
        "LoadName" => Op::LoadName(need_sym(world, "LoadName", &operands, 0)?),
        "StoreName" => Op::StoreName(need_sym(world, "StoreName", &operands, 0)?),
        "LoadSelf" => Op::LoadSelf,
        "DefineGlobal" => {
            Op::DefineGlobal(need_sym(world, "DefineGlobal", &operands, 0)?)
        }
        "Send" => {
            let sel = need_sym(world, "Send", &operands, 0)?;
            let argc_n = need_int(world, "Send", &operands, 1)?;
            let ic_n = need_int(world, "Send", &operands, 2)?;
            Op::Send {
                selector: sel,
                argc: fit_u8(world, "Send", argc_n)?,
                ic_idx: fit_u16(world, "Send", ic_n)?,
            }
        }
        "TailSend" => {
            let sel = need_sym(world, "TailSend", &operands, 0)?;
            let argc_n = need_int(world, "TailSend", &operands, 1)?;
            Op::TailSend {
                selector: sel,
                argc: fit_u8(world, "TailSend", argc_n)?,
            }
        }
        "SuperSend" => {
            let sel = need_sym(world, "SuperSend", &operands, 0)?;
            let argc_n = need_int(world, "SuperSend", &operands, 1)?;
            let ic_n = need_int(world, "SuperSend", &operands, 2)?;
            Op::SuperSend {
                selector: sel,
                argc: fit_u8(world, "SuperSend", argc_n)?,
                ic_idx: fit_u16(world, "SuperSend", ic_n)?,
            }
        }
        "PushClosure" => Op::PushClosure {
            chunk: need_form(world, "PushClosure", &operands, 0)?,
        },
        "Jump" => {
            let n = need_int(world, "Jump", &operands, 0)?;
            Op::Jump(fit_i16(world, "Jump", n)?)
        }
        "JumpIfFalse" => {
            let n = need_int(world, "JumpIfFalse", &operands, 0)?;
            Op::JumpIfFalse(fit_i16(world, "JumpIfFalse", n)?)
        }
        "Return" => Op::Return,
        other => {
            return Err(raise(
                world,
                "compile-error",
                format!("unknown opcode `{}`", other),
            ));
        }
    })
}

// install_method_reflection is gone. Method reflection methods
// (:body, :params, :consts, :bytecodes, :ics) now live in
// stdlib/method.moof, calling into the Chunks singleton for
// chunk-side-table access.
//
// the Opcode proto is initialized eagerly via ensure_opcode_proto
// during install_chunks_singleton (whose :opsListOf: needs it).

/// recover the FormId where a value's per-instance state should
/// be **written** (lazy alloc for tagged immediates).
///
/// thin convenience wrapper over `World::ensure_writable_form_id`
/// — kept here because the surrounding `install_global` callers
/// already had a helper of this name; internal-call signatures
/// stay tidy.
fn target_form_id(w: &mut World, v: Value, _op: &str) -> FormId {
    w.ensure_writable_form_id(v)
}

/// if `self_` is a Form whose `:name` meta is a Symbol, return
/// that name's text. used by instance `:toString` / `:inspect`
/// handlers to short-circuit when the receiver is a *proto-Form*
/// rather than an instance of the proto — since moof's flat
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

// ─────────────────────────────────────────────────────────────────
// Bytes primitives — :length and :at: give moof code the ability
// to iterate raw byte sequences. used by Bytes:toHexString in
// lib/mcos.moof and other moof-side byte-processing code.
// ─────────────────────────────────────────────────────────────────

fn install_bytes_methods(w: &mut World) {
    // [b length] — number of bytes in the Bytes form.
    w.install_native(w.protos.bytes, "length", |w, self_, _| {
        let n = w
            .bytes_data(self_)
            .map(|b| b.len() as i64)
            .ok_or_else(|| type_error(w, "length on non-Bytes"))?;
        Ok(Value::Int(n))
    }).expect("install_native at boot — substrate bug");

    // [b at: i] — byte at index i (0-based). returns an Integer 0..255.
    // raises 'index-out-of-bounds for i < 0 or i >= length.
    w.install_native(w.protos.bytes, "at:", |w, self_, args| {
        let idx = match args.first().copied() {
            Some(Value::Int(n)) => n,
            _ => return Err(type_error(w, "[Bytes at: i] requires an Integer index")),
        };
        // clone to avoid holding the borrow across type_error / raise.
        let data: Vec<u8> = match w.bytes_data(self_) {
            Some(b) => b.to_vec(),
            None => return Err(type_error(w, "at: on non-Bytes")),
        };
        if idx < 0 || idx as usize >= data.len() {
            return Err(raise(
                w,
                "index-out-of-bounds",
                format!("[Bytes at: {}] out of range (length {})", idx, data.len()),
            ));
        }
        Ok(Value::Int(data[idx as usize] as i64))
    }).expect("install_native at boot — substrate bug");
}

/// expose the canonical protos as moof globals (`Object`, `List`,
/// `Integer`, …). user code can refer to them by name to install
/// handlers, allocate instances, and inspect the proto chain.
fn install_proto_globals(w: &mut World) {
    // Nil is genuinely absent from the global env. nil is a true
    // singleton — observationally and namespace-wise. handlers
    // for nil (cons:, length, empty?, …) live on a hidden
    // proto-Form (`world.protos.nil`); the substrate's setHandler!
    // / slotSet! / metaSet! special-case Value::Nil receivers and
    // route writes to that hidden form, so moof code does
    // `(defmethod nil (length) 0)` directly. nil IS its proto.
    let bindings = [
        ("Object", w.protos.object),
        ("Bool", w.protos.bool_),
        ("Integer", w.protos.integer),
        ("Float", w.protos.float),
        ("Symbol", w.protos.symbol),
        ("Char", w.protos.char_),
        ("String", w.protos.string),
        ("Bytes", w.protos.bytes),
        ("Cons", w.protos.cons),
        ("Table", w.protos.table),
        ("Method", w.protos.method),
        ("Chunk", w.protos.chunk),
        ("Closure", w.protos.closure),
        ("Env", w.protos.env),
        ("ForeignHandle", w.protos.foreign),
        ("Frame", w.protos.frame),
    ];
    let global = w.here_form;
    let name_meta = w.intern("name");
    for (name, id) in bindings {
        let s = w.intern(name);
        w.env_bind(global, s, Value::Form(id))
            .expect("env_bind at boot — substrate bug");
        // also stash the name in the proto's `:name` meta so
        // `[Integer toString]` → `Integer`, not `<Form#3>`.
        w.form_meta_set(id, name_meta, Value::Sym(s))
            .expect("form_meta_set at boot — substrate bug");
    }
    // also expose the canonical macro registry as `Macros`. moof
    // code introspects via `[Macros slots]`, fetches via
    // `[Macros at: 'when]`, etc. honors reflection-contract R6.
    let macros_id = w.macros_form;
    let macros_sym = w.intern("Macros");
    w.env_bind(global, macros_sym, Value::Form(macros_id))
        .expect("env_bind at boot — substrate bug");
    w.form_meta_set(macros_id, name_meta, Value::Sym(macros_sym))
        .expect("form_meta_set at boot — substrate bug");

    // V3 — bind $here as a self-reference to the here_form.
    // moof code reaches the global env via this binding; reflection
    // (e.g. [Heap slotKeysOf: $here]) lists path-bound names.
    let here_sym = w.intern("$here");
    w.env_bind(w.here_form, here_sym, Value::Form(w.here_form))
        .expect("env_bind at boot — substrate bug");
}

// ─────────────────────────────────────────────────────────────────
// V3 Env proto methods — :bind:to:, :set:to:, :lookup:, :parent,
// :current. wraps the existing world env_* APIs so moof code can
// reach the env layer through ordinary message-send. used by the
// def / set! macros and (eventually) fexpr-style metaprogramming.
// per docs/superpowers/plans/2026-05-09-vat-V3-here-form.md task 5.
// ─────────────────────────────────────────────────────────────────

fn install_env_proto_methods(w: &mut World) {
    let env_proto = w.protos.env;

    // [env bind: 'name to: value] — non-walking bind. writes
    // name → value in self's slots only. returns the bound value
    // so callers can chain (matches V3 spec §4.1).
    w.install_native(env_proto, "bind:to:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            type_error(w, ":bind:to: receiver must be an Env Form")
        })?;
        let name = args
            .first()
            .copied()
            .and_then(Value::as_sym)
            .ok_or_else(|| type_error(w, ":bind:to: name must be a Symbol"))?;
        let value = args.get(1).copied().unwrap_or(Value::Nil);
        w.form_slot_set(env, name, value)?;
        Ok(value)
    }).expect("install_native :bind:to: at boot — substrate bug");

    // [env set: 'name to: value] — walks the parent chain (and
    // consults view-target). raises 'unbound on miss instead of
    // silently falling through (V3 tightens the V1 footgun).
    // returns the value on success.
    w.install_native(env_proto, "set:to:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            type_error(w, ":set:to: receiver must be an Env Form")
        })?;
        let name = args
            .first()
            .copied()
            .and_then(Value::as_sym)
            .ok_or_else(|| type_error(w, ":set:to: name must be a Symbol"))?;
        let value = args.get(1).copied().unwrap_or(Value::Nil);
        let found = w.env_set(env, name, value)?;
        if !found {
            let msg = format!("set!: '{} is unbound", w.resolve(name));
            return Err(raise(w, "unbound", msg));
        }
        Ok(value)
    }).expect("install_native :set:to: at boot — substrate bug");

    // [env lookup: 'name] — walks chain (with view-target
    // consultation). returns Nil on miss (caller-friendly default;
    // ':set:to:' is the strict variant).
    w.install_native(env_proto, "lookup:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            type_error(w, ":lookup: receiver must be an Env Form")
        })?;
        let name = args
            .first()
            .copied()
            .and_then(Value::as_sym)
            .ok_or_else(|| type_error(w, ":lookup: name must be a Symbol"))?;
        Ok(w.env_lookup(env, name).unwrap_or(Value::Nil))
    }).expect("install_native :lookup: at boot — substrate bug");

    // [env parent] — convenience accessor for `[env :meta at:
    // 'parent]`. returns the parent env Form, or Nil at chain root.
    w.install_native(env_proto, "parent", |w, self_, _args| {
        let env = self_.as_form_id().ok_or_else(|| {
            type_error(w, ":parent receiver must be a Form")
        })?;
        let parent_sym = w.parent_sym;
        Ok(w.form_meta(env, parent_sym))
    }).expect("install_native :parent at boot — substrate bug");

    // [Env current] — class-method-style. returns the LIVE current
    // frame's env (i.e., the caller's lexical env). receiver is
    // ignored: [Env current], [$here current], or any env-receiver
    // all return the same thing. natives don't push a VM frame
    // (verified at vm.rs:258), so frames.last().env IS the caller's
    // env. used by the future set! macro to find lexical scope at
    // the call site.
    w.install_native(env_proto, "current", |w, _self_, _args| {
        let env = w.vm.frames.last().map(|f| f.env).ok_or_else(|| {
            raise(
                w,
                "env-out-of-scope",
                "[Env current] called outside any active method dispatch",
            )
        })?;
        Ok(Value::Form(env))
    }).expect("install_native :current at boot — substrate bug");
}

// ─────────────────────────────────────────────────────────────────
// V3 Closure proto methods — :callIn:withSelf:. the irreducible
// "run closure body with explicit env+self" primitive. used by
// Object:eval: (lib/stdlib/object.moof, task 14) and future
// vau / fexpr (V8). bypasses the closure's own :env slot — caller
// specifies scope explicitly. per
// docs/superpowers/plans/2026-05-09-vat-V3-here-form.md task 6.
// ─────────────────────────────────────────────────────────────────

fn install_closure_proto_methods(w: &mut World) {
    // [closure callIn: env withSelf: self] — run the closure's body
    // chunk with `env` as the frame env and `self` as the receiver.
    // ignores the closure's stored :env slot (which :call would use
    // for lexical scope). defining_proto is FormId::NONE because
    // this isn't a method dispatch — super-sends from within will
    // raise the usual "no defining proto" error.
    w.install_native(w.protos.closure, "callIn:withSelf:", |w, self_, args| {
        let closure_id = self_.as_form_id().ok_or_else(|| {
            type_error(w, ":callIn:withSelf: on non-closure")
        })?;
        if args.len() != 2 {
            return Err(raise(w, "arity", ":callIn:withSelf: expects 2 args (env, self)"));
        }
        let call_env = args[0].as_form_id().ok_or_else(|| {
            type_error(w, ":callIn: requires a Form env")
        })?;
        let new_self = args[1];
        let body_v = w.form_slot(closure_id, w.body_sym);
        let chunk_id = body_v.as_form_id().ok_or_else(|| {
            type_error(w, "closure has no :body chunk")
        })?;
        crate::vm::run_method(w, chunk_id, call_env, new_self, FormId::NONE)
    }).expect("install_native :callIn:withSelf: at boot — substrate bug");
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
        let captured = world.form_slot(id, captured_sym);
        // closures-as-callables have no defining-proto in the OO
        // sense (they're not "found on" a proto). super-send from
        // inside a closure body raises a useful error.
        world.invoke(id, captured, args, FormId::NONE)
    }).expect("install_native at boot — substrate bug");
}

// ─────────────────────────────────────────────────────────────────
// Integer methods
// ─────────────────────────────────────────────────────────────────

fn install_integer_methods(w: &mut World) {
    // arithmetic auto-promotes when the rhs is a Float.
    // [Int + Int] → Int; [Int + Float] → Float.
    //
    // each op shares the dispatch shape: parse self as Int, switch
    // on rhs kind, fall through with a type-error. macroize.
    macro_rules! int_arith {
        ($w:expr, $sel:literal, $int_method:ident, $float_op:tt) => {
            $w.install_native($w.protos.integer, $sel, |w, self_, args| {
                let a = int_arg(w, self_, $sel)?;
                match args[0] {
                    Value::Int(b) => Ok(Value::Int(a.$int_method(b))),
                    Value::Float(_) => Ok(Value::float(
                        (a as f64) $float_op args[0].as_float().unwrap(),
                    )),
                    _ => Err(type_error(w, format!("{} expected a numeric rhs", $sel))),
                }
            }).expect("install_native at boot — substrate bug");
        };
    }
    int_arith!(w, "+", wrapping_add, +);
    int_arith!(w, "-", wrapping_sub, -);
    int_arith!(w, "*", wrapping_mul, *);

    // `/` is the lone outlier — divide-by-zero check on the int path.
    w.install_native(w.protos.integer, "/", |w, self_, args| {
        let a = int_arg(w, self_, "/")?;
        match args[0] {
            Value::Int(b) => {
                if b == 0 {
                    return Err(raise(w, "division-by-zero", "integer division by zero"));
                }
                Ok(Value::Int(a.wrapping_div(b)))
            }
            Value::Float(_) => Ok(Value::float(a as f64 / args[0].as_float().unwrap())),
            _ => Err(type_error(w, "/ expected a numeric rhs")),
        }
    }).expect("install_native at boot — substrate bug");

    // `=` allows Int-vs-Float comparison; `:!=`, `:<=`, `:>=`
    // derive in lib/bootstrap.moof.
    //
    // defensive against proto-Form receivers: lookup_handler checks
    // a Form's own handlers first, so `[Integer = nil]` lands in
    // *this* handler with self = the Integer-proto-Form. fall back
    // to identity comparison in that case (matching Object's `=`).
    w.install_native(w.protos.integer, "=", |_, self_, args| {
        Ok(Value::Bool(match (self_, args[0]) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::Float(_)) => (a as f64) == args[0].as_float().unwrap(),
            // non-Int self (e.g. Integer-proto itself receiving `=`)
            // → fall through to identity, like Object's `=`.
            _ => self_ == args[0],
        }))
    }).expect("install_native at boot — substrate bug");
    macro_rules! int_cmp {
        ($w:expr, $sel:literal, $op:tt) => {
            $w.install_native($w.protos.integer, $sel, |w, self_, args| {
                let a = match self_.as_int() {
                    Some(a) => a,
                    // proto-Form receivers can't be ordered.
                    None => return Err(type_error(w, format!(
                        "{} expected an Integer receiver", $sel))),
                };
                let b = num_f64(w, args[0], $sel)?;
                Ok(Value::Bool((a as f64) $op b))
            }).expect("install_native at boot — substrate bug");
        };
    }
    int_cmp!(w, "<", <);
    int_cmp!(w, ">", >);

    // :toString lives on Object — its default already handles
    // Int/Float/Bool/Sym/Char/Nil tagged immediates correctly,
    // AND falls through to `:name` for proto-Form receivers. so
    // `[42 toString]` → "42" and `[Integer toString]` → "Integer"
    // both work without an Integer-specific :toString shadowing
    // the proto's own self-name.
    w.install_native(w.protos.integer, "asFloat", |w, self_, _args| {
        let a = int_arg(w, self_, "asFloat")?;
        Ok(Value::float(a as f64))
    }).expect("install_native at boot — substrate bug");

    // [n asChar] — construct a Char-tagged-immediate from a non-
    // negative codepoint. inverse of Char:codepoint.
    w.install_native(w.protos.integer, "asChar", |w, self_, _| {
        match self_ {
            Value::Int(n) if n >= 0 => Ok(Value::Char(n as u32)),
            Value::Int(_) => Err(raise(
                w,
                "index-out-of-bounds",
                "asChar: codepoint must be non-negative",
            )),
            _ => Err(type_error(w, "asChar: receiver is not an Integer")),
        }
    }).expect("install_native at boot — substrate bug");
}

fn int_arg(w: &mut World, v: Value, op: &str) -> Result<i64, RaiseError> {
    v.as_int()
        .ok_or_else(|| type_error(w, format!("{} expected an Integer", op)))
}

// ─────────────────────────────────────────────────────────────────
// Float — IEEE-754 f64 with `Int → Float` promotion.
// ─────────────────────────────────────────────────────────────────

fn install_float_methods(w: &mut World) {
    fn float_arg(w: &mut World, v: Value, op: &str) -> Result<f64, RaiseError> {
        v.as_float()
            .ok_or_else(|| type_error(w, format!("{} expected a Float", op)))
    }

    // arithmetic primitives on Float: receiver must be Float;
    // rhs may be Int (auto-promotes) or Float. result is always Float.
    macro_rules! float_arith {
        ($w:expr, $sel:literal, $op:tt) => {
            $w.install_native($w.protos.float, $sel, |w, self_, args| {
                let a = float_arg(w, self_, $sel)?;
                let b = num_f64(w, args[0], $sel)?;
                Ok(Value::float(a $op b))
            }).expect("install_native at boot — substrate bug");
        };
    }
    float_arith!(w, "+", +);
    float_arith!(w, "-", -);
    float_arith!(w, "*", *);
    float_arith!(w, "/", /);

    // comparison primitives. `=` is Float-vs-Number identity (NaN
    // ≠ anything, including itself). `:!=`, `:<=`, `:>=` are derived
    // in lib/bootstrap.moof. defensive against proto-Form receivers
    // (see Integer `=` for the same reason).
    w.install_native(w.protos.float, "=", |_, self_, args| {
        let a = match self_.as_float() {
            Some(a) => a,
            None => return Ok(Value::Bool(self_ == args[0])),
        };
        Ok(Value::Bool(args[0].as_number_f64().map_or(false, |b| a == b)))
    }).expect("install_native at boot — substrate bug");
    macro_rules! float_cmp {
        ($w:expr, $sel:literal, $op:tt) => {
            $w.install_native($w.protos.float, $sel, |w, self_, args| {
                let a = float_arg(w, self_, $sel)?;
                let b = num_f64(w, args[0], $sel)?;
                Ok(Value::Bool(a $op b))
            }).expect("install_native at boot — substrate bug");
        };
    }
    float_cmp!(w, "<", <);
    float_cmp!(w, ">", >);
    // :toString — same story as Integer; Object's default handles
    // Float tagged immediates and routes proto-Forms to :name.

    // unary `f64`-method wrappers. `:method:` corresponds to
    // `f.method()`; the only outlier is `:log` → `f.ln()`, hence
    // an explicit name argument.
    macro_rules! float_unary {
        ($w:expr, $sel:expr, $method:ident) => {
            $w.install_native($w.protos.float, $sel, |w, self_, _| {
                let a = float_arg(w, self_, $sel)?;
                Ok(Value::float(a.$method()))
            }).expect("install_native at boot — substrate bug");
        };
    }
    float_unary!(w, "sqrt",  sqrt);
    float_unary!(w, "log",   ln);
    float_unary!(w, "exp",   exp);
    float_unary!(w, "sin",   sin);
    float_unary!(w, "cos",   cos);
    float_unary!(w, "floor", floor);
    float_unary!(w, "ceil",  ceil);
    float_unary!(w, "round", round);

    w.install_native(w.protos.float, "asInteger", |w, self_, _| {
        let a = float_arg(w, self_, "asInteger")?;
        Ok(Value::Int(a as i64))
    }).expect("install_native at boot — substrate bug");

    // f64-classification — same shape, parametrize over the
    // predicate.
    macro_rules! float_predicate {
        ($w:expr, $sel:expr, $pred:expr) => {
            $w.install_native($w.protos.float, $sel, |_, self_, _| {
                Ok(Value::Bool(self_.as_float().map_or(false, $pred)))
            }).expect("install_native at boot — substrate bug");
        };
    }
    float_predicate!(w, "nan?",    |f: f64| f.is_nan());
    float_predicate!(w, "finite?", |f: f64| f.is_finite());
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

/// `(cons h tail)` — allocate a List cell. shared by Nil-proto
/// and List, since the heap operation is identical: head + tail
/// in a fresh Form whose proto is List.
fn make_cons_method(w: &mut World, self_: Value, args: &[Value]) -> Result<Value, RaiseError> {
    let mut cell = Form::with_proto(Value::Form(w.protos.cons));
    cell.slots.insert(w.car_sym, args[0]);
    cell.slots.insert(w.cdr_sym, self_);
    Ok(Value::Form(w.alloc(cell)))
}

// ─────────────────────────────────────────────────────────────────
// List (cons-cell) methods
// ─────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────
// Cons + nil — heap-primitive methods needed during early defmethod
// expansion (`__decode-header` / `__decode-keyword` send :car, :cdr,
// :empty?, :reverse, :cons: while expanding macros). every other
// Cons method (length, map, filter, reduce, …) is moof-only in
// stdlib/cons.moof; the moof compiler bypasses dispatch via
// `(__list-* …)` primitives so it doesn't need those to exist.
// ─────────────────────────────────────────────────────────────────

fn install_cons_and_nil_primitives(w: &mut World) {
    // car, cdr, cons: — irreducible heap operations.
    w.install_native(w.protos.cons, "car", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "car on non-Cons")
        })?;
        let car_sym = w.car_sym;
        Ok(w.form_slot(id, car_sym))
    }).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.cons, "cdr", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "cdr on non-Cons")
        })?;
        let cdr_sym = w.cdr_sym;
        Ok(w.form_slot(id, cdr_sym))
    }).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.cons, "cons:", make_cons_method).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.nil, "cons:", make_cons_method).expect("install_native at boot — substrate bug");

    // empty? / null? / nonEmpty? — trivial constants. these get
    // shadowed by stdlib/cons.moof and stdlib/nil.moof's defmethods,
    // but they MUST exist before defmethod runs (used by
    // __decode-header / __decode-keyword on Cons or nil receivers).
    w.install_native(w.protos.cons, "empty?", |_, _, _| Ok(Value::Bool(false))).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.cons, "null?", |_, _, _| Ok(Value::Bool(false))).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.cons, "nonEmpty?", |_, _, _| Ok(Value::Bool(true))).expect("install_native at boot — substrate bug");
    w.install_native(w.protos.nil, "empty?", |_, _, _| Ok(Value::Bool(true))).expect("install_native at boot — substrate bug");

    // nil's :proto returns nil itself — observationally a singleton.
    // without this override Object:proto returns the hidden
    // nil-proto-Form (substrate-level structure).
    w.install_native(w.protos.nil, "proto", |_, _, _| Ok(Value::Nil)).expect("install_native at boot — substrate bug");

    // :reverse — used by __decode-keyword on the params accumulator.
    // rust impl is a tight loop; stdlib/cons.moof shadows with
    // recursive moof.
    w.install_native(w.protos.cons, "reverse", |w, self_, _| {
        let elems = w
            .list_to_vec(self_)
            .map_err(|_| type_error(w, "reverse on non-Cons"))?;
        let rev: Vec<Value> = elems.into_iter().rev().collect();
        Ok(w.make_list(&rev))
    }).expect("install_native at boot — substrate bug");
}

// ─────────────────────────────────────────────────────────────────
// Heap singleton — a primordial cap exposing every primitive heap
// operation as a method. moof code on user-type protos delegates
// here for proto/identity/slot/handler/meta access, allocation, and
// the few "raw list" operations the moof compiler bottoms out on.
//
// rust on user-type protos shrinks dramatically when these live on
// Heap instead: Object only needs :is, :=, :toString, :new, plus
// the dispatch fallbacks. All reflection moves to moof methods that
// say `[Heap protoOf: self]` etc.
// ─────────────────────────────────────────────────────────────────

fn install_heap_singleton(w: &mut World) {
    // allocate a Heap proto-Form inheriting from Object.
    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // [Heap protoOf: v] — proto-Form of any Value (handles tagged
    // immediates).
    w.install_native(proto, "protoOf:", |w, _self, args| {
        Ok(w.proto_of(args.first().copied().unwrap_or(Value::Nil)))
    }).expect("install_native at boot — substrate bug");

    // [Heap heapIdOf: v] — heap-id Int for Forms / Foreigns; 0 for
    // tagged immediates.
    w.install_native(proto, "heapIdOf:", |_, _self, args| {
        match args.first().copied().unwrap_or(Value::Nil) {
            Value::Form(id) => Ok(Value::Int(id.0 as i64)),
            Value::Foreign(id) => Ok(Value::Int(id.0 as i64)),
            _ => Ok(Value::Int(0)),
        }
    }).expect("install_native at boot — substrate bug");

    // [Heap allocFormWithProto: p] — heap-alloc a fresh Form whose
    // proto is `p` (which must be a Form).
    w.install_native(proto, "allocFormWithProto:", |w, _self, args| {
        let proto_v = args.first().copied().unwrap_or(Value::Nil);
        let proto_id = match proto_v {
            Value::Form(id) => id,
            _ => return Err(type_error(w, "allocFormWithProto: proto must be a Form")),
        };
        let f = Form::with_proto(Value::Form(proto_id));
        let id = w.alloc(f);
        Ok(Value::Form(id))
    }).expect("install_native at boot — substrate bug");

    // [Heap slotOf: v at: 'name] — single-slot read.
    w.install_native(proto, "slotOf:at:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let sym = args.get(1).copied().and_then(|x| x.as_sym()).ok_or_else(|| {
            type_error(w, "slotOf:at: expects a Symbol key")
        })?;
        match w.effective_form_id(v) {
            Some(id) => Ok(w.form_slot(id, sym)),
            None => Ok(Value::Nil),
        }
    }).expect("install_native at boot — substrate bug");

    // [Heap handlerOf: v at: 'sel] — single-handler read.
    w.install_native(proto, "handlerOf:at:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let sel = args.get(1).copied().and_then(|x| x.as_sym()).ok_or_else(|| {
            type_error(w, "handlerOf:at: expects a Symbol key")
        })?;
        match w.effective_form_id(v) {
            Some(id) => Ok(w.form_handler(id, sel).unwrap_or(Value::Nil)),
            None => Ok(Value::Nil),
        }
    }).expect("install_native at boot — substrate bug");

    // [Heap metaOf: v at: 'sym] — single-meta read.
    w.install_native(proto, "metaOf:at:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let sym = args.get(1).copied().and_then(|x| x.as_sym()).ok_or_else(|| {
            type_error(w, "metaOf:at: expects a Symbol key")
        })?;
        match v {
            Value::Form(id) => Ok(w.form_meta(id, sym)),
            _ => Ok(Value::Nil),
        }
    }).expect("install_native at boot — substrate bug");

    // [Heap slotKeysOf: v] — Cons of slot keys (Symbols). nil for
    // tagged immediates without a singleton-Form. nursery-aware:
    // unions canonical keys with delta keys during an active turn
    // for pre-existing forms (so in-turn writes show up in
    // reflection without waiting for commit).
    macro_rules! key_list_on_heap {
        ($w:expr, $proto:expr, $sel:literal, $keys_method:ident) => {
            $w.install_native($proto, $sel, |w, _self, args| {
                let v = args.first().copied().unwrap_or(Value::Nil);
                match w.effective_form_id(v) {
                    Some(id) => {
                        let keys: Vec<Value> = w
                            .$keys_method(id)
                            .into_iter()
                            .map(Value::Sym)
                            .collect();
                        Ok(w.make_list(&keys))
                    }
                    None => Ok(Value::Nil),
                }
            }).expect("install_native at boot — substrate bug");
        };
    }
    key_list_on_heap!(w, proto, "slotKeysOf:", form_slot_keys);
    key_list_on_heap!(w, proto, "handlerKeysOf:", form_handler_keys);
    key_list_on_heap!(w, proto, "metaKeysOf:", form_meta_keys);

    // bind globally as `Heap`. capitalized like Compiler / Match —
    // module-style, not primordial cap (no `$`).
    let global = w.here_form;
    let name = w.intern("Heap");
    let name_meta = w.intern("name");
    w.form_meta_set(proto, name_meta, Value::Sym(name))
        .expect("form_meta_set at boot — substrate bug");
    w.env_bind(global, name, Value::Form(proto))
        .expect("env_bind at boot — substrate bug");

}

// ─────────────────────────────────────────────────────────────────
// Chunks singleton — exposes chunk side-tables (chunk_ops,
// chunk_consts, chunk_ics) to moof. Method reflection methods
// (:body, :params, :consts, :bytecodes, :ics) live in
// stdlib/method.moof and call into Chunks.
// ─────────────────────────────────────────────────────────────────

fn install_chunks_singleton(w: &mut World) {
    let _ = ensure_opcode_proto(w);
    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // [Chunks isChunk?: v] — true iff v is a Form with chunk
    // side-tables (i.e. emitted bytecode lives at this id).
    w.install_native(proto, "isChunk?:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        if let Value::Form(id) = v {
            return Ok(Value::Bool(w.chunk_ops.contains_key(&id)));
        }
        Ok(Value::Bool(false))
    }).expect("install_native at boot — substrate bug");

    // [Chunks paramsListOf: m] — Cons of param symbols. closures
    // store them on :params; chunks store on the chunk's :params
    // slot. nil for non-Form receivers.
    w.install_native(proto, "paramsListOf:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let id = match v {
            Value::Form(id) => id,
            _ => return Ok(Value::Nil),
        };
        let p = w.form_slot(id, w.params_sym);
        if !p.is_nil() {
            return Ok(p);
        }
        if let Some(cid) = chunk_id_of(w, v) {
            return Ok(w.form_slot(cid, w.params_sym));
        }
        Ok(Value::Nil)
    }).expect("install_native at boot — substrate bug");

    // [Chunks constsListOf: m] — Cons of the chunk's constants.
    w.install_native(proto, "constsListOf:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let cid = match chunk_id_of(w, v) {
            Some(c) => c,
            None => return Ok(Value::Nil),
        };
        let consts = w.chunk_consts.get(&cid).cloned().unwrap_or_default();
        Ok(w.make_list(&consts))
    }).expect("install_native at boot — substrate bug");

    // [Chunks opsListOf: m] — Cons of opcode-Forms decoded from
    // the chunk. each Op enum variant becomes a Form via opcode_form.
    w.install_native(proto, "opsListOf:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let cid = match chunk_id_of(w, v) {
            Some(c) => c,
            None => return Ok(Value::Nil),
        };
        let ops = w.chunk_ops.get(&cid).cloned().unwrap_or_default();
        let entries: Vec<Value> = ops.into_iter().map(|op| opcode_form(w, op)).collect();
        Ok(w.make_list(&entries))
    }).expect("install_native at boot — substrate bug");

    // [Chunks icsListOf: m] — Cons of IC snapshot Forms. each entry
    // is a small Form with `:cached-proto, :cached-method,
    // :cached-defining, :cached-generation` slots.
    w.install_native(proto, "icsListOf:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let cid = match chunk_id_of(w, v) {
            Some(c) => c,
            None => return Ok(Value::Nil),
        };
        let ics = w.chunk_ics.get(&cid).cloned().unwrap_or_default();
        let cached_proto_sym = w.intern("cached-proto");
        let cached_method_sym = w.intern("cached-method");
        let cached_defining_sym = w.intern("cached-defining");
        let cached_generation_sym = w.intern("cached-generation");
        let object_proto = Value::Form(w.protos.object);
        let mut entries = Vec::with_capacity(ics.len());
        for ic in &ics {
            let mut form = Form::with_proto(object_proto);
            let proto_v = if ic.cached_proto.is_none() {
                Value::Nil
            } else {
                Value::Form(ic.cached_proto)
            };
            let method_v = if ic.cached_method.is_none() {
                Value::Nil
            } else {
                Value::Form(ic.cached_method)
            };
            let defining_v = if ic.cached_defining.is_none() {
                Value::Nil
            } else {
                Value::Form(ic.cached_defining)
            };
            form.slots.insert(cached_proto_sym, proto_v);
            form.slots.insert(cached_method_sym, method_v);
            form.slots.insert(cached_defining_sym, defining_v);
            form.slots
                .insert(cached_generation_sym, Value::Int(ic.cached_generation as i64));
            entries.push(Value::Form(w.alloc(form)));
        }
        Ok(w.make_list(&entries))
    }).expect("install_native at boot — substrate bug");

    // [Chunks bodyOf: m] — chunk-Form for a closure (via :body slot)
    // or self if m IS a chunk; nil otherwise.
    w.install_native(proto, "bodyOf:", |w, _self, args| {
        let v = args.first().copied().unwrap_or(Value::Nil);
        let id = match v {
            Value::Form(id) => id,
            _ => return Ok(Value::Nil),
        };
        let body = w.form_slot(id, w.body_sym);
        if let Some(bid) = body.as_form_id() {
            if w.chunk_ops.contains_key(&bid) {
                return Ok(Value::Form(bid));
            }
        }
        if w.chunk_ops.contains_key(&id) {
            return Ok(Value::Form(id));
        }
        Ok(Value::Nil)
    }).expect("install_native at boot — substrate bug");

    let global = w.here_form;
    let name = w.intern("Chunks");
    let name_meta = w.intern("name");
    w.form_meta_set(proto, name_meta, Value::Sym(name))
        .expect("form_meta_set at boot — substrate bug");
    w.env_bind(global, name, Value::Form(proto))
        .expect("env_bind at boot — substrate bug");
}

fn install_object_reflection(w: &mut World) {
    // :proto, :identity, :source, :slotKeys, :handlerKeys, :metaKeys,
    // :handlerAt:, :metaAt: are moof. they live as defmethods that
    // delegate to the Heap singleton (compiler/00-helpers.moof for
    // :proto since the compiler needs it before stdlib loads;
    // stdlib/object.moof for the rest).
    //
    // Object's rust contribution shrinks to the irreducible identity-
    // and-dispatch primitives:

    w.install_native(w.protos.object, "is", |_, self_, args| {
        // identity equality (same heap-id or same tagged-immediate).
        Ok(Value::Bool(self_ == args[0]))
    }).expect("install_native at boot — substrate bug");

    // Object's `:=` is identity equality by default. specific protos
    // (Integer, Symbol, etc.) override with structural equality.
    w.install_native(w.protos.object, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    }).expect("install_native at boot — substrate bug");

    w.install_native(w.protos.object, "toString", |w, self_, _| {
        // default rendering: `<Form#N>` for heap forms; tagged
        // immediates have their own to-string overrides.
        // forms carrying a `:name` meta render as that name — so
        // `[Integer toString]` → `Integer`, `[Macros toString]` →
        // `Macros`, etc.
        let text = match self_ {
            Value::Form(id) => {
                let name_meta = w.intern("name");
                match w.form_meta(id, name_meta) {
                    Value::Sym(s) => w.resolve(s).to_string(),
                    _ => format!("<Form#{}>", id.0),
                }
            }
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
    }).expect("install_native at boot — substrate bug");

    // :inspect and :!= are defmethods in stdlib/object.moof. :initialize
    // stays here because Object:new (above) sends :initialize to every
    // freshly allocated Form, including [Object new] in compiler/00-
    // helpers.moof which loads BEFORE stdlib/object.moof.
    w.install_native(w.protos.object, "initialize", |_, self_, _| Ok(self_)).expect("install_native at boot — substrate bug");

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
        // V2 task-9 — seal-after-initialize in frozen-by-default mode.
        if w.vat_mode == crate::VatMode::FrozenByDefault {
            w.freeze(id)?;
        }
        Ok(instance)
    }).expect("install_native at boot — substrate bug");

    // V2 task-10 — freeze primitives, reachable from moof.
    //   :freeze      — seal self; raises 'cannot-freeze-live for
    //                  forms whose proto chain hits live_protos.
    //   :frozen?     — Bool, nursery-aware (sees in-turn freezes).
    //   :freezable?  — Bool, !is_frozen and !is_live.
    // tagged immediates (Int, Bool, Sym, Char, Float, Nil) are
    // inherently immutable: :frozen? answers true, :freezable?
    // answers false (no mutable state to seal).
    w.install_native(w.protos.object, "freeze", |w, self_, _args| {
        // tagged immediates (Int, Bool, Sym, Char, Float, Nil) are
        // inherently immutable — no-op return self. matches the
        // spec §7.3 "non-Form values: no-op (already-immutable)"
        // policy and stays consistent with :frozen? returning true
        // and :freezable? returning false on the same receivers.
        match self_.as_form_id() {
            Some(id) => {
                w.freeze(id)?;
                Ok(self_)
            }
            None => Ok(self_),
        }
    })
    .expect("install_native :freeze at boot — substrate bug");

    w.install_native(w.protos.object, "frozen?", |w, self_, _args| {
        match self_.as_form_id() {
            Some(id) => Ok(Value::Bool(w.is_frozen(id))),
            None => Ok(Value::Bool(true)),
        }
    })
    .expect("install_native :frozen? at boot — substrate bug");

    w.install_native(w.protos.object, "freezable?", |w, self_, _args| {
        match self_.as_form_id() {
            Some(id) => Ok(Value::Bool(w.freezable(id))),
            None => Ok(Value::Bool(false)),
        }
    })
    .expect("install_native :freezable? at boot — substrate bug");

    // default :initialize is a no-op. user protos override.
    // :initialize is defined in lib/bootstrap.moof as an identity
    // no-op; user protos override it to construct.

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
    ).expect("install_native at boot — substrate bug");
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

    // V2 task-10: cap-bearing protos are live by spec §4 — any form
    // whose proto chain hits this one (the primordial $out / $err and
    // user-allocated Console subclasses) refuses :freeze with
    // 'cannot-freeze-live. without this insert, [$out freeze] would
    // silently seal a cap.
    w.live_protos.insert(console_proto);

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
        let fd_value = w.form_slot(id, fd_sym);
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
    }).expect("install_native at boot — substrate bug");

    // :close — phase A: no-op. phase B's mco wires up real fd cleanup.
    w.install_native(console_proto, "close", |_, _, _| Ok(Value::Nil)).expect("install_native at boot — substrate bug");

    // :next — Console is sink-only. raising lives here because we
    // don't yet expose a `raise` primitive to moof; phase B adds it
    // alongside the effect-intent model.
    w.install_native(console_proto, "next", |w, _, _| {
        Err(RaiseError::new(
            w.intern("not-supported"),
            ":next on a Console (write-only)",
        ))
    }).expect("install_native at boot — substrate bug");

    // :say:, :show:, :done? are derived in lib/bootstrap.moof.

    // primordial $out, $err — fd held in a real ForeignHandle.
    // the supervisor (here: the substrate at boot) is the *only*
    // place these are constructed. user code reaches them via
    // env_lookup; cannot mint new Console instances pointing at
    // stdout/stderr without supervisor authority.
    let out_id = make_primordial_console(w, console_proto, ConsoleTarget::Stdout);
    let err_id = make_primordial_console(w, console_proto, ConsoleTarget::Stderr);

    let global = w.here_form;
    let dollar_out = w.intern("$out");
    let dollar_err = w.intern("$err");
    w.env_bind(global, dollar_out, Value::Form(out_id))
        .expect("env_bind at boot — substrate bug");
    w.env_bind(global, dollar_err, Value::Form(err_id))
        .expect("env_bind at boot — substrate bug");

    // expose the proto by name so user code can subclass.
    // (`[Console new]` would yield a Console without an :fd slot;
    // `:emit:` would raise. real fd capture lands in phase A.9
    // when the mco loader exposes os-side primitives.)
    let console_name = w.intern("Console");
    w.env_bind(global, console_name, Value::Form(console_proto))
        .expect("env_bind at boot — substrate bug");
}

fn install_compiler_cap(w: &mut World) {
    // `$compiler` — primordial cap that controls which compiler is
    // canonical. one proto-Form, two methods. flipping useMoof
    // routes every subsequent compile through the Compiler singleton
    // defined in lib/compiler/. useSeed flips back (mostly a
    // diagnostics knob).
    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    w.install_native(proto, "useMoof", |w, _self, _args| {
        w.use_moof_compiler = true;
        Ok(Value::Nil)
    }).expect("install_native at boot — substrate bug");
    w.install_native(proto, "useSeed", |w, _self, _args| {
        w.use_moof_compiler = false;
        Ok(Value::Nil)
    }).expect("install_native at boot — substrate bug");

    let global = w.here_form;
    let dollar = w.intern("$compiler");
    w.env_bind(global, dollar, Value::Form(proto))
        .expect("env_bind at boot — substrate bug");
}

// ─────────────────────────────────────────────────────────────────
// global callables
// ─────────────────────────────────────────────────────────────────

fn install_globals(w: &mut World) {
    // moof discipline (process/docs-driven.md): free functions are
    // reserved for *constructors with no meaningful receiver* and
    // *substrate metaprogramming primitives*. user-data ops like
    // `length`, `map`, `+` etc. are methods on the receiver.

    // (intern "text") — produce a Symbol with that text. used by
    // moof-level macro plumbing (e.g. building keyword selectors
    // by joining the kw parts) and by user code that constructs
    // names dynamically. accepts a String or a Symbol (idempotent).
    install_global(w, "intern", |world, _, args| {
        if args.len() != 1 {
            return Err(raise(world, "arity", "(intern text)"));
        }
        let text = match args[0] {
            Value::Sym(s) => return Ok(Value::Sym(s)),
            v => world.string_text(v).map(|t| t.to_string()),
        };
        let text = text.ok_or_else(|| {
            type_error(world, "intern: expects a String or Symbol")
        })?;
        Ok(Value::Sym(world.intern(&text)))
    });

    // (currentFrame) — returns a Form snapshot of the topmost
    // running frame, or nil if not inside any frame. honors
    // reflection-contract.md R3 — the moof view of a frame is a
    // Form with proto `Frame` carrying `:chunk :pc :env :self
    // :stack-base :defining-proto`.
    install_global(w, "currentFrame", |world, _, args| {
        if !args.is_empty() {
            return Err(raise(world, "arity", "(currentFrame) takes no args"));
        }
        let n = world.vm.frames.len();
        if n == 0 {
            return Ok(Value::Nil);
        }
        Ok(world.frame_snapshot(n - 1).unwrap_or(Value::Nil))
    });

    // (callStack) — returns a List of Form snapshots, outermost
    // first, of every frame on the runtime call stack.
    install_global(w, "callStack", |world, _, args| {
        if !args.is_empty() {
            return Err(raise(world, "arity", "(callStack) takes no args"));
        }
        Ok(world.frame_stack_snapshot())
    });

    // (raise: kind message) — raise a moof-level error from inside
    // moof. `kind` is a Symbol naming the error category; `message`
    // is a String. caught by the same propagation machinery as
    // rust-side raises. used by user-extensible failure paths
    // (e.g., the `match` macro emits this when no clause matches).
    install_global(w, "raise:", |world, _, args| {
        if args.len() != 2 {
            return Err(raise(world, "arity", "(raise: kind message)"));
        }
        let kind = args[0]
            .as_sym()
            .ok_or_else(|| type_error(world, "raise: kind must be a Symbol"))?;
        let msg = world
            .string_text(args[1])
            .map(|s| s.to_string())
            .ok_or_else(|| type_error(world, "raise: message must be a String"))?;
        Err(RaiseError::new(kind, msg))
    });

    // (cons head tail) — list constructor with no meaningful
    // receiver among args. lowers to `[tail cons: car]`.
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
    //
    // matches the compiler's single-list calling convention: the
    // macro is invoked with one arg = the list `(a b)`.
    install_global(w, "macroexpand", |world, _, args| {
        if args.len() != 1 {
            return Err(raise(world, "arity", "macroexpand: (macroexpand 'form)"));
        }
        let elems = world
            .list_to_vec(args[0])
            .map_err(|_| type_error(world, "macroexpand: arg must be a list-form"))?;
        if elems.is_empty() {
            return Err(raise(world, "macroexpand", "empty form"));
        }
        let head = elems[0]
            .as_sym()
            .ok_or_else(|| raise(world, "macroexpand", "form head is not a symbol"))?;
        let macro_v = world.macro_at(head).ok_or_else(|| {
            raise(
                world,
                "macroexpand",
                format!("`{}` is not a macro", world.resolve(head)),
            )
        })?;
        let mid = macro_v
            .as_form_id()
            .ok_or_else(|| raise(world, "macroexpand", "macro entry is not a Form"))?;
        let args_list = world.make_list(&elems[1..]);
        world.invoke(mid, Value::Nil, &[args_list], FormId::NONE)
    });

    // (append xs ys …) — concatenate lists left-to-right. used by
    // quasiquote splicing. (append) → '(); (append xs) → xs.
    install_global(w, "append", |world, _, args| {
        // empty list is `Value::Nil`; a one-element list whose
        // only element is `nil` is `Form(head=Nil, tail=Nil)`.
        // iteration must distinguish those — the only termination
        // signal is `cur` becoming `Value::Nil`. earlier code
        // additionally broke on `head.is_nil() && tail.is_nil()`
        // and ate the trailing nil-element; that broke
        // `(append (list nil) …)` and quasiquote-splicing of
        // single-nil-element lists, which match's macro relies on.
        let mut out: Vec<Value> = Vec::new();
        let car_sym = world.intern("car");
        let cdr_sym = world.intern("cdr");
        for &arg in args {
            let mut cur = arg;
            while let Some(fid) = cur.as_form_id() {
                let f = world.heap.get(fid);
                if f.proto != Value::Form(world.protos.cons) {
                    break;
                }
                let head = f.slot(car_sym);
                let tail = f.slot(cdr_sym);
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
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        // singleton-only: tagged immediates without a singleton-
        // Form have no per-instance slots; return nil.
        match w.effective_form_id(args[0]) {
            Some(id) => Ok(w.form_slot(id, name)),
            None => Ok(Value::Nil),
        }
    });
    install_global(w, "slotSet!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(slot-set! v 'name value)",
            ));
        }
        // nil is a singleton — its writes route to the hidden
        // nil-handlers form.
        let id = target_form_id(w, args[0], "slot-set!");
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        w.form_slot_set(id, name, args[2])?;
        Ok(args[2])
    });
    // (metaSet! v 'name value) — analog of slotSet! for the meta
    // table. used by compiler.moof to write `:source` / `:macro` /
    // etc. metas at compile time.
    install_global(w, "metaSet!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(meta-set! v 'name value)",
            ));
        }
        let id = target_form_id(w, args[0], "meta-set!");
        let name = args[1].as_sym().ok_or_else(|| {
            type_error(w, "meta-set!: name must be a symbol")
        })?;
        w.form_meta_set(id, name, args[2])?;
        Ok(args[2])
    });
    // (globalEnv) — return the world's global env Form. used by
    // compiler.moof's compile-defmacro to populate the method's
    // `:env` slot, and anywhere else compile-time code needs the
    // canonical top-level env. honors reflection-contract R6 (the
    // env is just a Form; nothing hidden).
    install_global(w, "globalEnv", |w, _, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "(globalEnv) takes no args"));
        }
        Ok(Value::Form(w.here_form))
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
        let global = w.here_form;
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
        w.env_bind(global, name_sym, v)?;
        Ok(v)
    });
    // (set-handler! Proto 'sel fn) — moldable-substrate primitive.
    // bumps the proto's generation counter so existing inline
    // caches re-resolve on next dispatch.
    //
    // nil is a singleton: `(setHandler! nil 'foo fn)` routes to
    // the hidden nil-handlers form. enables `(defmethod nil …)`
    // in moof code without exposing the form under a name.
    install_global(w, "setHandler!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(set-handler! Proto 'sel fn)",
            ));
        }
        let proto_id = target_form_id(w, args[0], "set-handler!");
        let sel = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "set-handler! selector must be a symbol")
        })?;
        w.form_handler_set(proto_id, sel, args[2])?;
        // bump generation so existing ICs invalidate.
        // (`docs/laws/substrate-laws.md` L10.)
        w.bump_proto_generation(proto_id)?;
        Ok(args[2])
    });

    // (__instantiate-mco-bytes bytes) — instantiate a wasm-mco from
    // raw Bytes value. returns a fresh proto-Form. caller ($mco cap)
    // is responsible for fetching bytes and verifying hash.
    install_global(w, "__instantiate-mco-bytes", |w, _, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "(__instantiate-mco-bytes bytes)"));
        }
        let bytes: Vec<u8> = match w.bytes_data(args[0]) {
            Some(b) => b.to_vec(),
            None => return Err(type_error(w, "__instantiate-mco-bytes: arg must be Bytes")),
        };
        crate::wasm::load_wasm_bytes(w, &bytes, "embedded-mco")
    });

    // (__read-file-bytes path) → Bytes.
    // substrate-direct fs access (not WASI-routed). privileged
    // intrinsic used only by the $mco cap. per spec LB-3 / Q1.
    install_global(w, "__read-file-bytes", |w, _, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "(__read-file-bytes path)"));
        }
        // clone to string to drop the immutable borrow before further use.
        let path: String = match w.string_text(args[0]) {
            Some(s) => s.to_string(),
            None => return Err(type_error(w, "__read-file-bytes: arg must be String")),
        };
        match std::fs::read(&path) {
            Ok(bytes) => Ok(w.make_bytes(&bytes)),
            Err(e) => Err(raise(w, "io-error", format!("read {}: {}", path, e))),
        }
    });

    // every `__list-*` free fn is gone. the moof compiler walks lists
    // via `[Heap slotOf: x at: 'cdr]` and `[v is nil]` (both rust
    // install_natives), and uses self-recursion on Compiler for
    // counting (see `Compiler:argc:` in compiler/00-helpers.moof).
}

// ─────────────────────────────────────────────────────────────────
// compiler primitives — `docs/reference/compiler-primitives.md`
//
// the chunk-construction api exposed to moof. every entry here is
// the moof-visible counterpart of one rust-side compiler operation;
// `lib/compiler.moof` (when written) builds chunks by composing
// these sends. paired with the read-side reflection api
// (`[m bytecodes]`, `[m consts]`, `[m ics]`), they make bytecode
// bidirectionally moldable — substrate-laws.md L5 holds in both
// directions.
//
// shape: per `process/docs-driven.md`, sends-to-receivers, not
// free functions. constructors live as class-side methods on the
// `Opcode` proto (`[Opcode loadConst: 5]`); chunk lifecycle is a
// mix of class-side (`[Chunk new: ps source: src]`) and
// instance-side (`[c emit: op]`) methods on `Chunk`.
//
// validation discipline: per-variant constructors are pure value
// builders; range checks and shape validation happen at *emit*
// time, in `decode_op_form`. raises are tagged `'compile-error`
// (shape) or `'range-error` (operand bounds).
// ─────────────────────────────────────────────────────────────────

/// helper: extract the chunk-FormId from a receiver (the `self_`
/// of a `:emit:` / `:addConst:` / etc. send), raising a clean
/// type-error if it isn't a registered chunk.
fn chunk_self(world: &mut World, self_: Value, op: &str) -> Result<FormId, RaiseError> {
    let id = self_.as_form_id().ok_or_else(|| {
        type_error(world, format!("{}: receiver is not a Form", op))
    })?;
    if !world.chunk_ops.contains_key(&id) {
        return Err(type_error(
            world,
            format!("{}: receiver is not a registered chunk", op),
        ));
    }
    Ok(id)
}

fn install_compiler_primitives(w: &mut World) {
    // ensure the Opcode proto exists (idempotent — also called from
    // install_method_reflection; mk_op_form depends on it).
    let opcode_proto = ensure_opcode_proto(w);

    // ── opcode constructors — class-side on Opcode ───────────────
    //
    // sending one of these to `Opcode` returns a fresh opcode-Form.
    // the receiver is the proto-Form `Opcode`; in moof's flat
    // prototype model, "class-side" means handlers on the proto
    // itself (consulted before walking up the proto chain). same
    // pattern as `[Table new]`.
    //
    // each constructor stuffs its args into the Form's `:operands`
    // table and is type-checked when the form meets a chunk via
    // `[c emit: …]`.

    // nullary `[Opcode foo]` constructors.
    w.install_native(opcode_proto, "pushNil", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode pushNil] takes no args"));
        }
        Ok(mk_op_form(w, "PushNil", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "pushTrue", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode pushTrue] takes no args"));
        }
        Ok(mk_op_form(w, "PushTrue", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "pushFalse", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode pushFalse] takes no args"));
        }
        Ok(mk_op_form(w, "PushFalse", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "pop", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode pop] takes no args"));
        }
        Ok(mk_op_form(w, "Pop", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "dup", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode dup] takes no args"));
        }
        Ok(mk_op_form(w, "Dup", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "loadSelf", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode loadSelf] takes no args"));
        }
        Ok(mk_op_form(w, "LoadSelf", &[]))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "return", |w, _self, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[Opcode return] takes no args"));
        }
        Ok(mk_op_form(w, "Return", &[]))
    }).expect("install_native at boot — substrate bug");

    // unary `[Opcode foo: x]` constructors.
    w.install_native(opcode_proto, "loadConst:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[Opcode loadConst: x] takes 1 arg"));
        }
        Ok(mk_op_form(w, "LoadConst", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "loadName:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[Opcode loadName: 'n] takes 1 arg"));
        }
        Ok(mk_op_form(w, "LoadName", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "storeName:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[Opcode storeName: 'n] takes 1 arg"));
        }
        Ok(mk_op_form(w, "StoreName", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "defineGlobal:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(
                w,
                "arity",
                "[Opcode defineGlobal: 'n] takes 1 arg",
            ));
        }
        Ok(mk_op_form(w, "DefineGlobal", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "pushClosure:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(
                w,
                "arity",
                "[Opcode pushClosure: c] takes 1 arg",
            ));
        }
        Ok(mk_op_form(w, "PushClosure", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "jump:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[Opcode jump: o] takes 1 arg"));
        }
        Ok(mk_op_form(w, "Jump", args))
    }).expect("install_native at boot — substrate bug");
    w.install_native(opcode_proto, "jumpIfFalse:", |w, _self, args| {
        if args.len() != 1 {
            return Err(raise(
                w,
                "arity",
                "[Opcode jumpIfFalse: o] takes 1 arg",
            ));
        }
        Ok(mk_op_form(w, "JumpIfFalse", args))
    }).expect("install_native at boot — substrate bug");

    // [Opcode send: 'sel argc: a ic: i]
    w.install_native(opcode_proto, "send:argc:ic:", |w, _self, args| {
        if args.len() != 3 {
            return Err(raise(
                w,
                "arity",
                "[Opcode send: 'sel argc: a ic: i] takes 3 args",
            ));
        }
        Ok(mk_op_form(w, "Send", args))
    }).expect("install_native at boot — substrate bug");

    // [Opcode tailSend: 'sel argc: a]
    w.install_native(opcode_proto, "tailSend:argc:", |w, _self, args| {
        if args.len() != 2 {
            return Err(raise(
                w,
                "arity",
                "[Opcode tailSend: 'sel argc: a] takes 2 args",
            ));
        }
        Ok(mk_op_form(w, "TailSend", args))
    }).expect("install_native at boot — substrate bug");

    // [Opcode superSend: 'sel argc: a ic: i]
    w.install_native(opcode_proto, "superSend:argc:ic:", |w, _self, args| {
        if args.len() != 3 {
            return Err(raise(
                w,
                "arity",
                "[Opcode superSend: 'sel argc: a ic: i] takes 3 args",
            ));
        }
        Ok(mk_op_form(w, "SuperSend", args))
    }).expect("install_native at boot — substrate bug");

    // ── chunk lifecycle — Chunk class-side and instance-side ─────

    // [Chunk new: params source: source] — class-side constructor.
    // installed on the Chunk proto-Form's own handler table; sending
    // to `Chunk` consults it before walking up.
    w.install_native(w.protos.chunk, "new:source:", |w, _self, args| {
        if args.len() != 2 {
            return Err(raise(
                w,
                "arity",
                "[Chunk new: ps source: src] takes 2 args",
            ));
        }
        let params_v = args[0];
        let source_v = args[1];
        let params_vec = w.list_to_vec(params_v).map_err(|_| {
            type_error(w, "Chunk new:source:: params must be a list of Symbols")
        })?;
        for p in &params_vec {
            if !matches!(p, Value::Sym(_)) {
                return Err(type_error(
                    w,
                    "Chunk new:source:: each param must be a Symbol",
                ));
            }
        }
        let mut chunk_form = Form::with_proto(Value::Form(w.protos.chunk));
        chunk_form.slots.insert(w.params_sym, params_v);
        chunk_form.meta.insert(w.source_sym, source_v);
        let chunk_id = w.alloc(chunk_form);
        w.chunk_ops.insert(chunk_id, Vec::new());
        w.chunk_consts.insert(chunk_id, Vec::new());
        w.chunk_ics.insert(chunk_id, Vec::new());
        Ok(Value::Form(chunk_id))
    }).expect("install_native at boot — substrate bug");

    // [chunk emit: op-form] — append op; return its position.
    w.install_native(w.protos.chunk, "emit:", |w, self_, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[chunk emit: op] takes 1 arg"));
        }
        let chunk_id = chunk_self(w, self_, "emit:")?;
        let op = decode_op_form(w, args[0])?;
        let ops = w.chunk_ops.get_mut(&chunk_id).unwrap();
        let pos = ops.len();
        ops.push(op);
        Ok(Value::Int(pos as i64))
    }).expect("install_native at boot — substrate bug");

    // [chunk addConst: value] — append; return its index.
    w.install_native(w.protos.chunk, "addConst:", |w, self_, args| {
        if args.len() != 1 {
            return Err(raise(w, "arity", "[chunk addConst: v] takes 1 arg"));
        }
        let chunk_id = chunk_self(w, self_, "addConst:")?;
        let consts = w.chunk_consts.get_mut(&chunk_id).unwrap();
        let idx = consts.len();
        if idx >= u16::MAX as usize {
            return Err(raise(
                w,
                "range-error",
                "addConst:: constant pool exceeds 65535",
            ));
        }
        consts.push(args[0]);
        Ok(Value::Int(idx as i64))
    }).expect("install_native at boot — substrate bug");

    // [chunk addIc] — reserve an inline-cache slot; return idx.
    w.install_native(w.protos.chunk, "addIc", |w, self_, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[chunk addIc] takes no args"));
        }
        let chunk_id = chunk_self(w, self_, "addIc")?;
        let ics = w.chunk_ics.get_mut(&chunk_id).unwrap();
        let idx = ics.len();
        if idx >= u16::MAX as usize {
            return Err(raise(
                w,
                "range-error",
                "addIc: ic pool exceeds 65535",
            ));
        }
        ics.push(crate::world::ICache::default());
        Ok(Value::Int(idx as i64))
    }).expect("install_native at boot — substrate bug");

    // [chunk jumpTarget] — current ops length.
    w.install_native(w.protos.chunk, "jumpTarget", |w, self_, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[chunk jumpTarget] takes no args"));
        }
        let chunk_id = chunk_self(w, self_, "jumpTarget")?;
        Ok(Value::Int(w.chunk_ops[&chunk_id].len() as i64))
    }).expect("install_native at boot — substrate bug");

    // [chunk patchJump: pos to: target] — overwrite the offset of the
    // jump already at `pos`. computes `target - pos` per the VM's
    // `(pc - 1) + off` formula.
    w.install_native(w.protos.chunk, "patchJump:to:", |w, self_, args| {
        use crate::opcodes::Op;
        if args.len() != 2 {
            return Err(raise(
                w,
                "arity",
                "[chunk patchJump: pos to: tgt] takes 2 args",
            ));
        }
        let chunk_id = chunk_self(w, self_, "patchJump:to:")?;
        let pos = args[0].as_int().ok_or_else(|| {
            type_error(w, "patchJump:to:: pos must be Integer")
        })?;
        let tgt = args[1].as_int().ok_or_else(|| {
            type_error(w, "patchJump:to:: target must be Integer")
        })?;
        let off = tgt - pos;
        let off_i16 = i16::try_from(off).map_err(|_| {
            raise(
                w,
                "range-error",
                format!("patchJump:to:: offset {} doesn't fit i16", off),
            )
        })?;
        let pos_idx: usize = pos.try_into().map_err(|_| {
            raise(
                w,
                "range-error",
                "patchJump:to:: pos must be non-negative",
            )
        })?;
        if pos_idx >= w.chunk_ops[&chunk_id].len() {
            return Err(raise(
                w,
                "range-error",
                "patchJump:to:: pos out of range",
            ));
        }
        let cur_op = w.chunk_ops[&chunk_id][pos_idx];
        let new_op = match cur_op {
            Op::Jump(_) => Op::Jump(off_i16),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(off_i16),
            _ => {
                return Err(raise(
                    w,
                    "compile-error",
                    "patchJump:to:: op at pos is not a jump",
                ));
            }
        };
        w.chunk_ops.get_mut(&chunk_id).unwrap()[pos_idx] = new_op;
        Ok(Value::Nil)
    }).expect("install_native at boot — substrate bug");

    // [chunk asClosure] — wrap the chunk in a Closure-Form ready to
    // call. captures the global env + nil self (top-level).
    w.install_native(w.protos.chunk, "asClosure", |w, self_, args| {
        if !args.is_empty() {
            return Err(raise(w, "arity", "[chunk asClosure] takes no args"));
        }
        let chunk_id = chunk_self(w, self_, "asClosure")?;
        let mut f = Form::with_proto(Value::Form(w.protos.closure));
        f.slots.insert(w.body_sym, Value::Form(chunk_id));
        f.slots.insert(w.env_sym, Value::Form(w.here_form));
        let captured_self_sym = w.intern("captured-self");
        f.slots.insert(captured_self_sym, Value::Nil);
        let params = w.form_slot(chunk_id, w.params_sym);
        f.slots.insert(w.params_sym, params);
        let source = w.form_meta(chunk_id, w.source_sym);
        if !source.is_nil() {
            f.meta.insert(w.source_sym, source);
        }
        Ok(Value::Form(w.alloc(f)))
    }).expect("install_native at boot — substrate bug");
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
    let global = w.here_form;
    w.env_bind(global, name_sym, Value::Form(id))
        .expect("env_bind at boot — substrate bug");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(w: &mut World, src: &str) -> Result<Value, RaiseError> {
        // route through crate::eval so the read+compile+run_top
        // sequence runs inside an implicit turn — the moof-side
        // compiler dispatches sends that mutate via env_bind, which
        // requires `in_turn`.
        crate::eval(w, src)
    }

    fn fresh() -> World {
        // for tests that exercise stdlib methods (like :empty? on
        // List), we need the full new_world() with bootstrap.moof
        // loaded. tests of *intrinsics-only* behavior call new_world_bare().
        crate::new_world()
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
        let car_sym = w.intern("car");
        // build (1 2 3) and inspect
        let v = ev(&mut w, "(list 1 2 3)").unwrap();
        assert_eq!(w.send(v, car_sym, &[]).unwrap(), Value::Int(1));
        let cdr_sym = w.intern("cdr");
        let tail = w.send(v, cdr_sym, &[]).unwrap();
        assert_eq!(w.send(tail, car_sym, &[]).unwrap(), Value::Int(2));
        // (cons 0 (list 1 2 3)) → list with first element 0
        let consed = ev(&mut w, "(cons 0 (list 1 2 3))").unwrap();
        assert_eq!(w.send(consed, car_sym, &[]).unwrap(), Value::Int(0));
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
        // wrap a turn around manual compile + send (both can mutate
        // through nursery-aware setters).
        w.start_turn();
        let f = w.read("(+ 1 2)").unwrap();
        let chunk = crate::compiler::compile(&mut w, f).unwrap();
        let source_sel = w.intern("source");
        let r = w.send(Value::Form(chunk), source_sel, &[]).unwrap();
        let _ = w.commit_turn();
        // r should be the original parsed form (a list).
        assert_eq!(r, f);
    }

    #[test]
    fn reflection_slots_returns_table() {
        // [v slots] returns a Table keyed by slot-name → value
        // (concepts/forms.md, laws/reflection-contract.md R7).
        let mut w = fresh();
        // wrap a turn — `send` may mutate via dispatch-side writes.
        w.start_turn();
        let mut f = Form::with_proto(Value::Form(w.protos.object));
        let a = w.intern("a");
        let b = w.intern("b");
        f.slots.insert(a, Value::Int(1));
        f.slots.insert(b, Value::Int(2));
        let id = w.alloc(f);
        let slots_sel = w.intern("slots");
        let r = w.send(Value::Form(id), slots_sel, &[]).unwrap();
        let _ = w.commit_turn();
        // the returned Table has both slot names as keys.
        let r_repr = w.table_repr(r).unwrap();
        assert_eq!(r_repr.size(), 2);
        assert_eq!(r_repr.keyed.get(&Value::Sym(a)).copied(), Some(Value::Int(1)));
        assert_eq!(r_repr.keyed.get(&Value::Sym(b)).copied(), Some(Value::Int(2)));
    }

    #[test]
    fn integer_inspect_falls_through_to_to_string() {
        let mut w = fresh();
        let r = ev(&mut w, "[42 inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "42");
    }

    #[test]
    fn here_is_bound_to_self() {
        let mut w = crate::new_world_bare();
        let here_sym = w.intern("$here");
        let here_v = w.env_lookup(w.here_form, here_sym);
        assert_eq!(here_v, Some(Value::Form(w.here_form)));
    }

    #[test]
    fn out_cap_is_bound_in_here_form() {
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let v = w.env_lookup(w.here_form, dollar_out).unwrap();
        // it's a Form (a Console instance).
        let id = v.as_form_id().unwrap();
        // its proto is Console.
        let proto = w.heap.get(id).proto;
        // Console isn't on `Protos` (it's a user-visible intrinsic
        // proto living in the global env). check via name lookup.
        let console_sym = w.intern("Console");
        let console_proto = w.env_lookup(w.here_form, console_sym).unwrap();
        assert_eq!(proto, console_proto);
    }

    #[test]
    fn out_cap_responds_to_emit() {
        // we can't easily capture stdout from a unit test; verify
        // that :emit: dispatches without panicking on a valid call.
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.here_form, dollar_out).unwrap();
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
        // wrap a turn — `send` may mutate via dispatch-side writes.
        w.start_turn();
        // route to stderr so test runner stays happy.
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.here_form, dollar_out).unwrap();
        let label_sym = w.intern("label");
        let stderr_sym = w.intern("stderr");
        let id = out.as_form_id().unwrap();
        // direct canonical write — this $out form was allocated
        // pre-turn, so it's below the watermark; but raw heap.get_mut
        // bypasses nursery semantics by design (this is test-only
        // scaffolding to flip `label` before the send).
        w.heap.get_mut(id).slots.insert(label_sym, Value::Sym(stderr_sym));
        // the actual call:
        let say = w.intern("say:");
        let r = w.send(out, say, &[Value::Int(42)]).unwrap();
        let _ = w.commit_turn();
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
            let v = w.env_lookup(w.here_form, s);
            assert!(
                v.is_none(),
                "forbidden global `{}` is bound (must not be)",
                forbidden
            );
        }
    }

    // ── display / inspect — `:toString` vs `:inspect` split.
    // toString is for `say:` / interpolation (display-friendly);
    // inspect is for the REPL (re-readable).

    #[test]
    fn nil_to_string_is_nil() {
        let mut w = fresh();
        let r = ev(&mut w, "[nil toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "nil");
        let r = ev(&mut w, "[nil inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "nil");
    }

    #[test]
    fn nil_is_its_own_proto() {
        // observationally, nil is a singleton: `[nil proto]` is nil.
        let mut w = fresh();
        let r = ev(&mut w, "[nil proto]").unwrap();
        assert_eq!(r, Value::Nil);
        let r = ev(&mut w, "[[nil proto] toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "nil");
    }

    #[test]
    fn nil_is_a_true_singleton_no_global() {
        // nil is its own proto AND there's no `Nil` global pointing
        // at the underlying proto-Form. moof code uses `nil`
        // directly: `(defmethod nil (length) 0)`. setHandler!
        // routes nil-receivers to the hidden nil-handlers form.
        let mut w = fresh();
        let nil_proto_sym = w.intern("Nil-proto");
        let nil_sym = w.intern("Nil");
        assert!(w.env_lookup(w.here_form, nil_proto_sym).is_none());
        assert!(w.env_lookup(w.here_form, nil_sym).is_none());
        // nil's proto IS nil.
        let r = ev(&mut w, "[nil proto]").unwrap();
        assert_eq!(r, Value::Nil);
        // nil's installed methods (length, empty?, …) still work
        // because dispatch routes through the hidden form.
        let r = ev(&mut w, "[nil length]").unwrap();
        assert_eq!(r, Value::Int(0));
        let r = ev(&mut w, "[nil empty?]").unwrap();
        assert_eq!(r, Value::Bool(true));
    }

    #[test]
    fn tagged_immediates_have_their_own_singleton_state() {
        // ruby/Self model: writing on `5` allocates a singleton-
        // Form for 5, NOT for Integer. `5` and `7` have separate
        // per-instance state even though they share Integer for
        // inherited methods.
        let mut w = fresh();
        ev(
            &mut w,
            "(setHandler! 5 'witness (fn () 'set-on-5))",
        )
        .unwrap();
        // `5` has the singleton method.
        let r = ev(&mut w, "[5 witness]").unwrap();
        assert_eq!(r, Value::Sym(w.intern("set-on-5")));
        // `7` does NOT — it errors with doesNotUnderstand.
        let err = ev(&mut w, "[7 witness]").unwrap_err();
        assert_eq!(
            w.resolve(err.kind),
            "doesNotUnderstand",
            "writing on 5 should not affect 7 (got {:?})",
            err.message
        );
    }

    #[test]
    fn slots_are_per_immediate_not_per_proto() {
        // (slotSet! #true 'flag …) sets a slot on #true's
        // singleton-Form, not on Bool. #false doesn't see it.
        let mut w = fresh();
        ev(&mut w, "(slotSet! #true 'pinned 'yes)").unwrap();
        let r = ev(&mut w, "(slot #true 'pinned)").unwrap();
        assert_eq!(r, Value::Sym(w.intern("yes")));
        let r = ev(&mut w, "(slot #false 'pinned)").unwrap();
        assert_eq!(r, Value::Nil, "#false should not share #true's slot");
        // Bool itself also untouched.
        let r = ev(&mut w, "(slot Bool 'pinned)").unwrap();
        assert_eq!(r, Value::Nil, "writing on #true should not mutate Bool");
    }

    #[test]
    fn reflection_shows_only_singleton_state() {
        // [5 handlers] starts empty (no per-5 state yet).
        let mut w = fresh();
        let r = ev(&mut w, "[5 handlers]").unwrap();
        let r_repr = w.table_repr(r).unwrap();
        assert_eq!(
            r_repr.size(),
            0,
            "[5 handlers] before any singleton install should be empty"
        );
        // after install, [5 handlers] shows the singleton handler.
        ev(&mut w, "(setHandler! 5 'wave (fn () 'wavy))").unwrap();
        let r = ev(&mut w, "[5 handlers]").unwrap();
        let r_repr = w.table_repr(r).unwrap();
        assert_eq!(
            r_repr.size(),
            1,
            "[5 handlers] should now show exactly the singleton handler"
        );
        // Integer's own handler table is unaffected.
        let r = ev(&mut w, "[Integer handlerAt: 'wave]").unwrap();
        assert_eq!(
            r, Value::Nil,
            "Integer should not have :wave installed on it"
        );
    }

    #[test]
    fn lookup_falls_through_singleton_to_class() {
        // a singleton with no `:foo` should still inherit class
        // methods. install :foo on Integer, then on 5's singleton
        // — no shadowing ought to happen since we install
        // different methods. then verify both reachable.
        let mut w = fresh();
        // first allocate 5's singleton with an unrelated method.
        ev(&mut w, "(setHandler! 5 'mine (fn () 'just-5))").unwrap();
        // now Integer-class still wins for inherited methods.
        let r = ev(&mut w, "[5 + 1]").unwrap();
        assert_eq!(r, Value::Int(6), "inherited Integer :+ should still work");
        let r = ev(&mut w, "[5 mine]").unwrap();
        assert_eq!(r, Value::Sym(w.intern("just-5")));
    }

    #[test]
    fn defmethod_nil_works() {
        // user-level: install a method on nil with `(defmethod nil
        // …)`. dispatches normally afterwards.
        let mut w = fresh();
        ev(&mut w, "(defmethod nil (witness) 'nilmark)").unwrap();
        let r = ev(&mut w, "[nil witness]").unwrap();
        let mark = w.intern("nilmark");
        assert_eq!(r, Value::Sym(mark));
    }

    #[test]
    fn proto_to_string_uses_name_meta() {
        let mut w = fresh();
        let r = ev(&mut w, "[Integer toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "Integer");
        let r = ev(&mut w, "[Object toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "Object");
        let r = ev(&mut w, "[Macros toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "Macros");
    }

    #[test]
    fn char_inspect_is_hash_backslash() {
        let mut w = fresh();
        // a printable ASCII char.
        let r = ev(&mut w, "[#\\a inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "#\\a");
        // toString stays just the character.
        let r = ev(&mut w, "[#\\a toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "a");
        // named whitespace.
        let r = ev(&mut w, "[#\\space inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "#\\space");
        let r = ev(&mut w, "[#\\newline inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "#\\newline");
    }

    #[test]
    fn string_inspect_quotes_and_escapes() {
        let mut w = fresh();
        let r = ev(&mut w, "[\"hello\" inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "\"hello\"");
        // escapes: tab, newline, quote.
        let r = ev(&mut w, "[\"a\\tb\" inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "\"a\\tb\"");
        let r = ev(&mut w, "[\"x\\\"y\" inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "\"x\\\"y\"");
        // toString returns the raw text.
        let r = ev(&mut w, "[\"hello\" toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "hello");
    }

    #[test]
    fn table_inspect_recursively_inspects() {
        // Table :inspect distributes over positional + keyed
        // entries, matching the Cons :inspect behavior.
        let mut w = fresh();
        // toString — bare elements (display-friendly).
        let r = ev(&mut w, "[#[1 \"hi\" #\\a] toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "#[1 hi a]");
        // inspect — re-readable, per-element :inspect.
        let r = ev(&mut w, "[#[1 \"hi\" #\\a] inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "#[1 \"hi\" #\\a]");
    }

    #[test]
    fn nested_collection_inspect_propagates_all_the_way_down() {
        // a list of tables of strings — every level uses :inspect
        // recursively so the output is fully re-readable source.
        let mut w = fresh();
        let r = ev(
            &mut w,
            "[(list #[\"a\" \"b\"] #[\"c\" #\\x]) inspect]",
        )
        .unwrap();
        assert_eq!(
            w.string_text(r).unwrap(),
            "(#[\"a\" \"b\"] #[\"c\" #\\x])"
        );
    }

    #[test]
    fn list_inspect_recursively_inspects() {
        let mut w = fresh();
        // toString — bare elements.
        let r = ev(&mut w, "[(list 1 \"hi\" #\\a) toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "(1 hi a)");
        // inspect — elements re-readable.
        let r = ev(&mut w, "[(list 1 \"hi\" #\\a) inspect]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "(1 \"hi\" #\\a)");
    }

    #[test]
    fn closure_to_string_is_concise() {
        let mut w = fresh();
        let r = ev(&mut w, "[(fn (x y) [x + y]) toString]").unwrap();
        assert_eq!(w.string_text(r).unwrap(), "<closure (x y)>");
    }

    // ── compiler primitives — `docs/reference/compiler-primitives.md`
    //
    // these tests exercise the moof-side chunk-construction api.
    // the smoke test (push 1, push 2, send +, return) is the
    // forcing function for track 1: if it returns 3, every
    // primitive is honest end-to-end.

    #[test]
    fn compiler_primitives_smoke_test() {
        // a 4-op chunk built from moof, run as a closure.
        let mut w = fresh();
        let r = ev(
            &mut w,
            "(let ((c [Chunk new: '() source: nil]))
               [c addConst: 1]
               [c addConst: 2]
               [c emit: [Opcode loadConst: 0]]
               [c emit: [Opcode loadConst: 1]]
               [c emit: [Opcode send: '+ argc: 1 ic: [c addIc]]]
               [c emit: [Opcode return]]
               [[c asClosure] call])",
        )
        .unwrap();
        assert_eq!(r, Value::Int(3));
    }

    #[test]
    fn compiler_primitives_construct_opcode_form() {
        // [Opcode loadConst: 5] builds a Form with proto Opcode and
        // the expected :op / :operands shape.
        let mut w = fresh();
        let v = ev(&mut w, "[Opcode loadConst: 5]").unwrap();
        let id = v.as_form_id().unwrap();
        let opcode_sym = w.intern("Opcode");
        let opcode_proto_v = w.env_lookup(w.here_form, opcode_sym).unwrap();
        assert_eq!(w.heap.get(id).proto, opcode_proto_v);
        // :op is the symbol 'LoadConst.
        let op_sym = w.intern("op");
        let load_const_sym = w.intern("LoadConst");
        assert_eq!(w.heap.get(id).slot(op_sym), Value::Sym(load_const_sym));
        // :operands is a Table whose [0] is 5.
        let operands_sym = w.intern("operands");
        let operands = w.heap.get(id).slot(operands_sym);
        let r = w.table_repr(operands).unwrap();
        assert_eq!(r.positional, vec![Value::Int(5)]);
    }

    #[test]
    fn chunk_new_registers_side_tables() {
        // a chunk allocated from moof shows up in chunk_ops/consts/ics
        // and reflects through `[m bytecodes]` (currently empty).
        let mut w = fresh();
        let v = ev(&mut w, "[Chunk new: '() source: nil]").unwrap();
        let id = v.as_form_id().unwrap();
        assert!(w.chunk_ops.contains_key(&id));
        assert!(w.chunk_consts.contains_key(&id));
        assert!(w.chunk_ics.contains_key(&id));
        assert!(w.chunk_ops[&id].is_empty());
    }

    #[test]
    fn chunk_emit_returns_position() {
        // each emit returns the index it was emitted at.
        let mut w = fresh();
        ev(&mut w, "(def c [Chunk new: '() source: nil])").unwrap();
        let r0 = ev(&mut w, "[c emit: [Opcode pushNil]]").unwrap();
        let r1 = ev(&mut w, "[c emit: [Opcode pop]]").unwrap();
        assert_eq!(r0, Value::Int(0));
        assert_eq!(r1, Value::Int(1));
    }

    #[test]
    fn chunk_add_const_returns_index() {
        let mut w = fresh();
        ev(&mut w, "(def c [Chunk new: '() source: nil])").unwrap();
        let r0 = ev(&mut w, "[c addConst: 7]").unwrap();
        let r1 = ev(&mut w, "[c addConst: 'foo]").unwrap();
        assert_eq!(r0, Value::Int(0));
        assert_eq!(r1, Value::Int(1));
    }

    #[test]
    fn chunk_emit_raises_on_bad_op() {
        // a non-Opcode value emitted to a chunk raises 'compile-error.
        let mut w = fresh();
        ev(&mut w, "(def c [Chunk new: '() source: nil])").unwrap();
        let err = ev(&mut w, "[c emit: 42]").unwrap_err();
        assert_eq!(w.resolve(err.kind), "type-error");
    }

    #[test]
    fn chunk_emit_raises_on_argc_overflow() {
        // [Opcode send: 'foo argc: 999 ic: 0] doesn't fail at
        // constructor time — but [c emit:] catches it as a range-error.
        let mut w = fresh();
        ev(&mut w, "(def c [Chunk new: '() source: nil])").unwrap();
        let err = ev(
            &mut w,
            "[c emit: [Opcode send: 'foo argc: 999 ic: 0]]",
        )
        .unwrap_err();
        assert_eq!(w.resolve(err.kind), "range-error");
    }

    #[test]
    fn jump_target_and_patch_jump_round_trip() {
        // build a chunk that does:
        //   (if #true 1 2) → equivalent bytecode
        // via the same patchJump dance compile_if uses.
        // expects 1.
        let mut w = fresh();
        let r = ev(
            &mut w,
            "(let ((c [Chunk new: '() source: nil]))
               ;; cond
               [c emit: [Opcode pushTrue]]
               ;; jmp-to-else placeholder (offset patched later)
               (let ((j-else [c emit: [Opcode jumpIfFalse: 0]]))
                 ;; then: push 1
                 [c addConst: 1]
                 [c emit: [Opcode loadConst: 0]]
                 ;; jmp-to-end placeholder
                 (let ((j-end [c emit: [Opcode jump: 0]]))
                   ;; patch jmp-to-else here (else block start)
                   [c patchJump: j-else to: [c jumpTarget]]
                   ;; else: push 2
                   [c addConst: 2]
                   [c emit: [Opcode loadConst: 1]]
                   ;; patch jmp-to-end here (end of if)
                   [c patchJump: j-end to: [c jumpTarget]]
                   ;; return
                   [c emit: [Opcode return]]))
               [[c asClosure] call])",
        )
        .unwrap();
        assert_eq!(r, Value::Int(1));
    }

    #[test]
    fn as_closure_round_trips_through_bytecodes_reflection() {
        // a chunk built from moof reflects the same way one built
        // by the rust compiler does. R2 holds bidirectionally.
        // (reflection methods like :bytecodes live on Method, so we
        // route through the closure — `[m bytecodes]` is the canonical
        // entry point.)
        let mut w = fresh();
        ev(&mut w, "(def c [Chunk new: '() source: nil])").unwrap();
        ev(&mut w, "[c emit: [Opcode pushNil]]").unwrap();
        ev(&mut w, "[c emit: [Opcode return]]").unwrap();
        ev(&mut w, "(def cl [c asClosure])").unwrap();
        let bc = ev(&mut w, "[cl bytecodes]").unwrap();
        let r = w.table_repr(bc).unwrap();
        assert_eq!(r.positional.len(), 2);
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
        assert!(w.env_lookup(w.here_form, dollar_out).is_some());
        assert!(w.env_lookup(w.here_form, dollar_err).is_some());
        assert!(w.env_lookup(w.here_form, dollar_x).is_none());
    }

    // ─────────────────────────────────────────────────────────────
    // V3 Env proto methods — :bind:to:, :set:to:, :lookup:, :parent,
    // :current. dispatch-level coverage; the underlying world env_*
    // APIs are tested directly in world.rs.
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn env_bind_to_via_dispatch() {
        let mut w = fresh();
        let r = ev(&mut w, "[$here bind: 'newGlobal to: 42]").unwrap();
        assert_eq!(r, Value::Int(42));
        let r2 = ev(&mut w, "newGlobal").unwrap();
        assert_eq!(r2, Value::Int(42));
    }

    #[test]
    fn env_set_to_walks_chain_and_returns_value() {
        let mut w = fresh();
        ev(&mut w, "[$here bind: 'x to: 1]").unwrap();
        let r = ev(&mut w, "[$here set: 'x to: 99]").unwrap();
        assert_eq!(r, Value::Int(99));
        let r2 = ev(&mut w, "x").unwrap();
        assert_eq!(r2, Value::Int(99));
    }

    #[test]
    fn env_set_to_raises_unbound_when_not_in_chain() {
        let mut w = fresh();
        let r = ev(&mut w, "[$here set: 'definitelyNotBound to: 5]");
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(w.resolve(err.kind), "unbound");
    }

    #[test]
    fn env_lookup_returns_nil_on_miss() {
        let mut w = fresh();
        let r = ev(&mut w, "[$here lookup: 'definitelyNotBound]").unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn env_parent_returns_nil_at_root() {
        let mut w = fresh();
        let r = ev(&mut w, "[$here parent]").unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn env_current_returns_caller_env() {
        let mut w = fresh();
        let r = ev(&mut w, "[Env current]").unwrap();
        assert!(r.as_form_id().is_some(), "[Env current] should return a Form");
    }

    // ─────────────────────────────────────────────────────────────
    // V3 Closure proto methods — :callIn:withSelf:. the irreducible
    // "run body with explicit env+self" primitive. covers the
    // dispatch path; underlying vm::run_method is exercised by
    // every method invocation already.
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn closure_call_in_with_self_runs_body_with_explicit_env() {
        // (fn () x) — body is a single LoadName(x). normally walks
        // its captured env (the global, where x is unbound). via
        // :callIn:withSelf: we hand it a fresh env where x is bound,
        // and the body resolves x against THAT env, not the captured
        // one. proves the env-override path works end-to-end.
        let mut w = crate::new_world();
        let closure_v = crate::eval(&mut w, "(fn () x)").unwrap();

        let x_sym = w.intern("x");

        // env_a: x = 10. parent → here_form (so other globals still
        // resolve, though the body doesn't need them here).
        let env_a = w.alloc_env(Some(w.here_form));
        w.start_turn();
        w.form_slot_set(env_a, x_sym, Value::Int(10)).unwrap();
        let _ = w.commit_turn();

        // env_b: x = 20.
        let env_b = w.alloc_env(Some(w.here_form));
        w.start_turn();
        w.form_slot_set(env_b, x_sym, Value::Int(20)).unwrap();
        let _ = w.commit_turn();

        let call_in_sym = w.intern("callIn:withSelf:");
        let r1 = w.send(closure_v, call_in_sym, &[Value::Form(env_a), Value::Nil]).unwrap();
        assert_eq!(r1, Value::Int(10));

        let r2 = w.send(closure_v, call_in_sym, &[Value::Form(env_b), Value::Nil]).unwrap();
        assert_eq!(r2, Value::Int(20));
    }
}
