/// The MOOF VM execution engine.
///
/// Stack-based bytecode interpreter. "send" is the single privileged operation (§2).
/// The bytecode is the truth layer (§9.2).

use crate::runtime::value::{Value, HeapObject, BytecodeChunk};
use crate::runtime::heap::Heap;
use crate::ffi::bridge::{self, NativeLibrary, FfiType};
use super::opcodes::*;
use std::collections::HashMap;

/// A native function callable from MOOF. Receives a mutable heap and args.
pub type NativeFn = Box<dyn Fn(&mut Heap, &[Value]) -> Result<Value, String>>;

/// Registry of native functions. Functions are looked up by name.
pub struct NativeRegistry {
    functions: Vec<NativeFn>,
    names: Vec<String>,
    name_to_id: HashMap<String, usize>,
}

impl NativeRegistry {
    pub fn new() -> Self {
        NativeRegistry {
            functions: Vec::new(),
            names: Vec::new(),
            name_to_id: HashMap::new(),
        }
    }

    /// Register a native function. Returns its index.
    pub fn register(&mut self, name: String, func: NativeFn) -> usize {
        if let Some(&id) = self.name_to_id.get(&name) {
            // Replace existing registration
            self.functions[id] = func;
            return id;
        }
        let id = self.functions.len();
        self.functions.push(func);
        self.names.push(name.clone());
        self.name_to_id.insert(name, id);
        id
    }

