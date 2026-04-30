//! the bytecode interpreter + send dispatch.
//!
//! the heart of the substrate. one switch-loop interpreter; one
//! shared operand stack; one shared frame stack (so tail-calls and
//! recursive send-from-native stay in bounded memory).
//!
//! per `laws/substrate-laws.md` L3, *every* method call goes through
//! `World::send`. there is no privileged ABI for built-in operations;
//! arithmetic, reflection, even global lookups go through send.
//!
//! per L11, FormIds are stable; the interpreter never compacts or
//! renumbers heap addresses. inline caches (phase A defers; see
//! `ICache` in `world`) cache `(proto, method)` and re-resolve when
//! the proto generation bumps (phase C adds invalidation).
//!
//! per `laws/determinism-laws.md` D6, gc runs only at turn
//! boundaries — phase A doesn't have gc yet, but the discipline is
//! that the VM never triggers it mid-step.

use crate::form::{Form, FormId};
use crate::opcodes::Op;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{RaiseError, World};

/// one stack frame on the VM's call stack.
#[derive(Clone, Debug)]
pub struct Frame {
    /// the chunk currently executing.
    pub chunk: FormId,
    /// the program counter — index into `world.chunk_ops[chunk]`.
    pub pc: usize,
    /// the lexical environment.
    pub env: FormId,
    /// the receiver of the current method invocation.
    pub self_: Value,
    /// where this frame's operand stack contributions begin in
    /// the world's shared `stack`.
    pub stack_base: usize,
    /// the proto on which the currently-running method was found.
    /// `[super sel …]` walks from this proto's parent. for
    /// top-level chunks (no method dispatch), this is
    /// `FormId::NONE`.
    pub defining_proto: FormId,
}

/// the interpreter's per-vat state.
#[derive(Default)]
pub struct Vm {
    pub frames: Vec<Frame>,
    pub stack: Vec<Value>,
}

impl World {
    /// public dispatch entry point. walks the proto chain and
    /// invokes the matching method, falling through to
    /// `:does-not-understand:with:` when no handler is found.
    pub fn send(
        &mut self,
        receiver: Value,
        selector: SymId,
        args: &[Value],
    ) -> Result<Value, RaiseError> {
        match self.lookup_handler(receiver, selector) {
            Some((handler, defining)) => {
                let method = handler.as_form_id().ok_or_else(|| {
                    RaiseError::new(
                        self.intern("dispatch-error"),
                        "handler is not a method-Form",
                    )
                })?;
                self.invoke(method, receiver, args, defining)
            }
            None => self.dispatch_dnu(receiver, selector, args),
        }
    }

    /// dispatch with an inline-cache fast path. used by `Op::Send`
    /// from the bytecode interpreter; the cli's direct `World::send`
    /// goes through the slow path.
    ///
    /// per `docs/laws/substrate-laws.md` L10: ICs check the cached
    /// proto's generation; mismatch (because `set-handler!`
    /// rewrote the table) triggers re-resolution.
    pub fn send_via_ic(
        &mut self,
        receiver: Value,
        selector: SymId,
        args: &[Value],
        chunk: FormId,
        ic_idx: u16,
    ) -> Result<Value, RaiseError> {
        let receiver_proto = match self.proto_of(receiver) {
            Value::Form(id) => id,
            _ => {
                // tagged-immediate proto chain bottoms unexpectedly
                // — fall back to the slow path which will dnu.
                return self.send(receiver, selector, args);
            }
        };
        // attempt fast path
        let cached = self
            .chunk_ics
            .get(&chunk)
            .and_then(|ics| ics.get(ic_idx as usize))
            .copied()
            .unwrap_or_default();
        if !cached.cached_proto.is_none()
            && cached.cached_proto == receiver_proto
            && cached.cached_generation == self.proto_generation(receiver_proto)
        {
            // cache hit — invoke the cached method directly with the
            // cached defining-proto so super-sends in the body
            // resolve correctly.
            return self.invoke(
                cached.cached_method,
                receiver,
                args,
                cached.cached_defining,
            );
        }
        // cache miss or stale — slow path + populate.
        match self.lookup_handler(receiver, selector) {
            Some((handler, defining)) => {
                let method = handler.as_form_id().ok_or_else(|| {
                    RaiseError::new(
                        self.intern("dispatch-error"),
                        "handler is not a method-Form",
                    )
                })?;
                // populate the IC slot.
                if let Some(ics) = self.chunk_ics.get_mut(&chunk) {
                    if let Some(slot) = ics.get_mut(ic_idx as usize) {
                        slot.cached_proto = receiver_proto;
                        slot.cached_method = method;
                        slot.cached_defining = defining;
                        slot.cached_generation =
                            self.proto_generations.get(&receiver_proto).copied().unwrap_or(0);
                    }
                }
                self.invoke(method, receiver, args, defining)
            }
            None => self.dispatch_dnu(receiver, selector, args),
        }
    }

