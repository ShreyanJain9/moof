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

/// Well-known symbol IDs needed at JIT compile time. The JIT
/// specializes Op::Send when the selector matches one of these —
/// it can produce inline integer arithmetic + comparisons that
/// skip the dispatch loop entirely.
#[derive(Debug, Clone, Copy)]
pub struct OpSyms {
    pub plus: u32,
    pub minus: u32,
    pub mul: u32,
    pub lt: u32,
    pub le: u32,
    pub gt: u32,
    pub ge: u32,
    pub eq_op: u32,
}

impl OpSyms {
    pub fn from_heap(heap: &moof_core::heap::Heap) -> Self {
        OpSyms {
            plus: heap.sym_plus, minus: heap.sym_minus, mul: heap.sym_mul,
            lt: heap.sym_lt, le: heap.sym_le, gt: heap.sym_gt, ge: heap.sym_ge,
            eq_op: heap.sym_eq_op,
        }
    }
}

// Value bit-layout constants — duplicated here for codegen; keep in
// sync with moof_core::value.
#[allow(dead_code)]
const QNAN_BITS: u64       = 0x7FF8_0000_0000_0000;
const TAG_INT_PATTERN: u64 = 0x7FFB_0000_0000_0000;  // QNAN | (3 << 48)
const TAG_MASK: u64        = 0x7FFF_0000_0000_0000;
const PAYLOAD_MASK: u64    = 0x0000_FFFF_FFFF_FFFF;
const I48_MIN: i64         = -(1i64 << 47);
const I48_MAX: i64         = (1i64 << 47) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arith { Add, Sub, Mul }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendKind {
    Cmp(IntCC),
    Arith(Arith),
    Other,
}

