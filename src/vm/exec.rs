/// The MOOF VM execution engine.
///
/// Stack-based bytecode interpreter. "send" is the single privileged operation (§2).
/// The bytecode is the truth layer (§9.2).

use crate::runtime::value::{Value, HeapObject, BytecodeChunk};
use crate::runtime::heap::Heap;
use super::opcodes::*;

/// A call frame on the VM's call stack.
#[derive(Debug)]
struct CallFrame {
    /// The bytecode chunk being executed (heap id)
    chunk_id: u32,
    /// Instruction pointer into the chunk's code
    ip: usize,
    /// The environment for this frame (heap id)
    env_id: u32,
    /// Stack base — where this frame's locals start on the value stack
    stack_base: usize,
}

/// The MOOF virtual machine.
pub struct VM {
    pub heap: Heap,
    /// The value stack
    stack: Vec<Value>,
    /// The call stack
    frames: Vec<CallFrame>,
    /// Well-known symbols (cached for fast dispatch)
    pub sym_call: u32,
    pub sym_parent: u32,
    pub sym_does_not_understand: u32,
    /// The root environment (set after bootstrap)
    pub root_env: Option<u32>,
}

/// Result of VM execution.
pub type VMResult = Result<Value, String>;

impl VM {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let sym_call = heap.intern("call:");
        let sym_parent = heap.intern("parent");
        let sym_dnu = heap.intern("doesNotUnderstand:");

