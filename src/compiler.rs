//! compiler — Form → Chunk.
//!
//! phase 2 special-cases a handful of operatives at compile time:
//! `def`, `if`, `let`, `fn`, `do`, `quote`. each emits its own
//! bytecode pattern. all other fn-calls compile to "evaluate head
//! and args, then `Send :call` to the result."
//!
//! shutt's kernel ($vau) discipline says special forms are
//! operatives, not a separate category. for phase 2 we keep them as
//! compile-time builtins to get a working bootstrap. user-defined
//! operatives (defop) land later, when the substrate exposes a
//! runtime "this is an operative" check at the call site.

use crate::form::FormId;
use crate::opcodes::{Chunk, Op};
use crate::value::Value;
use crate::world::World;

pub fn compile(world: &mut World, form: Value) -> Result<Chunk, String> {
    let mut chunk = Chunk::new();
    chunk.source = Some(form);
    compile_expr(world, &mut chunk, form)?;
    chunk.ops.push(Op::Return);
    Ok(chunk)
}

fn compile_expr(world: &mut World, chunk: &mut Chunk, form: Value) -> Result<(), String> {
    match form {
        Value::Nil => {
            chunk.ops.push(Op::LoadNil);
            Ok(())
        }
        Value::Bool(b) => {
            chunk.ops.push(Op::LoadBool(b));
            Ok(())
        }
        Value::Int(n) => {
            if let Ok(small) = i32::try_from(n) {
                chunk.ops.push(Op::LoadInt(small));
            } else {
                let idx = chunk.add_const(Value::Int(n));
                chunk.ops.push(Op::LoadConst(idx));
            }
            Ok(())
        }
        Value::Sym(s) => {
            // symbol in code position = name lookup in current env chain.
            chunk.ops.push(Op::LoadName(s));
            Ok(())
        }
        Value::Form(id) => compile_form(world, chunk, id),
    }
}

fn compile_form(world: &mut World, chunk: &mut Chunk, id: FormId) -> Result<(), String> {
    let proto = world.heap.get(id).proto;

    // is this a send-form `[recv selector args...]`?
    if proto == world.send_form_proto {
        return compile_send_form(world, chunk, id);
    }

    // is this a List? if so, treat as fn-call or special form.
    if proto != world.list_proto {
        // a non-list Form treated as a literal. (rare in source.)
        let idx = chunk.add_const(Value::Form(id));
        chunk.ops.push(Op::LoadConst(idx));
        return Ok(());
    }

    let head = world.heap.get(id).head;
    let rest = world.heap.get(id).args;

    // empty list `()` evaluates to nil.
    if matches!(head, Value::Nil) && matches!(rest, Value::Nil) {
        // shouldn't happen normally — Value::Nil is the empty list.
        chunk.ops.push(Op::LoadNil);
        return Ok(());
    }

    // is head a known special form?
    if let Value::Sym(s) = head {
        let name = world.syms.name(s).to_string();
        match name.as_str() {
            "def" => return compile_def(world, chunk, rest),
            "if" => return compile_if(world, chunk, rest),
            "let" => return compile_let(world, chunk, rest),
            "fn" => return compile_fn(world, chunk, rest),
            "do" => return compile_do(world, chunk, rest),
            "quote" => return compile_quote(world, chunk, rest),
            "set!" => return compile_set(world, chunk, rest),
            "defproto" => return compile_defproto(world, chunk, rest),
            _ => {}
        }
    }

    // ordinary fn-call: compile head, compile args, Send :call arity.
    compile_call(world, chunk, head, rest)
}

/// compile a send-form (produced by `[…]` in the reader).
/// emits: receiver, args..., Send opcode.
fn compile_send_form(
    world: &mut World,
    chunk: &mut Chunk,
    id: FormId,
) -> Result<(), String> {
    let recv = world.heap.get(id).head;
    let inner = world.heap.get(id).args;
    let parts = list_to_vec(world, inner)?;
    if parts.is_empty() {
        return Err("send-form: missing selector".into());
    }
    let sel = match parts[0] {
        Value::Sym(s) => s,
        _ => return Err("send-form: selector must be a symbol".into()),
    };
    let args = &parts[1..];

    compile_expr(world, chunk, recv)?;
    for a in args {
        compile_expr(world, chunk, *a)?;
    }
    let arity: u8 = args
        .len()
        .try_into()
        .map_err(|_| "send-form: too many args".to_string())?;
    let ic_idx = chunk.alloc_ic();
    chunk.ops.push(Op::Send {
        sel,
        arity,
        ic_idx,
    });
    Ok(())
}