    /// fall-through when no handler is found anywhere on the proto
    /// chain. constructs `(does-not-understand:with: <selector>
    /// <args>)` and re-dispatches. if `:does-not-understand:with:`
    /// itself is missing, raises a substrate error.
    fn dispatch_dnu(
        &mut self,
        receiver: Value,
        selector: SymId,
        args: &[Value],
    ) -> Result<Value, RaiseError> {
        let dnu = self.dnu_sym;
        if selector == dnu {
            // we got here from a previous dnu fall-through —
            // there's no handler to escalate to.
            let kind = self.intern("unhandled-dnu");
            return Err(RaiseError::new(
                kind,
                format!(
                    "no does-not-understand:with: handler for `{}`",
                    self.resolve(selector)
                ),
            ));
        }
        let args_list = self.make_list(args);
        let dnu_args = [Value::Sym(selector), args_list];
        self.send(receiver, dnu, &dnu_args)
    }

    /// invoke a specific method-Form with the given receiver/args.
    /// `defining_proto` is the proto on which the method was found
    /// — used by the new frame's `super` sends to walk above.
    pub fn invoke(
        &mut self,
        method: FormId,
        self_v: Value,
        args: &[Value],
        defining_proto: FormId,
    ) -> Result<Value, RaiseError> {
        // native?
        if let Some(&native_fn) = self.native_fns.get(&method) {
            return native_fn(self, self_v, args);
        }
        // bytecode method.
        let body = self.heap.get(method).slot(self.body_sym);
        let chunk_id = body.as_form_id().ok_or_else(|| {
            RaiseError::new(
                self.intern("dispatch-error"),
                "method body is not a chunk-Form",
            )
        })?;
        let captured_env = self
            .heap
            .get(method)
            .slot(self.env_sym)
            .as_form_id()
            .unwrap_or(self.global_env);
        let params_v = self.heap.get(method).slot(self.params_sym);
        let params = self
            .list_to_vec(params_v)
            .map_err(|e| RaiseError::new(self.intern("arity"), e))?;
        if params.len() != args.len() {
            return Err(RaiseError::new(
                self.intern("arity"),
                format!(
                    "method expects {} args; got {}",
                    params.len(),
                    args.len()
                ),
            ));
        }
        let call_env = self.alloc_env(Some(captured_env));
        for (param, &arg) in params.iter().zip(args.iter()) {
            let name = param.as_sym().ok_or_else(|| {
                RaiseError::new(self.intern("bad-param"), "param is not a symbol")
            })?;
            self.env_bind(call_env, name, arg);
        }
        run_method(self, chunk_id, call_env, self_v, defining_proto)
    }

    /// run a top-level chunk (no enclosing method). used by the
    /// repl and the cli to evaluate a single expression in the
    /// global env. equivalent to invoking a zero-arg method whose
    /// body is `chunk`. `defining_proto` is `NONE` because no
    /// method dispatch led here.
    pub fn run_top(&mut self, chunk: FormId) -> Result<Value, RaiseError> {
        let env = self.global_env;
        run_method(self, chunk, env, Value::Nil, FormId::NONE)
    }
}

