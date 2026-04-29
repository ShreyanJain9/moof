//! native methods — phase 2's primitive operators.
//!
//! the rust line provides:
//! - arithmetic (`:+ :- :* :/`) on Integer.
//! - comparison (`:= :< :> :<= :>=`) on Integer.
//! - list ops (cons, head, tail, null?, list?, list).
//! - booleans (not).
//! - io stubs (println, print) — phase 4 turns these into proper caps.
//! - reflection (type-of).
//!
//! everything else lives in `lib/bootstrap.moof` and above (phase 2+).

use crate::form::Form;
use crate::sym::SymId;
use crate::value::Value;
use crate::vm;
use crate::world::World;

// ─────────────────────────────────────────────────────────────────
// Integer methods. invoked via send dispatch from `[a OP b]`.
// ─────────────────────────────────────────────────────────────────

pub fn int_plus(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "+")?;
    let b = expect_int_arg(args, 0, "+")?;
    a.checked_add(b)
        .map(Value::Int)
        .ok_or_else(|| "integer overflow in +".to_string())
}

pub fn int_minus(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "-")?;
    let b = expect_int_arg(args, 0, "-")?;
    a.checked_sub(b)
        .map(Value::Int)
        .ok_or_else(|| "integer overflow in -".to_string())
}

pub fn int_times(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "*")?;
    let b = expect_int_arg(args, 0, "*")?;
    a.checked_mul(b)
        .map(Value::Int)
        .ok_or_else(|| "integer overflow in *".to_string())
}

pub fn int_div(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "/")?;
    let b = expect_int_arg(args, 0, "/")?;
    if b == 0 {
        return Err("division by zero".to_string());
    }
    Ok(Value::Int(a / b))
}

pub fn int_eq(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "=")?;
    let b = expect_int_arg(args, 0, "=")?;
    Ok(Value::Bool(a == b))
}

pub fn int_lt(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "<")?;
    let b = expect_int_arg(args, 0, "<")?;
    Ok(Value::Bool(a < b))
}

pub fn int_gt(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, ">")?;
    let b = expect_int_arg(args, 0, ">")?;
    Ok(Value::Bool(a > b))
}

pub fn int_le(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, "<=")?;
    let b = expect_int_arg(args, 0, "<=")?;
    Ok(Value::Bool(a <= b))
}

pub fn int_ge(_w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = expect_int(recv, ">=")?;
    let b = expect_int_arg(args, 0, ">=")?;
    Ok(Value::Bool(a >= b))
}

// ─────────────────────────────────────────────────────────────────
// global callables. invoked via `(op a b ...)` fn-call.
//
// arithmetic ones forward via send — so the proto-chain is exercised
// and any user-defined override of `:+` (later phases) takes effect.
// ─────────────────────────────────────────────────────────────────

pub fn fn_plus(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    reduce_via_send(w, args, "+", "+")
}

pub fn fn_minus(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("- requires at least one arg".to_string());
    }
    if args.len() == 1 {
        let sel = w.syms.intern("-");
        return vm::send_dispatch(w, Value::Int(0), sel, &[args[0]]);
    }
    reduce_via_send(w, args, "-", "-")
}

pub fn fn_times(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    reduce_via_send(w, args, "*", "*")
}

pub fn fn_div(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err("/ requires at least two args".to_string());
    }
    reduce_via_send(w, args, "/", "/")
}

pub fn fn_eq(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Ok(Value::Bool(true));
    }
    for win in args.windows(2) {
        if !values_equal(w, win[0], win[1])? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// structural equality. tag-immediates compare directly; Forms
/// dispatch through `:=` so user-defined types can override.
/// (laws/substrate-laws.md L3 — equality is just another send when
/// the substrate can't trivially decide.)
fn values_equal(w: &mut World, a: Value, b: Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Nil, Value::Nil) => Ok(true),
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::Int(x), Value::Int(y)) => Ok(x == y),
        (Value::Sym(x), Value::Sym(y)) => Ok(x == y),
        (Value::Form(x), Value::Form(y)) if x == y => Ok(true),
        (Value::Form(_), _) | (_, Value::Form(_)) => {
            // dispatch `=` through the proto chain
            let sel = w.syms.intern("=");
            match vm::send_dispatch(w, a, sel, &[b]) {
                Ok(v) => Ok(v.is_truthy()),
                Err(_) => Ok(false), // doesNotUnderstand → not equal
            }
        }
        _ => Ok(false),
    }
}