fn compile_call(
    world: &mut World,
    chunk: &mut Chunk,
    head: Value,
    rest: Value,
) -> Result<(), String> {
    compile_expr(world, chunk, head)?;
    let args = list_to_vec(world, rest)?;
    for a in &args {
        compile_expr(world, chunk, *a)?;
    }
    let arity: u8 = args
        .len()
        .try_into()
        .map_err(|_| "too many arguments".to_string())?;
    let sel = world.syms.intern("call");
    let ic_idx = chunk.alloc_ic();
    chunk.ops.push(Op::Send {
        sel,
        arity,
        ic_idx,
    });
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// special forms
// ─────────────────────────────────────────────────────────────────

fn compile_def(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() != 2 {
        return Err(format!("def: expected `(def name expr)`, got {} parts", parts.len()));
    }
    let name = match parts[0] {
        Value::Sym(s) => s,
        _ => return Err("def: first arg must be a symbol".into()),
    };
    compile_expr(world, chunk, parts[1])?;
    chunk.ops.push(Op::DefineName(name));
    // def returns nil (so `(def x 5)` at the repl shows `()`).
    chunk.ops.push(Op::LoadNil);
    Ok(())
}

fn compile_if(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!(
            "if: expected `(if cond then [else])`, got {} parts",
            parts.len()
        ));
    }
    // compile cond
    compile_expr(world, chunk, parts[0])?;
    // BranchIfFalse to else
    let bf_pos = chunk.emit(Op::BranchIfFalse(0));
    // compile then
    compile_expr(world, chunk, parts[1])?;
    // Branch over else
    let br_pos = chunk.emit(Op::Branch(0));
    // patch BranchIfFalse to here (start of else)
    chunk.patch_branch_to_here(bf_pos)?;
    // compile else (or LoadNil if absent)
    if parts.len() == 3 {
        compile_expr(world, chunk, parts[2])?;
    } else {
        chunk.ops.push(Op::LoadNil);
    }
    // patch Branch to here (after else)
    chunk.patch_branch_to_here(br_pos)?;
    Ok(())
}

fn compile_let(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() < 2 {
        return Err("let: expected `(let ((name expr) ...) body...)`".into());
    }
    // bindings
    let bindings = list_to_vec(world, parts[0])?;
    // body (may be multiple forms; treat as implicit `do`)
    let body: Vec<Value> = parts[1..].to_vec();

    // PushScope, eval each binding+DefineName, eval body, PopScope.
    chunk.ops.push(Op::PushScope);
    for b in &bindings {
        let pair = list_to_vec(world, *b)?;
        if pair.len() != 2 {
            return Err("let: binding must be `(name expr)`".into());
        }
        let name = match pair[0] {
            Value::Sym(s) => s,
            _ => return Err("let: binding name must be a symbol".into()),
        };
        compile_expr(world, chunk, pair[1])?;
        chunk.ops.push(Op::DefineName(name));
    }
    // implicit do for body.
    if body.is_empty() {
        chunk.ops.push(Op::LoadNil);
    } else {
        for (i, expr) in body.iter().enumerate() {
            if i > 0 {
                chunk.ops.push(Op::Pop);
            }
            compile_expr(world, chunk, *expr)?;
        }
    }
    chunk.ops.push(Op::PopScope);
    Ok(())
}

fn compile_fn(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() < 2 {
        return Err("fn: expected `(fn (params...) body...)`".into());
    }
    // params
    let params_form = parts[0];
    let params_vec = list_to_vec(world, params_form)?;
    for p in &params_vec {
        if !matches!(p, Value::Sym(_)) {
            return Err("fn: params must be symbols".into());
        }
    }
    // compile body in a fresh chunk.
    let mut body_chunk = Chunk::new();
    body_chunk.source = Some(rest);
    // body is parts[1..] — implicit do.
    let body = &parts[1..];
    if body.is_empty() {
        body_chunk.ops.push(Op::LoadNil);
    } else {
        for (i, expr) in body.iter().enumerate() {
            if i > 0 {
                body_chunk.ops.push(Op::Pop);
            }
            compile_expr(world, &mut body_chunk, *expr)?;
        }
    }
    body_chunk.ops.push(Op::Return);

    let body_chunk_id = world.add_chunk(body_chunk);
    let chunk_idx = chunk.add_nested(body_chunk_id);
    let params_idx = chunk.add_const(params_form);

    chunk.ops.push(Op::MakeClosure {
        chunk_idx,
        params_idx,
    });
    Ok(())
}