/// run a chunk to completion, returning its top-of-stack on Return.
///
/// pushes a fresh frame, runs the loop until *that* frame returns,
/// pops the result.
fn run_method(
    world: &mut World,
    chunk: FormId,
    env: FormId,
    self_v: Value,
    defining_proto: FormId,
) -> Result<Value, RaiseError> {
    let starting_depth = world.vm.frames.len();
    world.vm.frames.push(Frame {
        chunk,
        pc: 0,
        env,
        self_: self_v,
        stack_base: world.vm.stack.len(),
        defining_proto,
    });
    while world.vm.frames.len() > starting_depth {
        step(world)?;
    }
    // the popped frame's last `Return` left its result on the stack.
    Ok(world.vm.stack.pop().unwrap_or(Value::Nil))
}

/// execute one bytecode op of the topmost frame.
fn step(world: &mut World) -> Result<(), RaiseError> {
    let frame_idx = world.vm.frames.len() - 1;
    let chunk = world.vm.frames[frame_idx].chunk;
    let pc = world.vm.frames[frame_idx].pc;

    let op = match world.chunk_ops.get(&chunk) {
        Some(ops) => ops
            .get(pc)
            .copied()
            .ok_or_else(|| {
                RaiseError::new(
                    world.intern("vm-error"),
                    format!("pc {} out of bounds in chunk {:?}", pc, chunk),
                )
            })?,
        None => {
            return Err(RaiseError::new(
                world.intern("vm-error"),
                "chunk has no bytecode (missing from chunk_ops)",
            ));
        }
    };
    world.vm.frames[frame_idx].pc += 1;

    match op {
        Op::PushNil => world.vm.stack.push(Value::Nil),
        Op::PushTrue => world.vm.stack.push(Value::Bool(true)),
        Op::PushFalse => world.vm.stack.push(Value::Bool(false)),
        Op::Pop => {
            world.vm.stack.pop();
        }
        Op::Dup => {
            let v = match world.vm.stack.last() {
                Some(&v) => v,
                None => {
                    return Err(RaiseError::new(
                        world.intern("vm-error"),
                        "Dup on empty stack",
                    ))
                }
            };
            world.vm.stack.push(v);
        }
        Op::LoadConst(idx) => {
            let v = world
                .chunk_consts
                .get(&chunk)
                .and_then(|c| c.get(idx as usize))
                .copied()
                .ok_or_else(|| {
                    RaiseError::new(
                        world.intern("vm-error"),
                        format!("const idx {} out of bounds", idx),
                    )
                })?;
            world.vm.stack.push(v);
        }
        Op::LoadName(name) => {
            let env = world.vm.frames[frame_idx].env;
            let v = world.env_lookup(env, name).ok_or_else(|| {
                RaiseError::new(
                    world.intern("unbound"),
                    format!("unbound name `{}`", world.resolve(name)),
                )
            })?;
            world.vm.stack.push(v);
        }
        Op::StoreName(name) => {
            let v = pop(world)?;
            let env = world.vm.frames[frame_idx].env;
            world.env_bind(env, name, v);
            // stores leave nil — `(set! x v)` expressions evaluate to nil.
            world.vm.stack.push(Value::Nil);
        }
        Op::DefineGlobal(name) => {
            let v = pop(world)?;
            let global = world.global_env;
            world.env_bind(global, name, v);
            // (def …) evaluates to the symbol it bound — useful in
            // a repl, mostly nothing in batch.
            world.vm.stack.push(Value::Sym(name));
        }
        Op::LoadSelf => {
            let s = world.vm.frames[frame_idx].self_;
            world.vm.stack.push(s);
        }
        Op::Send {
            selector,
            argc,
            ic_idx,
        } => {
            let (receiver, args) = pop_call_args(world, argc as usize)?;
            let result = world.send_via_ic(receiver, selector, &args, chunk, ic_idx)?;
            world.vm.stack.push(result);
        }
        Op::TailSend { selector, argc } => {
            let (receiver, args) = pop_call_args(world, argc as usize)?;
            // tail calls reuse the current frame for bytecode methods
            // — Rust-stack-bounded recursion is replaced with one
            // frame replace, satisfying tail-call optimization.
            let (resolved, defining) = match world.lookup_handler(receiver, selector) {
                Some((handler, defining)) => {
                    let id = handler.as_form_id().ok_or_else(|| {
                        RaiseError::new(
                            world.intern("dispatch-error"),
                            "handler is not a method-Form",
                        )
                    })?;
                    (id, defining)
                }
                None => {
                    // fall through to dnu — no TCO opportunity
                    // because the dispatch is itself a non-tail
                    // method call.
                    let result = world.dispatch_dnu(receiver, selector, &args)?;
                    world.vm.stack.push(result);
                    return Ok(());
                }
            };
            // native? same as Send's native path; pop args and push
            // result; no frame replacement.
            if let Some(&native_fn) = world.native_fns.get(&resolved) {
                let result = native_fn(world, receiver, &args)?;
                world.vm.stack.push(result);
                return Ok(());
            }
            // bytecode: replace the current frame.
            let body = world.heap.get(resolved).slot(world.body_sym);
            let chunk_id = body.as_form_id().ok_or_else(|| {
                RaiseError::new(
                    world.intern("dispatch-error"),
                    "method body is not a chunk-Form",
                )
            })?;
            let captured_env = world
                .heap
                .get(resolved)
                .slot(world.env_sym)
                .as_form_id()
                .unwrap_or(world.global_env);
            let params_v = world.heap.get(resolved).slot(world.params_sym);
            let params = world
                .list_to_vec(params_v)
                .map_err(|e| RaiseError::new(world.intern("arity"), e))?;
            if params.len() != args.len() {
                return Err(RaiseError::new(
                    world.intern("arity"),
                    format!(
                        "method expects {} args; got {}",
                        params.len(),
                        args.len()
                    ),
                ));
            }
            let call_env = world.alloc_env(Some(captured_env));
            for (param, &arg) in params.iter().zip(args.iter()) {
                let name = param.as_sym().ok_or_else(|| {
                    RaiseError::new(world.intern("bad-param"), "param is not a symbol")
                })?;
                world.env_bind(call_env, name, arg);
            }
            // truncate stack to the current frame's base — we
            // discard any leftover scratch from this frame's own
            // computation so the new tail-call frame starts clean.
            let base = world.vm.frames[frame_idx].stack_base;
            world.vm.stack.truncate(base);
            world.vm.frames[frame_idx] = Frame {
                chunk: chunk_id,
                pc: 0,
                env: call_env,
                self_: receiver,
                stack_base: base,
                defining_proto: defining,
            };
        }
        Op::SuperSend {
            selector,
            argc,
            ic_idx: _,
        } => {
            // [super selector args…] — receiver is the current
            // frame's self; lookup walks above frame.defining_proto.
            let argc_u = argc as usize;
            if world.vm.stack.len() < argc_u {
                return Err(RaiseError::new(
                    world.intern("vm-error"),
                    format!(
                        "super-send argc={} but stack has {}",
                        argc,
                        world.vm.stack.len()
                    ),
                ));
            }
            let split = world.vm.stack.len() - argc_u;
            let args: Vec<Value> = world.vm.stack.drain(split..).collect();
            let self_v = world.vm.frames[frame_idx].self_;
            let defining = world.vm.frames[frame_idx].defining_proto;
            if defining.is_none() {
                return Err(RaiseError::new(
                    world.intern("super-error"),
                    "super-send from a non-method frame (no defining proto)",
                ));
            }
            match world.lookup_handler_super(defining, selector) {
                Some((handler, new_defining)) => {
                    let method = handler.as_form_id().ok_or_else(|| {
                        RaiseError::new(
                            world.intern("dispatch-error"),
                            "super handler is not a method-Form",
                        )
                    })?;
                    let result = world.invoke(method, self_v, &args, new_defining)?;
                    world.vm.stack.push(result);
                }
                None => {
                    return Err(RaiseError::new(
                        world.intern("super-error"),
                        format!(
                            "no super-handler for `{}` above defining proto",
                            world.resolve(selector)
                        ),
                    ));
                }
            }
        }
        Op::PushClosure { chunk: closure_chunk } => {
            // capture the current env in a closure-Form whose body
            // is `closure_chunk`.
            let mut f = Form::with_proto(Value::Form(world.protos.closure));
            let env = world.vm.frames[frame_idx].env;
            f.slots.insert(world.body_sym, Value::Form(closure_chunk));
            f.slots.insert(world.env_sym, Value::Form(env));
            // the closure inherits the chunk's `:params` and `:source`.
            let params = world.heap.get(closure_chunk).slot(world.params_sym);
            f.slots.insert(world.params_sym, params);
            let source = world.heap.get(closure_chunk).meta_at(world.source_sym);
            if !source.is_nil() {
                f.meta.insert(world.source_sym, source);
            }
            let id = world.heap.alloc(f);
            world.vm.stack.push(Value::Form(id));
        }
        Op::Jump(off) => {
            let new_pc = (world.vm.frames[frame_idx].pc as isize - 1) + off as isize;
            if new_pc < 0 {
                return Err(RaiseError::new(
                    world.intern("vm-error"),
                    "jump went negative",
                ));
            }
            world.vm.frames[frame_idx].pc = new_pc as usize;
        }
        Op::JumpIfFalse(off) => {
            let v = pop(world)?;
            if !v.is_truthy() {
                let new_pc = (world.vm.frames[frame_idx].pc as isize - 1) + off as isize;
                if new_pc < 0 {
                    return Err(RaiseError::new(
                        world.intern("vm-error"),
                        "jump went negative",
                    ));
                }
                world.vm.frames[frame_idx].pc = new_pc as usize;
            }
        }
        Op::Return => {
            // top-of-stack is the return value. pop the frame; the
            // caller's top-of-stack will be the return value.
            let ret = pop(world)?;
            // discard any leftover stack belonging to this frame.
            let base = world.vm.frames[frame_idx].stack_base;
            world.vm.stack.truncate(base);
            world.vm.frames.pop();
            world.vm.stack.push(ret);
        }
    }
    Ok(())
}

