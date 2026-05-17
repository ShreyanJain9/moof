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
    ///
    /// V3 task 13 — the seed `compile_if` lowers to Send-based
    /// bytecode (so user code can override `:ifTrue:ifFalse:`); the
    /// peephole in `compile_send` recognizes the post-expansion
    /// shape and folds it back into Jump-based emission inline,
    /// using these helpers.
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
    ///
    /// V3 task 13 — see `emit_placeholder_jump`'s note: used by the
    /// if-peephole in `compile_send`.
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
        // V3 task 13 peephole — recognize the post-macro-expansion
        // shape of `if`:
        //   (__send__ (__send__ c '!!) 'ifTrue:ifFalse:
        //             (fn () t) (fn () e))
        // where the two thunks are syntactic (fn () body) literals.
        // emit Jump-based bytecode inline — no closure allocations,
        // no method dispatch on Bool. preserves the user-overridable
        // macro semantic at the source layer (user code calling
        // `[recv ifTrue:ifFalse: tThunk eThunk]` explicitly does NOT
        // match the syntactic shape and falls through to standard
        // Send dispatch).
        if let Some((c_form, t_body, e_body)) = self.match_if_pattern(elems) {
            return self.compile_if_inline(c_form, t_body, e_body, tail);
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

    /// V3 task 13 — recognize the post-macro-expansion shape of `if`:
    ///   `(__send__ (__send__ c '!!) 'ifTrue:ifFalse: (fn () t) (fn () e))`.
    /// returns Some((c, t-body, e-body)) on match; None otherwise.
    fn match_if_pattern(&self, elems: &[Value]) -> Option<(Value, Value, Value)> {
        // elems[0] = '__send__, elems[1] = receiver, elems[2] = selector,
        // elems[3..] = args. expect exactly 5 elements (recv + sel + 2 args).
        if elems.len() != 5 {
            return None;
        }
        let selector = elems[2].as_sym()?;
        if self.world.resolve(selector) != "ifTrue:ifFalse:" {
            return None;
        }
        // receiver must be `(__send__ c '!!)` — three elems.
        let recv_elems = self.list_elems_lenient(elems[1])?;
        if recv_elems.len() != 3 {
            return None;
        }
        let outer_head = recv_elems[0].as_sym()?;
        if self.world.resolve(outer_head) != "__send__" {
            return None;
        }
        let recv_inner_sel = recv_elems[2].as_sym()?;
        if self.world.resolve(recv_inner_sel) != "!!" {
            return None;
        }
        let c_form = recv_elems[1];
        // both args must be syntactic (fn () body) literals.
        let t_body = self.match_zero_arg_fn(elems[3])?;
        let e_body = self.match_zero_arg_fn(elems[4])?;
        Some((c_form, t_body, e_body))
    }

    /// recognize `(fn () body)`. returns Some(body) if matched.
    fn match_zero_arg_fn(&self, form: Value) -> Option<Value> {
        let elems = self.list_elems_lenient(form)?;
        if elems.len() != 3 {
            return None;
        }
        let head = elems[0].as_sym()?;
        if self.world.resolve(head) != "fn" {
            return None;
        }
        // empty params list is Value::Nil after make_list(&[]).
        if !matches!(elems[1], Value::Nil) {
            return None;
        }
        Some(elems[2])
    }

    /// like `list_elems` but Option instead of Result — used by the
    /// peephole matcher where mismatch is "no opt", not error.
    fn list_elems_lenient(&self, form: Value) -> Option<Vec<Value>> {
        self.world.list_to_vec(form).ok()
    }

    /// emit Jump-based bytecode for the if-shape inline:
    ///   <compile c>
    ///   Send :!! argc=0          ; coerce to Bool — preserves user
    ///                            ; overrides of :!!.
    ///   JumpIfFalse else_label
    ///   <compile t inline>
    ///   Jump end_label
    ///   else_label: <compile e inline>
    ///   end_label:
    fn compile_if_inline(
        &mut self,
        c_form: Value,
        t_body: Value,
        e_body: Value,
        tail: bool,
    ) -> Result<(), RaiseError> {
        // compile c — non-tail.
        self.compile_expr(c_form, false)?;
        // Send :!! argc=0 — preserves user overrides of :!!. (the
        // VM's JumpIfFalse uses is_truthy, which already maps nil
        // and #false to falsy; user types can override :!! to
        // redefine truthiness, so we keep the coercion.)
        let bang_bang = self.world.intern("!!");
        let ic_bang = self.next_ic();
        self.emit(Op::Send {
            selector: bang_bang,
            argc: 0,
            ic_idx: ic_bang,
        });
        // JumpIfFalse else_label (placeholder — patched later).
        let jif = self.emit_placeholder_jump(BranchKind::IfFalse);
        // compile then-branch inline; tail iff `if` was tail.
        self.compile_expr(t_body, tail)?;
        // Jump end_label (placeholder — patched after else compiles).
        let jmp = self.emit_placeholder_jump(BranchKind::Always);
        // patch JumpIfFalse to land here (start of else).
        self.patch_jump_to_here(jif);
        // compile else-branch inline; tail iff `if` was tail.
        self.compile_expr(e_body, tail)?;
        // patch unconditional Jump to land here (after else).
        self.patch_jump_to_here(jmp);
        Ok(())
    }

    fn list_elems(&self, form: Value) -> Result<Vec<Value>, RaiseError> {
        self.world
            .list_to_vec(form)
            .map_err(|_| RaiseError::new(SymId::NONE, "compiler: expected a list"))
    }

    fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // V3: (if c t [e]) compiles to Send-based bytecode equivalent to
        //   [[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]
        // The peephole optimizer (Task 13) recognizes this shape and
        // emits Jump-based bytecode inline — recovering pre-V3 perf
        // without sacrificing the user-overridable macro semantic at
        // the source layer.
        //
        // Op::JumpIfFalse / Op::Jump are still emitted by other forms
        // (let / desugarings); only `if` itself stops using them here.
        let (cond, then_branch, else_branch) = match elems.len() {
            3 => (elems[1], elems[2], Value::Nil),
            4 => (elems[1], elems[2], elems[3]),
            _ => return Err(self.err("if takes 2 or 3 args: (if cond then [else])")),
        };
        let bang_bang = self.world.intern("!!");
        let if_true_if_false = self.world.intern("ifTrue:ifFalse:");

        // compile c, then Send :!! to coerce to Bool.
        self.compile_expr(cond, false)?;
        let ic_bang = self.next_ic();
        self.emit(Op::Send {
            selector: bang_bang,
            argc: 0,
            ic_idx: ic_bang,
        });
        // wrap each branch as a zero-arg thunk (fn () branch).
        let t_chunk = self.compile_thunk(then_branch)?;
        self.emit(Op::PushClosure { chunk: t_chunk });
        let e_chunk = self.compile_thunk(else_branch)?;
        self.emit(Op::PushClosure { chunk: e_chunk });
        // Send :ifTrue:ifFalse: argc=2 — tail iff `if` was tail.
        if tail {
            self.emit(Op::TailSend {
                selector: if_true_if_false,
                argc: 2,
            });
        } else {
            let ic_dispatch = self.next_ic();
            self.emit(Op::Send {
                selector: if_true_if_false,
                argc: 2,
                ic_idx: ic_dispatch,
            });
        }
        Ok(())
    }

    /// V3 — compile `body` into a fresh zero-arg chunk-Form and return
    /// its FormId. Used by `compile_if` to wrap each branch as a thunk
    /// suitable for `:ifTrue:ifFalse:` Send dispatch.
    fn compile_thunk(&mut self, body: Value) -> Result<FormId, RaiseError> {
        compile_fn_body(self.world, Vec::new(), body)
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

/// V3 task 13 — used by the if-peephole optimizer in `compile_send`,
/// which recognizes the post-macro-expansion shape of `if` and folds
/// it back into Jump-based emission inline.
#[derive(Copy, Clone)]
enum BranchKind {
    Always,
    IfFalse,
}