    /// Look up a native function by name.
    pub fn get(&self, name: &str) -> Option<&NativeFn> {
        self.name_to_id.get(name).map(|&id| &self.functions[id])
    }
}

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
    /// Type prototypes — maps primitive types to their prototype object ids.
    /// Set after bootstrap defines them.
    pub proto_integer: Option<u32>,
    pub proto_boolean: Option<u32>,
    pub proto_string: Option<u32>,
    pub proto_cons: Option<u32>,
    pub proto_nil: Option<u32>,
    pub proto_symbol: Option<u32>,
    pub proto_lambda: Option<u32>,
    pub proto_operative: Option<u32>,
    pub proto_environment: Option<u32>,
    /// Native extension registry — all native functions live here
    pub native_registry: NativeRegistry,
    /// FFI: open native libraries (keyed by name, kept for symbol lookup)
    pub ffi_libs: HashMap<String, NativeLibrary>,
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
            proto_integer: None,
            proto_boolean: None,
            proto_string: None,
            proto_cons: None,
            proto_nil: None,
            proto_symbol: None,
            proto_lambda: None,
            proto_operative: None,
            proto_environment: None,
            native_registry: NativeRegistry::new(),
            ffi_libs: HashMap::new(),
        }
    }

    /// Register a native function and return a Value::Object pointing to a NativeFunction.
    pub fn register_native(&mut self, name: &str, func: NativeFn) -> Value {
        self.native_registry.register(name.to_string(), func);
        let obj = HeapObject::NativeFunction { name: name.to_string() };
        Value::Object(self.heap.alloc(obj))
    }

    /// Call a native function by name, temporarily taking the registry to avoid borrow conflicts.
    fn call_native(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        let registry = std::mem::replace(&mut self.native_registry, NativeRegistry::new());
        let result = match registry.get(name) {
            Some(func) => func(&mut self.heap, args),
            None => Err(format!("Native function '{}' not found in registry", name)),
        };
        self.native_registry = registry;
        result
    }

    /// Get the type prototype for a value (if registered).
    fn type_prototype(&self, val: Value) -> Option<u32> {
        match val {
            Value::Integer(_) => self.proto_integer,
            Value::Float(_) => self.proto_integer, // floats share Integer's prototype for now
            Value::True | Value::False => self.proto_boolean,
            Value::Nil => self.proto_nil,
            Value::Symbol(_) => self.proto_symbol,
            Value::Object(id) => match self.heap.get(id) {
                HeapObject::MoofString(_) => self.proto_string,
                HeapObject::Cons { .. } => self.proto_cons,
                HeapObject::Lambda { .. } => self.proto_lambda,
                HeapObject::Operative { .. } => self.proto_operative,
                HeapObject::Environment(_) => self.proto_environment,
                _ => None,
            },
        }
    }

    /// Register native handler lambdas on a type prototype.
    /// Each handler is a real callable lambda whose body uses OP_PRIM_SEND.
    pub fn register_native_handlers(&mut self, proto_id: u32, root_env: u32, type_name: &str, selectors: &[(&str, u8)]) {
        for &(sel, argc) in selectors {
            let handler = self.make_native_lambda(root_env, type_name, sel, argc);
            let sel_sym = self.heap.intern(sel);
            self.heap.add_handler(proto_id, sel_sym, handler);
        }
    }

    /// Create a real lambda that wraps a primitive operation via OP_PRIM_SEND.
    fn make_native_lambda(&mut self, def_env: u32, type_name: &str, selector: &str, argc: u8) -> Value {
        use crate::runtime::value::BytecodeChunk;

        let mut code = Vec::new();
        let mut constants: Vec<Value> = Vec::new();

        // OP_LOOKUP self
        let self_sym = self.heap.intern("self");
        let self_idx = constants.len() as u16;
        constants.push(Value::Symbol(self_sym));
        code.push(OP_LOOKUP);
        code.push((self_idx >> 8) as u8);
        code.push((self_idx & 0xFF) as u8);

        // Build param list: (self) or (self a) or (self a b)
        let param_names = ["a", "b", "c"];
        let mut param_syms = vec![Value::Symbol(self_sym)];
        for i in 0..argc as usize {
            let arg_sym = self.heap.intern(param_names[i]);
            param_syms.push(Value::Symbol(arg_sym));
            // OP_LOOKUP arg
            let arg_idx = constants.len() as u16;
            constants.push(Value::Symbol(arg_sym));
            code.push(OP_LOOKUP);
            code.push((arg_idx >> 8) as u8);
            code.push((arg_idx & 0xFF) as u8);
        }

        // OP_PRIM_SEND selector argc
        let sel_sym = self.heap.intern(selector);
        let sel_idx = constants.len() as u16;
        constants.push(Value::Symbol(sel_sym));
        code.push(OP_PRIM_SEND);
        code.push((sel_idx >> 8) as u8);
        code.push((sel_idx & 0xFF) as u8);
        code.push(argc);

        code.push(OP_RETURN);

        let chunk = BytecodeChunk { code, constants };
        let body_id = self.heap.alloc_chunk(chunk);

        let params = self.heap.list(&param_syms);
        let source_str = format!("<native {}.{}>", type_name, selector);
        let source = self.heap.alloc_string(&source_str);

        let lambda = HeapObject::Lambda {
            params,
            body: body_id,
            def_env,
            source,
        };
        Value::Object(self.heap.alloc(lambda))
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

                OP_PRIM_SEND => {
                    // Like OP_SEND but bypasses handler lookup — goes directly to primitive_send.
                    // Used by native handler lambdas to avoid infinite recursion.
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 4;
                    let selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("PRIM_SEND: expected symbol selector".into()),
                    };
                    let stack_start = self.stack.len() - argc - 1;
                    let receiver = self.stack[stack_start];
                    let args: Vec<Value> = self.stack[stack_start + 1..].to_vec();
                    self.stack.truncate(stack_start);
                    let result = self.primitive_send(receiver, selector, &args)?
                        .ok_or_else(|| {
                            let sel_name = self.heap.symbol_name(selector).to_string();
                            format!("No primitive handler for {} on {:?}", sel_name, receiver)
                        })?;
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
                    let obj = if name == "$_" {
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
                                HeapObject::NativeFunction { name } => {
                                    let name = name.clone();
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    self.call_native(&name, &evaled)?
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

                OP_TAIL_APPLY => {
                    // Tail-call variant: replaces current frame for lambdas/blocks
                    let args_list = self.stack.pop().ok_or("TAIL_APPLY: empty stack")?;
                    let callable = self.stack.pop().ok_or("TAIL_APPLY: empty stack")?;

                    match callable {
                        Value::Object(id) => {
                            match self.heap.get(id).clone() {
                                HeapObject::Lambda { params, body, def_env, .. } => {
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    let new_env_id = self.heap.alloc_env(Some(def_env));
                                    let evaled_list = self.heap.list(&evaled);
                                    self.bind_params(new_env_id, params, evaled_list);
                                    let frame = self.frames.last_mut().unwrap();
                                    self.stack.truncate(frame.stack_base);
                                    frame.chunk_id = body;
                                    frame.ip = 0;
                                    frame.env_id = new_env_id;
                                }
                                HeapObject::Operative { .. } => {
                                    // Can't TCO operatives — fall back to regular call
                                    let result = self.call_operative(callable, args_list, env_id)?;
                                    self.stack.push(result);
                                }
                                _ => {
                                    let raw_args = self.heap.list_to_vec(args_list);
                                    let mut evaled = Vec::new();
                                    for arg in raw_args {
                                        evaled.push(self.eval(arg, env_id)?);
                                    }
                                    let result = self.message_send(callable, self.sym_call, &evaled)?;
                                    self.stack.push(result);
                                }
                            }
                        }
                        _ => return Err(format!("Cannot apply {:?}", callable)),
                    }
                }

                OP_TAIL_CALL => {
                    // Tail-call variant of OP_CALL for known-lambda contexts (let)
                    let argc = code[ip + 1] as usize;
                    self.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.stack.len() - argc - 1;
                    let callable = self.stack[stack_start];
                    let args: Vec<Value> = self.stack[stack_start + 1..].to_vec();
                    self.stack.truncate(stack_start);

                    match callable {
                        Value::Object(id) => {
                            match self.heap.get(id).clone() {
                                HeapObject::Lambda { params, body, def_env, .. } => {
                                    let new_env_id = self.heap.alloc_env(Some(def_env));
                                    let args_list = self.heap.list(&args);
                                    self.bind_params(new_env_id, params, args_list);
                                    let frame = self.frames.last_mut().unwrap();
                                    self.stack.truncate(frame.stack_base);
                                    frame.chunk_id = body;
                                    frame.ip = 0;
                                    frame.env_id = new_env_id;
                                }
                                _ => {
                                    let result = self.call_value(callable, &args)?;
                                    self.stack.push(result);
                                }
                            }
                        }
                        _ => return Err(format!("Cannot call {:?}", callable)),
                    }
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
                    self.heap.add_handler(obj_id, sel_sym, handler);
                    self.stack.push(obj_val);
                }

                OP_SLOT_GET => {
                    let field_sym = self.stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let obj_val = self.stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let sym_id = field_sym.as_symbol().ok_or("SLOT_GET: field must be symbol")?;
                    let result = match obj_val {
                        Value::Object(id) => {
                            match self.heap.get(id) {
                                HeapObject::GeneralObject { slots, .. } => {
                                    slots.iter()
                                        .find(|(k, _)| *k == sym_id)
                                        .map(|(_, v)| *v)
                                        .unwrap_or(Value::Nil)
                                }
                                _ => return Err("SLOT_GET: not an object with slots".into()),
                            }
                        }
                        _ => return Err(format!("SLOT_GET: cannot access field on {:?}", obj_val)),
                    };
                    self.stack.push(result);
                }

                OP_SLOT_SET => {
                    let val = self.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let field_sym = self.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let obj_val = self.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let sym_id = field_sym.as_symbol().ok_or("SLOT_SET: field must be symbol")?;
                    let obj_id = obj_val.as_object().ok_or("SLOT_SET: expected object")?;
                    self.heap.set_slot(obj_id, sym_id, val);
                    self.stack.push(val);
                }

                OP_TYPE_OF => {
                    let val = self.stack.pop().ok_or("TYPE_OF: empty stack")?;
                    let type_name = match val {
                        Value::Nil => "Nil",
                        Value::True | Value::False => "Boolean",
                        Value::Integer(_) => "Integer",
                        Value::Float(_) => "Float",
                        Value::Symbol(_) => "Symbol",
                        Value::Object(id) => match self.heap.get(id) {
                            HeapObject::Cons { .. } => "Cons",
                            HeapObject::MoofString(_) => "String",
                            HeapObject::GeneralObject { .. } => "Object",
                            HeapObject::BytecodeChunk(_) => "Bytecode",
                            HeapObject::Operative { .. } => "Operative",
                            HeapObject::Lambda { .. } => "Lambda",
                            HeapObject::Environment(_) => "Environment",
                            HeapObject::NativeFunction { .. } => "NativeFunction",
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
                            _ => Value::Nil,
                        },
                        _ => Value::Nil,
                    };
                    self.stack.push(source);
                }

                OP_FFI_OPEN => {
                    let name_val = self.stack.pop().ok_or("FFI_OPEN: empty stack")?;
                    let name = match name_val {
                        Value::Object(id) => match self.heap.get(id).clone() {
                            HeapObject::MoofString(s) => s,
                            _ => return Err("ffi-open: expected string name".into()),
                        },
                        _ => return Err("ffi-open: expected string name".into()),
                    };
                    let lib = bridge::open_library(&name)?;
                    self.ffi_libs.insert(name.clone(), lib);
                    // Return a symbol representing the library
                    let lib_sym = self.heap.intern(&format!("ffi:{}", name));
                    self.stack.push(Value::Symbol(lib_sym));
                }

                OP_FFI_BIND => {
                    let ret_type_val = self.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let arg_types_val = self.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let func_name_val = self.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let lib_val = self.stack.pop().ok_or("FFI_BIND: empty stack")?;

                    // Get library name from the symbol
                    let lib_name = match lib_val {
                        Value::Symbol(id) => {
                            let full = self.heap.symbol_name(id).to_string();
                            full.strip_prefix("ffi:").unwrap_or(&full).to_string()
                        }
                        _ => return Err("ffi-bind: first arg must be a library handle".into()),
                    };

                    let func_name = match func_name_val {
                        Value::Object(id) => match self.heap.get(id).clone() {
                            HeapObject::MoofString(s) => s,
                            _ => return Err("ffi-bind: expected string function name".into()),
                        },
                        _ => return Err("ffi-bind: expected string function name".into()),
                    };

                    // Parse arg types from a list of symbols
                    let arg_type_syms = self.heap.list_to_vec(arg_types_val);
                    let mut arg_types = Vec::new();
                    for sym_val in &arg_type_syms {
                        if let Value::Symbol(sid) = sym_val {
                            let name = self.heap.symbol_name(*sid).to_string();
                            let ftype = FfiType::from_symbol_name(&name)
                                .ok_or_else(|| format!("ffi-bind: unknown type '{}'", name))?;
                            arg_types.push(ftype);
                        } else {
                            return Err("ffi-bind: arg types must be symbols".into());
                        }
                    }

                    let ret_type_name = match ret_type_val {
                        Value::Symbol(sid) => self.heap.symbol_name(sid).to_string(),
                        _ => return Err("ffi-bind: return type must be a symbol".into()),
                    };
                    let ret_type = FfiType::from_symbol_name(&ret_type_name)
                        .ok_or_else(|| format!("ffi-bind: unknown return type '{}'", ret_type_name))?;

                    // Bind the function via FFI bridge
                    let lib = self.ffi_libs.get(&lib_name)
                        .ok_or_else(|| format!("ffi-bind: library '{}' not open", lib_name))?;
                    let ff = bridge::bind_function(lib, &func_name, arg_types, ret_type)?;

                    // Wrap the bound FFI function as a NativeFn closure and register it
                    let native_name = format!("ffi:{}:{}", lib_name, func_name);
                    // The ForeignFunction is moved into the closure
                    let ff_closure: NativeFn = Box::new(move |heap, args| {
                        bridge::call_foreign(&ff, args, heap)
                    });
                    self.native_registry.register(native_name.clone(), ff_closure);

                    // Create a NativeFunction heap object
                    let nf_obj = HeapObject::NativeFunction { name: native_name };
                    let nf_id = self.heap.alloc(nf_obj);
                    self.stack.push(Value::Object(nf_id));
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
        self.heap.env_define(env_id, sym, val);
    }

    /// Set a binding by walking the environment chain. Errors if not found.
    fn env_set(&mut self, env_id: u32, sym: u32, val: Value) -> Result<(), String> {
        let mut current = Some(env_id);
        while let Some(eid) = current {
            let (found, parent) = match self.heap.get(eid) {
                HeapObject::Environment(env) => {
                    (env.lookup_local(sym).is_some(), env.parent)
                }
                _ => return Err("env_set: not an environment".into()),
            };
            if found {
                self.heap.env_define(eid, sym, val);
                return Ok(());
            }
            current = parent;
        }
        let name = self.heap.symbol_name(sym);
        Err(format!("Cannot set unbound symbol: {}", name))
    }

    /// Call a value as a function: [callable call: args...]
    fn call_value(&mut self, callable: Value, args: &[Value]) -> VMResult {
        match callable {
            Value::Object(id) => {
                match self.heap.get(id).clone() {
                    HeapObject::Lambda { params, body, def_env, .. } => {
                        self.call_lambda(params, body, def_env, args)
                    }
                    HeapObject::Operative { .. } => {
                        Err("Cannot call an operative with evaluated arguments — use vau syntax".into())
                    }
                    HeapObject::NativeFunction { name } => {
                        let name = name.clone();
                        self.call_native(&name, args)
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

    /// Call a lambda: create a new environment, bind params, execute body.
    /// Handles both positional params (a b c) and rest params (args) and
    /// dotted rest (a b . rest).
    fn call_lambda(&mut self, params: Value, body: u32, def_env: u32, args: &[Value]) -> VMResult {
        let new_env_id = self.heap.alloc_env(Some(def_env));
        // Use bind_params for proper destructuring (handles rest params)
        let args_list = self.heap.list(args);
        self.bind_params(new_env_id, params, args_list);
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
    ///
    /// Dispatch order:
    /// 1. User-defined handlers on the object itself (GeneralObject delegation chain)
    /// 2. Type prototype handlers (Integer, Boolean, String, etc.)
    /// 3. VM fast path (arithmetic — NativeHandler markers route here)
    /// 4. doesNotUnderstand:
    pub fn message_send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> VMResult {
        // 1. For GeneralObjects, check user-defined handlers (§4.2)
        if let Value::Object(id) = receiver {
            if let HeapObject::GeneralObject { .. } = self.heap.get(id) {
                if let Some(handler) = self.lookup_handler(id, selector) {
                    let mut full_args = vec![receiver];
                    full_args.extend_from_slice(args);
                    return self.call_value(handler, &full_args);
                }
            }
        }

        // 2. Check type prototype handlers (real callable lambdas)
        if let Some(proto_id) = self.type_prototype(receiver) {
            if let Some(handler) = self.lookup_handler(proto_id, selector) {
                let mut full_args = vec![receiver];
                full_args.extend_from_slice(args);
                return self.call_value(handler, &full_args);
            }
        }

        // 3. VM fast path fallback (for unregistered prototypes during bootstrap)
        if let Some(result) = self.primitive_send(receiver, selector, args)? {
            return Ok(result);
        }

        // 4. doesNotUnderstand:
        if selector != self.sym_does_not_understand {
            if let Value::Object(id) = receiver {
                if let Some(dnu_handler) = self.lookup_handler(id, self.sym_does_not_understand) {
                    let sel_sym = Value::Symbol(selector);
                    let args_list = self.heap.list(args);
                    let full_args = vec![receiver, sel_sym, args_list];
                    return self.call_value(dnu_handler, &full_args);
                }
            }
            // Also check type prototype for doesNotUnderstand:
            if let Some(proto_id) = self.type_prototype(receiver) {
                if let Some(dnu_handler) = self.lookup_handler(proto_id, self.sym_does_not_understand) {
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

        // Universal object protocol: handlerNames, parent, handlerAt:
        // These make primitive types behave as full objects via type prototypes.
        if let Some(proto_id) = self.type_prototype(receiver) {
            match sel_name {
                "handlerNames" => {
                    match self.heap.get(proto_id) {
                        HeapObject::GeneralObject { handlers, .. } => {
                            let names: Vec<Value> = handlers.iter()
                                .map(|(k, _)| Value::Symbol(*k))
                                .collect();
                            return Ok(Some(self.heap.list(&names)));
                        }
                        _ => return Ok(Some(Value::Nil)),
                    }
                }
                "parent" => {
                    return Ok(Some(Value::Object(proto_id)));
                }
                "handlerAt:" => {
                    let key = args.first()
                        .and_then(|v| v.as_symbol())
                        .ok_or("handlerAt: expects a symbol")?;
                    match self.heap.get(proto_id) {
                        HeapObject::GeneralObject { handlers, .. } => {
                            let handler = handlers.iter()
                                .find(|(k, _)| *k == key)
                                .map(|(_, v)| *v)
                                .unwrap_or(Value::Nil);
                            return Ok(Some(handler));
                        }
                        _ => return Ok(Some(Value::Nil)),
                    }
                }
                _ => {}
            }
        }

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
                    "toString" | "describe" => {
                        Ok(Some(self.heap.alloc_string(&format!("{}", a))))
                    }
                    "asString" => {
                        Ok(Some(self.heap.alloc_string(&format!("{}", a))))
                    }
                    "toFloat" => Ok(Some(Value::Float(a as f64))),
                    _ => Ok(None),
                }
            }

            Value::Float(a) => {
                match sel_name {
                    "+" => { let b = args.first().and_then(|v| v.as_float()).ok_or("+ expects number")?; Ok(Some(Value::Float(a + b))) }
                    "-" => { let b = args.first().and_then(|v| v.as_float()).ok_or("- expects number")?; Ok(Some(Value::Float(a - b))) }
                    "*" => { let b = args.first().and_then(|v| v.as_float()).ok_or("* expects number")?; Ok(Some(Value::Float(a * b))) }
                    "/" => { let b = args.first().and_then(|v| v.as_float()).ok_or("/ expects number")?; Ok(Some(Value::Float(a / b))) }
                    "%" => { let b = args.first().and_then(|v| v.as_float()).ok_or("% expects number")?; Ok(Some(Value::Float(a % b))) }
                    "<" => { let b = args.first().and_then(|v| v.as_float()).ok_or("< expects number")?; Ok(Some(if a < b { Value::True } else { Value::False })) }
                    ">" => { let b = args.first().and_then(|v| v.as_float()).ok_or("> expects number")?; Ok(Some(if a > b { Value::True } else { Value::False })) }
                    "=" => { let b = args.first().and_then(|v| v.as_float()).ok_or("= expects number")?; Ok(Some(if a == b { Value::True } else { Value::False })) }
                    "<=" => { let b = args.first().and_then(|v| v.as_float()).ok_or("<= expects number")?; Ok(Some(if a <= b { Value::True } else { Value::False })) }
                    ">=" => { let b = args.first().and_then(|v| v.as_float()).ok_or(">= expects number")?; Ok(Some(if a >= b { Value::True } else { Value::False })) }
                    "negate" => Ok(Some(Value::Float(-a))),
                    "abs" => Ok(Some(Value::Float(a.abs()))),
                    "floor" => Ok(Some(Value::Integer(a.floor() as i64))),
                    "ceil" => Ok(Some(Value::Integer(a.ceil() as i64))),
                    "round" => Ok(Some(Value::Integer(a.round() as i64))),
                    "sqrt" => Ok(Some(Value::Float(a.sqrt()))),
                    "sin" => Ok(Some(Value::Float(a.sin()))),
                    "cos" => Ok(Some(Value::Float(a.cos()))),
                    "toInteger" => Ok(Some(Value::Integer(a as i64))),
                    "toString" | "describe" | "asString" => Ok(Some(self.heap.alloc_string(&format!("{}", a)))),
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
                    "toString" | "describe" => Ok(Some(self.heap.alloc_string("true"))),
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
                    "toString" | "describe" => Ok(Some(self.heap.alloc_string("false"))),
                    _ => Ok(None),
                }
            }

            Value::Nil => {
                match sel_name {
                    "toString" | "describe" => Ok(Some(self.heap.alloc_string("nil"))),
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
                            "toString" | "describe" => {
                                let s = self.format_value(Value::Object(id));
                                Ok(Some(self.heap.alloc_string(&s)))
                            }
                            _ => Ok(None),
                        }
                    }
                    HeapObject::MoofString(ref s) => {
                        let s = s.clone(); // clone to release borrow
                        match sel_name {
                            "toString" | "describe" => Ok(Some(Value::Object(id))),
                            "length" => Ok(Some(Value::Integer(s.chars().count() as i64))),
                            "++" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(other) = self.heap.get(*other_id) {
                                        let new_s = format!("{}{}", s, other);
                                        return Ok(Some(self.heap.alloc_string(&new_s)));
                                    }
                                }
                                // Also allow ++ with integers, symbols via toString
                                if let Some(&arg) = args.first() {
                                    let other_s = self.format_value(arg);
                                    let new_s = format!("{}{}", s, other_s);
                                    return Ok(Some(self.heap.alloc_string(&new_s)));
                                }
                                Ok(None)
                            }
                            "substring:to:" => {
                                let start = args.first().and_then(|v| v.as_integer())
                                    .ok_or("substring:to: expects integer start")? as usize;
                                let end = args.get(1).and_then(|v| v.as_integer())
                                    .ok_or("substring:to: expects integer end")? as usize;
                                let chars: Vec<char> = s.chars().collect();
                                let end = end.min(chars.len());
                                let start = start.min(end);
                                let sub: String = chars[start..end].iter().collect();
                                Ok(Some(self.heap.alloc_string(&sub)))
                            }
                            "at:" => {
                                let idx = args.first().and_then(|v| v.as_integer())
                                    .ok_or("at: expects integer index")? as usize;
                                let chars: Vec<char> = s.chars().collect();
                                if idx < chars.len() {
                                    let ch: String = chars[idx..idx+1].iter().collect();
                                    Ok(Some(self.heap.alloc_string(&ch)))
                                } else {
                                    Ok(Some(Value::Nil))
                                }
                            }
                            "indexOf:" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(needle) = self.heap.get(*other_id) {
                                        if let Some(pos) = s.find(needle.as_str()) {
                                            // Convert byte offset to char offset
                                            let char_pos = s[..pos].chars().count();
                                            return Ok(Some(Value::Integer(char_pos as i64)));
                                        }
                                    }
                                }
                                Ok(Some(Value::Nil))
                            }
                            "split:" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(delim) = self.heap.get(*other_id) {
                                        let delim = delim.clone();
                                        let parts: Vec<Value> = s.split(&delim)
                                            .map(|part| self.heap.alloc_string(part))
                                            .collect();
                                        return Ok(Some(self.heap.list(&parts)));
                                    }
                                }
                                Ok(None)
                            }
                            "trim" => {
                                Ok(Some(self.heap.alloc_string(s.trim())))
                            }
                            "startsWith:" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(prefix) = self.heap.get(*other_id) {
                                        return Ok(Some(if s.starts_with(prefix.as_str()) { Value::True } else { Value::False }));
                                    }
                                }
                                Ok(Some(Value::False))
                            }
                            "endsWith:" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(suffix) = self.heap.get(*other_id) {
                                        return Ok(Some(if s.ends_with(suffix.as_str()) { Value::True } else { Value::False }));
                                    }
                                }
                                Ok(Some(Value::False))
                            }
                            "contains:" => {
                                if let Some(Value::Object(other_id)) = args.first() {
                                    if let HeapObject::MoofString(needle) = self.heap.get(*other_id) {
                                        return Ok(Some(if s.contains(needle.as_str()) { Value::True } else { Value::False }));
                                    }
                                }
                                Ok(Some(Value::False))
                            }
                            "toUpper" => Ok(Some(self.heap.alloc_string(&s.to_uppercase()))),
                            "toLower" => Ok(Some(self.heap.alloc_string(&s.to_lowercase()))),
                            "toSymbol" => {
                                let sym = self.heap.intern(&s);
                                Ok(Some(Value::Symbol(sym)))
                            }
                            "toInteger" => {
                                match s.trim().parse::<i64>() {
                                    Ok(n) => Ok(Some(Value::Integer(n))),
                                    Err(_) => Ok(Some(Value::Nil)),
                                }
                            }
                            "chars" => {
                                let chars: Vec<Value> = s.chars()
                                    .map(|c| self.heap.alloc_string(&c.to_string()))
                                    .collect();
                                Ok(Some(self.heap.list(&chars)))
                            }
                            "replace:with:" => {
                                if let (Some(Value::Object(from_id)), Some(Value::Object(to_id))) =
                                    (args.first(), args.get(1))
                                {
                                    if let (HeapObject::MoofString(from), HeapObject::MoofString(to)) =
                                        (self.heap.get(*from_id), self.heap.get(*to_id))
                                    {
                                        let result = s.replace(from.as_str(), to.as_str());
                                        return Ok(Some(self.heap.alloc_string(&result)));
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
                                self.heap.set_slot(id, key, val);
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
                    HeapObject::Lambda { params, source, .. } => {
                        match sel_name {
                            "source" => Ok(Some(source)),
                            "params" => Ok(Some(params)),
                            "arity" => {
                                let n = self.heap.list_to_vec(params).len();
                                Ok(Some(Value::Integer(n as i64)))
                            }
                            "call:" => {
                                // [f call: args...] — invoke the lambda
                                self.call_value(receiver, args).map(Some)
                            }
                            "toString" | "describe" => Ok(Some(self.heap.alloc_string("<lambda>"))),
                            _ => Ok(None),
                        }
                    }
                    HeapObject::Operative { params, source, env_param, .. } => {
                        match sel_name {
                            "source" => Ok(Some(source)),
                            "params" => Ok(Some(params)),
                            "envParam" => Ok(Some(Value::Symbol(env_param))),
                            "toString" | "describe" => Ok(Some(self.heap.alloc_string("<operative>"))),
                            _ => Ok(None),
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
                            "set:to:" => {
                                // Walk the env chain, set where found
                                let sym = args.first()
                                    .and_then(|v| v.as_symbol())
                                    .ok_or("set:to: expects a symbol")?;
                                let val = args.get(1).copied().unwrap_or(Value::Nil);
                                let _ = env;
                                self.env_set(id, sym, val)?;
                                Ok(Some(val))
                            }
                            "describe" => Ok(Some(self.heap.alloc_string("<environment>"))),
                            _ => Ok(None),
                        }
                    }
                    _ => Ok(None),
                }
            }

            Value::Symbol(sym_id) => {
                match sel_name {
                    "toString" | "describe" => {
                        let s = self.format_value(receiver);
                        Ok(Some(self.heap.alloc_string(&s)))
                    }
                    "asString" | "name" => {
                        // Raw symbol name without the leading quote
                        let name = self.heap.symbol_name(sym_id).to_string();
                        Ok(Some(self.heap.alloc_string(&name)))
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
            Value::Float(f) => format!("{}", f),
            Value::Symbol(id) => format!("'{}", self.heap.symbol_name(id)),
            Value::Object(id) => {
                match self.heap.get(id) {
                    HeapObject::Cons { .. } => self.format_list(val),
                    HeapObject::MoofString(s) => format!("\"{}\"", s),
                    HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
                    HeapObject::BytecodeChunk(_) => "<bytecode>".to_string(),
                    HeapObject::Operative { .. } => "<operative>".to_string(),
                    HeapObject::Lambda { .. } => "<lambda>".to_string(),
                    HeapObject::Environment(_) => "<environment>".to_string(),
                    HeapObject::NativeFunction { name } => {
                        format!("<native {}>", name)
                    }
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