/// pop the top of stack, raising an `RaiseError` on underflow.
fn pop(world: &mut World) -> Result<Value, RaiseError> {
    world.vm.stack.pop().ok_or_else(|| {
        RaiseError::new(world.intern("vm-error"), "operand stack underflow")
    })
}

/// pop `argc` argument values and the receiver. returns
/// `(receiver, args)` with args in declaration order (oldest first).
fn pop_call_args(
    world: &mut World,
    argc: usize,
) -> Result<(Value, Vec<Value>), RaiseError> {
    if world.vm.stack.len() < argc + 1 {
        return Err(RaiseError::new(
            world.intern("vm-error"),
            format!("send argc={} but stack has {}", argc, world.vm.stack.len()),
        ));
    }
    let split = world.vm.stack.len() - argc;
    let args: Vec<Value> = world.vm.stack.drain(split..).collect();
    let receiver = world
        .vm
        .stack
        .pop()
        .expect("split implies receiver present");
    Ok((receiver, args))
}

/// the world owns a `Vm`. add it to the struct via the trait below
/// (declared here to keep the storage decision local to vm.rs).
pub trait WithVm {
    fn vm(&mut self) -> &mut Vm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::Form;

    /// install a minimal `:plus` native on Integer for tests below.
    fn install_int_plus(w: &mut World) {
        w.install_native(w.protos.integer, "+", |_, self_, args| {
            let a = self_.as_int().unwrap();
            let b = args[0].as_int().unwrap();
            Ok(Value::Int(a + b))
        });
    }

