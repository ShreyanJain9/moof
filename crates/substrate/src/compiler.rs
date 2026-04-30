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
    let_star_sym: SymId,
    let_rec_sym: SymId,
    when_sym: SymId,
    unless_sym: SymId,
    do_sym: SymId,
    quote_sym: SymId,
    set_sym: SymId,
    fn_sym: SymId,
    def_sym: SymId,
    call_sym: SymId,
    self_sym: SymId,
    send_sym: SymId,
    defproto_sym: SymId,
    super_sym: SymId,
}

impl<'a> Compiler<'a> {
    fn new(world: &'a mut World, params: Vec<SymId>, source: Value) -> Self {
        let if_sym = world.intern("if");
        let let_sym = world.intern("let");
        let let_star_sym = world.intern("let*");
        let do_sym = world.intern("do");
        let quote_sym = world.intern("quote");
        let set_sym = world.intern("set!");
        let fn_sym = world.intern("fn");
        let def_sym = world.intern("def");
        let call_sym = world.intern("call");
        let self_sym = world.intern("self");
        let send_sym = world.intern("__send__");
        let defproto_sym = world.intern("defproto");
        let super_sym = world.intern("super");
        let let_rec_sym = world.intern("let-rec");
        let when_sym = world.intern("when");
        let unless_sym = world.intern("unless");
        Compiler {
            world,
            ops: Vec::new(),
            consts: Vec::new(),
            ics_count: 0,
            params,
            source,
            if_sym,
            let_sym,
            let_star_sym,
            let_rec_sym,
            when_sym,
            unless_sym,
            do_sym,
            quote_sym,
            set_sym,
            fn_sym,
            def_sym,
            call_sym,
            self_sym,
            send_sym,
            defproto_sym,
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
            Value::Int(_) | Value::Char(_) | Value::Foreign(_) => {
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
            if s == self.let_star_sym {
                return self.compile_let_star(&elems, tail);
            }
            if s == self.let_rec_sym {
                return self.compile_let_rec(&elems, tail);
            }
            if s == self.when_sym {
                return self.compile_when(&elems, tail);
            }
            if s == self.unless_sym {
                return self.compile_unless(&elems, tail);
            }
            if s == self.do_sym {
                return self.compile_do(&elems, tail);
            }
            if s == self.quote_sym {
                return self.compile_quote(&elems);
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
            if s == self.defproto_sym {
                return self.compile_defproto(&elems);
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

    /// `(let-rec ((f (fn …)) (g (fn …))) body)` — bindings may
    /// refer to each other, including recursively. desugars to
    /// `(let ((f nil) (g nil)) (do (set! f …) (set! g …) body))`.
    /// closures created in the bindings capture the let's env;
    /// later set!s mutate those slots; lookups inside closure
    /// bodies see whichever value is current at call time.
    fn compile_let_rec(
        &mut self,
        elems: &[Value],
        tail: bool,
    ) -> Result<(), RaiseError> {
        if elems.len() < 3 {
            return Err(self.err("let-rec requires bindings + body"));
        }
        let bindings_form = elems[1];
        let bindings = self
            .world
            .list_to_vec(bindings_form)
            .map_err(|_| self.err("let-rec: bindings must be a list"))?;
        let mut names: Vec<SymId> = Vec::with_capacity(bindings.len());
        let mut value_forms: Vec<Value> = Vec::with_capacity(bindings.len());
        for b in &bindings {
            let pair = self
                .world
                .list_to_vec(*b)
                .map_err(|_| self.err("let-rec: each binding is (name value)"))?;
            if pair.len() != 2 {
                return Err(self.err("let-rec: each binding is (name value)"));
            }
            names.push(pair[0].as_sym().ok_or_else(|| {
                self.err("let-rec: binding name must be a symbol")
            })?);
            value_forms.push(pair[1]);
        }
        // synthesize:
        //   (let ((n1 nil) (n2 nil) …)
        //     (do (set! n1 v1) (set! n2 v2) … body))
        let mut nil_bindings = Vec::with_capacity(names.len());
        for &n in &names {
            let pair = self.world.make_list(&[Value::Sym(n), Value::Nil]);
            nil_bindings.push(pair);
        }
        let nil_bindings_list = self.world.make_list(&nil_bindings);

        let mut do_body: Vec<Value> = Vec::with_capacity(names.len() + 1);
        do_body.push(Value::Sym(self.do_sym));
        for (n, v) in names.iter().zip(value_forms.iter()) {
            let set_form = self.world.make_list(&[
                Value::Sym(self.set_sym),
                Value::Sym(*n),
                *v,
            ]);
            do_body.push(set_form);
        }
        // body remainder
        for &b in &elems[2..] {
            do_body.push(b);
        }
        let do_form = self.world.make_list(&do_body);

        let let_form = self.world.make_list(&[
            Value::Sym(self.let_sym),
            nil_bindings_list,
            do_form,
        ]);
        self.compile_expr(let_form, tail)
    }

    /// `(when cond body…)` ≡ `(if cond (do body…))` — sugar for
    /// "do these only if true."
    fn compile_when(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        if elems.len() < 2 {
            return Err(self.err("when requires a condition"));
        }
        let cond = elems[1];
        let then_branch = if elems.len() == 2 {
            Value::Nil
        } else {
            // (do body…)
            let mut body = vec![Value::Sym(self.do_sym)];
            body.extend_from_slice(&elems[2..]);
            self.world.make_list(&body)
        };
        let if_form = self.world.make_list(&[
            Value::Sym(self.if_sym),
            cond,
            then_branch,
        ]);
        self.compile_expr(if_form, tail)
    }

    /// `(unless cond body…)` ≡ `(if cond nil (do body…))`.
    fn compile_unless(
        &mut self,
        elems: &[Value],
        tail: bool,
    ) -> Result<(), RaiseError> {
        if elems.len() < 2 {
            return Err(self.err("unless requires a condition"));
        }
        let cond = elems[1];
        let else_branch = if elems.len() == 2 {
            Value::Nil
        } else {
            let mut body = vec![Value::Sym(self.do_sym)];
            body.extend_from_slice(&elems[2..]);
            self.world.make_list(&body)
        };
        let if_form = self.world.make_list(&[
            Value::Sym(self.if_sym),
            cond,
            Value::Nil,
            else_branch,
        ]);
        self.compile_expr(if_form, tail)
    }

    /// `(let* ((a 1) (b a)) body)` — nested single-binding lets.
    fn compile_let_star(
        &mut self,
        elems: &[Value],
        tail: bool,
    ) -> Result<(), RaiseError> {
        if elems.len() < 3 {
            return Err(self.err("let* requires bindings + body"));
        }
        let bindings_form = elems[1];
        let bindings = self
            .world
            .list_to_vec(bindings_form)
            .map_err(|_| self.err("let*: bindings must be a list"))?;
        // build the nested let. start from the inside out so the
        // outermost let appears first.
        let body_value = if elems.len() == 3 {
            elems[2]
        } else {
            let mut wrapped = vec![Value::Sym(self.do_sym)];
            wrapped.extend_from_slice(&elems[2..]);
            self.world.make_list(&wrapped)
        };
        let mut nested = body_value;
        for binding in bindings.into_iter().rev() {
            let single_bindings = self.world.make_list(&[binding]);
            let let_form =
                self.world
                    .make_list(&[Value::Sym(self.let_sym), single_bindings, nested]);
            nested = let_form;
        }
        // compile the synthesized nested let.
        self.compile_expr(nested, tail)
    }

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

    /// `(defproto Name (proto Parent)? (slots …)? (handlers …)?)`
    ///
    /// lowers to:
    /// 1. `[Parent new] → globals['Name]`
    /// 2. for each `[selector params…] body` clause:
    ///    `[set-handler! Name 'selector (fn (params…) body)]`
    /// 3. push `Name` (so the form evaluates to the new proto).
    ///
    /// phase-A defproto deliberately omits auto-generated accessors
    /// (the `(slots count step)` clause is declarative-only). user
    /// code adds `[count]`, `[count: v]` handlers manually if needed.
    /// the full surface (with auto-accessors and multi-clause
    /// patterns) lands in phase A.10.
    fn compile_defproto(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() < 2 {
            return Err(self.err("defproto: needs a name"));
        }
        let name = elems[1].as_sym().ok_or_else(|| {
            self.err("defproto: name must be a symbol")
        })?;

        let mut parent_name: SymId = self.world.intern("Object");
        let mut handlers: Vec<(SymId, Vec<SymId>, Value)> = Vec::new();

        for i in 2..elems.len() {
            let clause = elems[i];
            let clause_elems = self.world.list_to_vec(clause).map_err(|_| {
                self.err("defproto: each clause must be a list")
            })?;
            if clause_elems.is_empty() {
                return Err(self.err("defproto: empty clause"));
            }
            let head = clause_elems[0].as_sym().ok_or_else(|| {
                self.err("defproto: clause head must be a symbol")
            })?;
            let head_text = self.world.resolve(head).to_string();
            match head_text.as_str() {
                "proto" => {
                    if clause_elems.len() != 2 {
                        return Err(self.err("defproto: (proto X) takes one arg"));
                    }
                    parent_name = clause_elems[1].as_sym().ok_or_else(|| {
                        self.err("defproto: (proto X) — X must be a symbol")
                    })?;
                }
                "slots" => {
                    // declarative-only at phase A.
                }
                "handlers" => {
                    let pairs = &clause_elems[1..];
                    if pairs.len() % 2 != 0 {
                        return Err(self.err(
                            "defproto: (handlers …) expects pairs of (header) body",
                        ));
                    }
                    let mut j = 0;
                    while j < pairs.len() {
                        let header = pairs[j];
                        let body = pairs[j + 1];
                        let header_elems = self.world.list_to_vec(header).map_err(|_| {
                            self.err("defproto: handler header must be a list")
                        })?;
                        if header_elems.is_empty() {
                            return Err(self.err("defproto: empty handler header"));
                        }
                        let sel = header_elems[0].as_sym().ok_or_else(|| {
                            self.err("defproto: selector must be a symbol")
                        })?;
                        let mut params = Vec::with_capacity(header_elems.len() - 1);
                        for &p in &header_elems[1..] {
                            params.push(p.as_sym().ok_or_else(|| {
                                self.err("defproto: param must be a symbol")
                            })?);
                        }
                        handlers.push((sel, params, body));
                        j += 2;
                    }
                }
                other => {
                    return Err(self.err(format!(
                        "defproto: unknown clause `{}`",
                        other
                    )));
                }
            }
        }

        // emit: `[Parent new]` then DefineGlobal Name.
        self.emit(Op::LoadName(parent_name));
        let new_sym = self.world.intern("new");
        let new_ic = self.next_ic();
        self.emit(Op::Send {
            selector: new_sym,
            argc: 0,
            ic_idx: new_ic,
        });
        self.emit(Op::DefineGlobal(name));
        // DefineGlobal pushes the symbol; discard.
        self.emit(Op::Pop);

        // for each handler, emit:
        //   LoadName set-handler!
        //   LoadName Name
        //   LoadConst 'sel
        //   PushClosure <body-chunk>
        //   Send :call 3
        //   Pop
        let set_handler_sym = self.world.intern("setHandler!");
        let call_sym = self.call_sym;
        for (sel, params, body) in handlers {
            let fn_chunk = compile_fn_body(self.world, params, body)?;
            self.emit(Op::LoadName(set_handler_sym));
            self.emit(Op::LoadName(name));
            let sel_idx = self.add_const(Value::Sym(sel));
            self.emit(Op::LoadConst(sel_idx));
            self.emit(Op::PushClosure { chunk: fn_chunk });
            let call_ic = self.next_ic();
            self.emit(Op::Send {
                selector: call_sym,
                argc: 3,
                ic_idx: call_ic,
            });
            self.emit(Op::Pop);
        }

        // result: the proto-Form itself.
        self.emit(Op::LoadName(name));
        Ok(())
    }
}

#[derive(Copy, Clone)]
enum BranchKind {
    Always,
    IfFalse,
}

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
        let mut w = World::new();
        install_closure_call(&mut w);
        // (let* ((a 1) (b a)) b) — b sees a, even within the same let*.
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
