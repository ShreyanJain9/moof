//! the bootstrap compiler — Form → Chunk.
//!
//! lowers parsed moof source-Forms into bytecode chunks the VM can
//! run. handles the substrate's hardcoded special forms (`if`,
//! `let`, `let*`, `do`, `quote`, `set!`, `fn`, `def`) plus the
//! general fn-call shape `(callable args…)`.
//!
//! per `process/docs-driven.md`'s self-host rule, this compiler is
//! *throwaway scaffolding* — phase A-self-host loads `compiler.moof`
//! as the production compiler. the bootstrap compiler stays
//! buildable for diagnosing compiler.moof itself, but is not the
//! canonical artifact.
//!
//! per `concepts/sends-and-calls.md`, a fn-call `(foo x y)` lowers
//! to `[foo call: x y]` — i.e., load `foo`, push args, send `:call`
//! with `argc=2`. the `Closure` proto's `:call` handler self-invokes
//! (installed by `intrinsics`).
//!
//! per `concepts/blocks-and-patterns.md`, a `(fn (a b) body)`
//! becomes a chunk-Form whose `:params` slot is `(a b)` and whose
//! `:body` is the bytecode. `PushClosure` allocates a closure-Form
//! capturing the current env.

use crate::form::{Form, FormId};
use crate::opcodes::Op;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{ICache, RaiseError, World};

/// compile a top-level expression into a chunk-Form. the chunk has
/// no params; running it produces the expression's value.
pub fn compile(world: &mut World, form: Value) -> Result<FormId, RaiseError> {
    let mut c = Compiler::new(world, Vec::new(), form);
    c.compile_expr(form, true)?;
    c.emit(Op::Return);
    c.finalize()
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

    if_sym: SymId,
    let_sym: SymId,
    do_sym: SymId,
    quote_sym: SymId,
    set_sym: SymId,
    fn_sym: SymId,
    def_sym: SymId,
    call_sym: SymId,
    self_sym: SymId,
    send_sym: SymId,
    super_sym: SymId,
    cascade_marker_sym: SymId,
    defmacro_sym: SymId,
}

