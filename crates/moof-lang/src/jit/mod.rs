// Cranelift-backed JIT compiler.
//
// Stage 0 (smoke tests): cranelift pipeline reachable, fn pointers
// callable.
//
// Stage 1 (this file): lower the simple opcodes to native machine
// code. Anything we can't yet handle deopts back to the interpreter
// at the offending PC. Jitted code never appears in production paths
// in this stage — it's only exercised by unit tests. Stage 2 will
// hook compiled chunks into push_closure_frame.
//
// Lowered opcodes (stage 1):
//
//   LoadNil / LoadTrue / LoadFalse  — store known bit pattern.
//   LoadInt                         — sign-extended i16 → i64 → Value::integer.
//   LoadConst                       — read from a constants slice we
//                                     pass in as a pointer arg.
//   Move                            — load reg[b], store reg[a].
//   Jump                            — block-to-block branch.
//   JumpIfFalse / JumpIfTrue        — branch on NIL/FALSE check.
//   Return                          — write reg[a] into reg[0],
//                                     return status -1 (ok).
//
// Deopted opcodes (stage 1): everything else. The jitted function
// returns a non-negative status (the offending PC) so the caller
// can resume the interpreter from that PC. Frame state — regs
// values up to that PC — is already live, since jitted code writes
// through the regs pointer in place.
//
// FFI contract:
//
//   extern "C" fn(regs: *mut u64, constants: *const u64) -> i64
//   - regs       : pointer to the frame's register file (Value as u64
//                  bit pattern). length implied by the chunk's
//                  num_regs at compile time.
//   - constants  : pointer to the chunk's constants slice. Looked up
//                  by LoadConst.
//   - returns -1 : ok, regs[0] holds the result value.
//   - returns >=0: deopt at this PC; interpreter should resume from
//                  here (regs[] are already updated for everything
//                  preceding this op).
//
// Why we pass `constants` as a runtime pointer rather than baking
// constants into the IR as iconsts: keeps the JIT'd function
// independent of any specific Chunk allocation. Stage 2 will pass
// the same constants array the interpreter's frame uses.

use std::collections::HashMap;

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use moof_core::value::Value;

use crate::opcodes::{Chunk, Op};

pub struct Jit {
    builder_ctx: FunctionBuilderContext,
    ctx: codegen::Context,
    module: JITModule,
    /// Source chunks for each compiled JIT fn, kept alive so the
    /// constants pointer stays valid for the lifetime of the JIT.
    /// Indexed in compile order; not used for execution.
    chunks: Vec<std::sync::Arc<Chunk>>,
}

/// Function pointer for a compiled chunk.
pub type JittedChunk = extern "C" fn(*mut u64, *const u64) -> i64;