pub fn fn_lt(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    cmp_chain(w, args, "<")
}

pub fn fn_gt(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    cmp_chain(w, args, ">")
}

pub fn fn_le(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    cmp_chain(w, args, "<=")
}

pub fn fn_ge(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    cmp_chain(w, args, ">=")
}

fn cmp_chain(w: &mut World, args: &[Value], selector: &str) -> Result<Value, String> {
    if args.len() < 2 {
        return Ok(Value::Bool(true));
    }
    let sel = w.syms.intern(selector);
    for win in args.windows(2) {
        let r = vm::send_dispatch(w, win[0], sel, &[win[1]])?;
        if !r.is_truthy() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ─────────────────────────────────────────────────────────────────
// list ops. Lists are cons-cell Forms; head/tail/cons live in rust
// because direct heap manipulation is needed. moof code uses these
// to define everything else (map, filter, reduce, etc.).
// ─────────────────────────────────────────────────────────────────

pub fn fn_cons(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let car = arg(args, 0, "cons")?;
    let cdr = arg(args, 1, "cons")?;
    let proto = w.list_proto;
    let id = w.heap.alloc(Form::cons(proto, car, cdr));
    Ok(Value::Form(id))
}

pub fn fn_head(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "head")?;
    match v {
        Value::Nil => Err("head: empty list".to_string()),
        Value::Form(id) => {
            let f = w.heap.get(id);
            if f.proto != w.list_proto {
                return Err(format!("head: not a list: {v:?}"));
            }
            Ok(f.head)
        }
        _ => Err(format!("head: not a list: {v:?}")),
    }
}

pub fn fn_tail(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "tail")?;
    match v {
        Value::Nil => Err("tail: empty list".to_string()),
        Value::Form(id) => {
            let f = w.heap.get(id);
            if f.proto != w.list_proto {
                return Err(format!("tail: not a list: {v:?}"));
            }
            Ok(f.args)
        }
        _ => Err(format!("tail: not a list: {v:?}")),
    }
}

pub fn fn_null_q(_w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "null?")?;
    Ok(Value::Bool(matches!(v, Value::Nil)))
}

pub fn fn_list_q(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "list?")?;
    let r = matches!(v, Value::Nil)
        || matches!(v, Value::Form(id) if w.heap.get(id).proto == w.list_proto);
    Ok(Value::Bool(r))
}

pub fn fn_list(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    // (list a b c) → '(a b c). variadic constructor.
    let proto = w.list_proto;
    let mut tail = Value::Nil;
    for v in args.iter().rev() {
        let id = w.heap.alloc(Form::cons(proto, *v, tail));
        tail = Value::Form(id);
    }
    Ok(tail)
}

// ─────────────────────────────────────────────────────────────────
// booleans
// ─────────────────────────────────────────────────────────────────

pub fn fn_not(_w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "not")?;
    Ok(Value::Bool(!v.is_truthy()))
}

pub fn fn_identity(_w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    arg(args, 0, "identity")
}

// ─────────────────────────────────────────────────────────────────
// io — stubs for $out cap (concepts/capabilities.md proper in phase 4).
// ─────────────────────────────────────────────────────────────────

pub fn fn_println(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&crate::print::display(w, *a));
    }
    println!("{}", out);
    Ok(Value::Nil)
}

pub fn fn_print(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&crate::print::display(w, *a));
    }
    print!("{}", out);
    Ok(Value::Nil)
}

