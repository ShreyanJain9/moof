/// Bytecode interpreter for the moof language shell.
///
/// Implements HandlerInvoker so the fabric can dispatch to bytecode-compiled
/// lambdas and operatives. The interpreter is stateless — all execution state
/// (stack, frames) lives on the Rust call stack, making re-entrant sends clean.

use moof_fabric::{Value, HeapObject, Heap, Fabric};
use moof_fabric::dispatch::{self, HandlerInvoker, InvokeContext};
use crate::opcodes::*;
use crate::compiler::Compiler;

/// Execute a bytecode chunk in the given environment.
/// This is the entry point for the REPL and bootstrap.
pub fn eval_chunk(fabric: &mut Fabric, chunk_id: u32, env_id: u32) -> Result<Value, String> {
    let mut stack = Vec::new();
    let mut frames = vec![CallFrame {
        chunk_id,
        ip: 0,
        env_id,
        stack_base: 0,
    }];

    // Destructure fabric to get separate borrows
    let sym_dnu = fabric.sym_dnu();
    let mut ctx = InvokeContext {
        heap: &mut fabric.heap,
        type_protos: &fabric.type_protos,
        invokers: &fabric.invokers,
        sym_does_not_understand: sym_dnu,
    };

    run(&mut ctx, &mut stack, &mut frames)
}

// ── Data structures ──

struct CallFrame {
    chunk_id: u32,
    ip: usize,
    env_id: u32,
    stack_base: usize,
}

/// Stateless bytecode invoker. Registered with the fabric to handle
/// lambda and operative handler objects.
pub struct BytecodeInvoker;

impl BytecodeInvoker {
    pub fn new() -> Self {
        BytecodeInvoker
    }
}

// ── HandlerInvoker impl ──

impl HandlerInvoker for BytecodeInvoker {
    fn can_invoke(&self, heap: &Heap, handler: Value) -> bool {
        if let Value::Object(id) = handler {
            if let Some(sym) = heap.symbol_lookup_only("type-tag") {
                let tag = heap.slot_get(id, sym);
                if let Value::Symbol(s) = tag {
                    let name = heap.symbol_name(s);
                    return name == "lambda" || name == "operative";
                }
            }
        }
        false
    }

    fn invoke(
        &self,
        ctx: &mut InvokeContext,
        handler: Value,
        _receiver: Value,
        args: &[Value],
    ) -> Result<Value, String> {
        let handler_id = handler.as_object().ok_or("bytecode handler must be object")?;

        // Read handler slots
        let type_tag_sym = ctx.heap.intern("type-tag");
        let params_sym = ctx.heap.intern("params");
        let body_sym = ctx.heap.intern("body");
        let def_env_sym = ctx.heap.intern("def-env");
        let env_param_sym = ctx.heap.intern("env-param");

        let tag = ctx.heap.slot_get(handler_id, type_tag_sym);
        let params = ctx.heap.slot_get(handler_id, params_sym);
        let body_val = ctx.heap.slot_get(handler_id, body_sym);
        let def_env_val = ctx.heap.slot_get(handler_id, def_env_sym);

        let body_id = body_val.as_object().ok_or("handler body must be object")?;
        let def_env_id = def_env_val.as_object().ok_or("handler def-env must be object")?;

        let is_lambda = match tag {
            Value::Symbol(s) => ctx.heap.symbol_name(s) == "lambda",
            _ => false,
        };

        // Create child environment of def-env
        let call_env = ctx.heap.alloc_env(Some(def_env_id));

        if is_lambda {
            // Lambda: bind params to args (args are already evaluated)
            let args_list = ctx.heap.list(args);
            bind_params(ctx.heap, call_env, params, args_list);
        } else {
            // Operative: bind params to unevaluated args, plus env-param to caller env
            let env_param_val = ctx.heap.slot_get(handler_id, env_param_sym);
            if let Value::Symbol(ep_sym) = env_param_val {
                // The caller's environment is not directly available here —
                // operatives invoked through the fabric get args as-is.
                // For now, bind env-param to nil (the caller should pass it).
                // In practice, operatives are called from within the bytecode loop
                // where we have the caller's env.
                ctx.heap.env_define(call_env, ep_sym, Value::Nil);
            }
            let args_list = ctx.heap.list(args);
            bind_params(ctx.heap, call_env, params, args_list);
        }

        // Set up execution state and run
        let mut stack = Vec::new();
        let mut frames = vec![CallFrame {
            chunk_id: body_id,
            ip: 0,
            env_id: call_env,
            stack_base: 0,
        }];

        run(ctx, &mut stack, &mut frames)
    }
}