    #[test]
    fn send_dispatches_native() {
        let mut w = World::new();
        install_int_plus(&mut w);
        let plus = w.intern("+");
        let r = w.send(Value::Int(3), plus, &[Value::Int(4)]).unwrap();
        assert_eq!(r, Value::Int(7));
    }

    #[test]
    fn send_walks_proto_chain() {
        let mut w = World::new();
        // install a method on Object that integers should still hit.
        w.install_native(w.protos.object, "echo", |_, self_, _| Ok(self_));
        let echo = w.intern("echo");
        assert_eq!(w.send(Value::Int(42), echo, &[]).unwrap(), Value::Int(42));
        assert_eq!(w.send(Value::Bool(true), echo, &[]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn send_unhandled_calls_dnu_then_raises() {
        let w_check = World::new();
        // no Object handler for :does-not-understand:with: yet —
        // that's installed in phase A.7. so: dispatch falls through
        // to dnu, which itself misses, which raises.
        let mut w = w_check;
        let mystery = w.intern("mystery");
        let err = w.send(Value::Int(5), mystery, &[]).unwrap_err();
        // should be unhandled-dnu, since dnu itself isn't bound.
        assert_eq!(w.resolve(err.kind), "unhandled-dnu");
    }

    #[test]
    fn send_dnu_user_override_intercepts() {
        let mut w = World::new();
        // install dnu on Object that returns the selector as its result.
        w.install_native(
            w.protos.object,
            "doesNotUnderstand:with:",
            |_, _self_, args| Ok(args[0]),
        );
        let mystery = w.intern("mystery-selector");
        let r = w.send(Value::Int(5), mystery, &[Value::Int(99)]).unwrap();
        // dnu received (selector args-list); we return selector.
        assert_eq!(r, Value::Sym(mystery));
    }

    #[test]
    fn run_top_executes_a_chunk_pushing_a_const() {
        let mut w = World::new();
        // build a chunk: LoadConst(0), Return.
        // const[0] = Int(7)
        let mut chunk_form = Form::with_proto(Value::Form(w.protos.chunk));
        // params: empty list
        chunk_form.slots.insert(w.params_sym, Value::Nil);
        let chunk_id = w.alloc(chunk_form);
        w.chunk_ops
            .insert(chunk_id, vec![Op::LoadConst(0), Op::Return]);
        w.chunk_consts.insert(chunk_id, vec![Value::Int(7)]);
        let r = w.run_top(chunk_id).unwrap();
        assert_eq!(r, Value::Int(7));
    }

    #[test]
    fn vm_handles_jump_if_false() {
        let mut w = World::new();
        // chunk:
        //   PushFalse                ; 0
        //   JumpIfFalse(+3)          ; 1: jumps to 4
        //   LoadConst(0)             ; 2 (skipped)
        //   Return                   ; 3 (skipped)
        //   LoadConst(1)             ; 4
        //   Return                   ; 5
        // const[0] = 'wrong, const[1] = 'right
        let wrong = w.intern("wrong");
        let right = w.intern("right");
        let mut chunk_form = Form::with_proto(Value::Form(w.protos.chunk));
        chunk_form.slots.insert(w.params_sym, Value::Nil);
        let chunk_id = w.alloc(chunk_form);
        w.chunk_ops.insert(
            chunk_id,
            vec![
                Op::PushFalse,
                Op::JumpIfFalse(3),
                Op::LoadConst(0),
                Op::Return,
                Op::LoadConst(1),
                Op::Return,
            ],
        );
        w.chunk_consts
            .insert(chunk_id, vec![Value::Sym(wrong), Value::Sym(right)]);
        let r = w.run_top(chunk_id).unwrap();
        assert_eq!(r, Value::Sym(right));
    }

    #[test]
    fn vm_send_op_dispatches_through_send() {
        // a chunk that pushes 3, pushes 4, sends `:+` with arity 1.
        let mut w = World::new();
        install_int_plus(&mut w);
        let plus = w.intern("+");
        let mut chunk_form = Form::with_proto(Value::Form(w.protos.chunk));
        chunk_form.slots.insert(w.params_sym, Value::Nil);
        let chunk_id = w.alloc(chunk_form);
        w.chunk_ops.insert(
            chunk_id,
            vec![
                Op::LoadConst(0),
                Op::LoadConst(1),
                Op::Send {
                    selector: plus,
                    argc: 1,
                    ic_idx: 0,
                },
                Op::Return,
            ],
        );
        w.chunk_consts
            .insert(chunk_id, vec![Value::Int(3), Value::Int(4)]);
        let r = w.run_top(chunk_id).unwrap();
        assert_eq!(r, Value::Int(7));
    }

    #[test]
    fn unbound_name_raises() {
        let mut w = World::new();
        let foo = w.intern("foo");
        let mut chunk_form = Form::with_proto(Value::Form(w.protos.chunk));
        chunk_form.slots.insert(w.params_sym, Value::Nil);
        let chunk_id = w.alloc(chunk_form);
        w.chunk_ops
            .insert(chunk_id, vec![Op::LoadName(foo), Op::Return]);
        w.chunk_consts.insert(chunk_id, vec![]);
        let err = w.run_top(chunk_id).unwrap_err();
        assert_eq!(w.resolve(err.kind), "unbound");
    }
}
