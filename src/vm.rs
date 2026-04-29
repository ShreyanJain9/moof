//! the bytecode interpreter and send dispatch.
//!
//! this module is the *substrate's universal verb*
//! (laws/substrate-laws.md L3): every operation that produces a
//! value or causes an effect goes through `send_dispatch`.
//!
//! phase 2 introduces frames (one per active method invocation),
//! lexical envs (Forms with `__parent__` chains), and Closure
//! invocation. send to a Bytecode method now pushes a frame; Return
//! pops it and resumes the caller.

use crate::form::{Form, FormId, MethodImpl};
use crate::opcodes::{ChunkId, ICache, Op};
use crate::sym::SymId;
use crate::value::Value;
use crate::world::World;

/// run a top-level chunk in the given starting env. this is what the
/// CLI invokes after the reader and compiler produce a Chunk.
pub fn run_chunk(world: &mut World, chunk: ChunkId, env: FormId) -> Result<Value, String> {
    let mut stack: Vec<Value> = Vec::with_capacity(64);
    let mut frames: Vec<Frame> = Vec::with_capacity(16);
    let mut frame = Frame {
        chunk,
        pc: 0,
        env,
    };

    loop {
        // fetch op
        let op = {
            let c = &world.chunks[frame.chunk.0 as usize];
            if frame.pc >= c.ops.len() {
                return Err(format!(
                    "ran off end of chunk {} at pc {}",
                    frame.chunk.0, frame.pc
                ));
            }
            c.ops[frame.pc].clone()
        };
        frame.pc += 1;

        match op {
            Op::LoadNil => stack.push(Value::Nil),
            Op::LoadBool(b) => stack.push(Value::Bool(b)),
            Op::LoadInt(n) => stack.push(Value::Int(n as i64)),
            Op::LoadConst(idx) => {
                let v = world.chunks[frame.chunk.0 as usize].consts[idx as usize];
                stack.push(v);
            }
            Op::LoadSym(s) => stack.push(Value::Sym(s)),

            Op::LoadName(name) => {
                let v = world.env_lookup(frame.env, name).ok_or_else(|| {
                    format!("undefined: {}", world.syms.name(name))
                })?;
                stack.push(v);
            }
            Op::DefineName(name) => {
                let v = stack
                    .pop()
                    .ok_or_else(|| "DefineName: empty stack".to_string())?;
                world.env_define(frame.env, name, v);
            }
            Op::SetName(name) => {
                let v = stack
                    .pop()
                    .ok_or_else(|| "SetName: empty stack".to_string())?;
                world.env_set(frame.env, name, v)?;
            }

            Op::Send { sel, arity, ic_idx } => {
                let n = arity as usize;
                if stack.len() < n + 1 {
                    return Err("send: not enough values on stack".into());
                }
                let args_start = stack.len() - n;
                let args: Vec<Value> = stack.drain(args_start..).collect();
                let recv = stack.pop().expect("recv");

                // look up method (with cache).
                let method = resolve_with_cache(world, frame.chunk, ic_idx, recv, sel)?;
                match method {
                    MethodImpl::Native(f) => {
                        let result = f(world, recv, &args)?;
                        stack.push(result);
                    }
                    MethodImpl::Bytecode {
                        chunk: callee_chunk,
                        captured_env,
                        params,
                    } => {
                        // method/closure invocation. set up new env + frame.
                        let new_env = setup_method_call(world, recv, captured_env, params, &args)?;
                        // save current frame, switch to callee.
                        frames.push(frame);
                        frame = Frame {
                            chunk: callee_chunk,
                            pc: 0,
                            env: new_env,
                        };
                    }
                }
            }

            Op::Branch(offset) => {
                frame.pc = (frame.pc as isize + offset as isize) as usize;
            }
            Op::BranchIfFalse(offset) => {
                let v = stack
                    .pop()
                    .ok_or_else(|| "BranchIfFalse: empty stack".to_string())?;
                if !v.is_truthy() {
                    frame.pc = (frame.pc as isize + offset as isize) as usize;
                }
            }

            Op::PushScope => {
                let new_env = world.alloc_env(Value::Form(frame.env));
                frame.env = new_env;
            }
            Op::PopScope => {
                let parent = world
                    .heap
                    .get(frame.env)
                    .slots
                    .get(&world.parent_sym)
                    .copied()
                    .ok_or_else(|| "PopScope: env has no parent".to_string())?;
                match parent {
                    Value::Form(p) => frame.env = p,
                    _ => return Err("PopScope: parent is not an env".into()),
                }
            }

            Op::MakeClosure {
                chunk_idx,
                params_idx,
            } => {
                let body_chunk = world.chunks[frame.chunk.0 as usize].nested[chunk_idx as usize];
                let params = world.chunks[frame.chunk.0 as usize].consts[params_idx as usize];
                let closure_id = make_closure(world, body_chunk, params, frame.env);
                stack.push(Value::Form(closure_id));
            }

            Op::Pop => {
                stack.pop();
            }
            Op::Return => {
                let result = stack
                    .pop()
                    .ok_or_else(|| "return: empty stack".to_string())?;
                if let Some(prev) = frames.pop() {
                    frame = prev;
                    stack.push(result);
                } else {
                    return Ok(result);
                }
            }
        }
    }
}

/// the executing frame.
struct Frame {
    chunk: ChunkId,
    pc: usize,
    env: FormId,
}