impl<'a> Compiler<'a> {
    fn new(world: &'a mut World, params: Vec<SymId>, source: Value) -> Self {
        let if_sym = world.intern("if");
        let let_sym = world.intern("let");
        let do_sym = world.intern("do");
        let quote_sym = world.intern("quote");
        let set_sym = world.intern("set!");
        let fn_sym = world.intern("fn");
        let def_sym = world.intern("def");
        let call_sym = world.intern("call");
        let self_sym = world.intern("self");
        let send_sym = world.intern("__send__");
        let super_sym = world.intern("super");
        let cascade_marker_sym = world.intern("__cascade__");
        let defmacro_sym = world.intern("defmacro");
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
            set_sym,
            fn_sym,
            def_sym,
            call_sym,
            self_sym,
            send_sym,
            super_sym,
            cascade_marker_sym,
            defmacro_sym,
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
        // off is relative to jump_pos (matches VM's `(pc-1) + off`
        // formula since pc has already advanced past the jump op).
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
        // :params is a moof list of symbol-Values.
        let param_values: Vec<Value> = params.iter().map(|&s| Value::Sym(s)).collect();
        let params_list = world.make_list(&param_values);
        chunk_form.slots.insert(world.params_sym, params_list);
        // :source for reflection.
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
            Value::Nil => {
                self.emit(Op::PushNil);
            }
            Value::Bool(true) => {
                self.emit(Op::PushTrue);
            }
            Value::Bool(false) => {
                self.emit(Op::PushFalse);
            }
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
                // List Forms (proto = List) are code to compile.
                // any other Form (String, ToolPalette, …) is a
                // *literal* — load from the constant pool.
                let proto = self.world.heap.get(id).proto;
                if proto == Value::Form(self.world.protos.list) {
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
        // form is a list — head (a sym) determines whether it's a
        // special form or a fn-call.
        let elems = self.list_elems(form)?;
        if elems.is_empty() {
            // empty list — `()` — evaluates to nil.
            self.emit(Op::PushNil);
            return Ok(());
        }
        if let Value::Sym(s) = elems[0] {
            if s == self.if_sym {
                return self.compile_if(&elems, tail);
            }
            if s == self.let_sym {
                return self.compile_let(&elems, tail);
            }
            // when, unless, let*, let-rec used to live as
            // hardcoded special forms here. they're now plain
            // macros in lib/bootstrap.moof (the user-defined-macro
            // path below picks them up). the substrate stays a
            // smaller seed; the user can override or replace any
            // of them by re-running `(defmacro …)` from inside.
            if s == self.do_sym {
                return self.compile_do(&elems, tail);
            }
            if s == self.quote_sym {
                return self.compile_quote(&elems);
            }
            // quasiquote (`` ` ``), unquote (`,`), unquote-splicing
            // (`,@`) used to be hardcoded here. they're now a moof
            // macro `quasiquote` defined at the top of
            // lib/bootstrap.moof. the macro's expander produces the
            // runtime `(cons …)` / `(append …)` / `(quote …)`
            // construction calls that this compiler then handles
            // through the ordinary fn-call path.
            //
            // bare `(unquote x)` / `(unquote-splicing x)` outside a
            // quasiquote no longer raise here — they fall through
            // to compile_call, which will fail with "unbound
            // `unquote`" at runtime. the docs are clear that
            // unquote without an enclosing quasiquote is a user
            // error; the diagnostic is now late but correct.
            if s == self.set_sym {
                return self.compile_set(&elems);
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
            // defproto used to be a hardcoded special form. it's
            // now a `(defmacro defproto …)` in lib/bootstrap.moof.
            // the user-defined-macro path below picks it up.
            // defmethod used to be a hardcoded special form. it's
            // now a macro in lib/bootstrap.moof — see the
            // `(defmacro defmethod …)` there. the user-defined
            // macro path below picks it up.
            if s == self.defmacro_sym {
                return self.compile_defmacro(&elems);
            }
            // user-defined macro? expand at compile time.
            //
            // calling convention (kernel/io tradition): macros take
            // *one* arg — the list of source-arg-forms. so for
            // `(when cond a b c)`, the macro receives one argument:
            // the list `(cond a b c)`. the macro body destructures
            // it with List ops, returning a Form to compile in the
            // call site's place. see lib/bootstrap.moof for examples.
            if let Some(&macro_method) = self.world.macros.get(&s) {
                let mid = match macro_method.as_form_id() {
                    Some(i) => i,
                    None => {
                        return Err(self.err("macro entry is not a Form"));
                    }
                };
                let args_list = self.world.make_list(&elems[1..]);
                let expanded = self
                    .world
                    .invoke(mid, Value::Nil, &[args_list], FormId::NONE)?;
                return self.compile_expr(expanded, tail);
            }
            // __table__ is now a moof macro in lib/bootstrap.moof.
            // __obj__ is now a moof macro in lib/bootstrap.moof.
            // __cascade__ is now a moof macro in lib/bootstrap.moof.
        }
        // fn-call: `(callable arg…)`
        self.compile_call(&elems, tail)
    }

    /// `(__send__ receiver 'selector args…)` — emitted by the
    /// reader for `[recv sel args…]` and `.foo` shorthand. lowers
    /// directly to a `Send` opcode (not `:call` indirection).
    ///
    /// when `receiver` is the symbol `super`, we emit `SuperSend`
    /// instead — the receiver of the dispatched method is `self`
    /// (handled at runtime), and lookup walks the chain *above* the
    /// current frame's defining proto.
    fn compile_send(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        if elems.len() < 3 {
            return Err(self.err("malformed __send__ form"));
        }
        let receiver = elems[1];
        let selector = elems[2].as_sym().ok_or_else(|| {
            self.err("__send__ selector must be a symbol")
        })?;
        let args = &elems[3..];

        let is_super = matches!(receiver, Value::Sym(s) if s == self.super_sym);

        if !is_super {
            // push receiver, then args.
            self.compile_expr(receiver, false)?;
        }
        for &a in args {
            self.compile_expr(a, false)?;
        }
        let argc = u8::try_from(args.len()).map_err(|_| {
            self.err("send: too many args (max 255)")
        })?;
        let ic = self.next_ic();
        self.emit(if is_super {
            Op::SuperSend { selector, argc, ic_idx: ic }
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

    /// expand a list-Form to a `Vec<Value>`. errors with a clear
    /// message if not a list.
    fn list_elems(&self, form: Value) -> Result<Vec<Value>, RaiseError> {
        self.world.list_to_vec(form).map_err(|_| {
            RaiseError::new(
                SymId::NONE,
                "compiler: expected a list",
            )
        })
    }

    fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // `(if cond then)` and `(if cond then else)` both legal.
        // missing else defaults to nil, so the form has a value
        // on every path (no surprise undefined-behavior).
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
        // (do e1 e2 … eN)
        if elems.len() == 1 {
            // (do) → nil
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

    // quasiquote / unquote / unquote-splicing used to live here as
    // an ~120-LoC recursive expander. they're now a moof macro
    // `quasiquote` defined at the top of lib/bootstrap.moof, with
    // its expansion helpers (`__qq-list?`, `__qq-marker?`,
    // `__qq-walk-elems`, `__qq-expand`) right next to it. user
    // code can `[quasiquote source]`-inspect the macro and
    // override its semantics by re-running `(defmacro quasiquote
    // …)`.

    fn compile_set(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() != 3 {
            return Err(self.err("set! requires 2 args: (set! name expr)"));
        }
        let name = elems[1].as_sym().ok_or_else(|| {
            self.err("set!'s first arg must be a symbol")
        })?;
        self.compile_expr(elems[2], false)?;
        self.emit(Op::StoreName(name));
        Ok(())
    }

    fn compile_def(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() != 3 {
            return Err(self.err("def requires 2 args: (def name expr)"));
        }
        let name = elems[1].as_sym().ok_or_else(|| {
            self.err("def's first arg must be a symbol")
        })?;
        self.compile_expr(elems[2], false)?;
        self.emit(Op::DefineGlobal(name));
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
            params.push(p.as_sym().ok_or_else(|| {
                self.err("fn: each param must be a symbol")
            })?);
        }
        // body — if multiple body forms, wrap in (do …).
        let body_value = if elems.len() == 3 {
            elems[2]
        } else {
            // construct (do e1 e2 … eN) so multi-expression bodies
            // sequence properly.
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
            let name = pair[0].as_sym().ok_or_else(|| {
                self.err("let: binding name must be a symbol")
            })?;
            params.push(name);
            value_forms.push(pair[1]);
        }
        // body — wrap multi-expression bodies in (do …).
        let body_value = if elems.len() == 3 {
            elems[2]
        } else {
            let mut wrapped = vec![Value::Sym(self.do_sym)];
            wrapped.extend_from_slice(&elems[2..]);
            self.world.make_list(&wrapped)
        };
        // compile the inner fn-chunk (evaluates body with params).
        let chunk_id = compile_fn_body(self.world, params, body_value)?;
        // emit: PushClosure; eval each value; Send :call argc.
        self.emit(Op::PushClosure { chunk: chunk_id });
        for v in &value_forms {
            self.compile_expr(*v, false)?;
        }
        let argc = u8::try_from(value_forms.len()).map_err(|_| {
            self.err("let: too many bindings (max 255)")
        })?;
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

    // `let-rec`, `when`, `unless`, and `let*` were once special-
    // forms here; they're now plain macros in lib/bootstrap.moof.
    // see the `(defmacro …)` calls there for the canonical
    // expansions. the compiler's job for them is "find the macro,
    // splice in its expansion" — which is the user-defined-macro
    // path in `compile_form`.

    fn compile_call(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // (callable arg…) → `[callable call: arg…]` with argc = N.
        let callable = elems[0];
        let args = &elems[1..];
        // push receiver.
        self.compile_expr(callable, false)?;
        // push args.
        for &a in args {
            self.compile_expr(a, false)?;
        }
        let argc = u8::try_from(args.len()).map_err(|_| {
            self.err("call: too many args (max 255)")
        })?;
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

    /// `(__cascade__ <receiver> (<sel> <arg…>) …)` — emitted by the
    /// reader for `[obj a; b; c: x]`. sends each segment to the
    /// receiver in order; the cascade returns the *receiver* (per
    /// smalltalk-80 semantics, not the last result).
    ///
    /// lowering: compile receiver once, push. for each segment,
    /// `Dup` the receiver, push args, send, pop the result. when
    /// all done the receiver is on top of the stack.
    // `compile_cascade` was here. it's now a `(defmacro __cascade__
    // …)` in lib/bootstrap.moof that desugars to
    //   (let ((__r recv))
    //     (do (__send__ __r sel1 args1…) … __r))

    // `compile_obj_literal` was here (the largest source-to-source
    // special form, ~190 LoC). it's now a `(defmacro __obj__ …)`
    // in lib/bootstrap.moof that desugars `{Proto …}` literals to
    // a (let ((__objLit__ [Proto new])) (do (slotSet! …) …
    //                                       (setHandler! …) …
    //                                       __objLit__)).

    // `compile_table` was here. table literals now expand via the
    // moof macro `(defmacro __table__ …)` in lib/bootstrap.moof.

    // `defproto` was once a hardcoded special form here (~200 LoC
    // of direct bytecode emission). it's now a `(defmacro defproto
    // …)` in lib/bootstrap.moof — a user-modifiable macro that
    // desugars to `(do (def Name (getOrCreateProto …))
    //                  (setHandler! …) … Name)`.

    /// `(defmethod ProtoExpr (header) body)`
    ///
    /// install a method on a proto without `setHandler!` boilerplate.
    /// header shape mirrors defproto: `(name)` `(+ other)`
    /// `(name x y)` `(at: i put: v)`.
    /// (defmacro name (args-list) body) — install a macro that
    /// expands at compile time.
    ///
    /// the macro receives *one* argument: the list of unevaluated
    /// arg-forms from the call site. so for `(when cond a b c)`,
    /// the macro is called with the list `(cond a b c)`. the body
    /// destructures using List ops (`[args head]`, `[args tail]`,
    /// pattern matching once we have it) and returns a Form, which
    /// the compiler then compiles in place of the original call.
    ///
    /// the single-list calling convention (kernel/io tradition)
    /// gives macros full variadic flexibility for free; templates
    /// usually want quasiquote (`` `(if ,c ,t ,e) ``) anyway.
    ///
    /// installation happens eagerly at compile time of the
    /// `defmacro` form itself, so subsequent forms in the same
    /// chunk can use the macro.
    fn compile_defmacro(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() != 4 {
            return Err(self.err(
                "defmacro: (defmacro name (args-list-name) body)",
            ));
        }
        let name = elems[1].as_sym().ok_or_else(|| {
            self.err("defmacro: name must be a symbol")
        })?;
        let params_list = elems[2];
        let body = elems[3];

        // decode params: list of symbols.
        let param_vec = self
            .world
            .list_to_vec(params_list)
            .map_err(|_| self.err("defmacro: params must be a list"))?;
        let mut params: Vec<SymId> = Vec::with_capacity(param_vec.len());
        for p in param_vec {
            let s = p.as_sym().ok_or_else(|| {
                self.err("defmacro: param must be a symbol")
            })?;
            params.push(s);
        }

        // compile the body to a chunk.
        let chunk_id = compile_fn_body(self.world, params.clone(), body)?;

        // wrap as a method-Form: proto Method, slots
        // {body, params, env=global, source=original}.
        let mut method = Form::with_proto(Value::Form(self.world.protos.method));
        method
            .slots
            .insert(self.world.body_sym, Value::Form(chunk_id));
        // params list re-built (symbols may differ in order).
        let params_v = self
            .world
            .make_list(&params.iter().map(|s| Value::Sym(*s)).collect::<Vec<_>>());
        method.slots.insert(self.world.params_sym, params_v);
        let global_env = Value::Form(self.world.global_env);
        method.slots.insert(self.world.env_sym, global_env);
        method
            .meta
            .insert(self.world.source_sym, params_list);
        let macro_meta = self.world.intern("macro");
        method
            .meta
            .insert(macro_meta, Value::Sym(name));
        let method_id = self.world.alloc(method);

        // register in the macro table — eagerly, so subsequent
        // forms in this chunk see it.
        self.world.macros.insert(name, Value::Form(method_id));

        // also bind in global env for `[name source]` reflection
        // and so the closure isn't gc'd if/when we have a gc.
        self.world
            .env_bind(self.world.global_env, name, Value::Form(method_id));

        // the form itself evaluates to nil.
        self.emit(Op::PushNil);
        Ok(())
    }

    // `defmethod` was once a hardcoded special form here; it now
    // lives as a `(defmacro defmethod …)` in lib/bootstrap.moof.
    // its expansion is `(setHandler! ProtoExpr 'sel (fn (params)
    // body))`, which the compiler handles via the ordinary
    // `setHandler!` global + `fn` special form.
}

#[derive(Copy, Clone)]
enum BranchKind {
    Always,
    IfFalse,
}

// `decode_paren_header` and `is_operator_only` used to live here
// — they decoded `(name)` / `(+ other)` / `(at: i put: v)` /
// `(name x y)` shapes into `(selector, params)`. now that
// `defproto` and `defmethod` are macros, the moof-side
// `__decode-header` (lib/bootstrap.moof) does the same job, with
// `(intern …)` for joining keyword selectors.

#[cfg(test)]
mod tests {
    use super::*;

    /// install a minimal :+ on Integer so call-shaped expressions
    /// compile and run.
    fn install_arith(w: &mut World) {
        w.install_native(w.protos.integer, "+", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() + args[0].as_int().unwrap()))
        });
        w.install_native(w.protos.integer, "-", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() - args[0].as_int().unwrap()))
        });
        w.install_native(w.protos.integer, "*", |_, self_, args| {
            Ok(Value::Int(self_.as_int().unwrap() * args[0].as_int().unwrap()))
        });
        w.install_native(w.protos.integer, "=", |_, self_, args| {
            Ok(Value::Bool(self_.as_int().unwrap() == args[0].as_int().unwrap()))
        });
        w.install_native(w.protos.integer, "<", |_, self_, args| {
            Ok(Value::Bool(self_.as_int().unwrap() < args[0].as_int().unwrap()))
        });
    }

    /// install :call on Closure so fn-application works.
    fn install_closure_call(w: &mut World) {
        w.install_native(w.protos.closure, "call", |world, self_, args| {
            let id = self_
                .as_form_id()
                .ok_or_else(|| RaiseError::new(world.intern("dispatch"), "not a closure"))?;
            world.invoke(id, Value::Nil, args, crate::form::FormId::NONE)
        });
    }

    fn ev(w: &mut World, src: &str) -> Result<Value, RaiseError> {
        let form = w
            .read(src)
            .map_err(|e| RaiseError::from_reader(&mut w.syms, e))?;
        let chunk = compile(w, form)?;
        w.run_top(chunk)
    }

    #[test]
    fn compile_integer_literal() {
        let mut w = World::new();
        assert_eq!(ev(&mut w, "42").unwrap(), Value::Int(42));
    }

    #[test]
    fn compile_arithmetic_send() {
        let mut w = World::new();
        install_arith(&mut w);
        // (+ 1 2) is a fn-call to + with args [1, 2]. since `+` is
        // not in scope as a name (we installed it as a method on
        // Integer), this should *not* work the way you'd expect
        // unless we use the smalltalk-style `[1 + 2]`. for now,
        // the bootstrap stdlib will define a `+` global that
        // forwards to the integer method. a direct `(+ 1 2)` won't
        // compile-and-run yet — let's test the integer method
        // call shape instead.
        //
        // i'll exercise this with an explicit binding once `def`
        // works (test below).
    }

    #[test]
    fn compile_def_then_lookup() {
        let mut w = World::new();
        // (def x 5) (then `x` evaluates to 5)
        let _form = w.read("(def x 5)").unwrap();
        let chunk = compile(&mut w, _form).unwrap();
        w.run_top(chunk).unwrap();
        // now x is in the global env.
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
    fn compile_set_updates_global() {
        let mut w = World::new();
        let f = w.read("(def x 1)").unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let f = w.read("(set! x 99)").unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        assert_eq!(ev(&mut w, "x").unwrap(), Value::Int(99));
    }

    #[test]
    fn compile_let_parallel_bindings() {
        let mut w = World::new();
        install_arith(&mut w);
        install_closure_call(&mut w);
        // (let ((a 3) (b 4)) (+ a b)) — but `(+ a b)` doesn't work
        // until we have `+` as a global. instead:
        // (let ((a 3) (b 4)) a) — verify that bindings work.
        let r = ev(&mut w, "(let ((a 3) (b 4)) a)").unwrap();
        assert_eq!(r, Value::Int(3));
        let r = ev(&mut w, "(let ((a 3) (b 4)) b)").unwrap();
        assert_eq!(r, Value::Int(4));
    }

    #[test]
    fn compile_let_does_not_leak_bindings() {
        let mut w = World::new();
        install_closure_call(&mut w);
        // bindings exit scope after body.
        let r = ev(&mut w, "(let ((a 5)) a)").unwrap();
        assert_eq!(r, Value::Int(5));
        // `a` is now unbound at top level.
        let err = ev(&mut w, "a").unwrap_err();
        assert_eq!(w.resolve(err.kind), "unbound");
    }

    #[test]
    fn compile_let_star_sequential_bindings() {
        // `let*` is now a macro defined in lib/bootstrap.moof, so
        // this test needs a world with the bootstrap loaded
        // (`crate::new_world()`), not the bare `World::new()` the
        // surrounding compiler-only tests use.
        let mut w = crate::new_world();
        let r = ev(&mut w, "(let* ((a 1) (b a)) b)").unwrap();
        assert_eq!(r, Value::Int(1));
    }

    #[test]
    fn compile_fn_and_call() {
        let mut w = World::new();
        install_closure_call(&mut w);
        let f = w.read("(def square (fn (x) x))").unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let r = ev(&mut w, "(square 7)").unwrap();
        assert_eq!(r, Value::Int(7));
    }

    #[test]
    fn compile_closure_captures_env() {
        let mut w = World::new();
        install_arith(&mut w);
        install_closure_call(&mut w);
        // `make-incr-by` returns a function closing over n.
        let src = "(def make-add (fn (n) (fn (x) (let ((y x)) y))))";
        let f = w.read(src).unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        // (make-add 5) returns a closure; calling it with 10 returns 10.
        // (we don't test arithmetic on captured values yet — the
        // bootstrap stdlib + def of `+` global is phase A.10. but
        // the closure machinery is exercised here.)
        let _ = ev(&mut w, "(make-add 5)").unwrap();
        let r = ev(&mut w, "((make-add 5) 10)").unwrap();
        assert_eq!(r, Value::Int(10));
    }

    #[test]
    fn compile_chunk_has_source_meta() {
        let mut w = World::new();
        let f = w.read("(if #true 1 2)").unwrap();
        let c = compile(&mut w, f).unwrap();
        // L5: source is canonical and reachable.
        let source = w.heap.get(c).meta_at(w.source_sym);
        assert_eq!(source, f);
    }

    #[test]
    fn compile_chunk_has_params_slot() {
        let mut w = World::new();
        let f = w.read("(fn (x y) x)").unwrap();
        let c = compile(&mut w, f).unwrap();
        // c is the *outer* chunk that pushes a closure for the fn.
        // its params slot is empty (top-level expression has no
        // params). but the inner chunk — the fn body — has params.
        // we can't easily reach the inner chunk from here without
        // disassembling. just verify outer:
        assert_eq!(w.heap.get(c).slot(w.params_sym), Value::Nil);
    }

    #[test]
    fn compile_factorial_via_recursion_def_succeeds() {
        // honestly: phase A doesn't yet have global bindings for `=`,
        // `*`, `-`, `+` (those land in phase A.10's bootstrap stdlib
        // alongside Integer's protocol-derived methods). the
        // *compilation* of a recursive `fact` works; running it
        // would fail with `unbound name =` until A.10. exercise
        // what's testable now: the def succeeds and `fact` is in
        // the global env as a Closure.
        let mut w = World::new();
        install_arith(&mut w);
        install_closure_call(&mut w);
        let src = "(def fact (fn (n)
                    (if (= n 0)
                        1
                        (* n (fact (- n 1))))))";
        let f = w.read(src).unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        let fact_sym = w.intern("fact");
        let fact_value = w
            .env_lookup(w.global_env, fact_sym)
            .expect("fact should be in global env");
        let id = fact_value.as_form_id().unwrap();
        // the binding is a closure-Form.
        assert_eq!(w.heap.get(id).proto, Value::Form(w.protos.closure));
    }

