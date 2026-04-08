//! Bytecode interpreter: register-based VM.
//!
//! Executes Chunk bytecode. Implements HandlerInvoker for the fabric's
//! dispatch system, so bytecode-compiled handlers participate in
//! message dispatch alongside native Rust handlers.

use moof_fabric::value::Value;

use crate::opcodes::{Chunk, Op};

/// A call frame in the VM.
struct Frame {
    chunk_id: u32,  // object ID of the Bytes object containing the chunk
    pc: usize,
    registers: Vec<Value>,
    return_reg: u8,
}

/// The moof bytecode VM.
pub struct VM {
    frames: Vec<Frame>,
}

impl VM {
    pub fn new() -> Self {
        VM {
            frames: Vec::new(),
        }
    }

    /// Execute a chunk, returning the result.
    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, String> {
        let mut registers = vec![Value::NIL; chunk.num_registers as usize + 1];
        let mut pc = 0;

        loop {
            if pc + 3 >= chunk.code.len() {
                return Ok(registers[0]);
            }

            let op = chunk.code[pc];
            let a = chunk.code[pc + 1];
            let b = chunk.code[pc + 2];
            let c = chunk.code[pc + 3];
            pc += 4;

            let Some(opcode) = Op::from_u8(op) else {
                return Err(format!("unknown opcode: {op}"));
            };

            match opcode {
                Op::LoadConst => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    if idx >= chunk.constants.len() {
                        return Err(format!("constant index out of bounds: {idx}"));
                    }
                    registers[a as usize] = Value::from_bits(chunk.constants[idx]);
                }
                Op::LoadNil => {
                    registers[a as usize] = Value::NIL;
                }
                Op::LoadTrue => {
                    registers[a as usize] = Value::TRUE;
                }
                Op::LoadFalse => {
                    registers[a as usize] = Value::FALSE;
                }
                Op::Move => {
                    registers[a as usize] = registers[b as usize];
                }
                Op::Return => {
                    return Ok(registers[a as usize]);
                }
                Op::Cons => {
                    // for now, just store as nil — needs store access
                    registers[a as usize] = Value::NIL;
                }
                Op::Eq => {
                    let va = registers[b as usize];
                    let vb = registers[c as usize];
                    registers[a as usize] = Value::boolean(va == vb);
                }
                Op::Jump => {
                    let offset = i16::from_be_bytes([a, b]);
                    pc = ((pc as i64) + (offset as i64)) as usize;
                }
                Op::JumpIfFalse => {
                    let test = registers[a as usize];
                    if !test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]);
                        pc = ((pc as i64) + (offset as i64)) as usize;
                    }
                }
                Op::JumpIfTrue => {
                    let test = registers[a as usize];
                    if test.is_truthy() {
                        let offset = i16::from_be_bytes([b, c]);
                        pc = ((pc as i64) + (offset as i64)) as usize;
                    }
                }
                Op::Halt => {
                    return Ok(registers[0]);
                }
                // TODO: implement remaining opcodes as the compiler emits them
                _ => {
                    return Err(format!("unimplemented opcode: {opcode:?}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::{Chunk, Op};

    #[test]
    fn load_and_return_integer() {
        let mut chunk = Chunk::new("test", 0);
        chunk.num_registers = 1;
        let idx = chunk.add_constant(Value::integer(42).to_bits());
        let [hi, lo] = idx.to_be_bytes();
        chunk.emit(Op::LoadConst, 0, hi, lo);
        chunk.emit(Op::Return, 0, 0, 0);

        let mut vm = VM::new();
        let result = vm.execute(&chunk).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }

    #[test]
    fn load_nil_and_return() {
        let mut chunk = Chunk::new("test", 0);
        chunk.num_registers = 1;
        chunk.emit(Op::LoadNil, 0, 0, 0);
        chunk.emit(Op::Return, 0, 0, 0);

        let mut vm = VM::new();
        let result = vm.execute(&chunk).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn eq_test() {
        let mut chunk = Chunk::new("test", 0);
        chunk.num_registers = 3;
        let c1 = chunk.add_constant(Value::integer(5).to_bits());
        let c2 = chunk.add_constant(Value::integer(5).to_bits());
        let [h1, l1] = c1.to_be_bytes();
        let [h2, l2] = c2.to_be_bytes();
        chunk.emit(Op::LoadConst, 0, h1, l1);
        chunk.emit(Op::LoadConst, 1, h2, l2);
        chunk.emit(Op::Eq, 2, 0, 1);
        chunk.emit(Op::Return, 2, 0, 0);

        let mut vm = VM::new();
        let result = vm.execute(&chunk).unwrap();
        assert!(result.is_true());
    }
}
