//! the seed compiler — compiles `lib/compiler.moof`, then steps aside.
//!
//! when `world.use_moof_compiler` is on (which `new_world()` flips
//! immediately after this compiler runs over compiler.moof), the
//! [`compile`] entry point delegates to the moof-side `compile-top`.
//! the moof compiler is the canonical artifact; this file is the
//! seed that bootstraps it.
//!
//! the seed handles only the special forms compiler.moof itself
//! uses:
//!
//! | form                           | emits                          |
//! |--------------------------------|--------------------------------|
//! | `def name expr`                | `[$here bind: 'name to: rhs]`  |
//! | `fn (params…) body…`           | sub-chunk + `PushClosure`      |
//! | `if cond then [else]`          | `JumpIfFalse` + `Jump`         |
//! | `let ((name val)…) body…`      | `((fn …) values…)` desugar     |
//! | `do e1 … eN`                   | pop intermediates              |
//! | `quote v`                      | `LoadConst`                    |
//! | `__send__ recv 'sel args…`     | `Send` (or `SuperSend`)        |
//! | `(callable args…)`             | `[callable call: args…]`       |
//!
//! that's it. no `set!`, no `defmacro`, no multi-clause def, no
//! user-macro lookup, no quasiquote / table / object / cascade /
//! defproto / defmethod / defn — those are *all* moof-side concerns
//! once compiler.moof loads. the seed has *one* responsibility:
//! turn compiler.moof into runnable bytecode. then it's done.
//!
//! per `docs/process/self-hosted-compiler.md`, the rust compiler is
//! dead code after the flag flip. it stays buildable (and unit-
//! tested) so that diagnosing a broken compiler.moof is possible
//! without circular dependency.

use crate::form::{Form, FormId};
use crate::opcodes::Op;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{ICache, RaiseError, World};

/// compile a top-level expression into a chunk-Form. the chunk has
/// no params; running it produces the expression's value.
///
/// when `world.use_moof_compiler` is `true`, delegates to the
/// moof-side `compile-top` (in `lib/compiler.moof`). otherwise
/// runs the rust seed compiler — sized to compile exactly
/// compiler.moof. see `docs/process/self-hosted-compiler.md`.
pub fn compile(world: &mut World, form: Value) -> Result<FormId, RaiseError> {
    if world.use_moof_compiler {
        return compile_via_moof(world, form);
    }
    let mut c = Compiler::new(world, Vec::new(), form);
    c.compile_expr(form, true)?;
    c.emit(Op::Return);
    c.finalize()
}

/// route the compile through moof's `[Compiler compileTop: form]`.
/// assumes `compiler.moof` is loaded.
fn compile_via_moof(world: &mut World, form: Value) -> Result<FormId, RaiseError> {
    let compiler_sym = world.intern("Compiler");
    let compiler = world
        .env_lookup(world.here_form, compiler_sym)
        .ok_or_else(|| {
            RaiseError::new(
                world.intern("bootstrap-error"),
                "use_moof_compiler is on but `Compiler` is unbound — \
                 compiler.moof not loaded?",
            )
        })?;
    let compile_top_sym = world.intern("compileTop:");
    let chunk_v = world.send(compiler, compile_top_sym, &[form])?;
    chunk_v.as_form_id().ok_or_else(|| {
        RaiseError::new(
            world.intern("bootstrap-error"),
            "[Compiler compileTop:] returned a non-chunk-Form",
        )
    })
}

/// compile a function body. `params` is the list of parameter
/// symbols. produces a chunk-Form whose `:params` slot is the
/// param list; the VM uses it for arity checking.
fn compile_fn_body(
    world: &mut World,
    params: Vec<SymId>,
    body: Value,
) -> Result<FormId, RaiseError> {
    let mut c = Compiler::new(world, params, body);
    c.compile_expr(body, true)?;
    c.emit(Op::Return);
    c.finalize()
}

