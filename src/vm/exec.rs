/// The MOOF VM execution engine.
///
/// Stack-based bytecode interpreter. "send" is the single privileged operation (§2).
/// The bytecode is the truth layer (§9.2).

use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;
use crate::ffi::bridge::{self, NativeLibrary, FfiType};
use super::opcodes::*;
use std::collections::{HashMap, VecDeque};

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

/// A message queued for delivery to a vat.
/// Eventual sends produce these; the scheduler delivers one per turn.
#[derive(Debug, Clone)]
pub struct Message {
    /// The receiver object (heap id)
    pub receiver: u32,
    /// The selector symbol (heap id)
    pub selector: u32,
    /// Arguments to the send
    pub args: Vec<Value>,
    /// Promise object to resolve with the result (if any)
    pub resolver: Option<u32>,
}

/// Per-vat execution state. Each vat has its own stack, frames, and root env.
/// Multiple vats share the same heap (via the VM/Runtime).
pub struct VatState {
    /// Vat identifier
    pub id: u32,
    /// The value stack
    pub stack: Vec<Value>,
    /// The call stack
    pub frames: Vec<CallFrame>,
    /// The root environment for this vat
    pub root_env: Option<u32>,
    /// Execution status
    pub status: VatStatus,
    /// Fuel remaining — instructions before yielding. 0 = unlimited.
    pub fuel: u32,
    /// Pending messages to process
    pub mailbox: VecDeque<Message>,
}

/// Default fuel per turn (0 = unlimited, used for REPL/seed mode)
pub const DEFAULT_FUEL: u32 = 0;
/// Fuel for scheduled vat turns
pub const VAT_TURN_FUEL: u32 = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub enum VatStatus {
    Ready,
    Running,
    Suspended { error: String },
}

/// Result of a vat turn.
#[derive(Debug)]
pub enum TurnResult {
    /// Turn completed normally with a value
    Completed(Value),
    /// Fuel exhausted — resume later
    Yielded,
    /// Unhandled error
    Error(String),
}

impl VatState {
    pub fn new(id: u32) -> Self {
        VatState {
            id,
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            root_env: None,
            status: VatStatus::Ready,
            fuel: DEFAULT_FUEL,
            mailbox: VecDeque::new(),
        }
    }

    /// Enqueue a message for this vat.
    pub fn enqueue(&mut self, msg: Message) {
        self.mailbox.push_back(msg);
    }

    /// Dequeue the next pending message (if any).
    pub fn dequeue(&mut self) -> Option<Message> {
        self.mailbox.pop_front()
    }

    /// Check if this vat has pending messages.
    pub fn has_messages(&self) -> bool {
        !self.mailbox.is_empty()
    }
}

/// A pending spawn request. Created by the __spawn native,
/// processed by the scheduler after the current turn completes.
#[derive(Debug)]
pub struct SpawnRequest {
    /// The function (lambda) to run in the new vat
    pub func: Value,
    /// The handle object id (allocated by __spawn, returned to caller)
    pub handle_id: u32,
}

/// The MOOF virtual machine.
/// Contains shared state (heap, natives, protos) and the current vat.
pub struct VM {
    pub heap: Heap,
    /// The current vat's execution state
    pub vat: VatState,
    /// Well-known symbols (cached for fast dispatch)
    pub sym_call: u32,
    pub sym_parent: u32,
    pub sym_does_not_understand: u32,
    pub sym_slot_at: u32,
    pub sym_slot_at_put: u32,
    /// Type prototypes — maps primitive types to their prototype object ids.
    pub proto_integer: Option<u32>,
    pub proto_float: Option<u32>,
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
    /// Pending spawn requests (drained by the scheduler after each turn)
    pub spawn_queue: Vec<SpawnRequest>,
}

/// Result of VM execution.
pub type VMResult = Result<Value, String>;

impl VM {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let sym_call = heap.intern("call:");
        let sym_parent = heap.intern("parent");
        let sym_dnu = heap.intern("doesNotUnderstand:");
        let sym_slot_at = heap.intern("slotAt:");
        let sym_slot_at_put = heap.intern("slotAt:put:");

