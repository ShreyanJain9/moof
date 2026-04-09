// The bytecode interpreter: register-based VM.
//
// Executes Chunk bytecode. Knows about two kinds of handler:
// - native (symbol → rust closure in heap.natives)
// - bytecode (nursery object with code/constants/arity/env slots)
//
// The VM is not a plugin host. It knows what it runs.

use crate::dispatch;
use crate::heap::Heap;
use crate::lang::compiler::{ClosureDesc, CompileResult};
use crate::opcodes::{Chunk, Op};
use crate::value::Value;

struct Frame {
    chunk_id: u32,
    pc: usize,
    base_reg: usize,
    env: Value, // the environment object for this scope
}

pub struct VM {
    registers: Vec<Value>,
    frames: Vec<Frame>,
    closure_descs: Vec<ClosureDesc>,
    desc_base: usize, // offset added to MakeClosure indices at runtime
}

impl VM {
    pub fn new() -> Self {
        VM {
            registers: vec![Value::NIL; 256],
            frames: Vec::new(),
            closure_descs: Vec::new(),
            desc_base: 0,
        }
    }

    pub fn add_closure_desc(&mut self, desc: crate::lang::compiler::ClosureDesc) {
        self.closure_descs.push(desc);
    }

    pub fn closure_descs_ref(&self) -> &[crate::lang::compiler::ClosureDesc] {
        &self.closure_descs
    }