/// the per-chunk compilation state.
struct Compiler<'a> {
    world: &'a mut World,
    ops: Vec<Op>,
    consts: Vec<Value>,
    ics_count: u16,
    params: Vec<SymId>,
    /// the source-form for `:source` reflection.
    source: Value,

    // cached SymIds for the seed's special forms. `set_sym`,
    // `defmacro_sym`, `cascade_marker_sym` deliberately absent —
    // the seed doesn't recognize them.
    if_sym: SymId,
    let_sym: SymId,
    do_sym: SymId,
    quote_sym: SymId,
    fn_sym: SymId,
    def_sym: SymId,
    call_sym: SymId,
    self_sym: SymId,
    send_sym: SymId,
    super_sym: SymId,
}

impl<'a> Compiler<'a> {
    fn new(world: &'a mut World, params: Vec<SymId>, source: Value) -> Self {
        let if_sym = world.intern("if");
        let let_sym = world.intern("let");
        let do_sym = world.intern("do");
        let quote_sym = world.intern("quote");
        let fn_sym = world.intern("fn");
        let def_sym = world.intern("def");
        let call_sym = world.intern("call");
        let self_sym = world.intern("self");
        let send_sym = world.intern("__send__");
        let super_sym = world.intern("super");
        Compiler {
            world,
            ops: Vec::new(),
            consts: Vec::new(),
            ics_count: 0,
            params,
            source,
            if_sym,
            let_sym,
            do_sym,
            quote_sym,
            fn_sym,
            def_sym,
            call_sym,
            self_sym,
            send_sym,
            super_sym,
        }
    }