        VM {
            heap,
            vat: VatState::new(0),
            sym_call,
            sym_parent,
            sym_does_not_understand: sym_dnu,
            sym_slot_at,
            sym_slot_at_put,
            proto_integer: None,
            proto_float: None,
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
            spawn_queue: Vec::new(),
        }
    }

    /// Register a native function and return a Value::Object pointing to a NativeFunction.
    pub fn register_native(&mut self, name: &str, func: NativeFn) -> Value {
        self.native_registry.register(name.to_string(), func);
        let obj = HeapObject::NativeFunction { name: name.to_string() };
        Value::Object(self.heap.alloc(obj))
    }

    /// Call a native function by name, temporarily taking the registry to avoid borrow conflicts.
    pub(crate) fn call_native(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        // VM-level natives that need full VM access (not just Heap)
        if name == "__save-image" {
            return self.native_save_image(args);
        }
        if name == "__try" {
            return self.native_try(args);
        }
        if name == "__spawn" {
            return self.native_spawn(args);
        }
        if name == "__eval-string" {
            return self.native_eval_string(args);
        }
        if name == "__ffi-open" {
            return self.native_ffi_open(args);
        }
        if name == "__ffi-bind" {
            return self.native_ffi_bind(args);
        }
        if name == "__lambda_call:" {
            // args[0] = the lambda/native receiver, args[1..] = actual call args
            let callable = args.first().copied().ok_or("call: needs receiver")?;
            return self.call_value(callable, &args[1..]);
        }
        if name == "__bool_ifTrue:" {
            return self.native_bool_iftrue(args);
        }
        if name == "__bool_ifFalse:" {
            return self.native_bool_iffalse(args);
        }
        if name == "__bool_ifTrue:ifFalse:" {
            return self.native_bool_iftrue_iffalse(args);
        }
        if name == "__bool_and:" {
            return self.native_bool_and(args);
        }
        if name == "__bool_or:" {
            return self.native_bool_or(args);
        }
        // Environment VM intercepts — need full VM access for eval/env ops
        if name == "__env_eval:" {
            return self.native_env_eval(args);
        }
        if name == "__env_lookup:" {
            return self.native_env_lookup(args);
        }
        if name == "__env_set:to:" {
            return self.native_env_set_to(args);
        }
        if name == "__env_define:to:" {
            return self.native_env_define_to(args);
        }
        if name == "__env_remove:" {
            return self.native_env_remove(args);
        }

        let registry = std::mem::replace(&mut self.native_registry, NativeRegistry::new());
        let result = match registry.get(name) {
            Some(func) => func(&mut self.heap, args),
            None => Err(format!("Native function '{}' not found in registry", name)),
        };
        self.native_registry = registry;
        result
    }

    /// Native: save the binary image to .moof/image.bin
    fn native_save_image(&mut self, _args: &[Value]) -> Result<Value, String> {
        use crate::persistence::snapshot;
        let path = std::path::PathBuf::from(".moof/image.bin");
        let root_env = self.vat.root_env.ok_or("no root env")?;
        let protos = self.get_protos();
        snapshot::save_image(&path, &self.heap, root_env, protos)?;
        Ok(Value::True)
    }

    /// Native: try/catch error containment.
    /// Args: [body_lambda, handler_lambda]
    /// Calls body with no args. If it errors, calls handler with error string.
    fn native_try(&mut self, args: &[Value]) -> Result<Value, String> {
        let body = args.first().copied().ok_or("__try: needs body")?;
        let handler = args.get(1).copied().ok_or("__try: needs handler")?;

        match self.call_value(body, &[]) {
            Ok(val) => Ok(val),
            Err(err_msg) => {
                let err_val = self.heap.alloc_string(&err_msg);
                self.call_value(handler, &[err_val])
            }
        }
    }

    /// Native: spawn a new vat.
    /// Args: [lambda]
    /// Creates a SpawnRequest. The scheduler processes it after the turn.
    /// Returns a VatHandle object with a `vat-id` slot (set to -1 until scheduler assigns it).
    fn native_spawn(&mut self, args: &[Value]) -> Result<Value, String> {
        let func = args.first().copied().ok_or("spawn: needs a function argument")?;

        // Validate it's callable
        match func {
            Value::Object(id) => match self.heap.get(id) {
                HeapObject::Lambda { .. } | HeapObject::Operative { .. } | HeapObject::NativeFunction { .. } => {}
                _ => return Err("spawn: argument must be a function".into()),
            },
            _ => return Err("spawn: argument must be a function".into()),
        }

        // Allocate a handle object — the scheduler will fill in vat-id
        let vat_id_sym = self.heap.intern("vat-id");
        let status_sym = self.heap.intern("status");
        let status_val = self.heap.alloc_string("pending");
        let handle_id = self.heap.alloc(HeapObject::GeneralObject {
            parent: Value::Nil,
            slots: vec![
                (vat_id_sym, Value::Integer(-1)),
                (status_sym, status_val),
            ],
            handlers: Vec::new(),
        });

        self.spawn_queue.push(SpawnRequest {
            func,
            handle_id,
        });

        Ok(Value::Object(handle_id))
    }

    /// Native: eval-string. Parses and evaluates a moof string.
    fn native_eval_string(&mut self, args: &[Value]) -> Result<Value, String> {
        let str_val = args.first().copied().ok_or("eval-string: needs argument")?;
        let source = match str_val {
            Value::Object(id) => match self.heap.get(id).clone() {
                HeapObject::MoofString(s) => s,
                _ => return Err("eval-string: expected string".into()),
            },
            _ => return Err("eval-string: expected string".into()),
        };
        let env_id = self.vat.root_env.unwrap_or(0);
        crate::eval_source(self, env_id, &source, "<eval-string>")
    }

    /// Native: ffi-open. Opens a native library.
    fn native_ffi_open(&mut self, args: &[Value]) -> Result<Value, String> {
        let name_val = args.first().copied().ok_or("ffi-open: needs library name")?;
        let name = match name_val {
            Value::Object(id) => match self.heap.get(id).clone() {
                HeapObject::MoofString(s) => s,
                _ => return Err("ffi-open: expected string name".into()),
            },
            _ => return Err("ffi-open: expected string name".into()),
        };
        let lib = bridge::open_library(&name)?;
        self.ffi_libs.insert(name.clone(), lib);
        let lib_sym = self.heap.intern(&format!("ffi:{}", name));
        Ok(Value::Symbol(lib_sym))
    }

    /// Native: ffi-bind. Binds a foreign function from an open library.
    fn native_ffi_bind(&mut self, args: &[Value]) -> Result<Value, String> {
        if args.len() != 4 {
            return Err("ffi-bind: requires lib, name, arg-types, ret-type".into());
        }
        let lib_val = args[0];
        let func_name_val = args[1];
        let arg_types_val = args[2];
        let ret_type_val = args[3];

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

        let lib = self.ffi_libs.get(&lib_name)
            .ok_or_else(|| format!("ffi-bind: library '{}' not open", lib_name))?;
        let ff = bridge::bind_function(lib, &func_name, arg_types, ret_type)?;

        let native_name = format!("ffi:{}:{}", lib_name, func_name);
        let ff_closure: NativeFn = Box::new(move |heap, args| {
            bridge::call_foreign(&ff, args, heap)
        });
        self.native_registry.register(native_name.clone(), ff_closure);
        let nf_obj = HeapObject::NativeFunction { name: native_name };
        let nf_id = self.heap.alloc(nf_obj);
        Ok(Value::Object(nf_id))
    }

    // ── Boolean conditional natives ──
    // These need call_value (VM access), so they're intercepted here.
    // args[0] = receiver (True/False), args[1..] = block arguments.

    fn native_bool_iftrue(&mut self, args: &[Value]) -> Result<Value, String> {
        match args[0] {
            Value::True => self.call_value(args[1], &[]),
            Value::False => Ok(Value::Nil),
            _ => Err("ifTrue: expects boolean receiver".into()),
        }
    }

    fn native_bool_iffalse(&mut self, args: &[Value]) -> Result<Value, String> {
        match args[0] {
            Value::True => Ok(Value::Nil),
            Value::False => self.call_value(args[1], &[]),
            _ => Err("ifFalse: expects boolean receiver".into()),
        }
    }

    fn native_bool_iftrue_iffalse(&mut self, args: &[Value]) -> Result<Value, String> {
        match args[0] {
            Value::True => self.call_value(args[1], &[]),
            Value::False => self.call_value(args[2], &[]),
            _ => Err("ifTrue:ifFalse: expects boolean receiver".into()),
        }
    }

    fn native_bool_and(&mut self, args: &[Value]) -> Result<Value, String> {
        match args[0] {
            Value::True => self.call_value(args[1], &[]),
            Value::False => Ok(Value::False),
            _ => Err("and: expects boolean receiver".into()),
        }
    }

    fn native_bool_or(&mut self, args: &[Value]) -> Result<Value, String> {
        match args[0] {
            Value::True => Ok(Value::True),
            Value::False => self.call_value(args[1], &[]),
            _ => Err("or: expects boolean receiver".into()),
        }
    }

    // ── Environment VM intercepts ──
    // args[0] = receiver (the environment object), args[1..] = message arguments

    fn native_env_eval(&mut self, args: &[Value]) -> Result<Value, String> {
        let env_id = args[0].as_object().ok_or("eval: expects environment receiver")?;
        let expr = args.get(1).copied().ok_or("eval: expects an expression")?;
        self.eval(expr, env_id)
    }

    fn native_env_lookup(&mut self, args: &[Value]) -> Result<Value, String> {
        let env_id = args[0].as_object().ok_or("lookup: expects environment receiver")?;
        let sym = args.get(1).and_then(|v| v.as_symbol())
            .ok_or("lookup: expects a symbol")?;
        self.env_lookup(env_id, sym)
    }

    fn native_env_set_to(&mut self, args: &[Value]) -> Result<Value, String> {
        let env_id = args[0].as_object().ok_or("set:to: expects environment receiver")?;
        let sym = args.get(1).and_then(|v| v.as_symbol())
            .ok_or("set:to: expects a symbol")?;
        let val = args.get(2).copied().unwrap_or(Value::Nil);
        self.env_set(env_id, sym, val)?;
        Ok(val)
    }

    fn native_env_define_to(&mut self, args: &[Value]) -> Result<Value, String> {
        let env_id = args[0].as_object().ok_or("define:to: expects environment receiver")?;
        let sym = args.get(1).and_then(|v| v.as_symbol())
            .ok_or("define:to: expects a symbol")?;
        let val = args.get(2).copied().unwrap_or(Value::Nil);
        self.heap.env_define(env_id, sym, val);
        Ok(val)
    }

    fn native_env_remove(&mut self, args: &[Value]) -> Result<Value, String> {
        let env_id = args[0].as_object().ok_or("remove: expects environment receiver")?;
        let sym = args.get(1).and_then(|v| v.as_symbol())
            .ok_or("remove: expects a symbol")?;
        self.heap.env_remove(env_id, sym);
        Ok(Value::Nil)
    }

    // Native handler registration has moved to vm::natives::register_all_natives.
    // All native operations are NativeFunction closures in the NativeRegistry.
    // One path. One mechanism.

    // Introspection / module-reading methods live in vm::introspect.

    /// Execute a bytecode chunk in a given environment. Returns the final value.
    /// Execute a chunk as a vat turn with fuel budget.
    /// Returns TurnResult instead of VMResult.
    pub fn execute_turn(&mut self, chunk_id: u32, env_id: u32, fuel: u32) -> TurnResult {
        self.vat.fuel = fuel;
        self.vat.status = VatStatus::Running;
        match self.execute(chunk_id, env_id) {
            Ok(val) => {
                self.vat.status = VatStatus::Ready;
                TurnResult::Completed(val)
            }
            Err(e) if e == "__yielded" => {
                self.vat.status = VatStatus::Ready;
                TurnResult::Yielded
            }
            Err(e) => {
                self.vat.status = VatStatus::Suspended { error: e.clone() };
                TurnResult::Error(e)
            }
        }
    }

    /// Execute a bytecode chunk. For direct use (REPL), fuel is 0 (unlimited).
    pub fn execute(&mut self, chunk_id: u32, env_id: u32) -> VMResult {
        let frame_depth = self.vat.frames.len();
        self.vat.frames.push(CallFrame {
            chunk_id,
            ip: 0,
            env_id,
            stack_base: self.vat.stack.len(),
        });

        self.run(frame_depth)
    }

    /// The main execution loop. Runs until we drop back to `base_depth` frames
    /// or fuel is exhausted.
    fn run(&mut self, base_depth: usize) -> VMResult {
        loop {
            if self.vat.frames.len() <= base_depth {
                return Ok(self.vat.stack.pop().unwrap_or(Value::Nil));
            }

            // Fuel check: yield if exhausted (0 = unlimited)
            if self.vat.fuel > 0 {
                self.vat.fuel -= 1;
                if self.vat.fuel == 0 {
                    self.vat.status = VatStatus::Ready;
                    return Err("__yielded".into());
                }
            }

            let frame = self.vat.frames.last().unwrap();
            let chunk_id = frame.chunk_id;
            let env_id = frame.env_id;

            // Read the current bytecode chunk
            let (code, constants) = match self.heap.get(chunk_id) {
                HeapObject::BytecodeChunk(chunk) => {
                    (chunk.code.clone(), chunk.constants.clone())
                }
                _ => return Err("Not a bytecode chunk".into()),
            };

            let ip = self.vat.frames.last().unwrap().ip;
            if ip >= code.len() {
                // End of chunk — return top of stack or nil
                let result = self.vat.stack.pop().unwrap_or(Value::Nil);
                let base = self.vat.frames.last().unwrap().stack_base;
                self.vat.frames.pop();
                self.vat.stack.truncate(base);
                self.vat.stack.push(result);
                continue;
            }

            let op = code[ip];
            // Advance ip
            self.vat.frames.last_mut().unwrap().ip = ip + 1;

            match op {
                OP_CONST => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3;
                    self.vat.stack.push(constants[idx]);
                }

                OP_NIL => self.vat.stack.push(Value::Nil),
                OP_TRUE => self.vat.stack.push(Value::True),
                OP_FALSE => self.vat.stack.push(Value::False),

                OP_POP => { self.vat.stack.pop(); }

                OP_LOOKUP => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("LOOKUP: expected symbol in constants".into()),
                    };
                    let val = self.env_lookup(env_id, sym)?;
                    self.vat.stack.push(val);
                }

                OP_DEF => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3;
                    let sym = match constants[idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("DEF: expected symbol in constants".into()),
                    };
                    let val = self.vat.stack.pop().ok_or("DEF: empty stack")?;
                    // Error if already bound in THIS env (not parent).
                    // Shadowing a parent binding is fine; redefining is not.
                    // Use <- to update existing bindings.
                    if let HeapObject::Environment(env) = self.heap.get(env_id) {
                        if env.lookup_local(sym).is_some() {
                            let name = self.heap.symbol_name(sym).to_string();
                            return Err(format!("def: '{}' is already defined (use <- to update)", name));
                        }
                    }
                    self.env_define(env_id, sym, val);
                    self.vat.stack.push(val);
                }

                OP_GET_ENV => {
                    self.vat.stack.push(Value::Object(env_id));
                }

                OP_SEND => {
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 4;
                    let selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("SEND: expected symbol selector".into()),
                    };
                    // Stack: [receiver, arg1, ..., argN]
                    let stack_start = self.vat.stack.len() - argc - 1;
                    let receiver = self.vat.stack[stack_start];
                    let args: Vec<Value> = self.vat.stack[stack_start + 1..].to_vec();
                    self.vat.stack.truncate(stack_start);
                    let result = self.message_send(receiver, selector, &args)?;
                    self.vat.stack.push(result);
                }

                OP_EVENTUAL_SEND => {
                    let sel_idx = read_u16(&code, ip + 1) as usize;
                    let argc = code[ip + 3] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 4;
                    let selector = match constants[sel_idx] {
                        Value::Symbol(s) => s,
                        _ => return Err("EVENTUAL_SEND: expected symbol selector".into()),
                    };
                    // Stack: [receiver, arg1, ..., argN]
                    let stack_start = self.vat.stack.len() - argc - 1;
                    let receiver = self.vat.stack[stack_start];
                    let args: Vec<Value> = self.vat.stack[stack_start + 1..].to_vec();
                    self.vat.stack.truncate(stack_start);

                    let receiver_id = match receiver {
                        Value::Object(id) => id,
                        _ => return Err("eventual send: receiver must be an object".into()),
                    };

                    // Create a Promise object
                    let val_sym = self.heap.intern("value");
                    let resolved_sym = self.heap.intern("resolved");
                    let waiters_sym = self.heap.intern("waiters");
                    let promise_id = self.heap.alloc(HeapObject::GeneralObject {
                        parent: Value::Nil,
                        slots: vec![
                            (val_sym, Value::Nil),
                            (resolved_sym, Value::False),
                            (waiters_sym, Value::Nil),
                        ],
                        handlers: Vec::new(),
                    });

                    // Enqueue message on this vat's mailbox
                    self.vat.enqueue(Message {
                        receiver: receiver_id,
                        selector,
                        args,
                        resolver: Some(promise_id),
                    });

                    self.vat.stack.push(Value::Object(promise_id));
                }

                OP_CONS => {
                    let cdr = self.vat.stack.pop().ok_or("CONS: empty stack")?;
                    let car = self.vat.stack.pop().ok_or("CONS: empty stack")?;
                    let val = self.heap.cons(car, cdr);
                    self.vat.stack.push(val);
                }

                OP_EQ => {
                    let b = self.vat.stack.pop().ok_or("EQ: empty stack")?;
                    let a = self.vat.stack.pop().ok_or("EQ: empty stack")?;
                    self.vat.stack.push(if a == b { Value::True } else { Value::False });
                }

                OP_QUOTE => {
                    let idx = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3;
                    self.vat.stack.push(constants[idx]);
                }

                OP_VAU => {
                    let params_idx = read_u16(&code, ip + 1) as usize;
                    let env_param_idx = read_u16(&code, ip + 3) as usize;
                    let body_idx = read_u16(&code, ip + 5) as usize;
                    let source_idx = read_u16(&code, ip + 7) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 9;

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
                    self.vat.stack.push(Value::Object(id));
                }

                OP_CALL => {
                    // Semantically: [callable call: args...]
                    // Fast path for Lambda/NativeFunction (skip handler lookup).
                    // Falls back to message_send for GeneralObjects with call: handlers.
                    let argc = code[ip + 1] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.vat.stack.len() - argc - 1;
                    let callable = self.vat.stack[stack_start];
                    let args: Vec<Value> = self.vat.stack[stack_start + 1..].to_vec();
                    self.vat.stack.truncate(stack_start);
                    // Classify callable to avoid borrow conflict
                    let is_direct = match callable {
                        Value::Object(id) => matches!(
                            self.heap.get(id),
                            HeapObject::Lambda { .. } | HeapObject::NativeFunction { .. }
                        ),
                        _ => false,
                    };
                    let is_operative = match callable {
                        Value::Object(id) => matches!(self.heap.get(id), HeapObject::Operative { .. }),
                        _ => false,
                    };
                    let result = if is_operative {
                        return Err("Cannot call an operative with evaluated arguments".into());
                    } else if is_direct {
                        self.call_value(callable, &args)?
                    } else {
                        // General case: [callable call: args...]
                        self.message_send(callable, self.sym_call, &args)?
                    };
                    self.vat.stack.push(result);
                }

                OP_RETURN => {
                    let result = self.vat.stack.pop().unwrap_or(Value::Nil);
                    let base = self.vat.frames.last().unwrap().stack_base;
                    self.vat.frames.pop();
                    self.vat.stack.truncate(base);
                    self.vat.stack.push(result);
                    if self.vat.frames.is_empty() {
                        return Ok(result);
                    }
                }

                OP_JUMP => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3 + offset;
                }

                OP_JUMP_IF_FALSE => {
                    let offset = read_u16(&code, ip + 1) as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 3;
                    let cond = self.vat.stack.pop().ok_or("JUMP_IF_FALSE: empty stack")?;
                    if !cond.is_truthy() {
                        self.vat.frames.last_mut().unwrap().ip = ip + 3 + offset;
                    }
                }

                OP_LOOP_BACK => {
                    let distance = read_u16(&code, ip + 1) as usize;
                    // Jump backwards: ip is currently at ip+1 (after reading opcode)
                    // We want to go to (ip + 3) - distance
                    let target = (ip + 3).wrapping_sub(distance);
                    self.vat.frames.last_mut().unwrap().ip = target;
                }

                OP_CALL_OPERATIVE => {
                    let argc = code[ip + 1] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.vat.stack.len() - argc - 1;
                    let operative = self.vat.stack[stack_start];
                    let args: Vec<Value> = self.vat.stack[stack_start + 1..].to_vec();
                    self.vat.stack.truncate(stack_start);

                    // Build the args as a cons list (unevaluated)
                    let args_list = self.heap.list(&args);
                    let result = self.call_operative(operative, args_list, env_id)?;
                    self.vat.stack.push(result);
                }

                OP_APPLY => {
                    // Semantically: [callable call: args...]
                    // Operatives receive raw args + caller env (fundamental to vau).
                    // Everything else: eval args, then call.
                    // Stack: [callable, quoted_args_list]
                    let args_list = self.vat.stack.pop().ok_or("APPLY: empty stack")?;
                    let callable = self.vat.stack.pop().ok_or("APPLY: empty stack")?;

                    // Classify to avoid borrow conflicts
                    let is_operative = match callable {
                        Value::Object(id) => matches!(self.heap.get(id), HeapObject::Operative { .. }),
                        _ => false,
                    };
                    let is_direct = match callable {
                        Value::Object(id) => matches!(
                            self.heap.get(id),
                            HeapObject::Lambda { .. } | HeapObject::NativeFunction { .. }
                        ),
                        _ => false,
                    };

                    let result = if is_operative {
                        self.call_operative(callable, args_list, env_id)?
                    } else {
                        // Eval args
                        let raw_args = self.heap.list_to_vec(args_list);
                        let mut evaled = Vec::new();
                        for arg in raw_args {
                            evaled.push(self.eval(arg, env_id)?);
                        }
                        if is_direct {
                            // Fast path: Lambda/NativeFunction
                            self.call_value(callable, &evaled)?
                        } else {
                            // General case: [callable call: args...]
                            self.message_send(callable, self.sym_call, &evaled)?
                        }
                    };
                    self.vat.stack.push(result);
                }

                OP_TAIL_APPLY => {
                    // Tail-call variant: replaces current frame for lambdas
                    let args_list = self.vat.stack.pop().ok_or("TAIL_APPLY: empty stack")?;
                    let callable = self.vat.stack.pop().ok_or("TAIL_APPLY: empty stack")?;

                    // Classify to avoid borrow conflicts
                    let variant = match callable {
                        Value::Object(id) => match self.heap.get(id) {
                            HeapObject::Lambda { .. } => 1,      // TCO path
                            HeapObject::Operative { .. } => 2,   // operative path
                            HeapObject::NativeFunction { .. } => 3, // direct call
                            _ => 4,                              // general message_send
                        },
                        _ => 4,
                    };

                    match variant {
                        1 => {
                            // Lambda TCO: replace frame instead of pushing new one
                            let (params, body, def_env) = match callable {
                                Value::Object(id) => match self.heap.get(id).clone() {
                                    HeapObject::Lambda { params, body, def_env, .. } => (params, body, def_env),
                                    _ => unreachable!(),
                                },
                                _ => unreachable!(),
                            };
                            let raw_args = self.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                evaled.push(self.eval(arg, env_id)?);
                            }
                            let new_env_id = self.heap.alloc_env(Some(def_env));
                            let evaled_list = self.heap.list(&evaled);
                            self.bind_params(new_env_id, params, evaled_list);
                            let frame = self.vat.frames.last_mut().unwrap();
                            self.vat.stack.truncate(frame.stack_base);
                            frame.chunk_id = body;
                            frame.ip = 0;
                            frame.env_id = new_env_id;
                        }
                        2 => {
                            let result = self.call_operative(callable, args_list, env_id)?;
                            self.vat.stack.push(result);
                        }
                        3 => {
                            // NativeFunction: eval args, direct call
                            let raw_args = self.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                evaled.push(self.eval(arg, env_id)?);
                            }
                            let result = self.call_value(callable, &evaled)?;
                            self.vat.stack.push(result);
                        }
                        _ => {
                            // General case: [callable call: args...]
                            let raw_args = self.heap.list_to_vec(args_list);
                            let mut evaled = Vec::new();
                            for arg in raw_args {
                                evaled.push(self.eval(arg, env_id)?);
                            }
                            let result = self.message_send(callable, self.sym_call, &evaled)?;
                            self.vat.stack.push(result);
                        }
                    }
                }

                OP_TAIL_CALL => {
                    // Tail-call variant of OP_CALL for known-lambda contexts (let)
                    let argc = code[ip + 1] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 2;
                    let stack_start = self.vat.stack.len() - argc - 1;
                    let callable = self.vat.stack[stack_start];
                    let args: Vec<Value> = self.vat.stack[stack_start + 1..].to_vec();
                    self.vat.stack.truncate(stack_start);

                    match callable {
                        Value::Object(id) => {
                            match self.heap.get(id).clone() {
                                HeapObject::Lambda { params, body, def_env, .. } => {
                                    let new_env_id = self.heap.alloc_env(Some(def_env));
                                    let args_list = self.heap.list(&args);
                                    self.bind_params(new_env_id, params, args_list);
                                    let frame = self.vat.frames.last_mut().unwrap();
                                    self.vat.stack.truncate(frame.stack_base);
                                    frame.chunk_id = body;
                                    frame.ip = 0;
                                    frame.env_id = new_env_id;
                                }
                                _ => {
                                    let result = self.call_value(callable, &args)?;
                                    self.vat.stack.push(result);
                                }
                            }
                        }
                        _ => return Err(format!("Cannot call {:?}", callable)),
                    }
                }

                OP_EVAL => {
                    let expr = self.vat.stack.pop().ok_or("EVAL: empty stack")?;
                    let result = self.eval(expr, env_id)?;
                    self.vat.stack.push(result);
                }

                OP_CAR => {
                    let val = self.vat.stack.pop().ok_or("CAR: empty stack")?;
                    self.vat.stack.push(self.heap.car(val));
                }

                OP_CDR => {
                    let val = self.vat.stack.pop().ok_or("CDR: empty stack")?;
                    self.vat.stack.push(self.heap.cdr(val));
                }

                OP_MAKE_OBJECT => {
                    let slot_count = code[ip + 1] as usize;
                    self.vat.frames.last_mut().unwrap().ip = ip + 2;
                    // Stack: [parent, key1, val1, key2, val2, ...]
                    let mut explicit_slots = Vec::new();
                    for _ in 0..slot_count {
                        let val = self.vat.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key = self.vat.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;
                        let key_sym = key.as_symbol().ok_or("MAKE_OBJECT: slot key must be symbol")?;
                        explicit_slots.push((key_sym, val));
                    }
                    explicit_slots.reverse();
                    let parent = self.vat.stack.pop().ok_or("MAKE_OBJECT: empty stack")?;

                    // Clone default slot values from parent prototype.
                    // Explicit slots override parent defaults.
                    let mut slots = Vec::new();
                    if let Value::Object(parent_id) = parent {
                        if let HeapObject::GeneralObject { slots: parent_slots, .. } = self.heap.get(parent_id) {
                            let parent_slots = parent_slots.clone();
                            for (key, val) in &parent_slots {
                                if !explicit_slots.iter().any(|(k, _)| k == key) {
                                    slots.push((*key, *val));
                                }
                            }
                        }
                    }
                    slots.extend(explicit_slots);

                    let obj = HeapObject::GeneralObject {
                        parent,
                        slots,
                        handlers: Vec::new(),
                    };
                    let id = self.heap.alloc(obj);
                    self.vat.stack.push(Value::Object(id));
                }

                OP_HANDLE => {
                    let handler = self.vat.stack.pop().ok_or("HANDLE: empty stack")?;
                    let selector = self.vat.stack.pop().ok_or("HANDLE: empty stack")?;
                    let obj_val = self.vat.stack.pop().ok_or("HANDLE: empty stack")?;
                    let sel_sym = selector.as_symbol().ok_or("HANDLE: selector must be symbol")?;
                    let obj_id = obj_val.as_object().ok_or("HANDLE: expected object")?;
                    self.heap.add_handler(obj_id, sel_sym, handler);
                    self.vat.stack.push(obj_val);
                }

                OP_SLOT_GET => {
                    // Semantically: [obj slotAt: 'name]
                    // Fast path: direct slot read on plain GeneralObjects.
                    // Falls back to message_send for custom slotAt: handlers (Membranes, etc).
                    let field_sym = self.vat.stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let obj_val = self.vat.stack.pop().ok_or("SLOT_GET: empty stack")?;
                    let sym_id = field_sym.as_symbol().ok_or("SLOT_GET: field must be symbol")?;
                    let sel = self.sym_slot_at;
                    let result = if let Value::Object(id) = obj_val {
                        // Fast path: check if this object has a custom slotAt: handler
                        // (via user-defined handler, NOT the Object proto default).
                        // If it does, route through message_send for interception.
                        let has_custom_handler = self.lookup_handler(id, sel).is_some();
                        if has_custom_handler {
                            self.message_send(obj_val, sel, &[field_sym])?
                        } else {
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
                    } else {
                        return Err(format!("SLOT_GET: cannot access field on {:?}", obj_val));
                    };
                    self.vat.stack.push(result);
                }

                OP_SLOT_SET => {
                    // Semantically: [obj slotAt: 'name put: val]
                    // Fast path: direct slot write. Falls back to message_send
                    // if receiver has a custom slotAt:put: handler.
                    let val = self.vat.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let field_sym = self.vat.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let obj_val = self.vat.stack.pop().ok_or("SLOT_SET: empty stack")?;
                    let sym_id = field_sym.as_symbol().ok_or("SLOT_SET: field must be symbol")?;
                    let sel = self.sym_slot_at_put;
                    if let Value::Object(id) = obj_val {
                        let has_custom_handler = self.lookup_handler(id, sel).is_some();
                        if has_custom_handler {
                            let result = self.message_send(obj_val, sel, &[field_sym, val])?;
                            self.vat.stack.push(result);
                        } else {
                            self.heap.set_slot(id, sym_id, val);
                            self.vat.stack.push(val);
                        }
                    } else {
                        return Err("SLOT_SET: expected object".into());
                    }
                }

                OP_APPEND => {
                    let b = self.vat.stack.pop().ok_or("APPEND: empty stack")?;
                    let a = self.vat.stack.pop().ok_or("APPEND: empty stack")?;
                    // append(a, b): walk a, cons each element onto b
                    let a_elems = self.heap.list_to_vec(a);
                    let mut result = b;
                    for &elem in a_elems.iter().rev() {
                        result = self.heap.cons(elem, result);
                    }
                    self.vat.stack.push(result);
                }

                // Legacy opcodes — redirect to native functions.
                // Old bytecode in the image may still use these.
                // New compilations go through the native fn path instead.
                0x51 => { // was OP_PRINT
                    let val = self.vat.stack.pop().ok_or("PRINT: empty stack")?;
                    let s = self.format_value(val);
                    println!("{}", s);
                    self.vat.stack.push(Value::Nil);
                }
                0x54 => { // was OP_TYPE_OF
                    let val = self.vat.stack.pop().ok_or("TYPE_OF: empty stack")?;
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
                    self.vat.stack.push(Value::Symbol(sym));
                }
                0x55 => { // was OP_LOAD — just error, no source files
                    return Err("load: removed — the image IS the program".into());
                }
                0x56 => { // was OP_SOURCE
                    let val = self.vat.stack.pop().ok_or("SOURCE: empty stack")?;
                    let source = match val {
                        Value::Object(id) => match self.heap.get(id) {
                            HeapObject::Lambda { source, .. } => *source,
                            HeapObject::Operative { source, .. } => *source,
                            _ => Value::Nil,
                        },
                        _ => Value::Nil,
                    };
                    self.vat.stack.push(source);
                }
                0x58 => { // was OP_EVAL_STRING
                    let str_val = self.vat.stack.pop().ok_or("EVAL_STRING: empty stack")?;
                    let source = match str_val {
                        Value::Object(id) => match self.heap.get(id).clone() {
                            HeapObject::MoofString(s) => s,
                            _ => return Err("eval-string: expected string".into()),
                        },
                        _ => return Err("eval-string: expected string".into()),
                    };
                    let result = crate::eval_source(self, env_id, &source, "<eval-string>")?;
                    self.vat.stack.push(result);
                }
                0x70 => { // was OP_FFI_OPEN
                    let name_val = self.vat.stack.pop().ok_or("FFI_OPEN: empty stack")?;
                    let result = self.native_ffi_open(&[name_val])?;
                    self.vat.stack.push(result);
                }
                0x71 => { // was OP_FFI_BIND
                    let ret = self.vat.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let arg_types = self.vat.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let func_name = self.vat.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let lib = self.vat.stack.pop().ok_or("FFI_BIND: empty stack")?;
                    let result = self.native_ffi_bind(&[lib, func_name, arg_types, ret])?;
                    self.vat.stack.push(result);
                }

                _ => return Err(format!("Unknown opcode: 0x{:02x}", op)),
            }
        }
    }

    /// Look up a symbol in the environment chain.
    pub fn env_lookup(&self, env_id: u32, sym: u32) -> VMResult {
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
    pub(crate) fn env_define(&mut self, env_id: u32, sym: u32, val: Value) {
        self.heap.env_define(env_id, sym, val);
    }

    /// Set a binding by walking the environment chain. Errors if not found.
    pub(crate) fn env_set(&mut self, env_id: u32, sym: u32, val: Value) -> Result<(), String> {
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

}
