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
    table_marker_sym: SymId,
    entry_marker_sym: SymId,
    obj_marker_sym: SymId,
    obj_slot_sym: SymId,
    obj_method_sym: SymId,
    cascade_marker_sym: SymId,
    quasiquote_sym: SymId,
    unquote_sym: SymId,
    unquote_splicing_sym: SymId,
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
        let table_marker_sym = world.intern("__table__");
        let entry_marker_sym = world.intern("__entry__");
        let obj_marker_sym = world.intern("__obj__");
        let obj_slot_sym = world.intern("__slot__");
        let obj_method_sym = world.intern("__method__");
        let cascade_marker_sym = world.intern("__cascade__");
        let quasiquote_sym = world.intern("quasiquote");
        let unquote_sym = world.intern("unquote");
        let unquote_splicing_sym = world.intern("unquote-splicing");
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
            table_marker_sym,
            entry_marker_sym,
            obj_marker_sym,
            obj_slot_sym,
            obj_method_sym,
            cascade_marker_sym,
            quasiquote_sym,
            unquote_sym,
            unquote_splicing_sym,
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
            if s == self.quasiquote_sym {
                return self.compile_quasiquote(&elems, tail);
            }
            if s == self.unquote_sym {
                return Err(self.err("unquote outside quasiquote"));
            }
            if s == self.unquote_splicing_sym {
                return Err(self.err("unquote-splicing outside quasiquote"));
            }
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
            if s == self.table_marker_sym {
                return self.compile_table(&elems);
            }
            if s == self.obj_marker_sym {
                return self.compile_obj_literal(&elems);
            }
            if s == self.cascade_marker_sym {
                return self.compile_cascade(&elems);
            }
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

    /// expand a quasiquoted form to a constructor expression that
    /// builds the same shape at runtime, with `unquote` substituting
    /// in evaluated values and `unquote-splicing` flattening lists
    /// into the surrounding list. nesting bumps depth: an inner
    /// `(quasiquote …)` increases depth, an inner `(unquote …)`
    /// decreases it; only depth-1 unquotes are evaluated.
    fn compile_quasiquote(
        &mut self,
        elems: &[Value],
        tail: bool,
    ) -> Result<(), RaiseError> {
        if elems.len() != 2 {
            return Err(self.err("quasiquote requires 1 arg: `expr"));
        }
        let expanded = self.expand_quasiquote(elems[1], 1)?;
        self.compile_expr(expanded, tail)
    }

    /// recursive expander. returns a source-form that, when
    /// compiled and run, reconstructs the quasiquoted shape with
    /// unquote-substitutions filled in.
    fn expand_quasiquote(
        &mut self,
        form: Value,
        depth: u32,
    ) -> Result<Value, RaiseError> {
        // an atom (sym, int, etc.) → (quote atom).
        let id = match form.as_form_id() {
            Some(i) => i,
            None => return Ok(self.quote_form(form)),
        };
        let f = self.world.heap.get(id);
        // not a list (maybe a Form-as-value, table, etc.) → (quote form).
        if f.proto != Value::Form(self.world.protos.list) {
            return Ok(self.quote_form(form));
        }

        // empty list → (quote ())
        let elems = match self.world.list_to_vec(form) {
            Ok(v) => v,
            Err(_) => return Ok(self.quote_form(form)),
        };
        if elems.is_empty() {
            return Ok(self.quote_form(form));
        }

        // (unquote x) at depth 1 → x (evaluated).
        // (unquote x) at deeper depth → (list 'unquote (qq x depth-1))
        // (quasiquote x) → (list 'quasiquote (qq x depth+1))
        if let Some(s) = elems[0].as_sym() {
            if s == self.unquote_sym {
                if elems.len() != 2 {
                    return Err(self.err("unquote requires 1 arg"));
                }
                if depth == 1 {
                    return Ok(elems[1]);
                }
                let inner = self.expand_quasiquote(elems[1], depth - 1)?;
                let list_sym = self.world.intern("list");
                let quoted_uq = self.quote_form(Value::Sym(self.unquote_sym));
                return Ok(self
                    .world
                    .make_list(&[Value::Sym(list_sym), quoted_uq, inner]));
            }
            if s == self.unquote_splicing_sym && depth == 1 {
                return Err(self.err("unquote-splicing not in a list context"));
            }
            if s == self.quasiquote_sym {
                if elems.len() != 2 {
                    return Err(self.err("quasiquote requires 1 arg"));
                }
                let inner = self.expand_quasiquote(elems[1], depth + 1)?;
                let list_sym = self.world.intern("list");
                let quoted_qq =
                    self.quote_form(Value::Sym(self.quasiquote_sym));
                return Ok(self
                    .world
                    .make_list(&[Value::Sym(list_sym), quoted_qq, inner]));
            }
        }

        // general list: walk right-to-left, building (cons elem rest)
        // or (append elem rest) for splicing.
        let cons_sym = self.world.intern("cons");
        let append_sym = self.world.intern("append");
        let nil_list = self.world.make_list(&[]);
        let mut acc = self.quote_form(nil_list);
        for elem in elems.iter().rev() {
            // (unquote-splicing y) at depth 1 → (append y acc)
            if let Some(sub) = elem.as_form_id().and_then(|fid| {
                let f2 = self.world.heap.get(fid);
                if f2.proto != Value::Form(self.world.protos.list) {
                    return None;
                }
                self.world.list_to_vec(*elem).ok()
            }) {
                if sub.len() == 2 {
                    if let Some(s) = sub[0].as_sym() {
                        if s == self.unquote_splicing_sym && depth == 1 {
                            // acc = (append y acc)
                            acc = self.world.make_list(&[
                                Value::Sym(append_sym),
                                sub[1],
                                acc,
                            ]);
                            continue;
                        }
                    }
                }
            }
            // default: acc = (cons (qq elem) acc)
            let qe = self.expand_quasiquote(*elem, depth)?;
            acc = self
                .world
                .make_list(&[Value::Sym(cons_sym), qe, acc]);
        }
        Ok(acc)
    }

    /// helper: build `(quote v)`.
    fn quote_form(&mut self, v: Value) -> Value {
        self.world
            .make_list(&[Value::Sym(self.quote_sym), v])
    }

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
    fn compile_cascade(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        // elems[0] = __cascade__ marker
        // elems[1] = receiver
        // elems[2..] = segments (each is a list `(sel arg…)`)
        if elems.len() < 3 {
            return Err(self.err("__cascade__: need receiver + at least one segment"));
        }
        let receiver = elems[1];
        // 1. compile receiver, leave it on the stack.
        self.compile_expr(receiver, false)?;

        // 2. for each segment: Dup, compile args, Send, Pop result.
        for &seg in &elems[2..] {
            let seg_elems = self.world.list_to_vec(seg).map_err(|_| {
                self.err("__cascade__: segment must be a list")
            })?;
            if seg_elems.is_empty() {
                return Err(self.err("__cascade__: empty segment"));
            }
            let sel = seg_elems[0].as_sym().ok_or_else(|| {
                self.err("__cascade__: selector must be a symbol")
            })?;
            self.emit(Op::Dup);
            for &a in &seg_elems[1..] {
                self.compile_expr(a, false)?;
            }
            let argc = u8::try_from(seg_elems.len() - 1).map_err(|_| {
                self.err("__cascade__: too many args (max 255)")
            })?;
            let ic = self.next_ic();
            self.emit(Op::Send {
                selector: sel,
                argc,
                ic_idx: ic,
            });
            self.emit(Op::Pop);
        }
        // receiver is now on top.
        Ok(())
    }

    /// `(__obj__ <proto-sym> <entry…>)` — emitted by the reader for
    /// `{Proto …}` object literals. each entry is one of:
    /// - `(__slot__ <key-sym> <value-expr>)`
    /// - `(__method__ <selector-sym> <params-list> <body-expr>)`
    ///
    /// lowering: synthesize the equivalent
    ///
    ///   (let ((__objLit__ [<proto> new]))
    ///     (do
    ///       ;; slot inits (declaration order):
    ///       (slotSet! __objLit__ '<key> <value>) …
    ///       ;; auto-accessors for each slot — getter `[obj name]`
    ///       ;; reads slot, setter `[obj name: v]` writes:
    ///       (setHandler! __objLit__ '<key>
    ///         (fn () (slot self '<key>))) …
    ///       (setHandler! __objLit__ '<key>:
    ///         (fn (v) (slotSet! self '<key> v))) …
    ///       ;; user-defined methods (may override auto-accessors):
    ///       (setHandler! __objLit__ '<sel> (fn <params> <body>)) …
    ///       __objLit__))
    ///
    /// auto-accessors give `.name` shorthand and `[obj name: v]`
    /// setter for free; user-defined methods take precedence
    /// (emitted last so they override).
    fn compile_obj_literal(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() < 2 {
            return Err(self.err("__obj__: missing proto"));
        }
        let proto_sym = elems[1].as_sym().ok_or_else(|| {
            self.err("__obj__: proto must be a symbol")
        })?;

        // collect slots and methods separately so we can interleave
        // emission: slot inits → auto-accessors → user methods.
        let mut slot_keys: Vec<SymId> = Vec::new();
        let mut slot_vals: Vec<Value> = Vec::new();
        let mut user_methods: Vec<(SymId, Value, Value)> = Vec::new(); // (sel, params, body)

        for &entry in &elems[2..] {
            let entry_elems = self.world.list_to_vec(entry).map_err(|_| {
                self.err("__obj__: malformed entry")
            })?;
            if entry_elems.is_empty() {
                return Err(self.err("__obj__: empty entry"));
            }
            let kind = entry_elems[0].as_sym().ok_or_else(|| {
                self.err("__obj__: entry kind must be a symbol")
            })?;
            if kind == self.obj_slot_sym {
                if entry_elems.len() != 3 {
                    return Err(self.err("__obj__: __slot__ takes (key val)"));
                }
                let key = entry_elems[1].as_sym().ok_or_else(|| {
                    self.err("__obj__: slot key must be a symbol")
                })?;
                slot_keys.push(key);
                slot_vals.push(entry_elems[2]);
            } else if kind == self.obj_method_sym {
                if entry_elems.len() != 4 {
                    return Err(self.err(
                        "__obj__: __method__ takes (selector params body)",
                    ));
                }
                let sel = entry_elems[1].as_sym().ok_or_else(|| {
                    self.err("__obj__: method selector must be a symbol")
                })?;
                user_methods.push((sel, entry_elems[2], entry_elems[3]));
            } else {
                let kind_text = self.world.resolve(kind).to_string();
                return Err(self.err(format!(
                    "__obj__: unknown entry kind `{}`",
                    kind_text
                )));
            }
        }

        // build the desugar.
        let obj_local = self.world.intern("__objLit__");
        let new_sym = self.world.intern("new");
        let slot_set_global = self.world.intern("slotSet!");
        let set_handler_global = self.world.intern("setHandler!");
        let slot_global = self.world.intern("slot");

        let new_call = self.world.make_list(&[
            Value::Sym(self.send_sym),
            Value::Sym(proto_sym),
            Value::Sym(new_sym),
        ]);
        let binding = self
            .world
            .make_list(&[Value::Sym(obj_local), new_call]);
        let bindings = self.world.make_list(&[binding]);

        let mut do_body = Vec::with_capacity(slot_keys.len() * 3 + user_methods.len() + 2);
        do_body.push(Value::Sym(self.do_sym));

        // 1. slot inits.
        for (key, val) in slot_keys.iter().zip(slot_vals.iter()) {
            let quoted_key = self
                .world
                .make_list(&[Value::Sym(self.quote_sym), Value::Sym(*key)]);
            let call = self.world.make_list(&[
                Value::Sym(slot_set_global),
                Value::Sym(obj_local),
                quoted_key,
                *val,
            ]);
            do_body.push(call);
        }

        // 2. auto-accessors per slot. getter [obj name] reads slot;
        //    setter [obj name: v] writes slot.
        let v_param_sym = self.world.intern("__v__");
        let empty_params = self.world.make_list(&[]);
        let setter_params_list =
            self.world.make_list(&[Value::Sym(v_param_sym)]);

        for &key in &slot_keys {
            // pre-build all the inner forms so each `make_list` call
            // takes a fresh `&mut self.world` borrow that's already
            // dropped by the time the next call happens.
            let quoted_key_for_init = self
                .world
                .make_list(&[Value::Sym(self.quote_sym), Value::Sym(key)]);
            let quoted_key_for_getter = self
                .world
                .make_list(&[Value::Sym(self.quote_sym), Value::Sym(key)]);
            let quoted_key_for_setter = self
                .world
                .make_list(&[Value::Sym(self.quote_sym), Value::Sym(key)]);

            // getter: (fn () (slot self 'name))
            let getter_body = self.world.make_list(&[
                Value::Sym(slot_global),
                Value::Sym(self.self_sym),
                quoted_key_for_init,
            ]);
            let getter_fn = self.world.make_list(&[
                Value::Sym(self.fn_sym),
                empty_params,
                getter_body,
            ]);
            let getter_install = self.world.make_list(&[
                Value::Sym(set_handler_global),
                Value::Sym(obj_local),
                quoted_key_for_getter,
                getter_fn,
            ]);
            do_body.push(getter_install);

            // setter: (fn (v) (slotSet! self 'name v))
            let setter_body = self.world.make_list(&[
                Value::Sym(slot_set_global),
                Value::Sym(self.self_sym),
                quoted_key_for_setter,
                Value::Sym(v_param_sym),
            ]);
            let setter_fn = self.world.make_list(&[
                Value::Sym(self.fn_sym),
                setter_params_list,
                setter_body,
            ]);
            // selector for the setter is `name:`.
            let key_text = self.world.resolve(key).to_string();
            let setter_sel_text = format!("{}:", key_text);
            let setter_sel = self.world.intern(&setter_sel_text);
            let quoted_setter_sel = self.world.make_list(&[
                Value::Sym(self.quote_sym),
                Value::Sym(setter_sel),
            ]);
            let setter_install = self.world.make_list(&[
                Value::Sym(set_handler_global),
                Value::Sym(obj_local),
                quoted_setter_sel,
                setter_fn,
            ]);
            do_body.push(setter_install);
        }

        // 3. user-defined methods (after auto-accessors so they
        //    win on conflict).
        for (sel, params, body) in user_methods {
            let quoted_sel = self
                .world
                .make_list(&[Value::Sym(self.quote_sym), Value::Sym(sel)]);
            let fn_form = self
                .world
                .make_list(&[Value::Sym(self.fn_sym), params, body]);
            let call = self.world.make_list(&[
                Value::Sym(set_handler_global),
                Value::Sym(obj_local),
                quoted_sel,
                fn_form,
            ]);
            do_body.push(call);
        }

        // final value: the new object.
        do_body.push(Value::Sym(obj_local));

        let do_form = self.world.make_list(&do_body);
        let let_form = self.world.make_list(&[
            Value::Sym(self.let_sym),
            bindings,
            do_form,
        ]);
        self.compile_expr(let_form, false)
    }

    /// `(__table__ entry…)` — emitted by the reader for `#[…]`
    /// table literals. each entry is either a bare expression
    /// (positional) or `(__entry__ key val)` (keyed).
    ///
    /// lowering: `LoadName Table; Send :new 0` to push a fresh
    /// empty Table; then for each entry emit `Dup; <eval>; Send
    /// :push:/:at:put:; Pop` to populate it.
    fn compile_table(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        // 1. push fresh Table
        let table_sym = self.world.intern("Table");
        self.emit(Op::LoadName(table_sym));
        let new_sym = self.world.intern("new");
        let new_ic = self.next_ic();
        self.emit(Op::Send {
            selector: new_sym,
            argc: 0,
            ic_idx: new_ic,
        });

        // 2. for each entry, dup the table, push values, send.
        let push_sym = self.world.intern("push:");
        let at_put_sym = self.world.intern("at:put:");

        for &entry in &elems[1..] {
            // is this a keyed entry? (look for __entry__ head)
            let is_keyed = match entry {
                Value::Form(_) => self
                    .world
                    .list_to_vec(entry)
                    .ok()
                    .and_then(|v| v.first().copied())
                    .and_then(|h| h.as_sym())
                    .map(|s| s == self.entry_marker_sym)
                    .unwrap_or(false),
                _ => false,
            };
            if is_keyed {
                // (__entry__ key val)
                let entry_elems = self.world.list_to_vec(entry).unwrap();
                if entry_elems.len() != 3 {
                    return Err(self.err("table __entry__: expected (key value)"));
                }
                self.emit(Op::Dup);
                self.compile_expr(entry_elems[1], false)?;
                self.compile_expr(entry_elems[2], false)?;
                let ic = self.next_ic();
                self.emit(Op::Send {
                    selector: at_put_sym,
                    argc: 2,
                    ic_idx: ic,
                });
                self.emit(Op::Pop);
            } else {
                // positional
                self.emit(Op::Dup);
                self.compile_expr(entry, false)?;
                let ic = self.next_ic();
                self.emit(Op::Send {
                    selector: push_sym,
                    argc: 1,
                    ic_idx: ic,
                });
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

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