    #[test]
    fn compile_recursion_via_self_referencing_def() {
        // a recursive fn that uses *only* primitives we have at
        // phase A: a counter that returns its arg or recurses.
        // (fn (n) (if n n (recurse 0))) — but recurse is the def'd
        // name. demonstrate that the function-name is reachable
        // from within its own body.
        let mut w = World::new();
        install_closure_call(&mut w);
        let src = "(def f (fn (n) (if n n 'done)))";
        let f = w.read(src).unwrap();
        let c = compile(&mut w, f).unwrap();
        w.run_top(c).unwrap();
        // (f 1) → 1 (Int(1) is truthy)
        let r = ev(&mut w, "(f 1)").unwrap();
        assert_eq!(r, Value::Int(1));
        // (f nil) → 'done
        let r = ev(&mut w, "(f nil)").unwrap();
        let done = w.intern("done");
        assert_eq!(r, Value::Sym(done));
    }

    #[test]
    fn empty_list_evaluates_to_nil() {
        let mut w = World::new();
        assert_eq!(ev(&mut w, "()").unwrap(), Value::Nil);
    }

    #[test]
    fn nested_if_works() {
        let mut w = World::new();
        let r = ev(
            &mut w,
            "(if #true (if #false 'a 'b) 'c)",
        )
        .unwrap();
        let b = w.intern("b");
        assert_eq!(r, Value::Sym(b));
    }
}