fn compile_do(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.is_empty() {
        chunk.ops.push(Op::LoadNil);
        return Ok(());
    }
    for (i, expr) in parts.iter().enumerate() {
        if i > 0 {
            chunk.ops.push(Op::Pop);
        }
        compile_expr(world, chunk, *expr)?;
    }
    Ok(())
}

fn compile_set(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() != 2 {
        return Err(format!(
            "set!: expected `(set! name expr)`, got {} parts",
            parts.len()
        ));
    }
    let name = match parts[0] {
        Value::Sym(s) => s,
        _ => return Err("set!: first arg must be a symbol".into()),
    };
    compile_expr(world, chunk, parts[1])?;
    chunk.ops.push(Op::SetName(name));
    chunk.ops.push(Op::LoadNil);
    Ok(())
}

/// `(defproto Name (slots a b ...) (handlers (selector (params...) body) ...))`
///
/// allocates a fresh proto Form, compiles each handler clause to a
/// bytecode method (with implicit `self` bound at call time), stores
/// methods in the proto's handler table, sets default slot values to
/// nil, and binds `Name` in the current env.
///
/// implementation note: defproto is a rust special form for phase 2.
/// once defop / macros land, defproto can be expressed in moof using
/// the substrate primitives `make-proto`, `proto-set-handler!`,
/// `set-default-slot!`. that's the docs-driven endpoint.
fn compile_defproto(
    world: &mut World,
    chunk: &mut Chunk,
    rest: Value,
) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.is_empty() {
        return Err("defproto: expected `(defproto Name ...)`".into());
    }
    let name = match parts[0] {
        Value::Sym(s) => s,
        _ => return Err("defproto: first arg must be a Name symbol".into()),
    };

    // optional sub-forms: (proto Parent), (slots a b c), (handlers ...).
    let mut parent_proto: Option<Value> = None;
    let mut slot_names: Vec<crate::sym::SymId> = Vec::new();
    let mut handler_clauses: Vec<Value> = Vec::new();

    for clause in &parts[1..] {
        let cparts = list_to_vec(world, *clause)?;
        if cparts.is_empty() {
            return Err("defproto: empty clause".into());
        }
        let tag = match cparts[0] {
            Value::Sym(s) => world.syms.name(s).to_string(),
            _ => return Err("defproto: clause head must be a symbol".into()),
        };
        match tag.as_str() {
            "proto" => {
                if cparts.len() != 2 {
                    return Err("defproto: (proto Parent) takes exactly one arg".into());
                }
                parent_proto = Some(cparts[1]);
            }
            "slots" => {
                for s in &cparts[1..] {
                    match s {
                        Value::Sym(sym) => slot_names.push(*sym),
                        _ => {
                            return Err(
                                "defproto: slot names must be symbols".into()
                            )
                        }
                    }
                }
            }
            "handlers" => {
                for h in &cparts[1..] {
                    handler_clauses.push(*h);
                }
            }
            other => {
                return Err(format!("defproto: unknown clause `{other}`"));
            }
        }
    }

    // emit code that, at runtime, allocates the proto, populates its
    // slots and handlers, and binds it to `name`.
    //
    // strategy: compile the parent expression (if any), use the
    // `make-proto` builtin to produce a fresh proto. for each
    // handler, compile a method-chunk and use `proto-set-handler!`
    // to install. finally, `define-name` for the name.
    //
    // we reach for the global `make-proto` etc. via LoadName.

    // 1. emit (make-proto parent) — leaves the new proto on the stack.
    let make_proto_sym = world.syms.intern("make-proto");
    chunk.ops.push(Op::LoadName(make_proto_sym));
    if let Some(parent) = parent_proto {
        compile_expr(world, chunk, parent)?;
    } else {
        chunk.ops.push(Op::LoadNil);
    }
    let call_sel = world.syms.intern("call");
    let ic_idx = chunk.alloc_ic();
    chunk.ops.push(Op::Send {
        sel: call_sel,
        arity: 1,
        ic_idx,
    });
    // stack top: [proto-form-id]

    // 3. for each slot, set its default to nil. the proto is on the
    // stack; we DUP it for each call. since we don't have a Dup op
    // yet, we'll store the proto in a scratch local using PushScope
    // + DefineName.
    chunk.ops.push(Op::PushScope);
    let proto_local = world.syms.intern("__defproto_target__");
    chunk.ops.push(Op::DefineName(proto_local));

    // emit slot defaults
    for s in &slot_names {
        // (set-default-slot! proto name nil)
        let set_default = world.syms.intern("set-default-slot!");
        chunk.ops.push(Op::LoadName(set_default));
        chunk.ops.push(Op::LoadName(proto_local));
        chunk.ops.push(Op::LoadSym(*s));
        chunk.ops.push(Op::LoadNil);
        let ic = chunk.alloc_ic();
        chunk.ops.push(Op::Send {
            sel: call_sel,
            arity: 3,
            ic_idx: ic,
        });
        chunk.ops.push(Op::Pop);
    }

    // emit handlers
    for clause in &handler_clauses {
        let cparts = list_to_vec(world, *clause)?;
        if cparts.len() < 2 {
            return Err(
                "defproto handler: expected `(selector (params...) body...)`"
                    .into(),
            );
        }
        let selector = match cparts[0] {
            Value::Sym(s) => s,
            _ => return Err("defproto handler: selector must be a symbol".into()),
        };
        let params_form = cparts[1];
        let body = &cparts[2..];

        // compile body as a method chunk. params_form is a list of
        // param names. self is auto-bound by setup_closure_call.
        let mut method_chunk = Chunk::new();
        method_chunk.source = Some(*clause);
        if body.is_empty() {
            method_chunk.ops.push(Op::LoadNil);
        } else {
            for (i, expr) in body.iter().enumerate() {
                if i > 0 {
                    method_chunk.ops.push(Op::Pop);
                }
                compile_expr(world, &mut method_chunk, *expr)?;
            }
        }
        method_chunk.ops.push(Op::Return);
        let method_chunk_id = world.add_chunk(method_chunk);
        let chunk_idx = chunk.add_nested(method_chunk_id);
        let params_idx = chunk.add_const(params_form);

        // emit: (proto-set-handler! proto 'selector (closure with this method))
        let set_handler = world.syms.intern("proto-set-handler!");
        chunk.ops.push(Op::LoadName(set_handler));
        chunk.ops.push(Op::LoadName(proto_local));
        chunk.ops.push(Op::LoadSym(selector));
        chunk.ops.push(Op::MakeClosure {
            chunk_idx,
            params_idx,
        });
        let ic = chunk.alloc_ic();
        chunk.ops.push(Op::Send {
            sel: call_sel,
            arity: 3,
            ic_idx: ic,
        });
        chunk.ops.push(Op::Pop);
    }

    // 4. retrieve proto and bind to name in OUTER scope
    chunk.ops.push(Op::LoadName(proto_local));
    chunk.ops.push(Op::PopScope);
    chunk.ops.push(Op::DefineName(name));
    chunk.ops.push(Op::LoadNil);
    Ok(())
}