// ─────────────────────────────────────────────────────────────────
// String methods — registered on String proto.
// the substrate provides minimal primitives; `lib/bootstrap.moof`
// builds the rest (concat-many, split, lines, reverse, etc.) on top.
// ─────────────────────────────────────────────────────────────────

pub fn str_length(w: &mut World, recv: Value, _args: &[Value]) -> Result<Value, String> {
    let s = w.as_str(recv).ok_or_else(|| "length: not a String".to_string())?;
    Ok(Value::Int(s.chars().count() as i64))
}

pub fn str_byte_length(w: &mut World, recv: Value, _args: &[Value]) -> Result<Value, String> {
    let s = w.as_str(recv).ok_or_else(|| "byte-length: not a String".to_string())?;
    Ok(Value::Int(s.len() as i64))
}

pub fn str_concat(w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = w.as_str(recv).ok_or_else(|| "++: receiver not a String".to_string())?.to_string();
    let b_v = arg(args, 0, "++")?;
    let b = w.as_str(b_v).ok_or_else(|| "++: arg not a String".to_string())?.to_string();
    let id = w.alloc_string(&format!("{a}{b}"));
    Ok(Value::Form(id))
}

pub fn str_eq(w: &mut World, recv: Value, args: &[Value]) -> Result<Value, String> {
    let a = w.as_str(recv).ok_or_else(|| "=: receiver not a String".to_string())?.to_string();
    let b_v = arg(args, 0, "=")?;
    let b = match w.as_str(b_v) {
        Some(s) => s,
        None => return Ok(Value::Bool(false)),
    };
    Ok(Value::Bool(a == b))
}

pub fn fn_str(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    // (str a b c ...) → concatenated display-strings of each arg.
    let mut out = String::new();
    for a in args {
        out.push_str(&crate::print::display(w, *a));
    }
    let id = w.alloc_string(&out);
    Ok(Value::Form(id))
}

pub fn fn_show(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "show")?;
    let id = w.alloc_string(&crate::print::show(w, v));
    Ok(Value::Form(id))
}

// ─────────────────────────────────────────────────────────────────
// REPL primitives — minimal rust support so the loop itself can
// live in moof (lib/bootstrap.moof). these call into the substrate
// for the parts that have to be in rust: reading from stdin, parsing
// a string into a Form, evaluating a Form.
//
// a real `$io` capability comes in phase 4 (concepts/capabilities.md);
// for now these stubs are bound globally.
// ─────────────────────────────────────────────────────────────────

pub fn fn_read_line(w: &mut World, _recv: Value, _args: &[Value]) -> Result<Value, String> {
    use std::io::{BufRead, Write};
    std::io::stdout().flush().ok();
    let stdin = std::io::stdin();
    let mut line = String::new();
    let n = stdin.lock().read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(Value::Nil); // EOF
    }
    // strip trailing newline
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    let id = w.alloc_string(&line);
    Ok(Value::Form(id))
}

pub fn fn_parse(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "parse")?;
    let s = w
        .as_str(v)
        .ok_or_else(|| "parse: arg must be a String".to_string())?
        .to_string();
    crate::reader::read(w, &s)
}

pub fn fn_parse_all(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "parse-all")?;
    let s = w
        .as_str(v)
        .ok_or_else(|| "parse-all: arg must be a String".to_string())?
        .to_string();
    let forms = crate::reader::read_all(w, &s)?;
    // wrap as a List
    let proto = w.list_proto;
    let mut tail = Value::Nil;
    for v in forms.iter().rev() {
        let id = w.heap.alloc(crate::form::Form::cons(proto, *v, tail));
        tail = Value::Form(id);
    }
    Ok(tail)
}

pub fn fn_eval(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let form = arg(args, 0, "eval")?;
    crate::eval_form(w, form)
}