fn classify_send(sel_sym: u32, ops: &OpSyms) -> SendKind {
    if      sel_sym == ops.plus  { SendKind::Arith(Arith::Add) }
    else if sel_sym == ops.minus { SendKind::Arith(Arith::Sub) }
    else if sel_sym == ops.mul   { SendKind::Arith(Arith::Mul) }
    else if sel_sym == ops.lt    { SendKind::Cmp(IntCC::SignedLessThan) }
    else if sel_sym == ops.le    { SendKind::Cmp(IntCC::SignedLessThanOrEqual) }
    else if sel_sym == ops.gt    { SendKind::Cmp(IntCC::SignedGreaterThan) }
    else if sel_sym == ops.ge    { SendKind::Cmp(IntCC::SignedGreaterThanOrEqual) }
    else if sel_sym == ops.eq_op { SendKind::Cmp(IntCC::Equal) }
    else { SendKind::Other }
}

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
    pub fn compile_chunk(&mut self, chunk: std::sync::Arc<Chunk>, op_syms: OpSyms) -> Result<JittedChunk, String> {
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
                    Op::Send => {
                        // Try the integer fast path. The selector and
                        // operand registers are baked in at compile
                        // time. If recv & arg are inline integers, do
                        // the op natively; otherwise return the deopt
                        // PC so the interpreter takes over.
                        //
                        // Only nargs == 1 is lowered (the generic
                        // fixnum binop shape). Other shapes deopt.
                        let sel_lo = c;
                        if pc + 4 >= code.len() {
                            // can't read sel_hi/nargs/a0 — deopt.
                            let deopt = builder.ins().iconst(types::I64, pc as i64);
                            builder.ins().return_(&[deopt]);
                            current_block_started = false;
                            pc = next_pc;
                            continue;
                        }
                        let sel_hi = code[pc + 4];
                        let nargs = code[pc + 5];
                        let a0 = code[pc + 6];
                        let sel_idx = ((sel_hi as usize) << 8) | (sel_lo as usize);

                        // Resolve the selector to a sym_id at compile
                        // time. The constants slice holds Value bits.
                        let sel_sym = if sel_idx < chunk.constants.len() {
                            let bits = chunk.constants[sel_idx];
                            // payload = sym_id (low 32 bits of the
                            // PAYLOAD_MASK region for a symbol).
                            (bits & 0xFFFF_FFFF) as u32
                        } else {
                            // bad index — let the interp handle it.
                            let deopt = builder.ins().iconst(types::I64, pc as i64);
                            builder.ins().return_(&[deopt]);
                            current_block_started = false;
                            pc = next_pc;
                            continue;
                        };

                        let send_kind = classify_send(sel_sym, &op_syms);
                        if nargs != 1 || send_kind == SendKind::Other {
                            let deopt = builder.ins().iconst(types::I64, pc as i64);
                            builder.ins().return_(&[deopt]);
                            current_block_started = false;
                            pc = next_pc;
                            continue;
                        }

                        // Compile the fast path. Branch target is
                        // either next_pc (success) or a freshly-made
                        // local deopt block.
                        let fall = *blocks.get(&next_pc)
                            .ok_or_else(|| format!("send fallthrough {next_pc} not a block"))?;
                        let deopt_blk = builder.create_block();
                        let pack_blk  = builder.create_block();

                        // load both operands
                        let recv = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (b as i32) * 8,
                        );
                        let arg = builder.ins().load(
                            types::I64, MemFlags::trusted(), regs_ptr,
                            (a0 as i32) * 8,
                        );
                        // both must be inline integers.
                        let mask = builder.ins().iconst(types::I64, TAG_MASK as i64);
                        let int_pat = builder.ins().iconst(types::I64, TAG_INT_PATTERN as i64);
                        let recv_t = builder.ins().band(recv, mask);
                        let arg_t  = builder.ins().band(arg, mask);
                        let recv_is_int = builder.ins().icmp(IntCC::Equal, recv_t, int_pat);
                        let arg_is_int  = builder.ins().icmp(IntCC::Equal, arg_t, int_pat);
                        let both_int = builder.ins().band(recv_is_int, arg_is_int);

                        // sign-extend i48 payload to i64.
                        let payload_mask = builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
                        let recv_pl = builder.ins().band(recv, payload_mask);
                        let arg_pl  = builder.ins().band(arg, payload_mask);
                        let sixteen = builder.ins().iconst(types::I8, 16);
                        let recv_shifted = builder.ins().ishl(recv_pl, sixteen);
                        let recv_i64 = builder.ins().sshr(recv_shifted, sixteen);
                        let arg_shifted = builder.ins().ishl(arg_pl, sixteen);
                        let arg_i64 = builder.ins().sshr(arg_shifted, sixteen);

                        match send_kind {
                            SendKind::Cmp(cc) => {
                                builder.ins().brif(both_int, pack_blk, &[], deopt_blk, &[]);
                                builder.switch_to_block(pack_blk);
                                let cmp = builder.ins().icmp(cc, recv_i64, arg_i64);
                                let true_bits = builder.ins().iconst(
                                    types::I64, Value::TRUE.to_bits() as i64);
                                let false_bits = builder.ins().iconst(
                                    types::I64, Value::FALSE.to_bits() as i64);
                                let result = builder.ins().select(cmp, true_bits, false_bits);
                                builder.ins().store(MemFlags::trusted(), result, regs_ptr,
                                    (a as i32) * 8);
                                builder.ins().jump(fall, &[]);
                            }
                            SendKind::Arith(ar) => {
                                // compute, then check overflow vs i48.
                                let raw = match ar {
                                    Arith::Add => builder.ins().iadd(recv_i64, arg_i64),
                                    Arith::Sub => builder.ins().isub(recv_i64, arg_i64),
                                    Arith::Mul => builder.ins().imul(recv_i64, arg_i64),
                                };
                                let i48_min = builder.ins().iconst(types::I64, I48_MIN);
                                let i48_max = builder.ins().iconst(types::I64, I48_MAX);
                                let lo_ok = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, raw, i48_min);
                                let hi_ok = builder.ins().icmp(IntCC::SignedLessThanOrEqual, raw, i48_max);
                                let in_range = builder.ins().band(lo_ok, hi_ok);
                                let all_ok = builder.ins().band(both_int, in_range);
                                builder.ins().brif(all_ok, pack_blk, &[], deopt_blk, &[]);
                                builder.switch_to_block(pack_blk);
                                let payload_mask2 = builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
                                let payload = builder.ins().band(raw, payload_mask2);
                                let tag_bits = builder.ins().iconst(types::I64, TAG_INT_PATTERN as i64);
                                let packed = builder.ins().bor(tag_bits, payload);
                                builder.ins().store(MemFlags::trusted(), packed, regs_ptr,
                                    (a as i32) * 8);
                                builder.ins().jump(fall, &[]);
                            }
                            SendKind::Other => unreachable!(),
                        }
                        builder.seal_block(pack_blk);

                        builder.switch_to_block(deopt_blk);
                        let deopt = builder.ins().iconst(types::I64, pc as i64);
                        builder.ins().return_(&[deopt]);
                        builder.seal_block(deopt_blk);

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
/// Includes PC 0, all branch targets, all PCs after branches, and
/// all PCs after Send (which we lower with an in-band branch into a
/// deopt arm + fall-through into the next op).
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
                Op::Send => {
                    // post-Send is a fall-through block (the lowered
                    // fast path branches to it on success).
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

    fn dummy_op_syms() -> OpSyms {
        // Tests that don't exercise Send don't care about these.
        OpSyms { plus: 0, minus: 0, mul: 0, lt: 0, le: 0, gt: 0, ge: 0, eq_op: 0 }
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
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
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, -1);
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(99));
    }

    /// Unsupported op (Send with unknown selector) → deopt.
    #[test]
    fn jit_deopt_on_send() {
        // Send with a selector that doesn't match a fast op → deopt.
        let code = vec![
            Op::LoadInt as u8, 0, 0, 7,             // 0
            Op::Send as u8,    0, 0, 0,             // 4: send recv=0 sel_const=0
            0, 1, 0, 0, 0,                          //    sel_hi=0, nargs=1, a0=0
            Op::Return as u8,  0, 0, 0,
        ];
        // selector const 0 = sym_id 99 (some random sym, NOT in our fast set).
        let constants = vec![Value::symbol(99).to_bits()];
        let chunk = make_chunk(code, constants, 2);
        let mut jit = Jit::new().unwrap();
        let f = jit.compile_chunk(chunk, dummy_op_syms()).unwrap();
        let (status, regs) = run(f, vec![0; 4], vec![]);
        assert_eq!(status, 4, "deopt PC should point at the Send");
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(7));
    }

    /// Send with `+` selector and two int operands — runs the fast
    /// path inline, no deopt.
    #[test]
    fn jit_send_int_add() {
        let plus_sym = 42u32;  // pretend "+" is interned at id 42
        let code = vec![
            Op::LoadInt as u8, 0, 0, 5,             // 0:  regs[0] = 5
            Op::LoadInt as u8, 1, 0, 7,             // 4:  regs[1] = 7
            Op::Send as u8,    2, 0, 0,             // 8:  regs[2] = recv(=regs[0]) + arg(=regs[1])
            0, 1, 1, 0, 0,                          //     sel_hi=0 nargs=1 a0=1
            Op::Return as u8,  2, 0, 0,             // 17: return regs[2]
        ];
        let constants = vec![Value::symbol(plus_sym).to_bits()];
        let chunk = make_chunk(code, constants, 4);
        let mut jit = Jit::new().unwrap();
        let mut ops = dummy_op_syms();
        ops.plus = plus_sym;
        let f = jit.compile_chunk(chunk, ops).unwrap();
        let (status, regs) = run(f, vec![0; 8], vec![]);
        assert_eq!(status, -1, "should run to completion in JIT");
        assert_eq!(Value::from_bits(regs[0]).as_integer(), Some(12));
    }

    /// Send with `<` selector — produces a Bool result.
    #[test]
    fn jit_send_int_lt() {
        let lt_sym = 17u32;
        let code = vec![
            Op::LoadInt as u8, 0, 0, 3,
            Op::LoadInt as u8, 1, 0, 5,
            Op::Send as u8,    2, 0, 0,             // regs[2] = (3 < 5)
            0, 1, 1, 0, 0,
            Op::Return as u8,  2, 0, 0,
        ];
        let constants = vec![Value::symbol(lt_sym).to_bits()];
        let chunk = make_chunk(code, constants, 4);
        let mut jit = Jit::new().unwrap();
        let mut ops = dummy_op_syms();
        ops.lt = lt_sym;
        let f = jit.compile_chunk(chunk, ops).unwrap();
        let (status, regs) = run(f, vec![0; 8], vec![]);
        assert_eq!(status, -1);
        assert!(Value::from_bits(regs[0]).is_truthy());
    }

    /// Send fast path with non-int operand — deopts.
    #[test]
    fn jit_send_int_add_deopts_on_non_int() {
        let plus_sym = 42u32;
        let code = vec![
            Op::LoadNil as u8, 0, 0, 0,             // recv = NIL (not int)
            Op::LoadInt as u8, 1, 0, 7,
            Op::Send as u8,    2, 0, 0,
            0, 1, 1, 0, 0,
            Op::Return as u8,  2, 0, 0,
        ];
        let constants = vec![Value::symbol(plus_sym).to_bits()];
        let chunk = make_chunk(code, constants, 4);
        let mut jit = Jit::new().unwrap();
        let mut ops = dummy_op_syms();
        ops.plus = plus_sym;
        let f = jit.compile_chunk(chunk, ops).unwrap();
        let (status, _regs) = run(f, vec![0; 8], vec![]);
        assert_eq!(status, 8, "deopt at the Send PC (recv was NIL, not int)");
    }
}