// ── Helpers ──

/// Read bytecode and constants from a chunk Object.
/// Returns owned copies so the heap can be mutated freely afterward.
fn read_chunk(heap: &Heap, chunk_id: u32) -> Result<(Vec<u8>, Vec<Value>), String> {
    let code_sym = heap.symbol_lookup_only("code")
        .ok_or("read_chunk: 'code' symbol not interned")?;
    let constants_sym = heap.symbol_lookup_only("constants")
        .ok_or("read_chunk: 'constants' symbol not interned")?;

    let code_val = heap.slot_get(chunk_id, code_sym);
    let code_id = code_val.as_object().ok_or("read_chunk: code slot not an object")?;
    let code = match heap.get(code_id) {
        HeapObject::Bytes(bytes) => bytes.clone(),
        _ => return Err("read_chunk: code is not Bytes".into()),
    };

    let constants_val = heap.slot_get(chunk_id, constants_sym);
    let constants = heap.list_to_vec(constants_val);

    Ok((code, constants))
}

/// Bind a params cons-list to an args cons-list in the given environment.
fn bind_params(heap: &mut Heap, env_id: u32, params: Value, args: Value) {
    let mut p = params;
    let mut a = args;
    loop {
        match p {
            Value::Nil => break,
            Value::Symbol(sym) => {
                // Rest parameter: bind remaining args as a list
                heap.env_define(env_id, sym, a);
                break;
            }
            Value::Object(pid) => {
                match heap.get(pid).clone() {
                    HeapObject::Cons { car, cdr } => {
                        let arg_val = heap.car(a);
                        if let Value::Symbol(sym) = car {
                            heap.env_define(env_id, sym, arg_val);
                        }
                        p = cdr;
                        a = heap.cdr(a);
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
}

/// Create a lambda Object on the heap.
pub fn create_lambda(heap: &mut Heap, params: Value, body: u32, def_env: u32, source: Value) -> u32 {
    let type_tag_sym = heap.intern("type-tag");
    let params_sym = heap.intern("params");
    let body_sym = heap.intern("body");
    let def_env_sym = heap.intern("def-env");
    let source_sym = heap.intern("source");
    let lambda_sym = heap.intern("lambda");

    heap.alloc(HeapObject::Object {
        parent: Value::Nil,
        slots: vec![
            (type_tag_sym, Value::Symbol(lambda_sym)),
            (params_sym, params),
            (body_sym, Value::Object(body)),
            (def_env_sym, Value::Object(def_env)),
            (source_sym, source),
        ],
        handlers: Vec::new(),
    })
}

/// Create an operative Object on the heap.
pub fn create_operative(
    heap: &mut Heap,
    params: Value,
    env_param: u32,
    body: u32,
    def_env: u32,
    source: Value,
) -> u32 {
    let type_tag_sym = heap.intern("type-tag");
    let params_sym = heap.intern("params");
    let env_param_sym = heap.intern("env-param");
    let body_sym = heap.intern("body");
    let def_env_sym = heap.intern("def-env");
    let source_sym = heap.intern("source");
    let operative_sym = heap.intern("operative");

    heap.alloc(HeapObject::Object {
        parent: Value::Nil,
        slots: vec![
            (type_tag_sym, Value::Symbol(operative_sym)),
            (params_sym, params),
            (env_param_sym, Value::Symbol(env_param)),
            (body_sym, Value::Object(body)),
            (def_env_sym, Value::Object(def_env)),
            (source_sym, source),
        ],
        handlers: Vec::new(),
    })
}

/// Classify a value as lambda, operative, or other.
/// Returns: 0 = other, 1 = lambda, 2 = operative
fn classify_callable(heap: &Heap, val: Value) -> u8 {
    if let Value::Object(id) = val {
        if let Some(sym) = heap.symbol_lookup_only("type-tag") {
            let tag = heap.slot_get(id, sym);
            if let Value::Symbol(s) = tag {
                let name = heap.symbol_name(s);
                if name == "lambda" { return 1; }
                if name == "operative" { return 2; }
            }
        }
        // Check for native handler
        if let Some(sym) = heap.symbol_lookup_only("native-name") {
            if heap.slot_get(id, sym) != Value::Nil {
                return 3; // native
            }
        }
    }
    0
}

/// Call a lambda by pushing a new frame.
fn push_lambda_frame(
    heap: &mut Heap,
    stack: &mut Vec<Value>,
    frames: &mut Vec<CallFrame>,
    handler_id: u32,
    args: &[Value],
) -> Result<(), String> {
    let params_sym = heap.symbol_lookup_only("params").unwrap();
    let body_sym = heap.symbol_lookup_only("body").unwrap();
    let def_env_sym = heap.symbol_lookup_only("def-env").unwrap();

    let params = heap.slot_get(handler_id, params_sym);
    let body_val = heap.slot_get(handler_id, body_sym);
    let def_env_val = heap.slot_get(handler_id, def_env_sym);

    let body_id = body_val.as_object().ok_or("lambda body must be object")?;
    let def_env_id = def_env_val.as_object().ok_or("lambda def-env must be object")?;

    let call_env = heap.alloc_env(Some(def_env_id));
    let args_list = heap.list(args);
    bind_params(heap, call_env, params, args_list);

    frames.push(CallFrame {
        chunk_id: body_id,
        ip: 0,
        env_id: call_env,
        stack_base: stack.len(),
    });
    Ok(())
}

/// Call an operative with a raw args list and caller env.
fn push_operative_frame(
    heap: &mut Heap,
    stack: &mut Vec<Value>,
    frames: &mut Vec<CallFrame>,
    handler_id: u32,
    args_list: Value,
    caller_env: u32,
) -> Result<(), String> {
    let params_sym = heap.symbol_lookup_only("params").unwrap();
    let body_sym = heap.symbol_lookup_only("body").unwrap();
    let def_env_sym = heap.symbol_lookup_only("def-env").unwrap();
    let env_param_sym_key = heap.symbol_lookup_only("env-param").unwrap();

    let params = heap.slot_get(handler_id, params_sym);
    let body_val = heap.slot_get(handler_id, body_sym);
    let def_env_val = heap.slot_get(handler_id, def_env_sym);
    let env_param_val = heap.slot_get(handler_id, env_param_sym_key);

    let body_id = body_val.as_object().ok_or("operative body must be object")?;
    let def_env_id = def_env_val.as_object().ok_or("operative def-env must be object")?;

    let call_env = heap.alloc_env(Some(def_env_id));

    // Bind the env parameter to the caller's environment
    if let Value::Symbol(ep_sym) = env_param_val {
        heap.env_define(call_env, ep_sym, Value::Object(caller_env));
    }

    bind_params(heap, call_env, params, args_list);

    frames.push(CallFrame {
        chunk_id: body_id,
        ip: 0,
        env_id: call_env,
        stack_base: stack.len(),
    });
    Ok(())
}

/// Replace the current frame with a lambda tail call.
fn tail_call_lambda(
    heap: &mut Heap,
    stack: &mut Vec<Value>,
    frames: &mut Vec<CallFrame>,
    handler_id: u32,
    args: &[Value],
) -> Result<(), String> {
    let params_sym = heap.symbol_lookup_only("params").unwrap();
    let body_sym = heap.symbol_lookup_only("body").unwrap();
    let def_env_sym = heap.symbol_lookup_only("def-env").unwrap();

    let params = heap.slot_get(handler_id, params_sym);
    let body_val = heap.slot_get(handler_id, body_sym);
    let def_env_val = heap.slot_get(handler_id, def_env_sym);

    let body_id = body_val.as_object().ok_or("lambda body must be object")?;
    let def_env_id = def_env_val.as_object().ok_or("lambda def-env must be object")?;

    let call_env = heap.alloc_env(Some(def_env_id));
    let args_list = heap.list(args);
    bind_params(heap, call_env, params, args_list);

    let frame = frames.last_mut().unwrap();
    stack.truncate(frame.stack_base);
    frame.chunk_id = body_id;
    frame.ip = 0;
    frame.env_id = call_env;
    Ok(())
}

// ── The run loop ──

fn run(
    ctx: &mut InvokeContext,
    stack: &mut Vec<Value>,
    frames: &mut Vec<CallFrame>,
) -> Result<Value, String> {
    let base_depth = frames.len() - 1;

    loop {
        if frames.is_empty() {
            return Ok(stack.pop().unwrap_or(Value::Nil));
        }

        // Read chunk code + constants into locals (avoiding borrow conflicts)
        let (chunk_id, mut ip, env_id, stack_base) = {
            let f = frames.last().unwrap();
            (f.chunk_id, f.ip, f.env_id, f.stack_base)
        };

        let (code, constants) = read_chunk(ctx.heap, chunk_id)?;

        // Inner loop: execute instructions from this chunk until we need
        // to reload (frame change, send, etc.)
        loop {
            if ip >= code.len() {
                // Implicit return at end of chunk
                let result = stack.pop().unwrap_or(Value::Nil);
                stack.truncate(stack_base);
                frames.pop();
                if frames.len() <= base_depth {
                    return Ok(result);
                }
                stack.push(result);
                break; // reload outer loop for new frame's chunk
            }

            let op = code[ip];
            match op {
                OP_CONST => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    ip += 3;
                    stack.push(constants[idx]);
                }

                OP_NIL => {
                    ip += 1;
                    stack.push(Value::Nil);
                }

                OP_TRUE => {
                    ip += 1;
                    stack.push(Value::True);
                }

                OP_FALSE => {
                    ip += 1;
                    stack.push(Value::False);
                }

                OP_POP => {
                    ip += 1;
                    stack.pop();
                }

                OP_LOOKUP => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    ip += 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("LOOKUP: expected symbol in constants".into()),
                    };
                    match ctx.heap.env_lookup(env_id, sym) {
                        Some(val) => stack.push(val),
                        None => {
                            let name = ctx.heap.symbol_name(sym).to_string();
                            return Err(format!("unbound: {}", name));
                        }
                    }
                }

                OP_DEF => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    ip += 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("DEF: expected symbol in constants".into()),
                    };
                    let val = stack.pop().ok_or("DEF: empty stack")?;
                    ctx.heap.env_define(env_id, sym, val);
                    stack.push(val);
                }

                OP_GET_ENV => {
                    ip += 1;
                    stack.push(Value::Object(env_id));
                }

                OP_SEND => {
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    ip += 4;
                    // Save ip before re-entrant call
                    frames.last_mut().unwrap().ip = ip;

                    let selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("SEND: expected symbol selector".into()),
                    };

                    let stack_start = stack.len() - argc - 1;
                    let receiver = stack[stack_start];
                    let args: Vec<Value> = stack[stack_start + 1..].to_vec();
                    stack.truncate(stack_start);

                    // Intercept eval: on environments (moof-lang concept, not fabric)
                    let sel_name = ctx.heap.symbol_name(selector).to_string();
                    if sel_name == "eval:" {
                        if let Value::Object(eid) = receiver {
                            if matches!(ctx.heap.get(eid), HeapObject::Environment { .. }) {
                                let expr = args.first().copied().unwrap_or(Value::Nil);
                                let result = eval_value(ctx, stack, frames, expr, eid)?;
                                stack.push(result);
                                break;
                            }
                        }
                    }

                    // Re-entrant: may invoke this interpreter again via dispatch
                    let result = dispatch::send(
                        ctx.heap,
                        ctx.type_protos,
                        ctx.invokers,
                        ctx.sym_does_not_understand,
                        receiver,
                        selector,
                        &args,
                    )?;
                    stack.push(result);
                    break; // reload chunk (heap may have changed)
                }

                OP_EVENTUAL_SEND => {
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    ip += 4;
                    frames.last_mut().unwrap().ip = ip;

                    let _selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("EVENTUAL_SEND: expected symbol selector".into()),
                    };

                    let stack_start = stack.len() - argc - 1;
                    let receiver = stack[stack_start];
                    let _args: Vec<Value> = stack[stack_start + 1..].to_vec();
                    stack.truncate(stack_start);

                    let _receiver_id = match receiver {
                        Value::Object(id) => id,
                        _ => return Err("eventual send: receiver must be an object".into()),
                    };

                    // Create a promise object
                    let val_sym = ctx.heap.intern("value");
                    let resolved_sym = ctx.heap.intern("resolved");
                    let promise_id = ctx.heap.alloc(HeapObject::Object {
                        parent: Value::Nil,
                        slots: vec![
                            (val_sym, Value::Nil),
                            (resolved_sym, Value::False),
                        ],
                        handlers: Vec::new(),
                    });

                    // TODO: enqueue message on vat mailbox when scheduler is wired up
                    stack.push(Value::Object(promise_id));
                    break; // reload
                }

                OP_CONS => {
                    ip += 1;
                    let cdr = stack.pop().ok_or("CONS: empty stack")?;
                    let car = stack.pop().ok_or("CONS: empty stack")?;
                    let val = ctx.heap.cons(car, cdr);
                    stack.push(val);
                }

                OP_EQ => {
                    ip += 1;
                    let b = stack.pop().ok_or("EQ: empty stack")?;
                    let a = stack.pop().ok_or("EQ: empty stack")?;
                    stack.push(if a == b { Value::True } else { Value::False });
                }

                OP_QUOTE => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    ip += 3;
                    stack.push(constants[idx]);
                }

                OP_VAU => {
                    let params_idx = read_u16(&code, ip + 1) as usize;
                    let env_param_idx = read_u16(&code, ip + 3) as usize;
                    let body_idx = read_u16(&code, ip + 5) as usize;
                    let source_idx = read_u16(&code, ip + 7) as usize;
                    ip += 9;
                    frames.last_mut().unwrap().ip = ip;

                    let params = constants[params_idx];
                    let env_param_val = match constants[env_param_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("VAU: expected symbol for env param".into()),
                    };
                    let body_chunk = match constants[body_idx] {
                        Value::Object(id) => id,
                        _ => return Err("VAU: expected object for body chunk".into()),
                    };
                    let source = constants[source_idx];

                    // Convention: $_ means "lambda" (wrapped operative that evals args)
                    let name = ctx.heap.symbol_name(env_param_val).to_string();
                    let obj_id = if name == "$_" {
                        create_lambda(ctx.heap, params, body_chunk, env_id, source)
                    } else {
                        create_operative(ctx.heap, params, env_param_val, body_chunk, env_id, source)
                    };
                    stack.push(Value::Object(obj_id));
                    break; // reload (heap changed)
                }

                OP_CALL => {
                    let argc = code[ip + 1] as usize;
                    ip += 2;
                    frames.last_mut().unwrap().ip = ip;

                    let stack_start = stack.len() - argc - 1;
                    let callable = stack[stack_start];
                    let args: Vec<Value> = stack[stack_start + 1..].to_vec();
                    stack.truncate(stack_start);

                    let kind = classify_callable(ctx.heap, callable);
                    match kind {
                        1 => {
                            // Lambda: push frame
                            let cid = callable.as_object().unwrap();
                            push_lambda_frame(ctx.heap, stack, frames, cid, &args)?;
                            break; // reload for new frame
                        }
                        2 => {
                            return Err("cannot call an operative with evaluated arguments".into());
                        }
                        3 | 0 => {
                            // Native or general: send call: message
                            let sym_call = ctx.heap.intern("call:");
                            let result = dispatch::send(
                                ctx.heap,
                                ctx.type_protos,
                                ctx.invokers,
                                ctx.sym_does_not_understand,
                                callable,
                                sym_call,
                                &args,
                            )?;
                            stack.push(result);
                            break; // reload
                        }
                        _ => unreachable!(),
                    }
                }

                OP_TAIL_CALL => {
                    let argc = code[ip + 1] as usize;
                    ip += 2;
                    frames.last_mut().unwrap().ip = ip;

                    let stack_start = stack.len() - argc - 1;
                    let callable = stack[stack_start];
                    let args: Vec<Value> = stack[stack_start + 1..].to_vec();
                    stack.truncate(stack_start);

                    let kind = classify_callable(ctx.heap, callable);
                    match kind {
                        1 => {
                            // Lambda TCO: replace current frame
                            let cid = callable.as_object().unwrap();
                            tail_call_lambda(ctx.heap, stack, frames, cid, &args)?;
                            break; // reload for replaced frame
                        }
                        _ => {
                            // Non-lambda: fall back to regular call via send
                            let sym_call = ctx.heap.intern("call:");
                            let result = dispatch::send(
                                ctx.heap,
                                ctx.type_protos,
                                ctx.invokers,
                                ctx.sym_does_not_understand,
                                callable,
                                sym_call,
                                &args,
                            )?;
                            stack.push(result);
                            break; // reload
                        }
                    }
                }

                OP_RETURN => {
                    let result = stack.pop().unwrap_or(Value::Nil);
                    let base = frames.last().unwrap().stack_base;
                    frames.pop();
                    stack.truncate(base);
                    if frames.len() <= base_depth {
                        return Ok(result);
                    }
                    stack.push(result);
                    break; // reload for caller's frame
                }

                OP_JUMP => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    ip = ip + 3 + offset;
                }

                OP_JUMP_IF_FALSE => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    ip += 3;
                    let cond = stack.pop().ok_or("JUMP_IF_FALSE: empty stack")?;
                    if !cond.is_truthy() {
                        ip = ip + offset;
                    }
                }

                OP_LOOP_BACK => {
                    let distance = read_u16(&code, ip + 1) as usize;
                    ip = (ip + 3).wrapping_sub(distance);
                }

                OP_CALL_OPERATIVE => {
                    let argc = code[ip + 1] as usize;
                    ip += 2;
                    frames.last_mut().unwrap().ip = ip;

                    let stack_start = stack.len() - argc - 1;
                    let operative = stack[stack_start];
                    let args: Vec<Value> = stack[stack_start + 1..].to_vec();
                    stack.truncate(stack_start);

                    let args_list = ctx.heap.list(&args);

                    let kind = classify_callable(ctx.heap, operative);
                    if kind == 2 {
                        let oid = operative.as_object().unwrap();
                        push_operative_frame(
                            ctx.heap, stack, frames, oid, args_list, env_id,
                        )?;
                        break; // reload
                    } else {
                        return Err("CALL_OPERATIVE: target is not an operative".into());
                    }
                }

                OP_APPLY => {
                    ip += 1;
                    frames.last_mut().unwrap().ip = ip;

                    let args_list = stack.pop().ok_or("APPLY: empty stack")?;
                    let callable = stack.pop().ok_or("APPLY: empty stack")?;

                    let kind = classify_callable(ctx.heap, callable);
                    match kind {
                        2 => {
                            // Operative: pass raw args + caller env
                            let oid = callable.as_object().unwrap();
                            push_operative_frame(
                                ctx.heap, stack, frames, oid, args_list, env_id,
                            )?;
                            break; // reload
                        }
                        1 => {
                            // Lambda: eval each arg, then push frame
                            let raw_args = ctx.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                let val = eval_value(ctx, stack, frames, arg, env_id)?;
                                evaled.push(val);
                            }
                            let cid = callable.as_object().unwrap();
                            push_lambda_frame(ctx.heap, stack, frames, cid, &evaled)?;
                            break; // reload
                        }
                        _ => {
                            // General: eval args, send call:
                            let raw_args = ctx.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                let val = eval_value(ctx, stack, frames, arg, env_id)?;
                                evaled.push(val);
                            }
                            let sym_call = ctx.heap.intern("call:");
                            let result = dispatch::send(
                                ctx.heap,
                                ctx.type_protos,
                                ctx.invokers,
                                ctx.sym_does_not_understand,
                                callable,
                                sym_call,
                                &evaled,
                            )?;
                            stack.push(result);
                            break; // reload
                        }
                    }
                }

                OP_TAIL_APPLY => {
                    ip += 1;
                    frames.last_mut().unwrap().ip = ip;

                    let args_list = stack.pop().ok_or("TAIL_APPLY: empty stack")?;
                    let callable = stack.pop().ok_or("TAIL_APPLY: empty stack")?;

                    let kind = classify_callable(ctx.heap, callable);
                    match kind {
                        2 => {
                            // Operative: can't TCO operatives easily, just push frame
                            let oid = callable.as_object().unwrap();
                            push_operative_frame(
                                ctx.heap, stack, frames, oid, args_list, env_id,
                            )?;
                            break;
                        }
                        1 => {
                            // Lambda TCO: eval args, replace frame
                            let raw_args = ctx.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                let val = eval_value(ctx, stack, frames, arg, env_id)?;
                                evaled.push(val);
                            }
                            let cid = callable.as_object().unwrap();
                            tail_call_lambda(ctx.heap, stack, frames, cid, &evaled)?;
                            break;
                        }
                        _ => {
                            // General: eval args, send call:
                            let raw_args = ctx.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                let val = eval_value(ctx, stack, frames, arg, env_id)?;
                                evaled.push(val);
                            }
                            let sym_call = ctx.heap.intern("call:");
                            let result = dispatch::send(
                                ctx.heap,
                                ctx.type_protos,
                                ctx.invokers,
                                ctx.sym_does_not_understand,
                                callable,
                                sym_call,
                                &evaled,
                            )?;
                            stack.push(result);
                            break;
                        }
                    }
                }

                OP_EVAL => {
                    ip += 1;
                    frames.last_mut().unwrap().ip = ip;

                    let expr = stack.pop().ok_or("EVAL: empty stack")?;
                    let result = eval_value(ctx, stack, frames, expr, env_id)?;
                    stack.push(result);
                    break; // reload (compile + execute mutates heap)
                }

                OP_CAR => {
                    ip += 1;
                    let val = stack.pop().ok_or("CAR: empty stack")?;
                    stack.push(ctx.heap.car(val));
                }

                OP_CDR => {
                    ip += 1;
                    let val = stack.pop().ok_or("CDR: empty stack")?;
                    stack.push(ctx.heap.cdr(val));
                }

                OP_APPEND => {
                    ip += 1;
                    let b = stack.pop().ok_or("APPEND: empty stack")?;
                    let a = stack.pop().ok_or("APPEND: empty stack")?;
                    let a_elems = ctx.heap.list_to_vec(a);
                    let mut result = b;
                    for &elem in a_elems.iter().rev() {
                        result = ctx.heap.cons(elem, result);
                    }
                    stack.push(result);
                }

                OP_MAKE_OBJECT => {
                    let slot_count = code[ip + 1] as usize;
                    ip += 2;
                    frames.last_mut().unwrap().ip = ip;

                    let mut slots = Vec::new();
                    for _ in 0..slot_count {
                        let val = stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key = stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key_sym = key.as_symbol()
                            .ok_or("MAKE_OBJECT: slot key must be symbol")?;
                        slots.push((key_sym, val));
                    }
                    slots.reverse();
                    let parent = stack.pop().ok_or("MAKE_OBJECT: empty stack")?;

                    // Clone default slots from parent, explicit slots override
                    let mut final_slots = Vec::new();
                    if let Value::Object(parent_id) = parent {
                        if let HeapObject::Object { slots: parent_slots, .. } = ctx.heap.get(parent_id).clone() {
                            for (key, val) in &parent_slots {
                                if !slots.iter().any(|(k, _)| k == key) {
                                    final_slots.push((*key, *val));
                                }
                            }
                        }
                    }
                    final_slots.extend(slots);

                    let obj_id = ctx.heap.alloc(HeapObject::Object {
                        parent,
                        slots: final_slots,
                        handlers: Vec::new(),
                    });
                    stack.push(Value::Object(obj_id));
                    break; // reload (heap changed)
                }

                OP_HANDLE => {
                    ip += 1;
                    frames.last_mut().unwrap().ip = ip;

                    let handler = stack.pop().ok_or("HANDLE: empty stack")?;
                    let selector = stack.pop().ok_or("HANDLE: empty stack")?;
                    let obj_val = stack.pop().ok_or("HANDLE: empty stack")?;
                    let sel_sym = selector.as_symbol()
                        .ok_or("HANDLE: selector must be symbol")?;
                    let obj_id = obj_val.as_object()
                        .ok_or("HANDLE: expected object")?;
                    ctx.heap.add_handler(obj_id, sel_sym, handler);
                    stack.push(obj_val);
                }

                OP_SLOT_GET => {
                    ip += 1;
                    let field = stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let obj_val = stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let sym_id = field.as_symbol()
                        .ok_or("SLOT_GET: field must be symbol")?;
                    let result = match obj_val {
                        Value::Object(id) => ctx.heap.slot_get(id, sym_id),
                        _ => return Err(format!("SLOT_GET: cannot access field on {:?}", obj_val)),
                    };
                    stack.push(result);
                }

                OP_SLOT_SET => {
                    ip += 1;
                    let val = stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let field = stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let obj_val = stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let sym_id = field.as_symbol()
                        .ok_or("SLOT_SET: field must be symbol")?;
                    match obj_val {
                        Value::Object(id) => {
                            ctx.heap.slot_set(id, sym_id, val);
                            stack.push(val);
                        }
                        _ => return Err("SLOT_SET: expected object".into()),
                    }
                }

                _ => {
                    return Err(format!("unknown opcode: 0x{:02x} at ip={}", op, ip));
                }
            }

            // Update ip in the frame (for opcodes that don't break out)
            if let Some(frame) = frames.last_mut() {
                frame.ip = ip;
            }
        }
    }
}

/// Evaluate a single expression by compiling + running it.
/// Used for OP_EVAL and for evaluating args in OP_APPLY.
fn eval_value(
    ctx: &mut InvokeContext,
    _stack: &mut Vec<Value>,
    _frames: &mut Vec<CallFrame>,
    expr: Value,
    env_id: u32,
) -> Result<Value, String> {
    // Compile the expression
    let mut compiler = Compiler::new();
    let chunk = compiler.compile_expr(ctx.heap, expr)?;
    let chunk_id = chunk.store_in(ctx.heap);

    // Run in a fresh local state (recursive on the Rust stack)
    let mut eval_stack = Vec::new();
    let mut eval_frames = vec![CallFrame {
        chunk_id,
        ip: 0,
        env_id,
        stack_base: 0,
    }];

    run(ctx, &mut eval_stack, &mut eval_frames)
}
