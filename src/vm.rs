// The bytecode interpreter: register-based VM with frame stack.
//
// ONE opcode loop. Closure calls push frames, returns pop them.
// No duplicated opcode handling. Fuel counting and TCO built in.

use crate::dispatch;
use crate::heap::Heap;
use crate::lang::compiler::{ClosureDesc, CompileResult};
use crate::opcodes::{Chunk, Op};
use crate::value::Value;

/// Result of running the VM. Distinguishes normal completion, yield, and error.
pub enum RunResult {
    /// Normal completion with a value.
    Done(Value),
    /// Fuel exhausted. Frame stack is preserved — refuel and call run() again.
    Yielded,
    /// Unrecoverable error.
    Error(String),
}

struct Frame {
    regs: Vec<Value>,
    pc: usize,
    code: Vec<u8>,
    constants: Vec<u64>,
    desc_base: usize,
    result_reg: u8,     // register in CALLER's frame to store result
}

pub struct VM {
    frames: Vec<Frame>,
    closure_descs: Vec<ClosureDesc>,
    pub fuel: u64,      // 0 = unlimited
}

impl VM {
    pub fn new() -> Self {
        VM {
            frames: Vec::new(),
            closure_descs: Vec::new(),
            fuel: 0,
        }
    }

    pub fn add_closure_desc(&mut self, desc: ClosureDesc) {
        self.closure_descs.push(desc);
    }

    pub fn closure_descs_ref(&self) -> &[ClosureDesc] {
        &self.closure_descs
    }