    /// Execute a chunk in the given environment, returning the result.
    pub fn execute(&mut self, heap: &mut Heap, chunk: &Chunk, env: Value) -> Result<Value, String> {
        let base = 0;
        self.registers.resize(base + chunk.num_regs as usize + 1, Value::NIL);

        let mut pc = 0;
        let code = &chunk.code;
        let constants = &chunk.constants;

        loop {
            if pc + 3 >= code.len() {
                return Ok(self.registers[base]);
            }

            let op = code[pc];
            let a = code[pc + 1];
            let b = code[pc + 2];
            let c = code[pc + 3];
            pc += 4;

            let Some(opcode) = Op::from_u8(op) else {
                return Err(format!("unknown opcode: {op}"));
            };

            match opcode {
                Op::LoadConst => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    self.registers[base + a as usize] = Value::from_bits(constants[idx]);
                }
                Op::LoadNil => self.registers[base + a as usize] = Value::NIL,
                Op::LoadTrue => self.registers[base + a as usize] = Value::TRUE,
                Op::LoadFalse => self.registers[base + a as usize] = Value::FALSE,
                Op::Move => self.registers[base + a as usize] = self.registers[base + b as usize],
                Op::LoadInt => {
                    let val = i16::from_be_bytes([b, c]) as i64;
                    self.registers[base + a as usize] = Value::integer(val);
                }
                Op::Return => {
                    return Ok(self.registers[base + a as usize]);
                }

                Op::Send => {
                    // SEND dst, recv, sel_const — next 4 bytes: nargs, arg0, arg1, arg2
                    let dst = a as usize;
                    let recv = self.registers[base + b as usize];
                    let sel_idx = c as usize;
                    let sel_sym = if sel_idx < constants.len() {
                        Value::from_bits(constants[sel_idx]).as_symbol()
                            .ok_or("send: selector constant is not a symbol")?
                    } else {
                        return Err("send: selector constant out of bounds".into());
                    };

                    // read nargs from next instruction slot
                    if pc + 3 >= code.len() {
                        return Err("send: truncated nargs".into());
                    }
                    let nargs = code[pc] as usize;
                    let arg_start = pc + 1;
                    pc += 4; // skip the args instruction

                    let mut args = Vec::with_capacity(nargs);
                    for i in 0..nargs.min(3) {
                        args.push(self.registers[base + code[arg_start + i] as usize]);
                    }

                    // dispatch
                    let result = self.dispatch_send(heap, recv, sel_sym, &args)?;
                    self.registers[base + dst] = result;
                }

                Op::Call => {
                    // legacy: (f a b c) now compiles to SEND call:, but keep
                    // this opcode for tests. just delegates to dispatch_send.
                    let dst = a as usize;
                    let func = self.registers[base + b as usize];
                    let nargs = c as usize;
                    let mut args = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        args.push(self.registers[base + b as usize + 1 + i]);
                    }
                    let result = self.dispatch_send(heap, func, heap.sym_call, &args)?;
                    self.registers[base + dst] = result;
                }

                Op::Jump => {
                    let offset = i16::from_be_bytes([a, b]) as isize;
                    pc = (pc as isize + offset) as usize;
                }
                Op::JumpIfFalse => {
                    let test = self.registers[base + a as usize];
                    if !test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        pc = (pc as isize + offset) as usize;
                    }
                }
                Op::JumpIfTrue => {
                    let test = self.registers[base + a as usize];
                    if test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        pc = (pc as isize + offset) as usize;
                    }
                }

                Op::Cons => {
                    let car = self.registers[base + b as usize];
                    let cdr = self.registers[base + c as usize];
                    self.registers[base + a as usize] = heap.cons(car, cdr);
                }

                Op::Eq => {
                    let va = self.registers[base + b as usize];
                    let vb = self.registers[base + c as usize];
                    self.registers[base + a as usize] = Value::boolean(va == vb);
                }

                Op::MakeObj => {
                    let parent = self.registers[base + b as usize];
                    let nslots = c as usize;
                    // slot name/value pairs follow in subsequent instructions
                    let mut slot_names = Vec::with_capacity(nslots);
                    let mut slot_values = Vec::with_capacity(nslots);
                    for _ in 0..nslots {
                        if pc + 3 >= code.len() { break; }
                        let name_const = u16::from_be_bytes([code[pc], code[pc + 1]]) as usize;
                        let val_reg = code[pc + 2] as usize;
                        pc += 4;
                        let name_sym = Value::from_bits(constants[name_const]).as_symbol()
                            .ok_or("make_obj: slot name is not a symbol")?;
                        slot_names.push(name_sym);
                        slot_values.push(self.registers[base + val_reg]);
                    }
                    self.registers[base + a as usize] =
                        heap.make_object_with_slots(parent, slot_names, slot_values);
                }

                Op::SetSlot => {
                    let obj_id = self.registers[base + a as usize].as_any_object()
                        .ok_or("set_slot: not an object")?;
                    let name_const = b as usize;
                    let name_sym = Value::from_bits(constants[name_const]).as_symbol()
                        .ok_or("set_slot: name is not a symbol")?;
                    let val = self.registers[base + c as usize];
                    heap.get_mut(obj_id).slot_set(name_sym, val);
                }

                Op::SetHandler => {
                    let obj_id = self.registers[base + a as usize].as_any_object()
                        .ok_or("set_handler: not an object")?;
                    let sel_const = b as usize;
                    let sel_sym = Value::from_bits(constants[sel_const]).as_symbol()
                        .ok_or("set_handler: selector is not a symbol")?;
                    let handler = self.registers[base + c as usize];
                    heap.get_mut(obj_id).handler_set(sel_sym, handler);
                }

                Op::MakeTable => {
                    let nseq = b as usize;
                    let nmap = c as usize;
                    let total_regs = nseq + nmap * 2;
                    let padded = (total_regs + 3) & !3; // round up to 4

                    let mut seq = Vec::with_capacity(nseq);
                    for i in 0..nseq {
                        seq.push(self.registers[base + code[pc + i] as usize]);
                    }
                    let mut map = Vec::with_capacity(nmap);
                    for i in 0..nmap {
                        let ki = nseq + i * 2;
                        let vi = nseq + i * 2 + 1;
                        let key = self.registers[base + code[pc + ki] as usize];
                        let val = self.registers[base + code[pc + vi] as usize];
                        map.push((key, val));
                    }
                    pc += padded;

                    self.registers[base + a as usize] =
                        heap.alloc_val(crate::object::HeapObject::Table { seq, map });
                }

                Op::MakeClosure => {
                    let raw_idx = u16::from_be_bytes([b, c]) as usize;
                    let idx = raw_idx + self.desc_base;
                    if idx < self.closure_descs.len() {
                        let desc = &self.closure_descs[idx];
                        let arity = desc.chunk.arity;
                        let is_op = desc.is_operative;
                        let parent_regs = desc.capture_parent_regs.clone();
                        let capture_names = desc.capture_names.clone();
                        let cap_pairs: Vec<(u32, Value)> = capture_names.iter().zip(parent_regs.iter())
                            .map(|(&name, &r)| (name, self.registers[base + r as usize]))
                            .collect();
                        let closure = heap.make_closure(idx, arity, is_op, &cap_pairs);
                        self.registers[base + a as usize] = closure;
                    } else {
                        return Err(format!("MakeClosure: desc index {idx} out of bounds"));
                    }
                }

                Op::Eval => {
                    // eval AST, optionally in a local environment
                    // a = dst, b = AST, c = env (0 = no env register, else register index)
                    let ast = self.registers[base + b as usize];
                    let env_val = if c != 0 { self.registers[base + c as usize] } else { Value::NIL };

                    // if env is an object, temporarily inject its slots as globals
                    let mut injected_keys: Vec<u32> = Vec::new();
                    let mut saved_values: Vec<(u32, Option<Value>)> = Vec::new();
                    if let Some(env_id) = env_val.as_any_object() {
                        let slot_names = heap.get(env_id).slot_names();
                        let slot_vals: Vec<Value> = slot_names.iter()
                            .map(|&n| heap.get(env_id).slot_get(n).unwrap_or(Value::NIL))
                            .collect();
                        for (&name, &val) in slot_names.iter().zip(slot_vals.iter()) {
                            saved_values.push((name, heap.globals.get(&name).copied()));
                            heap.globals.insert(name, val);
                            injected_keys.push(name);
                        }
                    }

                    let compile_result = crate::lang::compiler::Compiler::compile_toplevel(heap, ast)
                        .map_err(|e| format!("eval compile: {e}"))?;
                    let result = self.eval_result(heap, compile_result);

                    // restore globals
                    for (name, old_val) in saved_values {
                        match old_val {
                            Some(v) => { heap.globals.insert(name, v); }
                            None => { heap.globals.remove(&name); }
                        }
                    }

                    self.registers[base + a as usize] = result?;
                }

                Op::GetGlobal => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let name_sym = Value::from_bits(constants[idx]).as_symbol()
                        .ok_or("get_global: name constant is not a symbol")?;
                    let val = heap.globals.get(&name_sym).copied()
                        .ok_or_else(|| format!("unbound: '{}'", heap.symbol_name(name_sym)))?;
                    self.registers[base + a as usize] = val;
                }

                Op::DefGlobal => {
                    let idx = u16::from_be_bytes([a, b]) as usize;
                    let name_sym = Value::from_bits(constants[idx]).as_symbol()
                        .ok_or("def_global: name constant is not a symbol")?;
                    let val = self.registers[base + c as usize];
                    heap.globals.insert(name_sym, val);
                    // check if the value is an operative closure — mark it
                    if let Some((_, true)) = heap.as_closure(val) {
                        heap.operatives.insert(name_sym);
                    }
                }

                Op::TryCatch => {
                    // TryCatch dst, body_reg, handler_reg
                    // call body (a zero-arg closure), on error call handler with Error object
                    let body = self.registers[base + b as usize];
                    let handler = self.registers[base + c as usize];
                    let result = self.dispatch_send(heap, body, heap.sym_call, &[]);
                    match result {
                        Ok(val) => self.registers[base + a as usize] = val,
                        Err(msg) => {
                            let error_obj = heap.make_error(&msg);
                            let arg_list = heap.list(&[error_obj]);
                            let catch_result = self.dispatch_send(heap, handler, heap.sym_call, &[arg_list]);
                            self.registers[base + a as usize] = catch_result?;
                        }
                    }
                }

                Op::Throw => {
                    // Throw src — signal an error with value in src
                    let val = self.registers[base + a as usize];
                    let msg = if let Some(id) = val.as_any_object() {
                        match heap.get(id) {
                            crate::object::HeapObject::Text(s) => s.clone(),
                            _ => heap.format_value(val),
                        }
                    } else {
                        heap.format_value(val)
                    };
                    return Err(msg);
                }

                _ => {
                    return Err(format!("unimplemented opcode: {opcode:?}"));
                }
            }
        }
    }

    /// Call a closure that is a heap object.
    fn call_closure_obj(&mut self, heap: &mut Heap, closure_val: Value, code_idx: usize, args: &[Value]) -> Result<Value, String> {
        if code_idx >= self.closure_descs.len() {
            return Err(format!("closure code_idx {} out of bounds (have {})", code_idx, self.closure_descs.len()));
        }

        // read captures from the heap object, build (local_reg, value) pairs
        let capture_local_regs = self.closure_descs[code_idx].capture_local_regs.clone();
        let captures_from_obj = heap.closure_captures(closure_val);
        let mut cap_reg_vals: Vec<(u8, Value)> = Vec::new();
        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
            if i < capture_local_regs.len() {
                cap_reg_vals.push((capture_local_regs[i], *val));
            }
        }

        // delegate to call_closure with captures pre-loaded
        self.closure_descs[code_idx].capture_values = captures_from_obj.iter().map(|(_, v)| *v).collect();
        self.call_closure(heap, code_idx, args)
    }

    /// Call a closure by its descriptor index.
    fn call_closure(&mut self, heap: &mut Heap, desc_idx: usize, args: &[Value]) -> Result<Value, String> {
        if desc_idx >= self.closure_descs.len() {
            return Err(format!("closure index {} out of bounds (have {})", desc_idx, self.closure_descs.len()));
        }
        // clone chunk + capture data to avoid borrow conflicts
        let chunk = self.closure_descs[desc_idx].chunk.clone();
        let captures = self.closure_descs[desc_idx].capture_values.clone();
        let closure_desc_base = self.closure_descs[desc_idx].desc_base;
        let capture_local_regs = self.closure_descs[desc_idx].capture_local_regs.clone();
        // save and set desc_base for this closure's context
        let saved_desc_base = self.desc_base;
        self.desc_base = closure_desc_base;

        // use a LOCAL register array
        let arity = chunk.arity as usize;
        let rest_reg = self.closure_descs[desc_idx].rest_param_reg;
        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 16];
        // args[0] is the cons list of actual arguments — unpack into param registers
        let arg_list = args.first().copied().unwrap_or(Value::NIL);
        let unpacked = heap.list_to_vec(arg_list);
        // load positional params
        for i in 0..arity.min(unpacked.len()) {
            regs[i] = unpacked[i];
        }
        // load rest param if present
        if let Some(rest_r) = rest_reg {
            // collect remaining args into a cons list
            let rest_args: Vec<Value> = unpacked.iter().skip(arity).copied().collect();
            regs[rest_r as usize] = heap.list(&rest_args);
        }
        // load captured values into their allocated registers
        // (the compiler allocated local regs for captured vars after params)
        // find where the capture regs start — they were allocated AFTER params
        // load captured values into their actual compiler-assigned registers
        for (i, val) in captures.iter().enumerate() {
            if i < capture_local_regs.len() {
                let reg = capture_local_regs[i] as usize;
                if reg < regs.len() {
                    regs[reg] = *val;
                }
            }
        }

        // execute the closure's chunk
        let mut pc = 0;
        let code = &chunk.code;
        let constants = &chunk.constants;
        let base = 0;

        let result = loop {
            if pc + 3 >= code.len() {
                break Ok(regs[0]);
            }
            let op = code[pc];
            let a = code[pc + 1];
            let b = code[pc + 2];
            let c = code[pc + 3];
            pc += 4;

            let Some(opcode) = Op::from_u8(op) else {
                break Err(format!("unknown opcode in closure: {op}"));
            };

            match opcode {
                Op::Return => break Ok(regs[base + a as usize]),
                Op::LoadConst => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    regs[base + a as usize] = Value::from_bits(constants[idx]);
                }
                Op::LoadNil => regs[base + a as usize] = Value::NIL,
                Op::LoadTrue => regs[base + a as usize] = Value::TRUE,
                Op::LoadFalse => regs[base + a as usize] = Value::FALSE,
                Op::LoadInt => {
                    let val = i16::from_be_bytes([b, c]) as i64;
                    regs[base + a as usize] = Value::integer(val);
                }
                Op::Move => regs[base + a as usize] = regs[base + b as usize],
                Op::GetGlobal => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let name_sym = Value::from_bits(constants[idx]).as_symbol()
                        .ok_or("get_global: not a symbol")?;
                    let val = heap.globals.get(&name_sym).copied()
                        .ok_or_else(|| format!("unbound: '{}'", heap.symbol_name(name_sym)))?;
                    regs[base + a as usize] = val;
                }
                Op::Send => {
                    let dst = a as usize;
                    let recv = regs[base + b as usize];
                    let sel_idx = c as usize;
                    let sel_sym = Value::from_bits(constants[sel_idx]).as_symbol()
                        .ok_or("send: selector not a symbol")?;
                    if pc + 3 >= code.len() { break Err("send: truncated".into()); }
                    let nargs = code[pc] as usize;
                    let arg_start = pc + 1;
                    pc += 4;
                    let mut send_args = Vec::with_capacity(nargs);
                    for i in 0..nargs.min(3) {
                        send_args.push(regs[base + code[arg_start + i] as usize]);
                    }
                    let result = self.dispatch_send(heap, recv, sel_sym, &send_args)?;
                    regs[base + dst] = result;
                }
                Op::Call => {
                    // legacy — just delegate to send call:
                    let dst = a as usize;
                    let func = regs[base + b as usize];
                    let nargs = c as usize;
                    let mut call_args = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        call_args.push(regs[base + b as usize + 1 + i]);
                    }
                    let res = self.dispatch_send(heap, func, heap.sym_call, &call_args)?;
                    regs[base + dst] = res;
                }
                Op::Eq => {
                    let va = regs[base + b as usize];
                    let vb = regs[base + c as usize];
                    regs[base + a as usize] = Value::boolean(va == vb);
                }
                Op::Cons => {
                    let car = regs[base + b as usize];
                    let cdr = regs[base + c as usize];
                    regs[base + a as usize] = heap.cons(car, cdr);
                }
                Op::JumpIfFalse => {
                    let test = regs[base + a as usize];
                    if !test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        pc = (pc as isize + offset) as usize;
                    }
                }
                Op::JumpIfTrue => {
                    let test = regs[base + a as usize];
                    if test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        pc = (pc as isize + offset) as usize;
                    }
                }
                Op::Jump => {
                    let offset = i16::from_be_bytes([a, b]) as isize;
                    pc = (pc as isize + offset) as usize;
                }
                Op::MakeTable => {
                    let nseq = b as usize;
                    let nmap = c as usize;
                    let total_regs = nseq + nmap * 2;
                    let padded = (total_regs + 3) & !3;
                    let mut seq = Vec::with_capacity(nseq);
                    for i in 0..nseq {
                        seq.push(regs[base + code[pc + i] as usize]);
                    }
                    let mut map = Vec::with_capacity(nmap);
                    for i in 0..nmap {
                        let ki = nseq + i * 2;
                        let vi = nseq + i * 2 + 1;
                        let key = regs[base + code[pc + ki] as usize];
                        let val = regs[base + code[pc + vi] as usize];
                        map.push((key, val));
                    }
                    pc += padded;
                    regs[base + a as usize] =
                        heap.alloc_val(crate::object::HeapObject::Table { seq, map });
                }
                Op::Eval => {
                    let ast = regs[base + b as usize];
                    let env_val = if c != 0 { regs[base + c as usize] } else { Value::NIL };

                    let mut saved_values: Vec<(u32, Option<Value>)> = Vec::new();
                    if let Some(env_id) = env_val.as_any_object() {
                        let slot_names = heap.get(env_id).slot_names();
                        let slot_vals: Vec<Value> = slot_names.iter()
                            .map(|&n| heap.get(env_id).slot_get(n).unwrap_or(Value::NIL))
                            .collect();
                        for (&name, &val) in slot_names.iter().zip(slot_vals.iter()) {
                            saved_values.push((name, heap.globals.get(&name).copied()));
                            heap.globals.insert(name, val);
                        }
                    }

                    let compile_result = crate::lang::compiler::Compiler::compile_toplevel(heap, ast)
                        .map_err(|e| format!("eval compile: {e}"))?;
                    let result = self.eval_result(heap, compile_result);

                    for (name, old_val) in saved_values {
                        match old_val {
                            Some(v) => { heap.globals.insert(name, v); }
                            None => { heap.globals.remove(&name); }
                        }
                    }

                    regs[base + a as usize] = result?;
                }
                Op::DefGlobal => {
                    let idx = u16::from_be_bytes([a, b]) as usize;
                    let name_sym = Value::from_bits(constants[idx]).as_symbol()
                        .ok_or("def_global: not a symbol")?;
                    let val = regs[base + c as usize];
                    heap.globals.insert(name_sym, val);
                    if let Some((_, true)) = heap.as_closure(val) {
                        heap.operatives.insert(name_sym);
                    }
                }
                Op::MakeObj => {
                    let parent = regs[base + b as usize];
                    let nslots = c as usize;
                    let mut slot_names = Vec::with_capacity(nslots);
                    let mut slot_values = Vec::with_capacity(nslots);
                    for _ in 0..nslots {
                        if pc + 3 >= code.len() { break; }
                        let nc = u16::from_be_bytes([code[pc], code[pc + 1]]) as usize;
                        let vr = code[pc + 2] as usize;
                        pc += 4;
                        let ns = Value::from_bits(constants[nc]).as_symbol()
                            .ok_or("make_obj: slot name not a symbol")?;
                        slot_names.push(ns);
                        slot_values.push(regs[base + vr]);
                    }
                    regs[base + a as usize] =
                        heap.make_object_with_slots(parent, slot_names, slot_values);
                }
                Op::SetHandler => {
                    let obj_id = regs[base + a as usize].as_any_object()
                        .ok_or("set_handler: not an object")?;
                    let sel_const = b as usize;
                    let sel_sym = Value::from_bits(constants[sel_const]).as_symbol()
                        .ok_or("set_handler: selector not a symbol")?;
                    let handler = regs[base + c as usize];
                    heap.get_mut(obj_id).handler_set(sel_sym, handler);
                }
                Op::MakeClosure => {
                    let raw_idx = u16::from_be_bytes([b, c]) as usize;
                    let idx = raw_idx + self.desc_base;
                    if idx < self.closure_descs.len() {
                        let desc = &self.closure_descs[idx];
                        let arity = desc.chunk.arity;
                        let is_op = desc.is_operative;
                        let parent_regs_v = desc.capture_parent_regs.clone();
                        let capture_names_v = desc.capture_names.clone();
                        let cap_pairs: Vec<(u32, Value)> = capture_names_v.iter().zip(parent_regs_v.iter())
                            .map(|(&name, &r)| (name, regs[base + r as usize]))
                            .collect();
                        let closure = heap.make_closure(idx, arity, is_op, &cap_pairs);
                        regs[base + a as usize] = closure;
                    } else {
                        break Err(format!("MakeClosure: idx {idx} out of bounds"));
                    }
                }
                Op::TryCatch => {
                    let body = regs[base + b as usize];
                    let handler = regs[base + c as usize];
                    let result = self.dispatch_send(heap, body, heap.sym_call, &[]);
                    match result {
                        Ok(val) => regs[base + a as usize] = val,
                        Err(msg) => {
                            let error_obj = heap.make_error(&msg);
                            let arg_list = heap.list(&[error_obj]);
                            let catch_result = self.dispatch_send(heap, handler, heap.sym_call, &[arg_list]);
                            regs[base + a as usize] = catch_result?;
                        }
                    }
                }
                Op::Throw => {
                    let val = regs[base + a as usize];
                    let msg = if let Some(id) = val.as_any_object() {
                        match heap.get(id) {
                            crate::object::HeapObject::Text(s) => s.clone(),
                            _ => heap.format_value(val),
                        }
                    } else {
                        heap.format_value(val)
                    };
                    break Err(msg);
                }
                _ => break Err(format!("unimplemented in closure: {opcode:?}")),
            }
        };

        self.desc_base = saved_desc_base;
        result
    }

    /// Dispatch a message send, handling both native and bytecode handlers.
    fn dispatch_send(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        match dispatch::lookup_handler(heap, receiver, selector) {
            Ok((handler, _)) => self.call_handler(heap, handler, receiver, selector, args),
            Err(err) => {
                // try doesNotUnderstand: before propagating (avoid infinite recursion)
                if selector != heap.sym_dnu {
                    if let Ok((dnu_handler, _)) = dispatch::lookup_handler(heap, receiver, heap.sym_dnu) {
                        let sel_sym = Value::symbol(selector);
                        let args_list = heap.list(args);
                        return self.call_handler(heap, dnu_handler, receiver, heap.sym_dnu, &[sel_sym, args_list]);
                    }
                }
                Err(err)
            }
        }
    }

    /// Call a resolved handler (native or bytecode closure).
    fn call_handler(&mut self, heap: &mut Heap, handler: Value, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        if dispatch::is_native(heap, handler) {
            dispatch::call_native(heap, handler, receiver, args)
        } else if let Some((code_idx, _)) = heap.as_closure(handler) {
            if selector == heap.sym_call {
                self.call_closure_obj(heap, handler, code_idx, args)
            } else {
                let mut full_args = vec![receiver];
                full_args.extend_from_slice(args);
                let arg_list = heap.list(&full_args);
                self.call_closure_obj(heap, handler, code_idx, &[arg_list])
            }
        } else {
            Err(format!("handler {:?} is not callable", handler))
        }
    }
}