        VM {
            heap,
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            sym_call,
            sym_parent,
            sym_does_not_understand: sym_dnu,
            root_env: None,
        }
    }

    /// Execute a bytecode chunk in a given environment. Returns the final value.
    pub fn execute(&mut self, chunk_id: u32, env_id: u32) -> VMResult {
        let frame_depth = self.frames.len();
        self.frames.push(CallFrame {
            chunk_id,
            ip: 0,
            env_id,
            stack_base: self.stack.len(),
        });

        self.run(frame_depth)
    }

    /// The main execution loop. Runs until we drop back to `base_depth` frames.
    fn run(&mut self, base_depth: usize) -> VMResult {
        loop {
            if self.frames.len() <= base_depth {
                return Ok(self.stack.pop().unwrap_or(Value::Nil));
            }

            let frame = self.frames.last().unwrap();
            let chunk_id = frame.chunk_id;
            let env_id = frame.env_id;

            // Read the current bytecode chunk
            let (code, constants) = match self.heap.get(chunk_id) {
                HeapObject::BytecodeChunk(chunk) => {
                    (chunk.code.clone(), chunk.constants.clone())
                }
                _ => return Err("Not a bytecode chunk".into()),
            };

            let ip = self.frames.last().unwrap().ip;
            if ip >= code.len() {
                // End of chunk — return top of stack or nil
                let result = self.stack.pop().unwrap_or(Value::Nil);
                let base = self.frames.last().unwrap().stack_base;
                self.frames.pop();
                self.stack.truncate(base);
                self.stack.push(result);
                continue;
            }

            let op = code[ip];
            // Advance ip
            self.frames.last_mut().unwrap().ip = ip + 1;

            match op {
                OP_CONST => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3;
                    self.stack.push(constants[idx]);
                }

                OP_NIL => self.stack.push(Value::Nil),
                OP_TRUE => self.stack.push(Value::True),
                OP_FALSE => self.stack.push(Value::False),

                OP_POP => { self.stack.pop(); }

                OP_LOOKUP => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("LOOKUP: expected symbol in constants".into()),
                    };
                    let val = self.env_lookup(env_id, sym)?;
                    self.stack.push(val);
                }

                OP_DEF => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("DEF: expected symbol in constants".into()),
                    };
                    let val = self.stack.pop().ok_or("DEF: empty stack")?;
                    self.env_define(env_id, sym, val);
                    self.stack.push(val); // def returns the value
                }

                OP_GET_ENV => {
                    self.stack.push(Value::Object(env_id));
                }

                OP_SEND => {
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 4;
                    let selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("SEND: expected symbol selector".into()),
                    };
                    // Stack: [receiver, arg1, ..., argN]
                    let stack_start = self.stack.len() - argc - 1;
                    let receiver = self.stack[stack_start];
                    let args: Vec<Value> = self.stack[stack_start + 1..].to_vec();
                    self.stack.truncate(stack_start);
                    let result = self.message_send(receiver, selector, &args)?;
                    self.stack.push(result);
                }

                OP_CONS => {
                    let cdr = self.stack.pop().ok_or("CONS: empty stack")?;
                    let car = self.stack.pop().ok_or("CONS: empty stack")?;
                    let val = self.heap.cons(car, cdr);
                    self.stack.push(val);
                }

                OP_EQ => {
                    let b = self.stack.pop().ok_or("EQ: empty stack")?;
                    let a = self.stack.pop().ok_or("EQ: empty stack")?;
                    self.stack.push(if a == b { Value::True } else { Value::False });
                }

                OP_QUOTE => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3;
                    self.stack.push(constants[idx]);
                }

                OP_VAU => {
                    let params_idx = read_u16(&code, ip + 1) as usize;
                    let env_param_idx = read_u16(&code, ip + 3) as usize;
                    let body_idx = read_u16(&code, ip + 5) as usize;
                    let source_idx = read_u16(&code, ip + 7) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 9;

                    let params = constants[params_idx];
                    let env_param_sym = match constants[env_param_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("VAU: expected symbol for env param".into()),
                    };
                    let body_chunk = match constants[body_idx] {
                        Value::Object(id) => id,
                        _ => return Err("VAU: expected object for body chunk".into()),
                    };
                    let source = constants[source_idx];

                    // Convention: $_ means "lambda" (wrapped operative that evals args)
                    let name = self.heap.symbol_name(env_param_sym).to_string();
                    let obj = if name == "$_" || name == "$_block_env" {
                        HeapObject::Lambda {
                            params,
                            body: body_chunk,
                            def_env: env_id,
                            source,
                        }
                    } else {
                        HeapObject::Operative {
                            params,
                            env_param: env_param_sym,
                            body: body_chunk,
                            def_env: env_id,
                            source,
                        }
                    };
                    let id = self.heap.alloc(obj);
                    self.stack.push(Value::Object(id));
                }

                OP_CALL => {
                    let argc = code[ip + 1] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.stack.len() - argc - 1;
                    let callable = self.stack[stack_start];
                    let args: Vec<Value> = self.stack[stack_start + 1..].to_vec();
                    self.stack.truncate(stack_start);
                    let result = self.call_value(callable, &args)?;
                    self.stack.push(result);
                }

                OP_RETURN => {
                    let result = self.stack.pop().unwrap_or(Value::Nil);
                    let base = self.frames.last().unwrap().stack_base;
                    self.frames.pop();
                    self.stack.truncate(base);
                    self.stack.push(result);
                    if self.frames.is_empty() {
                        return Ok(result);
                    }
                }

                OP_JUMP => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3 + offset;
                }

                OP_JUMP_IF_FALSE => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    self.frames.last_mut().unwrap().ip = ip + 3;
                    let cond = self.stack.pop().ok_or("JUMP_IF_FALSE: empty stack")?;
                    if !cond.is_truthy() {
                        self.frames.last_mut().unwrap().ip = ip + 3 + offset;
                    }
                }

                OP_LOOP_BACK => {
                    let distance = read_u16(&code, ip + 1) as usize;
                    // Jump backwards: ip is currently at ip+1 (after reading opcode)
                    // We want to go to (ip + 3) - distance
                    let target = (ip + 3).wrapping_sub(distance);
                    self.frames.last_mut().unwrap().ip = target;
                }

                OP_CALL_OPERATIVE => {
                    let argc = code[ip + 1] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.stack.len() - argc - 1;
                    let operative = self.stack[stack_start];
                    let args: Vec<Value> = self.stack[stack_start + 1..].to_vec();
                    self.stack.truncate(stack_start);

                    // Build the args as a cons list (unevaluated)
                    let args_list = self.heap.list(&args);
                    let result = self.call_operative(operative, args_list, env_id)?;
                    self.stack.push(result);
                }

                OP_APPLY => {
                    // Generic apply: runtime check for operative vs applicative
                    // Stack: [callable, quoted_args_list]
                    let args_list = self.stack.pop().ok_or("APPLY: empty stack")?;
                    let callable = self.stack.pop().ok_or("APPLY: empty stack")?;

                    let result = match callable {
                        Value::Object(id) => {
                            match self.heap.get(id).clone() {
                                HeapObject::Operative { .. } => {
                                    // Operative: pass raw args + caller env
                                    self.call_operative(callable, args_list, env_id)?
                                }
                                HeapObject::Lambda { params, body, def_env, .. } => {
                                    // Lambda: eval each arg in caller env, then call
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    self.call_lambda(params, body, def_env, &evaled)?
                                }
                                HeapObject::Block { params, body, def_env, .. } => {
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    self.call_lambda(params, body, def_env, &evaled)?
                                }
                                _ => {
                                    // General object: eval args, then send call:
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    self.message_send(callable, self.sym_call, &evaled)?
                                }
                            }
                        }
                        _ => return Err(format!("Cannot apply {:?}", callable)),
                    };
                    self.stack.push(result);
                }

                OP_EVAL => {
                    let expr = self.stack.pop().ok_or("EVAL: empty stack")?;
                    let result = self.eval(expr, env_id)?;
                    self.stack.push(result);
                }

                OP_PRINT => {
                    let val = self.stack.pop().ok_or("PRINT: empty stack")?;
                    let s = self.format_value(val);
                    println!("{}", s);
                    self.stack.push(Value::Nil);
                }

                OP_CAR => {
                    let val = self.stack.pop().ok_or("CAR: empty stack")?;
                    self.stack.push(self.heap.car(val));
                }

                OP_CDR => {
                    let val = self.stack.pop().ok_or("CDR: empty stack")?;
                    self.stack.push(self.heap.cdr(val));
                }

                OP_MAKE_OBJECT => {
                    let slot_count = code[ip + 1] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 2;
                    // Stack: [parent, key1, val1, key2, val2, ...]
                    let mut slots = Vec::new();
                    for _ in 0..slot_count {
                        let val = self.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key = self.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key_sym = key.as_symbol().ok_or("MAKE_OBJECT: slot key must be symbol")?;
                        slots.push((key_sym, val));
                    }
                    slots.reverse(); // restore original order
                    let parent = self.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                    let obj = HeapObject::GeneralObject {
                        parent,
                        slots,
                        handlers: Vec::new(),
                    };
                    let id = self.heap.alloc(obj);
                    self.stack.push(Value::Object(id));
                }

                OP_HANDLE => {
                    let handler = self.stack.pop().ok_or("HANDLE: empty stack")?;
                    let selector = self.stack.pop().ok_or("HANDLE: empty stack")?;
                    let obj_val = self.stack.pop().ok_or("HANDLE: empty stack")?;
                    let sel_sym = selector.as_symbol().ok_or("HANDLE: selector must be symbol")?;
                    let obj_id = obj_val.as_object().ok_or("HANDLE: expected object")?;
                    match self.heap.get_mut(obj_id) {
                        HeapObject::GeneralObject { handlers, .. } => {
                            // Replace existing handler or add new one
                            if let Some(entry) = handlers.iter_mut().find(|(k, _)| *k == sel_sym) {
                                entry.1 = handler;
                            } else {
                                handlers.push((sel_sym, handler));
                            }
                        }
                        _ => return Err("HANDLE: target must be a GeneralObject".into()),
                    }
                    self.stack.push(obj_val); // return the object
                }

                OP_TYPE_OF => {
                    let val = self.stack.pop().ok_or("TYPE_OF: empty stack")?;
                    let type_name = match val {
                        Value::Nil => "Nil",
                        Value::True | Value::False => "Boolean",
                        Value::Integer(_) => "Integer",
                        Value::Symbol(_) => "Symbol",
                        Value::Object(id) => match self.heap.get(id) {
                            HeapObject::Cons { .. } => "Cons",
                            HeapObject::MoofString(_) => "String",
                            HeapObject::GeneralObject { .. } => "Object",
                            HeapObject::BytecodeChunk(_) => "Bytecode",
                            HeapObject::Operative { .. } => "Operative",
                            HeapObject::Lambda { .. } => "Lambda",
                            HeapObject::Environment(_) => "Environment",
                            HeapObject::Block { .. } => "Block",
                        },
                    };
                    let sym = self.heap.intern(type_name);
                    self.stack.push(Value::Symbol(sym));
                }

                OP_LOAD => {
                    let path_val = self.stack.pop().ok_or("LOAD: empty stack")?;
                    let path = match path_val {
                        Value::Object(id) => match self.heap.get(id).clone() {
                            HeapObject::MoofString(s) => s,
                            _ => return Err("load: expected string path".into()),
                        },
                        _ => return Err("load: expected string path".into()),
                    };
                    let load_env = self.root_env.unwrap_or(env_id);
                    let source = std::fs::read_to_string(&path)
                        .map_err(|e| format!("load: cannot read {}: {}", path, e))?;
                    // Use the public eval_source from main
                    let result = crate::eval_source(self, load_env, &source, &path)?;
                    self.stack.push(result);
                }

                OP_SOURCE => {
                    let val = self.stack.pop().ok_or("SOURCE: empty stack")?;
                    let source = match val {
                        Value::Object(id) => match self.heap.get(id) {
                            HeapObject::Lambda { source, .. } => *source,
                            HeapObject::Operative { source, .. } => *source,
                            HeapObject::Block { source, .. } => *source,
                            _ => Value::Nil,
                        },
                        _ => Value::Nil,
                    };
                    self.stack.push(source);
                }

                _ => return Err(format!("Unknown opcode: 0x{:02x}", op)),
            }
        }
    }

    /// Look up a symbol in the environment chain.
    fn env_lookup(&self, env_id: u32, sym: u32) -> VMResult {
        let mut current = Some(env_id);
        while let Some(eid) = current {
            match self.heap.get(eid) {
                HeapObject::Environment(env) => {
                    if let Some(val) = env.lookup_local(sym) {
                        return Ok(val);
                    }
                    current = env.parent;
                }
                _ => return Err("env_lookup: not an environment".into()),
            }
        }
        let name = self.heap.symbol_name(sym);
        Err(format!("Unbound symbol: {}", name))
    }

    /// Define a binding in an environment.
    fn env_define(&mut self, env_id: u32, sym: u32, val: Value) {
        match self.heap.get_mut(env_id) {
            HeapObject::Environment(env) => {
                env.define(sym, val);
            }
            _ => panic!("env_define: not an environment"),
        }
    }

    /// Call a value as a function: [callable call: args...]
    fn call_value(&mut self, callable: Value, args: &[Value]) -> VMResult {
        match callable {
            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Lambda { params, body, def_env, .. } => {
                        self.call_lambda(params, body, def_env, args)
                    }
                    HeapObject::Block { params, body, def_env, .. } => {
                        self.call_lambda(params, body, def_env, args)
                    }
                    HeapObject::Operative { .. } => {
                        Err("Cannot call an operative with evaluated arguments — use vau syntax".into())
                    }
                    _ => {
                        // Try message send: [callable call: args...]
                        self.message_send(callable, self.sym_call, args)
                    }
                }
            }
            _ => Err(format!("Cannot call {:?}", callable)),
        }
    }

    /// Call a lambda/block: create a new environment, bind params, execute body.
    fn call_lambda(&mut self, params: Value, body: u32, def_env: u32, args: &[Value]) -> VMResult {
        let new_env_id = self.heap.alloc_env(Some(def_env));
        // Bind parameters
        let param_syms = self.heap.list_to_vec(params);
        for (i, &p) in param_syms.iter().enumerate() {
            if let Value::Symbol(sym) = p {
                let val = args.get(i).copied().unwrap_or(Value::Nil);
                self.env_define(new_env_id, sym, val);
            }
        }
        self.execute(body, new_env_id)
    }

    /// Call an operative with unevaluated args and the caller's environment.
    fn call_operative(&mut self, operative: Value, args_list: Value, caller_env: u32) -> VMResult {
        match operative {
            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Operative { params, env_param, body, def_env, .. } => {
                        let new_env_id = self.heap.alloc_env(Some(def_env));
                        // Bind the parameter list to the unevaluated args
                        self.bind_params(new_env_id, params, args_list);
                        // Bind the environment parameter to the caller's env
                        self.env_define(new_env_id, env_param, Value::Object(caller_env));
                        self.execute(body, new_env_id)
                    }
                    _ => Err("call_operative: not an operative".into()),
                }
            }
            _ => Err(format!("call_operative: expected object, got {:?}", operative)),
        }
    }

    /// Bind parameters to arguments (destructuring cons lists).
    fn bind_params(&mut self, env_id: u32, params: Value, args: Value) {
        match params {
            Value::Symbol(sym) => {
                // Rest parameter — bind the whole list
                self.env_define(env_id, sym, args);
            }
            Value::Nil => {} // no params
            Value::Object(pid) => {
                match self.heap.get(pid).clone() {
                    HeapObject::Cons { car, cdr } => {
                        let arg_car = self.heap.car(args);
                        let arg_cdr = self.heap.cdr(args);
                        self.bind_params(env_id, car, arg_car);
                        self.bind_params(env_id, cdr, arg_cdr);
                    }
                    _ => {} // ignore non-cons
                }
            }
            _ => {} // ignore non-bindable
        }
    }

    /// The core message send: look up a handler and invoke it.
    /// "the vm's single privileged operation is `send`" (§0)
    pub fn message_send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> VMResult {
        // For general objects, check user-defined handlers FIRST (§4.2)
        if let Value::Object(id) = receiver {
            if let Some(handler) = self.lookup_handler(id, selector) {
                let mut full_args = vec![receiver];
                full_args.extend_from_slice(args);
                return self.call_value(handler, &full_args);
            }
        }

        // Then try built-in handlers for primitive types
        if let Some(result) = self.primitive_send(receiver, selector, args)? {
            return Ok(result);
        }

        // doesNotUnderstand: — fire if the object has a handler for it (§4.2)
        if selector != self.sym_does_not_understand {
            if let Value::Object(id) = receiver {
                if let Some(dnu_handler) = self.lookup_handler(id, self.sym_does_not_understand) {
                    let sel_sym = Value::Symbol(selector);
                    let args_list = self.heap.list(args);
                    let full_args = vec![receiver, sel_sym, args_list];
                    return self.call_value(dnu_handler, &full_args);
                }
            }
        }

        let sel_name = self.heap.symbol_name(selector).to_string();
        Err(format!("doesNotUnderstand: {} on {:?}", sel_name, receiver))
    }

    /// Look up a handler in the delegation chain (§4.2).
    fn lookup_handler(&self, obj_id: u32, selector: u32) -> Option<Value> {
        let mut current = Some(obj_id);
        while let Some(id) = current {
            match self.heap.get(id) {
                HeapObject::GeneralObject { parent, handlers, .. } => {
                    for &(sel, handler) in handlers {
                        if sel == selector {
                            return Some(handler);
                        }
                    }
                    // Delegate to parent
                    match parent {
                        Value::Object(pid) => current = Some(*pid),
                        _ => current = None,
                    }
                }
                _ => return None,
            }
        }
        None
    }

    /// Built-in message handlers for primitive types.
    /// These are the "fast paths" mentioned in §9.2 — semantically it's all
    /// message dispatch, but the VM handles these directly.
    fn primitive_send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> Result<Option<Value>, String> {
        let sel_name = self.heap.symbol_name(selector);

        match receiver {
            Value::Integer(a) => {
                match sel_name {
                    "+" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("+ expects integer argument")?;
                        Ok(Some(Value::Integer(a + b)))
                    }
                    "-" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("- expects integer argument")?;
                        Ok(Some(Value::Integer(a - b)))
                    }
                    "*" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("* expects integer argument")?;
                        Ok(Some(Value::Integer(a * b)))
                    }
                    "/" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("/ expects integer argument")?;
                        if b == 0 { return Err("Division by zero".into()); }
                        Ok(Some(Value::Integer(a / b)))
                    }
                    "%" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("% expects integer argument")?;
                        if b == 0 { return Err("Modulo by zero".into()); }
                        Ok(Some(Value::Integer(a % b)))
                    }
                    "<" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("< expects integer argument")?;
                        Ok(Some(if a < b { Value::True } else { Value::False }))
                    }
                    ">" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("> expects integer argument")?;
                        Ok(Some(if a > b { Value::True } else { Value::False }))
                    }
                    "=" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("= expects integer argument")?;
                        Ok(Some(if a == b { Value::True } else { Value::False }))
                    }
                    "<=" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or("<= expects integer argument")?;
                        Ok(Some(if a <= b { Value::True } else { Value::False }))
                    }
                    ">=" => {
                        let b = args.first().and_then(|v| v.as_integer())
                            .ok_or(">= expects integer argument")?;
                        Ok(Some(if a >= b { Value::True } else { Value::False }))
                    }
                    "negate" => Ok(Some(Value::Integer(-a))),
                    "abs" => Ok(Some(Value::Integer(a.abs()))),
                    "describe" => {
                        let s = self.heap.alloc_string(&format!("{}", a));
                        Ok(Some(s))
                    }
                    _ => Ok(None),
                }
            }

            Value::True => {
                match sel_name {
                    "ifTrue:ifFalse:" | "ifTrue:" => {
                        let block = args.first().copied().ok_or("ifTrue: expects a block")?;
                        self.call_value(block, &[]).map(Some)
                    }
                    "not" => Ok(Some(Value::False)),
                    "and:" => {
                        // true and: block → evaluate block
                        let block = args.first().copied().ok_or("and: expects a block")?;
                        self.call_value(block, &[]).map(Some)
                    }
                    "or:" => Ok(Some(Value::True)),
                    "describe" => Ok(Some(self.heap.alloc_string("true"))),
                    _ => Ok(None),
                }
            }

            Value::False => {
                match sel_name {
                    "ifTrue:ifFalse:" => {
                        let block = args.get(1).copied().ok_or("ifFalse: expects a block")?;
                        self.call_value(block, &[]).map(Some)
                    }
                    "ifTrue:" => Ok(Some(Value::Nil)),
                    "ifFalse:" => {
                        let block = args.first().copied().ok_or("ifFalse: expects a block")?;
                        self.call_value(block, &[]).map(Some)
                    }
                    "not" => Ok(Some(Value::True)),
                    "and:" => Ok(Some(Value::False)),
                    "or:" => {
                        let block = args.first().copied().ok_or("or: expects a block")?;
                        self.call_value(block, &[]).map(Some)
                    }
                    "describe" => Ok(Some(self.heap.alloc_string("false"))),
                    _ => Ok(None),
                }
            }

            Value::Nil => {
                match sel_name {
                    "describe" => Ok(Some(self.heap.alloc_string("nil"))),
                    "isNil" => Ok(Some(Value::True)),
                    _ => Ok(None),
                }
            }

            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Cons { car, cdr } => {
                        match sel_name {
                            "car" => Ok(Some(car)),
                            "cdr" => Ok(Some(cdr)),
                            "describe" => {
                                let s = self.format_value(Value::Object(id));
                                Ok(Some(self.heap.alloc_string(&s)))
                            }
                            _ => Ok(None),
                        }
                    }
                    HeapObject::MoofString(ref s) => {
                        match sel_name {
                            "describe" => Ok(Some(Value::Object(id))),
                            "length" => Ok(Some(Value::Integer(s.len() as i64))),
                            "++" => {
                                // String concatenation
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(other) = self.heap.get(*other_id) {
                                        let new_s = format!("{}{}", s, other);
                                        return Ok(Some(self.heap.alloc_string(&new_s)));
                                    }
                                }
                                Ok(None)
                            }
                            _ => Ok(None),
                        }
                    }
                    HeapObject::GeneralObject { ref slots, .. } => {
                        match sel_name {
                            "slotAt:" => {
                                let key = args.first()
                                    .and_then(|v| v.as_symbol())
                                    .ok_or("slotAt: expects a symbol")?;
                                let val = slots.iter()
                                    .find(|(k, _)| *k == key)
                                    .map(|(_, v)| *v)
                                    .unwrap_or(Value::Nil);
                                Ok(Some(val))
                            }
                            "slotAt:put:" => {
                                let key = args.first()
                                    .and_then(|v| v.as_symbol())
                                    .ok_or("slotAt:put: expects a symbol key")?;
                                let val = args.get(1).copied().unwrap_or(Value::Nil);
                                let _ = slots;
                                match self.heap.get_mut(id) {
                                    HeapObject::GeneralObject { slots, .. } => {
                                        if let Some(entry) = slots.iter_mut().find(|(k, _)| *k == key) {
                                            entry.1 = val;
                                        } else {
                                            slots.push((key, val));
                                        }
                                    }
                                    _ => unreachable!(),
                                }
                                Ok(Some(val))
                            }
                            "slotNames" => {
                                let names: Vec<Value> = slots.iter()
                                    .map(|(k, _)| Value::Symbol(*k))
                                    .collect();
                                let list = self.heap.list(&names);
                                Ok(Some(list))
                            }
                            "handlerNames" => {
                                // Need to get handlers from the actual object
                                drop(slots);
                                match self.heap.get(id) {
                                    HeapObject::GeneralObject { handlers, .. } => {
                                        let names: Vec<Value> = handlers.iter()
                                            .map(|(k, _)| Value::Symbol(*k))
                                            .collect();
                                        Ok(Some(self.heap.list(&names)))
                                    }
                                    _ => Ok(Some(Value::Nil)),
                                }
                            }
                            "handlerAt:" => {
                                let key = args.first()
                                    .and_then(|v| v.as_symbol())
                                    .ok_or("handlerAt: expects a symbol")?;
                                drop(slots);
                                match self.heap.get(id) {
                                    HeapObject::GeneralObject { handlers, .. } => {
                                        let handler = handlers.iter()
                                            .find(|(k, _)| *k == key)
                                            .map(|(_, v)| *v)
                                            .unwrap_or(Value::Nil);
                                        Ok(Some(handler))
                                    }
                                    _ => Ok(Some(Value::Nil)),
                                }
                            }
                            "parent" => {
                                drop(slots);
                                match self.heap.get(id) {
                                    HeapObject::GeneralObject { parent, .. } => Ok(Some(*parent)),
                                    _ => Ok(Some(Value::Nil)),
                                }
                            }
                            "describe" => {
                                Ok(Some(self.heap.alloc_string(&format!("<object #{}>", id))))
                            }
                            _ => Ok(None), // fall through to handler lookup
                        }
                    }
                    HeapObject::Environment(ref env) => {
                        match sel_name {
                            "eval:" => {
                                // [env eval: expr] — the reflective tower hook (§7.3)
                                let expr = args.first().copied().ok_or("eval: expects an expression")?;
                                let _ = env;
                                self.eval(expr, id).map(Some)
                            }
                            "lookup:" => {
                                let sym = args.first()
                                    .and_then(|v| v.as_symbol())
                                    .ok_or("lookup: expects a symbol")?;
                                self.env_lookup(id, sym).map(Some)
                            }
                            "describe" => Ok(Some(self.heap.alloc_string("<environment>"))),
                            _ => Ok(None),
                        }
                    }
                    _ => Ok(None),
                }
            }

            Value::Symbol(_) => {
                match sel_name {
                    "describe" => {
                        let s = self.format_value(receiver);
                        Ok(Some(self.heap.alloc_string(&s)))
                    }
                    _ => Ok(None),
                }
            }
        }
    }

    /// Evaluate an expression in an environment (used by eval:, the REPL, etc).
    /// This is the tree-walking fallback for when we need to eval AST directly.
    pub fn eval(&mut self, expr: Value, env_id: u32) -> VMResult {
        use crate::compiler::compile::Compiler;
        let mut compiler = Compiler::new();
        let chunk = compiler.compile_expr(&mut self.heap, expr)?;
        let chunk_id = self.heap.alloc_chunk(chunk);
        self.execute(chunk_id, env_id)
    }

    /// Format a value for display.
    pub fn format_value(&self, val: Value) -> String {
        match val {
            Value::Nil => "nil".to_string(),
            Value::True => "true".to_string(),
            Value::False => "false".to_string(),
            Value::Integer(n) => n.to_string(),
            Value::Symbol(id) => format!("#{}", self.heap.symbol_name(id)),
            Value::Object(id) => {
                match self.heap.get(id) {
                    HeapObject::Cons { .. } => self.format_list(val),
                    HeapObject::MoofString(s) => format!("\"{}\"", s),
                    HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
                    HeapObject::BytecodeChunk(_) => "<bytecode>".to_string(),
                    HeapObject::Operative { .. } => "<operative>".to_string(),
                    HeapObject::Lambda { .. } => "<lambda>".to_string(),
                    HeapObject::Environment(_) => "<environment>".to_string(),
                    HeapObject::Block { .. } => "<block>".to_string(),
                }
            }
        }
    }

    /// Format a cons-list for display.
    fn format_list(&self, val: Value) -> String {
        let mut parts = Vec::new();
        let mut current = val;
        loop {
            match current {
                Value::Nil => break,
                Value::Object(id) => {
                    match self.heap.get(id) {
                        HeapObject::Cons { car, cdr } => {
                            parts.push(self.format_value(*car));
                            current = *cdr;
                        }
                        _ => {
                            parts.push(format!(". {}", self.format_value(current)));
                            break;
                        }
                    }
                }
                other => {
                    parts.push(format!(". {}", self.format_value(other)));
                    break;
                }
            }
        }
        format!("({})", parts.join(" "))
    }
}