impl Jit {
    pub fn new() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false")
            .map_err(|e| format!("flag set: {e}"))?;
        flag_builder.set("is_pic", "false")
            .map_err(|e| format!("flag set: {e}"))?;
        let isa_builder = cranelift_native::builder()
            .map_err(|e| format!("isa builder: {e}"))?;
        let isa = isa_builder.finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("isa finish: {e}"))?;
        let builder = JITBuilder::with_isa(
            isa,
            cranelift_module::default_libcall_names(),
        );
        let module = JITModule::new(builder);
        Ok(Jit {
            builder_ctx: FunctionBuilderContext::new(),
            ctx: module.make_context(),
            module,
            chunks: Vec::new(),
        })
    }

    /// Compile a single chunk to a native function.
    ///
    /// Walks the bytecode twice: first pass collects branch targets
    /// so each one gets its own Cranelift Block; second pass emits IR
    /// per opcode. Opcodes the JIT can't yet lower terminate the
    /// current block with `return deopt_pc`.
    pub fn compile_chunk(&mut self, chunk: std::sync::Arc<Chunk>) -> Result<JittedChunk, String> {
        self.module.clear_context(&mut self.ctx);

        let ptr_ty = self.module.target_config().pointer_type();
        self.ctx.func.signature.params.push(AbiParam::new(ptr_ty));   // regs
        self.ctx.func.signature.params.push(AbiParam::new(ptr_ty));   // constants
        self.ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let code = chunk.code.clone();
        let block_at = collect_block_starts(&code);

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);

            // Make a Cranelift block for each branch target / fall-through start.
            let mut blocks: HashMap<usize, Block> = HashMap::new();
            for &pc in &block_at {
                blocks.insert(pc, builder.create_block());
            }
            // Entry block (pc 0) — also where the function params come in.
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);

            let regs_ptr = builder.block_params(entry)[0];
            let consts_ptr = builder.block_params(entry)[1];

            // Jump from entry into the bytecode block at PC 0.
            let pc0_block = *blocks.get(&0).expect("PC 0 must be a block start");
            builder.ins().jump(pc0_block, &[]);
            builder.seal_block(entry);

            // Emit IR for each opcode, in PC order.
            let mut pc = 0usize;
            let mut current_block_started = false;
            while pc + 3 < code.len() {
                if let Some(&blk) = blocks.get(&pc) {
                    if current_block_started {
                        // straight-line fall-through into a new block —
                        // terminate the previous one with a jump.
                        builder.ins().jump(blk, &[]);
                    }
                    builder.switch_to_block(blk);
                    current_block_started = true;
                } else if !current_block_started {
                    // unreachable bytecode (after a terminator and
                    // before the next block start). skip emission.
                    let op_byte = code[pc];
                    pc += match Op::from_u8(op_byte) {
                        Some(Op::Send) | Some(Op::TailCall) => 9,
                        _ => 4,
                    };
                    continue;
                }
                let op_byte = code[pc];
                let a = code[pc + 1];
                let b = code[pc + 2];
                let c = code[pc + 3];
                let op = match Op::from_u8(op_byte) {
                    Some(o) => o,
                    None => {
                        // unknown op → deopt
                        let deopt = builder.ins().iconst(types::I64, pc as i64);
                        builder.ins().return_(&[deopt]);
                        current_block_started = false;
                        pc += 4;
                        continue;
                    }
                };

                let next_pc = pc + op_byte_len(op);

                match op {
                    Op::LoadNil   => emit_store_const(&mut builder, regs_ptr, a, Value::NIL.to_bits()),
                    Op::LoadTrue  => emit_store_const(&mut builder, regs_ptr, a, Value::TRUE.to_bits()),
                    Op::LoadFalse => emit_store_const(&mut builder, regs_ptr, a, Value::FALSE.to_bits()),
                    Op::LoadInt   => {
                        let n = i16::from_be_bytes([b, c]) as i64;
                        let v = Value::integer(n).to_bits();
                        emit_store_const(&mut builder, regs_ptr, a, v);
                    }
                    Op::LoadConst => {
                        let idx = u16::from_be_bytes([b, c]) as i32;
                        // load constants[idx] (u64) into regs[a]
                        let val = builder.ins().load(
                            types::I64,
                            MemFlags::trusted(),
                            consts_ptr,
                            idx * 8,
                        );
                        builder.ins().store(MemFlags::trusted(), val, regs_ptr,
                            (a as i32) * 8);
                    }
                    Op::Move => {
                        let val = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (b as i32) * 8,
                        );
                        builder.ins().store(MemFlags::trusted(), val, regs_ptr,
                            (a as i32) * 8);
                    }
                    Op::Return => {
                        // copy regs[a] → regs[0], then return -1 (ok).
                        let val = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (a as i32) * 8,
                        );
                        if a != 0 {
                            builder.ins().store(MemFlags::trusted(), val, regs_ptr, 0);
                        }
                        let ok = builder.ins().iconst(types::I64, -1);
                        builder.ins().return_(&[ok]);
                        current_block_started = false;
                        pc = next_pc;
                        continue;
                    }
                    Op::Jump => {
                        let offset = i16::from_be_bytes([a, b]) as isize;
                        let target = (next_pc as isize - 4 + 4 + offset) as usize;
                        // Note: in the interpreter, after pc += 4, jump uses
                        // f.pc + offset where f.pc has already advanced past
                        // the jump op. So target = (pc + 4) + offset.
                        let target = (pc + 4) as isize + offset;
                        let target = target as usize;
                        let blk = *blocks.get(&target)
                            .ok_or_else(|| format!("jump target {target} not a block"))?;
                        builder.ins().jump(blk, &[]);
                        current_block_started = false;
                        pc = next_pc;
                        continue;
                    }
                    Op::JumpIfFalse => {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        let target = (pc + 4) as isize + offset;
                        let target = target as usize;
                        let blk_taken = *blocks.get(&target)
                            .ok_or_else(|| format!("jumpiffalse target {target} not a block"))?;
                        let blk_fall = *blocks.get(&next_pc)
                            .ok_or_else(|| format!("jumpiffalse fallthrough {next_pc} not a block"))?;
                        // truthy = (val != NIL) && (val != FALSE)
                        let val = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (a as i32) * 8,
                        );
                        let nil_bits = builder.ins().iconst(types::I64, Value::NIL.to_bits() as i64);
                        let false_bits = builder.ins().iconst(types::I64, Value::FALSE.to_bits() as i64);
                        let is_nil = builder.ins().icmp(IntCC::Equal, val, nil_bits);
                        let is_false = builder.ins().icmp(IntCC::Equal, val, false_bits);
                        let is_falsy = builder.ins().bor(is_nil, is_false);
                        builder.ins().brif(is_falsy, blk_taken, &[], blk_fall, &[]);
                        current_block_started = false;
                        pc = next_pc;
                        continue;
                    }
                    Op::JumpIfTrue => {
                        let offset = i16::from_be_bytes([b, c]) as isize;
                        let target = (pc + 4) as isize + offset;
                        let target = target as usize;
                        let blk_taken = *blocks.get(&target)
                            .ok_or_else(|| format!("jumpiftrue target {target} not a block"))?;
                        let blk_fall = *blocks.get(&next_pc)
                            .ok_or_else(|| format!("jumpiftrue fallthrough {next_pc} not a block"))?;
                        let val = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (a as i32) * 8,
                        );
                        let nil_bits = builder.ins().iconst(types::I64, Value::NIL.to_bits() as i64);
                        let false_bits = builder.ins().iconst(types::I64, Value::FALSE.to_bits() as i64);
                        let is_nil = builder.ins().icmp(IntCC::Equal, val, nil_bits);
                        let is_false = builder.ins().icmp(IntCC::Equal, val, false_bits);
                        let is_falsy = builder.ins().bor(is_nil, is_false);
                        builder.ins().brif(is_falsy, blk_fall, &[], blk_taken, &[]);
                        current_block_started = false;
                        pc = next_pc;
                        continue;
                    }
                    _ => {
                        // unsupported (yet): deopt at this PC.
                        let deopt = builder.ins().iconst(types::I64, pc as i64);
                        builder.ins().return_(&[deopt]);
                        current_block_started = false;
                        pc = next_pc;
                        continue;
                    }
                }

                pc = next_pc;
            }

            // Trailing fall-off: terminate any open block.
            if current_block_started {
                let deopt = builder.ins().iconst(types::I64, pc as i64);
                builder.ins().return_(&[deopt]);
            }

            // Seal all blocks. We've emitted all branches; no further
            // predecessors will appear.
            for (_, blk) in &blocks {
                builder.seal_block(*blk);
            }

            builder.finalize();
        }

        let func_id = self.module.declare_function(
            &format!("moof_chunk_{}", self.chunks.len()),
            Linkage::Local,
            &self.ctx.func.signature,
        ).map_err(|e| format!("declare: {e}"))?;
        self.module.define_function(func_id, &mut self.ctx)
            .map_err(|e| format!("define: {e}"))?;
        self.module.finalize_definitions()
            .map_err(|e| format!("finalize: {e}"))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        let typed: JittedChunk = unsafe { std::mem::transmute(code_ptr) };

        // keep the chunk alive so its constants slice stays addressable
        self.chunks.push(chunk);

        Ok(typed)
    }
}