    fn emit(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// add a constant; return its index.
    fn add_const(&mut self, v: Value) -> u16 {
        let idx = self.consts.len();
        assert!(idx < u16::MAX as usize, "constant pool overflow");
        self.consts.push(v);
        idx as u16
    }

    /// reserve a fresh ic slot; return its index.
    fn next_ic(&mut self) -> u16 {
        let idx = self.ics_count;
        self.ics_count = self.ics_count.checked_add(1).expect("ic pool overflow");
        idx
    }

    /// emit a placeholder jump. returns the position of the jump op
    /// for later patching.
    fn emit_placeholder_jump(&mut self, branch_kind: BranchKind) -> usize {
        let pos = self.ops.len();
        self.emit(match branch_kind {
            BranchKind::Always => Op::Jump(0),
            BranchKind::IfFalse => Op::JumpIfFalse(0),
        });
        pos
    }

    /// patch a previously-emitted jump to land at the next op
    /// position.
    fn patch_jump_to_here(&mut self, jump_pos: usize) {
        let target = self.ops.len();
        let off = target as isize - jump_pos as isize;
        let off_i16 = i16::try_from(off).expect("jump offset out of i16 range");
        self.ops[jump_pos] = match self.ops[jump_pos] {
            Op::Jump(_) => Op::Jump(off_i16),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(off_i16),
            other => panic!("not a jump op at {}: {:?}", jump_pos, other),
        };
    }

    /// finalize the chunk: allocate a chunk-Form, register ops/
    /// consts/ics in the world's side tables, return the FormId.
    fn finalize(self) -> Result<FormId, RaiseError> {
        let Compiler {
            world,
            ops,
            consts,
            ics_count,
            params,
            source,
            ..
        } = self;
        let mut chunk_form = Form::with_proto(Value::Form(world.protos.chunk));
        let param_values: Vec<Value> = params.iter().map(|&s| Value::Sym(s)).collect();
        let params_list = world.make_list(&param_values);
        chunk_form.slots.insert(world.params_sym, params_list);
        chunk_form.meta.insert(world.source_sym, source);
        let chunk_id = world.alloc(chunk_form);
        world.chunk_ops.insert(chunk_id, ops);
        world.chunk_consts.insert(chunk_id, consts);
        world
            .chunk_ics
            .insert(chunk_id, vec![ICache::default(); ics_count as usize]);
        Ok(chunk_id)
    }

    fn compile_expr(&mut self, form: Value, tail: bool) -> Result<(), RaiseError> {
        match form {
            Value::Nil => self.emit(Op::PushNil),
            Value::Bool(true) => self.emit(Op::PushTrue),
            Value::Bool(false) => self.emit(Op::PushFalse),
            Value::Int(_) | Value::Float(_) | Value::Char(_) | Value::Foreign(_) => {
                let idx = self.add_const(form);
                self.emit(Op::LoadConst(idx));
            }
            Value::Sym(s) => {
                if s == self.self_sym {
                    self.emit(Op::LoadSelf);
                } else {
                    self.emit(Op::LoadName(s));
                }
            }
            Value::Form(id) => {
                // Cons-Forms are code; any other Form is a literal.
                let proto = self.world.heap.get(id).proto;
                if proto == Value::Form(self.world.protos.cons) {
                    self.compile_form(form, tail)?;
                } else {
                    let idx = self.add_const(form);
                    self.emit(Op::LoadConst(idx));
                }
            }
        }
        Ok(())
    }

    fn compile_form(&mut self, form: Value, tail: bool) -> Result<(), RaiseError> {
        let elems = self.list_elems(form)?;
        if elems.is_empty() {
            self.emit(Op::PushNil);
            return Ok(());
        }
        // dispatch on the head symbol for the seven special forms
        // compiler.moof uses. NO user-macro lookup — compiler.moof
        // uses none, and `bootstrap.moof` (which has the user
        // macros) loads through the moof compiler post-flip.
        if let Value::Sym(s) = elems[0] {
            if s == self.if_sym {
                return self.compile_if(&elems, tail);
            }
            if s == self.let_sym {
                return self.compile_let(&elems, tail);
            }
            if s == self.do_sym {
                return self.compile_do(&elems, tail);
            }
            if s == self.quote_sym {
                return self.compile_quote(&elems);
            }
            if s == self.fn_sym {
                return self.compile_fn(&elems);
            }
            if s == self.def_sym {
                return self.compile_def(&elems);
            }
            if s == self.send_sym {
                return self.compile_send(&elems, tail);
            }
        }
        // fn-call: `(callable arg…)`
        self.compile_call(&elems, tail)
    }

    /// `(__send__ receiver 'selector args…)` — emitted by the
    /// reader for `[recv sel args…]`. lowers to a `Send` opcode
    /// (or `SuperSend` if the receiver is the symbol `super`).
    fn compile_send(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        if elems.len() < 3 {
            return Err(self.err("malformed __send__ form"));
        }
        let receiver = elems[1];
        let selector = elems[2]
            .as_sym()
            .ok_or_else(|| self.err("__send__ selector must be a symbol"))?;
        let args = &elems[3..];

        let is_super = matches!(receiver, Value::Sym(s) if s == self.super_sym);

        if !is_super {
            self.compile_expr(receiver, false)?;
        }
        for &a in args {
            self.compile_expr(a, false)?;
        }
        let argc = u8::try_from(args.len())
            .map_err(|_| self.err("send: too many args (max 255)"))?;
        let ic = self.next_ic();
        self.emit(if is_super {
            Op::SuperSend {
                selector,
                argc,
                ic_idx: ic,
            }
        } else if tail {
            Op::TailSend { selector, argc }
        } else {
            Op::Send {
                selector,
                argc,
                ic_idx: ic,
            }
        });
        Ok(())
    }

    fn list_elems(&self, form: Value) -> Result<Vec<Value>, RaiseError> {
        self.world
            .list_to_vec(form)
            .map_err(|_| RaiseError::new(SymId::NONE, "compiler: expected a list"))
    }

    fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        let (cond, then_branch, else_branch) = match elems.len() {
            3 => (elems[1], elems[2], Value::Nil),
            4 => (elems[1], elems[2], elems[3]),
            _ => return Err(self.err("if takes 2 or 3 args: (if cond then [else])")),
        };
        self.compile_expr(cond, false)?;
        let jmp_to_else = self.emit_placeholder_jump(BranchKind::IfFalse);
        self.compile_expr(then_branch, tail)?;
        let jmp_to_end = self.emit_placeholder_jump(BranchKind::Always);
        self.patch_jump_to_here(jmp_to_else);
        self.compile_expr(else_branch, tail)?;
        self.patch_jump_to_here(jmp_to_end);
        Ok(())
    }