    /// Execute a chunk, returning the result.
    pub fn execute(&mut self, heap: &mut Heap, chunk: &Chunk, _env: Value) -> Result<Value, String> {
        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 1];
        self.frames.push(Frame {
            regs: Vec::new(), // placeholder — we swap it in
            pc: 0,
            code: chunk.code.clone(),
            constants: chunk.constants.clone(),
            desc_base: self.current_desc_base(),
            result_reg: 0,
        });
        // swap regs in (avoid clone)
        std::mem::swap(&mut self.frames.last_mut().unwrap().regs, &mut regs);
        self.run(heap)
    }

    fn current_desc_base(&self) -> usize {
        if let Some(f) = self.frames.last() {
            f.desc_base
        } else {
            0
        }
    }

    /// Push a closure frame. Returns Ok(()) if frame was pushed.
    fn push_closure_frame(
        &mut self,
        heap: &mut Heap,
        closure_val: Value,
        code_idx: usize,
        args: &[Value],
        result_reg: u8,
    ) -> Result<(), String> {
        if code_idx >= self.closure_descs.len() {
            return Err(format!("closure code_idx {} out of bounds (have {})", code_idx, self.closure_descs.len()));
        }

        let chunk = self.closure_descs[code_idx].chunk.clone();
        let closure_desc_base = self.closure_descs[code_idx].desc_base;
        let capture_local_regs = self.closure_descs[code_idx].capture_local_regs.clone();
        let rest_reg = self.closure_descs[code_idx].rest_param_reg;
        let is_operative = self.closure_descs[code_idx].is_operative;
        let arity = chunk.arity as usize;

        // read captures from the heap closure object
        let captures_from_obj = heap.closure_captures(closure_val);

        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 16];

        // unpack args: args[0] is the cons list of actual arguments
        let arg_list = args.first().copied().unwrap_or(Value::NIL);
        let unpacked = heap.list_to_vec(arg_list);

        if is_operative && rest_reg.is_some() && arity > 0 {
            // operative with rest param: $env is last positional, gets last arg (env).
            // rest param captures everything between positional params and env.
            let n_before_env = arity - 1;
            for i in 0..n_before_env.min(unpacked.len()) {
                regs[i] = unpacked[i];
            }
            // last positional ($e) gets last element of args (the env)
            if !unpacked.is_empty() {
                regs[arity - 1] = *unpacked.last().unwrap();
            }
            // rest param captures the middle (operands after positionals, before env)
            if let Some(rest_r) = rest_reg {
                let start = n_before_env;
                let end = if unpacked.len() > 0 { unpacked.len() - 1 } else { 0 };
                let rest_args: Vec<Value> = if start < end {
                    unpacked[start..end].to_vec()
                } else {
                    Vec::new()
                };
                regs[rest_r as usize] = heap.list(&rest_args);
            }
        } else {
            // normal case: fill positional params from start, rest gets remainder
            for i in 0..arity.min(unpacked.len()) {
                regs[i] = unpacked[i];
            }
            if let Some(rest_r) = rest_reg {
                let rest_args: Vec<Value> = unpacked.iter().skip(arity).copied().collect();
                regs[rest_r as usize] = heap.list(&rest_args);
            }
        }
        // load captured values into their compiler-assigned registers
        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
            if i < capture_local_regs.len() {
                let reg = capture_local_regs[i] as usize;
                if reg < regs.len() {
                    regs[reg] = *val;
                }
            }
        }

        self.frames.push(Frame {
            regs,
            pc: 0,
            code: chunk.code.clone(),
            constants: chunk.constants.clone(),
            desc_base: closure_desc_base,
            result_reg,
        });
        Ok(())
    }

    /// The ONE opcode loop. Reads from the current (top) frame.
    /// Runs until the frame stack returns to base_depth.
    fn run(&mut self, heap: &mut Heap) -> Result<Value, String> {
        let base_depth = self.frames.len() - 1; // the frame we just pushed
        loop {
            // fuel counting — yield preserves frame stack
            if self.fuel > 0 {
                self.fuel -= 1;
                if self.fuel == 0 {
                    return Err("__yield__".into());
                }
            }

            let depth = self.frames.len();
            if depth == 0 {
                return Ok(Value::NIL);
            }
            let f = self.frames.last_mut().unwrap();
            let pc = f.pc;

            if pc + 3 >= f.code.len() {
                // end of code without Return — return regs[0]
                let val = f.regs[0];
                let result_reg = f.result_reg;
                self.frames.pop();
                if self.frames.len() <= base_depth {
                    return Ok(val);
                }
                self.frames.last_mut().unwrap().regs[result_reg as usize] = val;
                continue;
            }

            let op = f.code[pc];
            let a = f.code[pc + 1];
            let b = f.code[pc + 2];
            let c = f.code[pc + 3];
            f.pc += 4;

            let Some(opcode) = Op::from_u8(op) else {
                return Err(format!("unknown opcode: {op}"));
            };

            match opcode {
                Op::LoadConst => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let val = Value::from_bits(f.constants[idx]);
                    f.regs[a as usize] = val;
                }
                Op::LoadNil => f.regs[a as usize] = Value::NIL,
                Op::LoadTrue => f.regs[a as usize] = Value::TRUE,
                Op::LoadFalse => f.regs[a as usize] = Value::FALSE,
                Op::Move => f.regs[a as usize] = f.regs[b as usize],
                Op::LoadInt => {
                    let val = i16::from_be_bytes([b, c]) as i64;
                    f.regs[a as usize] = Value::integer(val);
                }

                Op::Return => {
                    let val = f.regs[a as usize];
                    let result_reg = f.result_reg;
                    self.frames.pop();
                    if self.frames.len() <= base_depth {
                        // returned from the frame we were called to run
                        return Ok(val);
                    }
                    self.frames.last_mut().unwrap().regs[result_reg as usize] = val;
                    continue;
                }

                Op::Send => {
                    let dst = a;
                    let recv = f.regs[b as usize];
                    let sel_idx = c as usize;
                    let sel_sym = if sel_idx < f.constants.len() {
                        Value::from_bits(f.constants[sel_idx]).as_symbol()
                            .ok_or("send: selector constant is not a symbol")?
                    } else {
                        return Err("send: selector constant out of bounds".into());
                    };

                    if f.pc + 3 >= f.code.len() {
                        return Err("send: truncated nargs".into());
                    }
                    let nargs = f.code[f.pc] as usize;
                    let arg_start = f.pc + 1;
                    let mut send_args = Vec::with_capacity(nargs);
                    for i in 0..nargs.min(3) {
                        send_args.push(f.regs[f.code[arg_start + i] as usize]);
                    }
                    f.pc += 4;

                    // look up handler
                    let lookup = dispatch::lookup_handler(heap, recv, sel_sym);
                    let (handler, _) = match lookup {
                        Ok(h) => h,
                        Err(err) => {
                            // try doesNotUnderstand:
                            if sel_sym != heap.sym_dnu {
                                if let Ok((dnu_handler, _)) = dispatch::lookup_handler(heap, recv, heap.sym_dnu) {
                                    let s = Value::symbol(sel_sym);
                                    let al = heap.list(&send_args);
                                    // DNU: call as recursive (simple path)
                                    let result = self.call_handler_recursive(heap, dnu_handler, recv, heap.sym_dnu, &[s, al])?;
                                    self.frames.last_mut().unwrap().regs[dst as usize] = result;
                                    continue;
                                }
                            }
                            return Err(err);
                        }
                    };

                    // dispatch: native or closure?
                    if dispatch::is_native(heap, handler) {
                        let result = dispatch::call_native(heap, handler, recv, &send_args)?;
                        self.frames.last_mut().unwrap().regs[dst as usize] = result;
                    } else if let Some((code_idx, _)) = heap.as_closure(handler) {
                        // closure call — push frame!
                        let arg_list = if sel_sym == heap.sym_call {
                            // call: — args[0] is the args list, pass directly
                            send_args.first().copied().unwrap_or(Value::NIL)
                        } else {
                            // method: prepend receiver as self
                            let mut full = vec![recv];
                            full.extend_from_slice(&send_args);
                            heap.list(&full)
                        };
                        self.push_closure_frame(heap, handler, code_idx, &[arg_list], dst)?;
                    } else {
                        return Err(format!("handler is not callable"));
                    }
                }

                Op::TailCall => {
                    // TailCall: same as Send but reuses the current frame for closures.
                    // This turns recursive calls into O(1) frame usage.
                    let dst = a;
                    let recv = f.regs[b as usize];
                    let sel_idx = c as usize;
                    let sel_sym = if sel_idx < f.constants.len() {
                        Value::from_bits(f.constants[sel_idx]).as_symbol()
                            .ok_or("tail_call: selector not a symbol")?
                    } else {
                        return Err("tail_call: selector out of bounds".into());
                    };

                    if f.pc + 3 >= f.code.len() {
                        return Err("tail_call: truncated".into());
                    }
                    let nargs = f.code[f.pc] as usize;
                    let arg_start = f.pc + 1;
                    let mut send_args = Vec::with_capacity(nargs);
                    for i in 0..nargs.min(3) {
                        send_args.push(f.regs[f.code[arg_start + i] as usize]);
                    }
                    f.pc += 4;

                    let lookup = dispatch::lookup_handler(heap, recv, sel_sym);
                    let (handler, _) = match lookup {
                        Ok(h) => h,
                        Err(err) => {
                            if sel_sym != heap.sym_dnu {
                                if let Ok((dnu_handler, _)) = dispatch::lookup_handler(heap, recv, heap.sym_dnu) {
                                    let s = Value::symbol(sel_sym);
                                    let al = heap.list(&send_args);
                                    let result = self.call_handler_recursive(heap, dnu_handler, recv, heap.sym_dnu, &[s, al])?;
                                    self.frames.last_mut().unwrap().regs[dst as usize] = result;
                                    continue;
                                }
                            }
                            return Err(err);
                        }
                    };

                    if dispatch::is_native(heap, handler) {
                        // native: call directly, store result, then the Return after will pop
                        let result = dispatch::call_native(heap, handler, recv, &send_args)?;
                        self.frames.last_mut().unwrap().regs[dst as usize] = result;
                    } else if let Some((code_idx, _)) = heap.as_closure(handler) {
                        // closure: REPLACE current frame instead of pushing
                        let arg_list = if sel_sym == heap.sym_call {
                            send_args.first().copied().unwrap_or(Value::NIL)
                        } else {
                            let mut full = vec![recv];
                            full.extend_from_slice(&send_args);
                            heap.list(&full)
                        };
                        // build new frame contents
                        if code_idx >= self.closure_descs.len() {
                            return Err(format!("tail_call: code_idx {} out of bounds", code_idx));
                        }
                        let chunk = self.closure_descs[code_idx].chunk.clone();
                        let closure_desc_base = self.closure_descs[code_idx].desc_base;
                        let capture_local_regs = self.closure_descs[code_idx].capture_local_regs.clone();
                        let rest_reg = self.closure_descs[code_idx].rest_param_reg;
                        let is_operative = self.closure_descs[code_idx].is_operative;
                        let arity = chunk.arity as usize;
                        let captures_from_obj = heap.closure_captures(handler);

                        let f = self.frames.last_mut().unwrap();
                        // reuse the frame: reset everything
                        f.regs.clear();
                        f.regs.resize(chunk.num_regs as usize + 16, Value::NIL);
                        f.pc = 0;
                        f.code = chunk.code.clone();
                        f.constants = chunk.constants.clone();
                        f.desc_base = closure_desc_base;
                        // result_reg stays the same (we're replacing, not pushing)

                        // unpack args
                        let unpacked = heap.list_to_vec(arg_list);
                        if is_operative && rest_reg.is_some() && arity > 0 {
                            let n_before_env = arity - 1;
                            for i in 0..n_before_env.min(unpacked.len()) {
                                f.regs[i] = unpacked[i];
                            }
                            if !unpacked.is_empty() {
                                f.regs[arity - 1] = *unpacked.last().unwrap();
                            }
                            if let Some(rest_r) = rest_reg {
                                let start = n_before_env;
                                let end = if unpacked.len() > 0 { unpacked.len() - 1 } else { 0 };
                                let rest_args: Vec<Value> = if start < end {
                                    unpacked[start..end].to_vec()
                                } else {
                                    Vec::new()
                                };
                                f.regs[rest_r as usize] = heap.list(&rest_args);
                            }
                        } else {
                            for i in 0..arity.min(unpacked.len()) {
                                f.regs[i] = unpacked[i];
                            }
                            if let Some(rest_r) = rest_reg {
                                let rest_args: Vec<Value> = unpacked.iter().skip(arity).copied().collect();
                                f.regs[rest_r as usize] = heap.list(&rest_args);
                            }
                        }
                        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
                            if i < capture_local_regs.len() {
                                let reg = capture_local_regs[i] as usize;
                                if reg < f.regs.len() {
                                    f.regs[reg] = *val;
                                }
                            }
                        }
                        // continue loop — will execute the new frame's code
                    } else {
                        return Err(format!("handler is not callable"));
                    }
                }

                Op::Call => {
                    let dst = a;
                    let func = f.regs[b as usize];
                    let nargs = c as usize;
                    let mut call_args = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        call_args.push(f.regs[b as usize + 1 + i]);
                    }
                    let result = self.dispatch_send(heap, func, heap.sym_call, &call_args)?;
                    self.frames.last_mut().unwrap().regs[dst as usize] = result;
                }

                Op::Jump => {
                    let offset = i16::from_be_bytes([a, b]) as isize;
                    let f = self.frames.last_mut().unwrap();
                    f.pc = (f.pc as isize + offset) as usize;
                }
                Op::JumpIfFalse => {
                    let f = self.frames.last_mut().unwrap();
                    let test = f.regs[a as usize];
                    if !test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        f.pc = (f.pc as isize + offset) as usize;
                    }
                }
                Op::JumpIfTrue => {
                    let f = self.frames.last_mut().unwrap();
                    let test = f.regs[a as usize];
                    if test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        f.pc = (f.pc as isize + offset) as usize;
                    }
                }

                Op::Cons => {
                    let f = self.frames.last_mut().unwrap();
                    let car = f.regs[b as usize];
                    let cdr = f.regs[c as usize];
                    f.regs[a as usize] = heap.cons(car, cdr);
                }
                Op::Eq => {
                    let f = self.frames.last_mut().unwrap();
                    let va = f.regs[b as usize];
                    let vb = f.regs[c as usize];
                    f.regs[a as usize] = Value::boolean(va == vb);
                }

                Op::MakeObj => {
                    let f = self.frames.last_mut().unwrap();
                    let parent = f.regs[b as usize];
                    let clone_parent = (c & 0x80) != 0;
                    let nslots = (c & 0x7F) as usize;

                    // read explicitly provided slots from bytecode
                    let mut slot_names = Vec::with_capacity(nslots);
                    let mut slot_values = Vec::with_capacity(nslots);
                    for _ in 0..nslots {
                        if f.pc + 3 >= f.code.len() { break; }
                        let nc = u16::from_be_bytes([f.code[f.pc], f.code[f.pc + 1]]) as usize;
                        let vr = f.code[f.pc + 2] as usize;
                        f.pc += 4;
                        let ns = Value::from_bits(f.constants[nc]).as_symbol()
                            .ok_or("make_obj: slot name not a symbol")?;
                        slot_names.push(ns);
                        slot_values.push(f.regs[vr]);
                    }

                    if clone_parent {
                        // clone: copy parent's slots as defaults, overlay with provided slots
                        if let Some(pid) = parent.as_any_object() {
                            let parent_slot_names = heap.get(pid).slot_names();
                            let mut merged_names = Vec::new();
                            let mut merged_values = Vec::new();

                            // copy parent's slots (defaults)
                            for &pn in &parent_slot_names {
                                let pv = heap.get(pid).slot_get(pn).unwrap_or(Value::NIL);
                                merged_names.push(pn);
                                merged_values.push(pv);
                            }

                            // overlay with explicitly provided slots
                            for (i, &sn) in slot_names.iter().enumerate() {
                                if let Some(pos) = merged_names.iter().position(|&n| n == sn) {
                                    // override existing
                                    merged_values[pos] = slot_values[i];
                                } else {
                                    // new slot
                                    merged_names.push(sn);
                                    merged_values.push(slot_values[i]);
                                }
                            }
                            f.regs[a as usize] = heap.make_object_with_slots(parent, merged_names, merged_values);
                        } else {
                            // parent is not an object (e.g. nil) — just use provided slots
                            f.regs[a as usize] = heap.make_object_with_slots(parent, slot_names, slot_values);
                        }
                    } else {
                        // no clone — just delegate (old behavior)
                        f.regs[a as usize] = heap.make_object_with_slots(parent, slot_names, slot_values);
                    }
                }

                Op::SetSlot => {
                    let f = self.frames.last_mut().unwrap();
                    let obj_id = f.regs[a as usize].as_any_object()
                        .ok_or("set_slot: not an object")?;
                    let name_const = b as usize;
                    let name_sym = Value::from_bits(f.constants[name_const]).as_symbol()
                        .ok_or("set_slot: name is not a symbol")?;
                    let val = f.regs[c as usize];
                    heap.get_mut(obj_id).slot_set(name_sym, val);
                }

                Op::SetHandler => {
                    let f = self.frames.last_mut().unwrap();
                    let obj_id = f.regs[a as usize].as_any_object()
                        .ok_or("set_handler: not an object")?;
                    let sel_const = b as usize;
                    let sel_sym = Value::from_bits(f.constants[sel_const]).as_symbol()
                        .ok_or("set_handler: selector not a symbol")?;
                    let handler = f.regs[c as usize];
                    heap.get_mut(obj_id).handler_set(sel_sym, handler);
                }

                Op::MakeTable => {
                    let f = self.frames.last_mut().unwrap();
                    let nseq = b as usize;
                    let nmap = c as usize;
                    let total_regs = nseq + nmap * 2;
                    let padded = (total_regs + 3) & !3;
                    let mut seq = Vec::with_capacity(nseq);
                    for i in 0..nseq {
                        seq.push(f.regs[f.code[f.pc + i] as usize]);
                    }
                    let mut map = Vec::with_capacity(nmap);
                    for i in 0..nmap {
                        let ki = nseq + i * 2;
                        let vi = nseq + i * 2 + 1;
                        let key = f.regs[f.code[f.pc + ki] as usize];
                        let val = f.regs[f.code[f.pc + vi] as usize];
                        map.push((key, val));
                    }
                    f.pc += padded;
                    f.regs[a as usize] = heap.alloc_val(crate::object::HeapObject::Table { seq, map });
                }

                Op::MakeClosure => {
                    let f = self.frames.last_mut().unwrap();
                    let raw_idx = u16::from_be_bytes([b, c]) as usize;
                    let idx = raw_idx + f.desc_base;
                    if idx >= self.closure_descs.len() {
                        return Err(format!("MakeClosure: desc index {idx} out of bounds"));
                    }
                    let desc = &self.closure_descs[idx];
                    let arity = desc.chunk.arity;
                    let is_op = desc.is_operative;
                    let parent_regs = desc.capture_parent_regs.clone();
                    let capture_names = desc.capture_names.clone();
                    let f = self.frames.last_mut().unwrap();
                    let cap_pairs: Vec<(u32, Value)> = capture_names.iter().zip(parent_regs.iter())
                        .map(|(&name, &r)| (name, f.regs[r as usize]))
                        .collect();
                    let closure = heap.make_closure(idx, arity, is_op, &cap_pairs);
                    f.regs[a as usize] = closure;
                }

                Op::GetGlobal => {
                    let f = self.frames.last_mut().unwrap();
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let name_sym = Value::from_bits(f.constants[idx]).as_symbol()
                        .ok_or("get_global: name constant is not a symbol")?;
                    let val = heap.env_get(name_sym)
                        .ok_or_else(|| format!("unbound: '{}'", heap.symbol_name(name_sym)))?;
                    f.regs[a as usize] = val;
                }

                Op::DefGlobal => {
                    let f = self.frames.last_mut().unwrap();
                    let idx = u16::from_be_bytes([a, b]) as usize;
                    let name_sym = Value::from_bits(f.constants[idx]).as_symbol()
                        .ok_or("def_global: name constant is not a symbol")?;
                    let val = f.regs[c as usize];
                    if let Some(old) = heap.env_get(name_sym) {
                        if old != val { heap.rebound.insert(name_sym); }
                    }
                    heap.env_def(name_sym, val);
                }

                Op::Eval => {
                    let f = self.frames.last_mut().unwrap();
                    let ast = f.regs[b as usize];
                    let env_val = if c != 0 { f.regs[c as usize] } else { Value::NIL };

                    // temporarily inject env slots as bindings
                    let mut saved_values: Vec<(u32, Option<Value>)> = Vec::new();
                    if let Some(env_id) = env_val.as_any_object() {
                        let slot_names = heap.get(env_id).slot_names();
                        let slot_vals: Vec<Value> = slot_names.iter()
                            .map(|&n| heap.get(env_id).slot_get(n).unwrap_or(Value::NIL))
                            .collect();
                        for (&name, &val) in slot_names.iter().zip(slot_vals.iter()) {
                            saved_values.push((name, heap.env_get(name)));
                            heap.env_def(name, val);
                        }
                    }

                    let compile_result = crate::lang::compiler::Compiler::compile_toplevel(heap, ast)
                        .map_err(|e| format!("eval compile: {e}"))?;
                    let result = self.eval_result(heap, compile_result);

                    // restore bindings
                    for (name, old_val) in saved_values {
                        match old_val {
                            Some(v) => { heap.env_def(name, v); }
                            None => { heap.env_remove(name); }
                        }
                    }

                    self.frames.last_mut().unwrap().regs[a as usize] = result?;
                }

                Op::TryCatch => {
                    // TryCatch still uses recursive dispatch (it's a natural error boundary)
                    let f = self.frames.last_mut().unwrap();
                    let body = f.regs[b as usize];
                    let handler = f.regs[c as usize];
                    let result = self.dispatch_send(heap, body, heap.sym_call, &[]);
                    match result {
                        Ok(val) => self.frames.last_mut().unwrap().regs[a as usize] = val,
                        Err(msg) => {
                            let error_obj = heap.make_error(&msg);
                            let arg_list = heap.list(&[error_obj]);
                            let catch_result = self.dispatch_send(heap, handler, heap.sym_call, &[arg_list]);
                            self.frames.last_mut().unwrap().regs[a as usize] = catch_result?;
                        }
                    }
                }

                Op::Throw => {
                    let f = self.frames.last_mut().unwrap();
                    let val = f.regs[a as usize];
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

                _ => return Err(format!("unimplemented opcode: {opcode:?}")),
            }
        }
    }

    /// Recursive dispatch (used by TryCatch, Call opcode, and DNU).
    /// For most sends, the frame-based run() loop handles dispatch directly.
    fn dispatch_send(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        match dispatch::lookup_handler(heap, receiver, selector) {
            Ok((handler, _)) => self.call_handler_recursive(heap, handler, receiver, selector, args),
            Err(err) => {
                if selector != heap.sym_dnu {
                    if let Ok((dnu_handler, _)) = dispatch::lookup_handler(heap, receiver, heap.sym_dnu) {
                        let sel_sym = Value::symbol(selector);
                        let args_list = heap.list(args);
                        return self.call_handler_recursive(heap, dnu_handler, receiver, heap.sym_dnu, &[sel_sym, args_list]);
                    }
                }
                Err(err)
            }
        }
    }

    /// Call a handler recursively (pushes frame, runs, returns result).
    fn call_handler_recursive(&mut self, heap: &mut Heap, handler: Value, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        if dispatch::is_native(heap, handler) {
            dispatch::call_native(heap, handler, receiver, args)
        } else if let Some((code_idx, _)) = heap.as_closure(handler) {
            let arg_list = if selector == heap.sym_call {
                args.first().copied().unwrap_or(Value::NIL)
            } else {
                let mut full = vec![receiver];
                full.extend_from_slice(args);
                heap.list(&full)
            };
            self.push_closure_frame(heap, handler, code_idx, &[arg_list], 0)?;
            self.run(heap)
        } else {
            Err(format!("handler is not callable"))
        }
    }

    /// Public interface: send a message to a value.
    pub fn send_message(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        self.dispatch_send(heap, receiver, selector, args)
    }

    /// Check if the VM yielded (fuel exhausted, frame stack preserved).
    pub fn is_yielded(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Resume execution after yield. Refuel and continue from where we stopped.
    pub fn resume(&mut self, heap: &mut Heap, fuel: u64) -> Result<Value, String> {
        self.fuel = fuel;
        self.run(heap)
    }

    /// Check if a result indicates a yield (vs a real error).
    pub fn is_yield_error(err: &str) -> bool {
        err == "__yield__"
    }

    /// Evaluate a CompileResult, accumulating closure descs.
    pub fn eval_result(&mut self, heap: &mut Heap, result: CompileResult) -> Result<Value, String> {
        let base_idx = self.closure_descs.len();
        self.closure_descs.extend(result.closure_descs);
        let chunk = result.chunk;
        for i in base_idx..self.closure_descs.len() {
            self.closure_descs[i].desc_base = base_idx;
        }
        // push frame with desc_base set correctly
        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 1];
        self.frames.push(Frame {
            regs,
            pc: 0,
            code: chunk.code.clone(),
            constants: chunk.constants.clone(),
            desc_base: base_idx,
            result_reg: 0,
        });
        self.run(heap)
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
        chunk.emit(Op::LoadInt, 0, 0, 5);
        chunk.emit(Op::LoadInt, 1, 0, 5);
        chunk.emit(Op::Eq, 2, 0, 1);
        chunk.emit(Op::Return, 2, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        assert!(result.is_true());
    }

    #[test]
    fn cons_test() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 3;
        chunk.emit(Op::LoadInt, 0, 0, 1);
        chunk.emit(Op::LoadInt, 1, 0, 2);
        chunk.emit(Op::Cons, 2, 0, 1);
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
        chunk.emit(Op::LoadFalse, 0, 0, 0);
        chunk.emit(Op::JumpIfFalse, 0, 0, 4);
        chunk.emit(Op::LoadInt, 1, 0, 99);
        chunk.emit(Op::LoadInt, 1, 0, 42);
        chunk.emit(Op::Return, 1, 0, 0);

        let result = eval_chunk(&mut heap, &chunk).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }
}