/// look up a method, using the chunk's IC slot at `ic_idx`.
/// fast path on cache hit; slow path walks the proto chain.
fn resolve_with_cache(
    world: &mut World,
    chunk: ChunkId,
    ic_idx: u16,
    recv: Value,
    sel: SymId,
) -> Result<MethodImpl, String> {
    let start = world.dispatch_start(recv);

    // ── fast path
    {
        let ic = &world.chunks[chunk.0 as usize].ics[ic_idx as usize];
        if let (Some(cached), Some(method)) = (ic.cached_proto, ic.cached_method.clone()) {
            if cached == start {
                return Ok(method);
            }
        }
    }

    // ── slow path: walk chain.
    let (_resolved, method) = resolve_method(world, start, sel).ok_or_else(|| {
        format!(
            "does-not-understand: {} (no handler for {} on {:?})",
            world.syms.name(sel),
            world.syms.name(sel),
            recv
        )
    })?;

    {
        let ic = &mut world.chunks[chunk.0 as usize].ics[ic_idx as usize];
        *ic = ICache {
            cached_proto: Some(start),
            cached_method: Some(method.clone()),
        };
    }

    Ok(method)
}

/// set up the env for a method (or free-closure) invocation.
/// allocates a fresh env (parent = captured_env), auto-binds `self`
/// to the receiver, and binds each parameter to the corresponding
/// argument.
///
/// `recv` is the dispatch receiver — for `[c incr]` it's the instance;
/// for `(f x)` it's the closure itself.
fn setup_method_call(
    world: &mut World,
    recv: Value,
    captured_env: FormId,
    params_value: Value,
    args: &[Value],
) -> Result<FormId, String> {
    let new_env = world.alloc_env(Value::Form(captured_env));

    // auto-bind `self` to the receiver.
    // (concepts/objects-and-protos.md, syntax/methods-and-handlers.md.)
    let self_sym = world.syms.intern("self");
    world.env_define(new_env, self_sym, recv);

    // walk params (a List) and bind each to the corresponding arg.
    let mut cur = params_value;
    let mut idx = 0usize;
    loop {
        match cur {
            Value::Nil => {
                if idx != args.len() {
                    return Err(format!(
                        "arity mismatch: closure expects {}, got {}",
                        idx,
                        args.len()
                    ));
                }
                break;
            }
            Value::Form(fid) => {
                let (head, next) = {
                    let f = world.heap.get(fid);
                    if f.proto != world.list_proto {
                        return Err("closure params: not a list".into());
                    }
                    (f.head, f.args)
                };
                let pname = match head {
                    Value::Sym(s) => s,
                    _ => return Err("closure params: not a symbol".into()),
                };
                let arg = args
                    .get(idx)
                    .copied()
                    .ok_or_else(|| format!("arity mismatch: missing arg {idx}"))?;
                world.env_define(new_env, pname, arg);
                cur = next;
                idx += 1;
            }
            _ => return Err(format!("closure params: improper list")),
        }
    }

    Ok(new_env)
}

/// allocate a closure Form. its `:call` handler is a Bytecode method
/// whose captured-env + params are baked in at MakeClosure time.
fn make_closure(
    world: &mut World,
    body: ChunkId,
    params: Value,
    captured: FormId,
) -> FormId {
    let mut form = Form::with_proto(world.closure_proto);
    // also stash captured env + params as observable slots for
    // reflection (laws/reflection-contract.md R2). the dispatch path
    // doesn't read them — it uses the MethodImpl below — but `[c slots]`
    // sees them.
    let captured_env_sym = world.syms.intern("__captured_env__");
    let params_sym = world.syms.intern("__params__");
    form.slots.insert(captured_env_sym, Value::Form(captured));
    form.slots.insert(params_sym, params);
    form.handlers.insert(
        world.call_sym,
        MethodImpl::Bytecode {
            chunk: body,
            captured_env: captured,
            params,
        },
    );
    world.heap.alloc(form)
}

/// public send entry point — used by builtins for internal dispatches
/// without an inline-cache slot.
pub fn send_dispatch(
    world: &mut World,
    recv: Value,
    sel: SymId,
    args: &[Value],
) -> Result<Value, String> {
    let start = world.dispatch_start(recv);
    let (_p, method) = resolve_method(world, start, sel).ok_or_else(|| {
        format!(
            "does-not-understand: {} (no handler for {} on {:?})",
            world.syms.name(sel),
            world.syms.name(sel),
            recv
        )
    })?;
    invoke_resolved(world, method, recv, args)
}

/// invoke a resolved method directly (no IC). used by send_dispatch
/// for non-bytecode-context calls.
fn invoke_resolved(
    world: &mut World,
    method: MethodImpl,
    recv: Value,
    args: &[Value],
) -> Result<Value, String> {
    match method {
        MethodImpl::Native(f) => f(world, recv, args),
        MethodImpl::Bytecode {
            chunk,
            captured_env,
            params,
        } => {
            // run the chunk as a sub-execution. (this re-enters the
            // interpreter; rust call stack grows by one. fine for
            // phase 2 — phase 3 may unify the dispatch loops.)
            let new_env = setup_method_call(world, recv, captured_env, params, args)?;
            run_chunk(world, chunk, new_env)
        }
    }
}

/// walk the proto chain looking for the selector.
fn resolve_method(
    world: &World,
    start: FormId,
    sel: SymId,
) -> Option<(FormId, MethodImpl)> {
    let mut cur = start;
    while !cur.is_none() {
        let f = world.heap.get(cur);
        if let Some(m) = f.handlers.get(&sel) {
            return Some((cur, m.clone()));
        }
        cur = f.proto;
    }
    None
}