    fn compile_do(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        if elems.len() == 1 {
            self.emit(Op::PushNil);
            return Ok(());
        }
        let body = &elems[1..];
        for (i, &expr) in body.iter().enumerate() {
            let last = i == body.len() - 1;
            self.compile_expr(expr, tail && last)?;
            if !last {
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn compile_quote(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() != 2 {
            return Err(self.err("quote requires 1 arg: (quote v)"));
        }
        let idx = self.add_const(elems[1]);
        self.emit(Op::LoadConst(idx));
        Ok(())
    }

    fn compile_def(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        // V3: (def name value) compiles to Send-based bytecode equivalent
        // to (do [$here bind: 'name to: value] 'name). Op::DefineGlobal is
        // no longer emitted — the only path to env_bind on $here is now
        // method dispatch on Env's :bind:to:.
        //
        // single-binding only: multi-clause defs are a moof-side concern
        // (`defn` macro + compileDef:'s detect-and-reroute). the seed
        // never sees those because compiler.moof uses only single-binding.
        if elems.len() != 3 {
            return Err(self.err(
                "seed compiler: def requires 2 args (multi-clause is moof-only)",
            ));
        }
        let name = elems[1]
            .as_sym()
            .ok_or_else(|| self.err("def's first arg must be a symbol"))?;
        let here_sym = self.world.intern("$here");
        let bind_to_sym = self.world.intern("bind:to:");

        // LoadName $here  (push receiver)
        self.emit(Op::LoadName(here_sym));
        // LoadConst 'name  (push first arg — the symbol)
        let name_const = self.add_const(Value::Sym(name));
        self.emit(Op::LoadConst(name_const));
        // compile rhs  (push second arg — the value)
        // NOTE: use compile_expr (not compile_form) — the rhs may be
        // any expression, including a literal like Value::Int(42).
        self.compile_expr(elems[2], false)?;
        // Send :bind:to: arity=2  (pops receiver + 2 args, pushes result)
        let ic_idx = self.next_ic();
        self.emit(Op::Send {
            selector: bind_to_sym,
            argc: 2,
            ic_idx,
        });
        // discard bind result (the value); push 'name as def's return value
        self.emit(Op::Pop);
        let name_const2 = self.add_const(Value::Sym(name));
        self.emit(Op::LoadConst(name_const2));
        Ok(())
    }

    fn compile_fn(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        // (fn (a b ...) body)
        if elems.len() < 3 {
            return Err(self.err("fn requires params list and body"));
        }
        let params_form = elems[1];
        let params_vec = self
            .world
            .list_to_vec(params_form)
            .map_err(|_| self.err("fn: params must be a list"))?;
        let mut params: Vec<SymId> = Vec::with_capacity(params_vec.len());
        for p in params_vec {
            params.push(
                p.as_sym()
                    .ok_or_else(|| self.err("fn: each param must be a symbol"))?,
            );
        }
        let body_value = if elems.len() == 3 {
            elems[2]
        } else {
            // multi-expression body → wrap in (do …).
            let mut wrapped = vec![Value::Sym(self.do_sym)];
            wrapped.extend_from_slice(&elems[2..]);
            self.world.make_list(&wrapped)
        };
        let chunk_id = compile_fn_body(self.world, params, body_value)?;
        self.emit(Op::PushClosure { chunk: chunk_id });
        Ok(())
    }

    /// `(let ((a 1) (b 2)) body)` ≡ `((fn (a b) body) 1 2)`.
    /// bindings are evaluated in *parallel* (the current env), then
    /// a single new env binds them all before body runs.
    fn compile_let(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        if elems.len() < 3 {
            return Err(self.err("let requires bindings + body"));
        }
        let bindings_form = elems[1];
        let bindings = self
            .world
            .list_to_vec(bindings_form)
            .map_err(|_| self.err("let: bindings must be a list"))?;
        let mut params: Vec<SymId> = Vec::with_capacity(bindings.len());
        let mut value_forms: Vec<Value> = Vec::with_capacity(bindings.len());
        for binding in &bindings {
            let pair = self
                .world
                .list_to_vec(*binding)
                .map_err(|_| self.err("let: each binding is (name value)"))?;
            if pair.len() != 2 {
                return Err(self.err("let: each binding is (name value)"));
            }
            let name = pair[0]
                .as_sym()
                .ok_or_else(|| self.err("let: binding name must be a symbol"))?;
            params.push(name);
            value_forms.push(pair[1]);
        }
        let body_value = if elems.len() == 3 {
            elems[2]
        } else {
            let mut wrapped = vec![Value::Sym(self.do_sym)];
            wrapped.extend_from_slice(&elems[2..]);
            self.world.make_list(&wrapped)
        };
        let chunk_id = compile_fn_body(self.world, params, body_value)?;
        self.emit(Op::PushClosure { chunk: chunk_id });
        for v in &value_forms {
            self.compile_expr(*v, false)?;
        }
        let argc =
            u8::try_from(value_forms.len()).map_err(|_| self.err("let: too many bindings"))?;
        let ic = self.next_ic();
        self.emit(if tail {
            Op::TailSend {
                selector: self.call_sym,
                argc,
            }
        } else {
            Op::Send {
                selector: self.call_sym,
                argc,
                ic_idx: ic,
            }
        });
        Ok(())
    }

    fn compile_call(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        let callable = elems[0];
        let args = &elems[1..];
        self.compile_expr(callable, false)?;
        for &a in args {
            self.compile_expr(a, false)?;
        }
        let argc = u8::try_from(args.len())
            .map_err(|_| self.err("call: too many args (max 255)"))?;
        let ic = self.next_ic();
        self.emit(if tail {
            Op::TailSend {
                selector: self.call_sym,
                argc,
            }
        } else {
            Op::Send {
                selector: self.call_sym,
                argc,
                ic_idx: ic,
            }
        });
        Ok(())
    }

    fn err(&mut self, msg: impl Into<String>) -> RaiseError {
        let kind = self.world.intern("compile-error");
        RaiseError::new(kind, msg)
    }
}

#[derive(Copy, Clone)]
enum BranchKind {
    Always,
    IfFalse,
}

#[cfg(test)]
mod tests {
    //! the seed compiler's unit tests. they exercise the *bare*
    //! compile path (`World::new()` — no bootstrap, no compiler.moof,
    //! flag off) on the seven forms compiler.moof uses. dropped:
    //! tests for `set!`, `defmacro`, multi-clause `def`, user-macro
    //! lookup — those are moof-side and tested in
    //! `tests/moof_compiler.rs` and `tests/doc_alignment.rs`.
    use super::*;

    /// install_native mutates handler/meta tables — these helpers
    /// defensively wrap a turn so callers can use them on a
    /// just-constructed `World::new()` without explicit turn mgmt.
    fn install_arith(w: &mut World) {
        let was_in_turn = w.in_turn();
        if !was_in_turn { w.start_turn(); }
        w.install_native(w.protos.integer, "+", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() + args[0].as_int().unwrap()))
        }).expect("install_native in mutable test");
        w.install_native(w.protos.integer, "-", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() - args[0].as_int().unwrap()))
        }).expect("install_native in mutable test");
        w.install_native(w.protos.integer, "*", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() * args[0].as_int().unwrap()))
        }).expect("install_native in mutable test");
        w.install_native(w.protos.integer, "=", |_, self_, args| {
            Ok(Value::Bool(self_.as_int().unwrap() == args[0].as_int().unwrap()))
        }).expect("install_native in mutable test");
        if !was_in_turn { let _ = w.commit_turn(); }
    }

    fn install_closure_call(w: &mut World) {
        let was_in_turn = w.in_turn();
        if !was_in_turn { w.start_turn(); }
        w.install_native(w.protos.closure, "call", |world, self_, args| {
            let id = self_
                .as_form_id()
                .ok_or_else(|| RaiseError::new(world.intern("dispatch"), "not a closure"))?;
            world.invoke(id, Value::Nil, args, crate::form::FormId::NONE)
        }).expect("install_native in mutable test");
        if !was_in_turn { let _ = w.commit_turn(); }
    }

    /// V3 — the seed `compile_def` now emits `LoadName $here` +
    /// `Send :bind:to: argc=2`. tests using `World::new()` directly
    /// (which skips `intrinsics::install`) must wire up just enough:
    /// bind `$here` self-referentially and install `Env :bind:to:`.
    /// matches what `intrinsics::install_proto_globals` and
    /// `install_env_proto_methods` do for V3 def to work.
    fn install_def_prereqs(w: &mut World) {
        let was_in_turn = w.in_turn();
        if !was_in_turn { w.start_turn(); }
        // bind $here self-referentially in the global env.
        let here_sym = w.intern("$here");
        w.env_bind(w.here_form, here_sym, Value::Form(w.here_form))
            .expect("env_bind in mutable test");
        // install Env's :bind:to: so the Send dispatches.
        // matches `intrinsics::install_env_proto_methods`'s impl:
        // form_slot_set on self, return the bound value.
        w.install_native(w.protos.env, "bind:to:", |world, self_, args| {
            let env_id = self_.as_form_id()
                .ok_or_else(|| RaiseError::new(world.intern("type-error"), ":bind:to: receiver must be a Form"))?;
            let name = args.first().copied().and_then(Value::as_sym)
                .ok_or_else(|| RaiseError::new(world.intern("type-error"), ":bind:to: name must be a Symbol"))?;
            let value = args.get(1).copied().unwrap_or(Value::Nil);
            world.form_slot_set(env_id, name, value)?;
            Ok(value)
        }).expect("install_native in mutable test");
        if !was_in_turn { let _ = w.commit_turn(); }
    }

    fn ev(w: &mut World, src: &str) -> Result<Value, RaiseError> {
        // route through crate::eval so read+compile+run_top all run
        // inside an implicit turn (the moof-side compiler dispatches
        // sends that mutate via env_bind, which requires `in_turn`).
        crate::eval(w, src)
    }

    #[test]
    fn compile_integer_literal() {
        let mut w = World::new();
        assert_eq!(ev(&mut w, "42").unwrap(), Value::Int(42));
    }

    #[test]
    fn compile_def_then_lookup() {
        let mut w = World::new();
        // V3: (def x 5) lowers to Send :bind:to: on $here.
        // wire up the prereqs (since World::new() skips intrinsics).
        install_def_prereqs(&mut w);
        // (def x 5) goes through the seed compiler — and at the
        // moof-compiler flag-on path, `compile` itself dispatches
        // sends that mutate. wrap a turn around the manual
        // compile + run_top, then ev (which manages its own).
        w.start_turn();
        let f = w.read("(def x 5)").unwrap();
        let chunk = compile(&mut w, f).unwrap();
        w.run_top(chunk).unwrap();
        let _ = w.commit_turn();
        assert_eq!(ev(&mut w, "x").unwrap(), Value::Int(5));
    }

    #[test]
    fn compile_if_true_branch() {
        let mut w = World::new();
        let r = ev(&mut w, "(if #true 'yes 'no)").unwrap();
        let yes = w.intern("yes");
        assert_eq!(r, Value::Sym(yes));
    }

    #[test]
    fn compile_if_false_branch() {
        let mut w = World::new();
        let r = ev(&mut w, "(if #false 'yes 'no)").unwrap();
        let no = w.intern("no");
        assert_eq!(r, Value::Sym(no));
    }

    #[test]
    fn compile_if_nil_is_falsy() {
        let mut w = World::new();
        let r = ev(&mut w, "(if nil 'yes 'no)").unwrap();
        let no = w.intern("no");
        assert_eq!(r, Value::Sym(no));
    }

    #[test]
    fn compile_do_returns_last() {
        let mut w = World::new();
        let r = ev(&mut w, "(do 1 2 3)").unwrap();
        assert_eq!(r, Value::Int(3));
    }

    #[test]
    fn compile_quote_preserves_form() {
        let mut w = World::new();
        let r = ev(&mut w, "(quote foo)").unwrap();
        let foo = w.intern("foo");
        assert_eq!(r, Value::Sym(foo));
    }

    #[test]
    fn compile_let_parallel_bindings() {
        let mut w = World::new();
        install_arith(&mut w);
        install_closure_call(&mut w);
        let r = ev(&mut w, "(let ((a 3) (b 4)) a)").unwrap();
        assert_eq!(r, Value::Int(3));
        let r = ev(&mut w, "(let ((a 3) (b 4)) b)").unwrap();
        assert_eq!(r, Value::Int(4));
    }

    #[test]
    fn compile_let_does_not_leak_bindings() {
        let mut w = World::new();
        install_closure_call(&mut w);
        let r = ev(&mut w, "(let ((a 5)) a)").unwrap();
        assert_eq!(r, Value::Int(5));
        let err = ev(&mut w, "a").unwrap_err();
        assert_eq!(w.resolve(err.kind), "unbound");
    }

    #[test]
    fn compile_fn_and_call() {
        let mut w = World::new();
        install_closure_call(&mut w);
        install_def_prereqs(&mut w);
        // wrap manual compile + run_top in a turn (compile may send-
        // dispatch via the moof-compiler path, which mutates).
        w.start_turn();
        let f = w.read("(def square (fn (x) x))").unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let _ = w.commit_turn();
        let r = ev(&mut w, "(square 7)").unwrap();
        assert_eq!(r, Value::Int(7));
    }

    #[test]
    fn compile_closure_captures_env() {
        let mut w = World::new();
        install_arith(&mut w);
        install_closure_call(&mut w);
        install_def_prereqs(&mut w);
        let src = "(def make-add (fn (n) (fn (x) (let ((y x)) y))))";
        // wrap manual compile + run_top in a turn.
        w.start_turn();
        let f = w.read(src).unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let _ = w.commit_turn();
        let _ = ev(&mut w, "(make-add 5)").unwrap();
        let r = ev(&mut w, "((make-add 5) 10)").unwrap();
        assert_eq!(r, Value::Int(10));
    }

    #[test]
    fn compile_chunk_has_source_meta() {
        let mut w = World::new();
        let f = w.read("(if #true 1 2)").unwrap();
        let c = compile(&mut w, f).unwrap();
        let source = w.form_meta(c, w.source_sym);
        assert_eq!(source, f);
    }

    #[test]
    fn compile_chunk_has_params_slot() {
        let mut w = World::new();
        let f = w.read("(fn (x y) x)").unwrap();
        let c = compile(&mut w, f).unwrap();
        assert_eq!(w.form_slot(c, w.params_sym), Value::Nil);
    }

    #[test]
    fn empty_list_evaluates_to_nil() {
        let mut w = World::new();
        assert_eq!(ev(&mut w, "()").unwrap(), Value::Nil);
    }

    #[test]
    fn nested_if_works() {
        let mut w = World::new();
        let r = ev(&mut w, "(if #true (if #false 'a 'b) 'c)").unwrap();
        let b = w.intern("b");
        assert_eq!(r, Value::Sym(b));
    }

    #[test]
    fn compile_recursion_via_def() {
        // recursion through a `def`-bound name. tested without
        // arithmetic to avoid unbound `=` etc.
        let mut w = World::new();
        install_closure_call(&mut w);
        install_def_prereqs(&mut w);
        // wrap manual compile + run_top in a turn.
        w.start_turn();
        let f = w.read("(def f (fn (n) (if n n 'done)))").unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let _ = w.commit_turn();
        let r = ev(&mut w, "(f 1)").unwrap();
        assert_eq!(r, Value::Int(1));
        let r = ev(&mut w, "(f nil)").unwrap();
        let done = w.intern("done");
        assert_eq!(r, Value::Sym(done));
    }

    /// the through-line: the seed compiles compiler.moof. this
    /// integration-flavored test doesn't construct compiler.moof
    /// itself (that's `crate::new_world()`'s job), but verifies the
    /// flag round-trips: with it on, `compile()` delegates to moof.
    #[test]
    fn flag_routes_through_moof_compiler() {
        let mut w = crate::new_world();
        // `new_world` flips the flag during boot.
        assert!(w.use_moof_compiler);
        // a compile after boot runs through the moof compiler and
        // produces a runnable chunk. wrap a turn around the manual
        // compile + run_top — the moof compiler send-dispatches mutate.
        w.start_turn();
        let f = w.read("[1 + 2]").unwrap();
        let c = compile(&mut w, f).unwrap();
        let r = w.run_top(c).unwrap();
        let _ = w.commit_turn();
        assert_eq!(r, Value::Int(3));
    }
}