pub fn fn_try_eval(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    // returns either ('ok value) or ('error message-string)
    let form = arg(args, 0, "try-eval")?;
    let proto = w.list_proto;
    match crate::eval_form(w, form) {
        Ok(v) => {
            let ok_sym = w.syms.intern("ok");
            let nil = Value::Nil;
            let cdr = w.heap.alloc(crate::form::Form::cons(proto, v, nil));
            let outer = w.heap.alloc(crate::form::Form::cons(proto, Value::Sym(ok_sym), Value::Form(cdr)));
            Ok(Value::Form(outer))
        }
        Err(msg) => {
            let err_sym = w.syms.intern("error");
            let s = w.alloc_string(&msg);
            let nil = Value::Nil;
            let cdr = w.heap.alloc(crate::form::Form::cons(proto, Value::Form(s), nil));
            let outer = w.heap.alloc(crate::form::Form::cons(proto, Value::Sym(err_sym), Value::Form(cdr)));
            Ok(Value::Form(outer))
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// slot access — read/write a Form's slot by symbol.
// the substrate-level primitive for object state. user code can
// build syntactic sugar (`.name`, etc.) on top in later phases.
// ─────────────────────────────────────────────────────────────────

pub fn fn_slot(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let target = arg(args, 0, "slot")?;
    let name_v = arg(args, 1, "slot")?;
    let id = expect_form(target, "slot")?;
    let name = expect_sym(name_v, "slot")?;
    Ok(w.heap.get(id).slots.get(&name).copied().unwrap_or(Value::Nil))
}

pub fn fn_slot_set(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let target = arg(args, 0, "slot-set!")?;
    let name_v = arg(args, 1, "slot-set!")?;
    let value = arg(args, 2, "slot-set!")?;
    let id = expect_form(target, "slot-set!")?;
    let name = expect_sym(name_v, "slot-set!")?;
    w.heap.get_mut(id).slots.insert(name, value);
    Ok(Value::Nil)
}

pub fn fn_has_slot_q(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let target = arg(args, 0, "has-slot?")?;
    let name_v = arg(args, 1, "has-slot?")?;
    let id = expect_form(target, "has-slot?")?;
    let name = expect_sym(name_v, "has-slot?")?;
    Ok(Value::Bool(w.heap.get(id).slots.contains_key(&name)))
}

// ─────────────────────────────────────────────────────────────────
// proto manipulation — used by `defproto` to build user types.
// concepts/objects-and-protos.md: protos are mutable; adding a
// handler to a proto makes it visible on all instances.
// ─────────────────────────────────────────────────────────────────

pub fn fn_make_proto(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let parent = args.get(0).copied().unwrap_or(Value::Nil);
    let parent_id = match parent {
        Value::Nil => w.object,
        Value::Form(id) => id,
        _ => return Err("make-proto: parent must be a Form or nil".into()),
    };
    let id = w.heap.alloc(crate::form::Form::with_proto(parent_id));
    Ok(Value::Form(id))
}

pub fn fn_proto_set_handler(
    w: &mut World,
    _recv: Value,
    args: &[Value],
) -> Result<Value, String> {
    let target = arg(args, 0, "proto-set-handler!")?;
    let selector_v = arg(args, 1, "proto-set-handler!")?;
    let method = arg(args, 2, "proto-set-handler!")?;
    let id = expect_form(target, "proto-set-handler!")?;
    let sel = expect_sym(selector_v, "proto-set-handler!")?;
    // method must be a callable Form (Closure or Builtin).
    let method_id = expect_form(
        method,
        "proto-set-handler!: method must be callable",
    )?;
    let call_sym = w.call_sym;
    let inner = w
        .heap
        .get(method_id)
        .handlers
        .get(&call_sym)
        .cloned()
        .ok_or_else(|| {
            "proto-set-handler!: provided value has no :call handler".to_string()
        })?;
    // when the callable is a closure (Bytecode method), we want the
    // proto's handler entry to carry the same chunk/env/params, so
    // that when an *instance* receives this message later, dispatch
    // sets `self` to the instance — not to the original closure.
    // (the inner MethodImpl already has chunk/env/params; we just
    // store it as-is.)
    w.heap.get_mut(id).handlers.insert(sel, inner);
    Ok(Value::Nil)
}

pub fn fn_set_default_slot(
    w: &mut World,
    _recv: Value,
    args: &[Value],
) -> Result<Value, String> {
    let target = arg(args, 0, "set-default-slot!")?;
    let name_v = arg(args, 1, "set-default-slot!")?;
    let default = arg(args, 2, "set-default-slot!")?;
    let id = expect_form(target, "set-default-slot!")?;
    let name = expect_sym(name_v, "set-default-slot!")?;
    w.heap.get_mut(id).slots.insert(name, default);
    Ok(Value::Nil)
}

pub fn fn_new(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    // (new Proto) → fresh instance with proto's slot defaults copied in.
    let target = arg(args, 0, "new")?;
    let proto_id = expect_form(target, "new")?;
    let mut form = crate::form::Form::with_proto(proto_id);
    // copy proto's default slot values to the new instance. (the
    // proto holds them as initial slot values; instances inherit a
    // shallow copy.)
    let defaults: Vec<(crate::sym::SymId, Value)> = w
        .heap
        .get(proto_id)
        .slots
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    for (k, v) in defaults {
        form.slots.insert(k, v);
    }
    let id = w.heap.alloc(form);
    Ok(Value::Form(id))
}

// ─────────────────────────────────────────────────────────────────
// reflection
// ─────────────────────────────────────────────────────────────────

pub fn fn_type_of(w: &mut World, _recv: Value, args: &[Value]) -> Result<Value, String> {
    let v = arg(args, 0, "type-of")?;
    let name = match v {
        Value::Nil => "Nil",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Integer",
        Value::Sym(_) => "Symbol",
        Value::Form(id) => {
            let proto = w.heap.get(id).proto;
            if proto == w.list_proto {
                "List"
            } else if proto == w.string_proto {
                "String"
            } else if proto == w.builtin_proto {
                "Builtin"
            } else if proto == w.closure_proto {
                "Closure"
            } else if proto == w.env_proto {
                "Env"
            } else {
                "Form"
            }
        }
    };
    let sym = w.syms.intern(name);
    Ok(Value::Sym(sym))
}

// ─────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────

fn reduce_via_send(
    w: &mut World,
    args: &[Value],
    selector: &str,
    op_name: &str,
) -> Result<Value, String> {
    if args.is_empty() {
        return Err(format!("{op_name} requires at least one arg"));
    }
    let sel = w.syms.intern(selector);
    let mut acc = args[0];
    for a in &args[1..] {
        acc = vm::send_dispatch(w, acc, sel, &[*a])?;
    }
    Ok(acc)
}

fn arg(args: &[Value], idx: usize, op: &str) -> Result<Value, String> {
    args.get(idx)
        .copied()
        .ok_or_else(|| format!("{op}: missing arg {idx}"))
}

fn expect_int(v: Value, op: &str) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(n),
        _ => Err(format!("{op} expects Integer, got {v:?}")),
    }
}

fn expect_int_arg(args: &[Value], idx: usize, op: &str) -> Result<i64, String> {
    args.get(idx)
        .copied()
        .ok_or_else(|| format!("{op} missing arg {idx}"))
        .and_then(|v| expect_int(v, op))
}

fn expect_form(v: Value, op: &str) -> Result<crate::form::FormId, String> {
    match v {
        Value::Form(id) => Ok(id),
        _ => Err(format!("{op}: expects a Form, got {v:?}")),
    }
}

fn expect_sym(v: Value, op: &str) -> Result<SymId, String> {
    match v {
        Value::Sym(s) => Ok(s),
        _ => Err(format!("{op}: expects a Symbol, got {v:?}")),
    }
}

#[allow(dead_code)]
fn interned(w: &mut World, s: &str) -> SymId {
    w.syms.intern(s)
}