/// Length in bytes of a single op (opcode + 3-byte operand block,
/// plus 5 trailing bytes for Send/TailCall).
fn op_byte_len(op: Op) -> usize {
    match op {
        Op::Send | Op::TailCall => 9,
        _ => 4,
    }
}

/// First pass: identify every PC that should start a Cranelift block.
/// Includes PC 0, all branch targets, and all PCs immediately after
/// branches (fall-through start).
fn collect_block_starts(code: &[u8]) -> Vec<usize> {
    let mut starts: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    starts.insert(0);

    let mut pc = 0usize;
    while pc + 3 < code.len() {
        let op_byte = code[pc];
        let a = code[pc + 1];
        let b = code[pc + 2];
        let c = code[pc + 3];
        let next_pc = match Op::from_u8(op_byte) {
            Some(Op::Send) | Some(Op::TailCall) => pc + 9,
            _ => pc + 4,
        };
        if let Some(op) = Op::from_u8(op_byte) {
            match op {
                Op::Jump => {
                    let offset = i16::from_be_bytes([a, b]) as isize;
                    let target = (pc + 4) as isize + offset;
                    starts.insert(target as usize);
                    starts.insert(next_pc);
                }
                Op::JumpIfFalse | Op::JumpIfTrue => {
                    let offset = i16::from_be_bytes([b, c]) as isize;
                    let target = (pc + 4) as isize + offset;
                    starts.insert(target as usize);
                    starts.insert(next_pc);
                }
                _ => {}
            }
        }
        pc = next_pc;
    }
    starts.into_iter().filter(|&p| p < code.len()).collect()
}