fn compile_quote(world: &mut World, chunk: &mut Chunk, rest: Value) -> Result<(), String> {
    let parts = list_to_vec(world, rest)?;
    if parts.len() != 1 {
        return Err("quote: expected exactly one argument".into());
    }
    let q = parts[0];
    // emit as a constant — the form structure is preserved.
    match q {
        Value::Nil => chunk.ops.push(Op::LoadNil),
        Value::Bool(b) => chunk.ops.push(Op::LoadBool(b)),
        Value::Int(n) => {
            if let Ok(small) = i32::try_from(n) {
                chunk.ops.push(Op::LoadInt(small));
            } else {
                let idx = chunk.add_const(Value::Int(n));
                chunk.ops.push(Op::LoadConst(idx));
            }
        }
        Value::Sym(s) => chunk.ops.push(Op::LoadSym(s)),
        Value::Form(_) => {
            // a list — keep as a constant.
            let idx = chunk.add_const(q);
            chunk.ops.push(Op::LoadConst(idx));
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────

/// flatten a List value into a Vec of its elements.
/// errors on improper lists.
fn list_to_vec(world: &World, list: Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cur = list;
    loop {
        match cur {
            Value::Nil => return Ok(out),
            Value::Form(id) => {
                let f = world.heap.get(id);
                if f.proto != world.list_proto {
                    return Err(format!("not a list: {cur:?}"));
                }
                out.push(f.head);
                cur = f.args;
            }
            _ => return Err(format!("improper list (cdr is {cur:?})")),
        }
    }
}
