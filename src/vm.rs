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
                    let dst = a as usize;
                    let func = self.registers[base + b as usize];
                    let nargs = c as usize;

                    let mut args = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        args.push(self.registers[base + b as usize + 1 + i]);
                    }

                    // check if func is a closure (negative integer hack)
                    if let Some(n) = func.as_integer() {
                        if n < 0 {
                            let idx = (-(n + 1)) as usize;
                            let result = self.call_closure(heap, idx, &args)?;
                            self.registers[base + dst] = result;
                            continue;
                        }
                    }

                    // check if func is a native (symbol pointing to a registered native)
                    if dispatch::is_native(heap, func) {
                        let result = dispatch::call_native(heap, func, Value::NIL, &args)?;
                        self.registers[base + dst] = result;
                        continue;
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
                    // capture values from current registers
                    if idx < self.closure_descs.len() {
                        let parent_regs = self.closure_descs[idx].capture_parent_regs.clone();
                        let captures: Vec<Value> = parent_regs.iter()
                            .map(|&r| self.registers[base + r as usize])
                            .collect();
                        self.closure_descs[idx].capture_values = captures;
                    }
                    self.registers[base + a as usize] = Value::integer(-(idx as i64) - 1);
                }

                Op::Eval => {
                    let ast = self.registers[base + b as usize];
                    let compile_result = crate::lang::compiler::Compiler::compile_toplevel(heap, ast)
                        .map_err(|e| format!("eval compile: {e}"))?;
                    let result = self.eval_result(heap, compile_result)?;
                    self.registers[base + a as usize] = result;
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
                    if let Some(n) = val.as_integer() {
                        if n < 0 {
                            let idx = (-(n + 1)) as usize;
                            if idx < self.closure_descs.len() && self.closure_descs[idx].is_operative {
                                heap.operatives.insert(name_sym);
                            }
                        }
                    }
                }

                _ => {
                    return Err(format!("unimplemented opcode: {opcode:?}"));
                }
            }
        }
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
        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 16]; // extra room for captures
        // load args into param registers
        for (i, arg) in args.iter().enumerate() {
            if i < regs.len() {
                regs[i] = *arg;
            }
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
                    let dst = a as usize;
                    let func = regs[base + b as usize];
                    let nargs = c as usize;
                    let mut call_args = Vec::with_capacity(nargs);
                    for i in 0..nargs {
                        call_args.push(regs[base + b as usize + 1 + i]);
                    }
                    if let Some(n) = func.as_integer() {
                        if n < 0 {
                            let idx = (-(n + 1)) as usize;
                            let res = self.call_closure(heap, idx, &call_args)?;
                            regs[base + dst] = res;
                            continue;
                        }
                    }
                    if dispatch::is_native(heap, func) {
                        let res = dispatch::call_native(heap, func, Value::NIL, &call_args)?;
                        regs[base + dst] = res;
                        continue;
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
                    let compile_result = crate::lang::compiler::Compiler::compile_toplevel(heap, ast)
                        .map_err(|e| format!("eval compile: {e}"))?;
                    let result = self.eval_result(heap, compile_result)?;
                    regs[base + a as usize] = result;
                }
                Op::DefGlobal => {
                    let idx = u16::from_be_bytes([a, b]) as usize;
                    let name_sym = Value::from_bits(constants[idx]).as_symbol()
                        .ok_or("def_global: not a symbol")?;
                    let val = regs[base + c as usize];
                    heap.globals.insert(name_sym, val);
                    if let Some(n) = val.as_integer() {
                        if n < 0 {
                            let ci = (-(n + 1)) as usize;
                            if ci < self.closure_descs.len() && self.closure_descs[ci].is_operative {
                                heap.operatives.insert(name_sym);
                            }
                        }
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
                    // capture from LOCAL regs
                    if idx < self.closure_descs.len() {
                        let parent_regs = self.closure_descs[idx].capture_parent_regs.clone();
                        let captures: Vec<Value> = parent_regs.iter()
                            .map(|&r| regs[base + r as usize])
                            .collect();
                        self.closure_descs[idx].capture_values = captures;
                    } else {
                    }
                    regs[base + a as usize] = Value::integer(-(idx as i64) - 1);
                }
                _ => break Err(format!("unimplemented in closure: {opcode:?}")),
            }
        };

        self.desc_base = saved_desc_base;
        result
    }

    /// Dispatch a message send, handling both native and bytecode handlers.
    fn dispatch_send(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        let (handler, _) = dispatch::lookup_handler(heap, receiver, selector)?;

        if dispatch::is_native(heap, handler) {
            dispatch::call_native(heap, handler, receiver, args)
        } else if let Some(n) = handler.as_integer() {
            if n < 0 {
                // closure handler — prepend receiver as first arg (self)
                let idx = (-(n + 1)) as usize;
                let mut full_args = vec![receiver];
                full_args.extend_from_slice(args);
                self.call_closure(heap, idx, &full_args)
            } else {
                Err(format!("handler is integer {n}, not callable"))
            }
        } else {
            Err(format!("handler {:?} is not callable", handler))
        }
    }
}

impl VM {
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