/// Emit `regs[reg] = value` for an immediate value bit pattern.
fn emit_store_const(builder: &mut FunctionBuilder, regs_ptr: Value_, reg: u8, bits: u64) {
    let v = builder.ins().iconst(types::I64, bits as i64);
    builder.ins().store(MemFlags::trusted(), v, regs_ptr, (reg as i32) * 8);
}

// alias: cranelift's `Value` shadows ours. Only used in helpers.
type Value_ = cranelift::prelude::Value;

#[cfg(test)]
mod tests {
    use super::*;

    fn run(jit_fn: JittedChunk, mut regs: Vec<u64>, constants: Vec<u64>) -> (i64, Vec<u64>) {
        let regs_ptr = regs.as_mut_ptr();
        let consts_ptr = constants.as_ptr();
        let status = jit_fn(regs_ptr, consts_ptr);
        (status, regs)
    }

    fn make_chunk(code: Vec<u8>, constants: Vec<u64>, num_regs: u8) -> std::sync::Arc<Chunk> {
        let mut c = Chunk::new("test", 0, num_regs);
        c.code = code;
        c.constants = constants;
        std::sync::Arc::new(c)
    }

    /// LoadInt → Return: jit'd function returns the constant.
    #[test]
    fn jit_load_int_return() {
        let code = vec![
            Op::LoadInt as u8, 0, 0x00, 0x2a,    // regs[0] = 42
            Op::Return as u8,  0, 0,    0,       // return regs[0]
        ];
        let chunk = make_chunk(code, vec![], 1);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1, "ok");
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(42));
    }

    /// LoadNil → Return: jit'd function returns NIL.
    #[test]
    fn jit_load_nil_return() {
        let code = vec![
            Op::LoadNil as u8, 0, 0, 0,
            Op::Return as u8,  0, 0, 0,
        ];
        let chunk = make_chunk(code, vec![], 1);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1);
        assert!(Value::from_bits(regs[0]).is_nil());
    }

    /// LoadConst pulls from the constants pointer arg.
    #[test]
    fn jit_load_const() {
        let code = vec![
            Op::LoadConst as u8, 0, 0, 0,        // regs[0] = constants[0]
            Op::Return as u8,    0, 0, 0,
        ];
        let chunk = make_chunk(code, vec![], 1);
        let constants = vec![Value::integer(99).to_bits()];
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], constants);
        assert_eq!(status, -1);
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(99));
    }

    /// Move from one reg to another, then return.
    #[test]
    fn jit_move_return() {
        let code = vec![
            Op::LoadInt as u8, 1, 0x00, 0x07,    // regs[1] = 7
            Op::Move as u8,    0, 1,    0,       // regs[0] = regs[1]
            Op::Return as u8,  0, 0,    0,
        ];
        let chunk = make_chunk(code, vec![], 2);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1);
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(7));
    }

    /// JumpIfFalse: branches when the cond is FALSE.
    /// Code:
    ///   LoadFalse 0
    ///   JumpIfFalse 0, +8         (skip the LoadInt 1)
    ///   LoadInt 1, 99             (would set regs[1] = 99)
    ///   LoadInt 1, 1              (target — sets regs[1] = 1)
    ///   Move 0 1
    ///   Return 0
    /// JumpIfFalse: branches over a single 4-byte op when the cond
    /// is FALSE. Offset is applied to the PC AFTER the JumpIfFalse
    /// op (matching the interpreter's `f.pc += 4` then `pc + offset`),
    /// so to skip the next op we pass offset=4.
    #[test]
    fn jit_jump_if_false_taken() {
        let code = vec![
            Op::LoadFalse as u8,    0, 0, 0,        // 0
            Op::JumpIfFalse as u8,  0, 0, 4,        // 4 → jump +4 to PC 12
            Op::LoadInt as u8,      1, 0, 99,       // 8  (skipped)
            Op::LoadInt as u8,      1, 0, 1,        // 12 (taken)
            Op::Move as u8,         0, 1, 0,        // 16
            Op::Return as u8,       0, 0, 0,        // 20
        ];
        let chunk = make_chunk(code, vec![], 2);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1);
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(1));
    }

    /// Proper if-then-else shape for JumpIfFalse. cond TRUE → run THEN
    /// arm and skip the else. cond FALSE → jump over the THEN arm.
    ///
    ///   0: LoadTrue 0
    ///   4: JumpIfFalse 0 0 8    — if FALSE, jump to ELSE (PC 16)
    ///   8: LoadInt 1 0 99       — THEN arm
    ///  12: Jump 0 4 0            — skip ELSE (jump to PC 20)
    ///  16: LoadInt 1 0 1         — ELSE arm
    ///  20: Move 0 1 0
    ///  24: Return 0 0 0
    #[test]
    fn jit_jump_if_false_fallthrough() {
        let code = vec![
            Op::LoadTrue as u8,     0, 0, 0,        // 0
            Op::JumpIfFalse as u8,  0, 0, 8,        // 4  → skip THEN if FALSE
            Op::LoadInt as u8,      1, 0, 99,       // 8  THEN arm
            Op::Jump as u8,         0, 4, 0,        // 12 skip ELSE
            Op::LoadInt as u8,      1, 0, 1,        // 16 ELSE arm
            Op::Move as u8,         0, 1, 0,        // 20
            Op::Return as u8,       0, 0, 0,        // 24
        ];
        let chunk = make_chunk(code, vec![], 2);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1);
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(99));
    }

    /// Unsupported op (Send) → deopt with the offending PC.
    #[test]
    fn jit_deopt_on_send() {
        let code = vec![
            Op::LoadInt as u8, 0, 0, 7,             // 0: regs[0] = 7
            Op::Send as u8,    0, 0, 0,             // 4: deopt here
            0, 0, 0, 0, 0,                          //    (5 send trailing bytes)
            Op::Return as u8,  0, 0, 0,             // 13: never reached
        ];
        let chunk = make_chunk(code, vec![], 2);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, 4, "deopt PC should point at the Send");
        // regs[0] was set before the deopt
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(7));
    }
}