impl VM {
    /// Public interface: send a message to a value, used by REPL for show protocol etc.
    pub fn send_message(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        self.dispatch_send(heap, receiver, selector, args)
    }

    /// Evaluate a CompileResult, accumulating closure descs across calls.
    pub fn eval_result(&mut self, heap: &mut Heap, result: CompileResult) -> Result<Value, String> {
        // merge new closure descs — offset any MakeClosure indices in the chunk
        let base_idx = self.closure_descs.len();
        self.closure_descs.extend(result.closure_descs);
        // if base_idx > 0 we'd need to patch MakeClosure operands, but for now
        // each compile_toplevel starts from 0 — we rely on the compiler
        // producing indices relative to its own descs, then offset here
        let chunk = result.chunk;
        // set desc_base on each newly added desc
        for i in base_idx..self.closure_descs.len() {
            self.closure_descs[i].desc_base = base_idx;
        }
        self.desc_base = base_idx;
        self.execute(heap, &chunk, Value::NIL)
    }
}

/// Convenience: evaluate a chunk in a fresh VM (for tests).
pub fn eval_chunk(heap: &mut Heap, chunk: &Chunk) -> Result<Value, String> {
    let mut vm = VM::new();
    vm.execute(heap, chunk, Value::NIL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_and_return() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 1;
        let idx = chunk.add_constant(Value::integer(42).to_bits());
        chunk.emit(Op::LoadConst, 0, (idx >> 8) as u8, idx as u8);
        chunk.emit(Op::Return, 0, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }

    #[test]
    fn eq_test() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 3;
        chunk.emit(Op::LoadInt, 0, 0, 5); // r0 = 5
        chunk.emit(Op::LoadInt, 1, 0, 5); // r1 = 5
        chunk.emit(Op::Eq, 2, 0, 1);      // r2 = (r0 == r1)
        chunk.emit(Op::Return, 2, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        assert!(result.is_true());
    }

    #[test]
    fn cons_test() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 3;
        chunk.emit(Op::LoadInt, 0, 0, 1); // r0 = 1
        chunk.emit(Op::LoadInt, 1, 0, 2); // r1 = 2
        chunk.emit(Op::Cons, 2, 0, 1);    // r2 = (1 . 2)
        chunk.emit(Op::Return, 2, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        let id = result.as_any_object().unwrap();
        assert_eq!(heap.car(id).as_integer(), Some(1));
        assert_eq!(heap.cdr(id).as_integer(), Some(2));
    }

    #[test]
    fn jump_if_false() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 2;
        chunk.emit(Op::LoadFalse, 0, 0, 0);       // r0 = false
        chunk.emit(Op::JumpIfFalse, 0, 0, 4);      // if !r0, skip 4 bytes (1 instr)
        chunk.emit(Op::LoadInt, 1, 0, 99);          // r1 = 99 (skipped)
        chunk.emit(Op::LoadInt, 1, 0, 42);          // r1 = 42 (landed here)
        chunk.emit(Op::Return, 1, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }
}
