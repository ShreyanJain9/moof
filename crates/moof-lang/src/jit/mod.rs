// Cranelift-backed JIT compiler — stage 0: ground-truth pipeline.
//
// The contract this stage proves: we can build cranelift IR from
// moof bytecode, hand it to cranelift-jit, get back a function
// pointer, transmute it into a callable Rust fn, and invoke it.
//
// Nothing in this stage produces a perf win — the actual lowering
// of bytecode to native code is stage 1. What we get here is the
// scaffolding: a `Jit` struct that owns a JITModule, a way to
// compile bytecode chunks, and a smoke test that compiles + calls
// a "return constant 42" function end-to-end.
//
// The JIT contract for a compiled chunk:
//
//   extern "C" fn(regs: *mut Value, n_regs: usize) -> i64
//
// Caller passes a pointer to the frame's regs and the count; the
// jitted code reads/writes through that pointer. Return is an i64
// sentinel: 0 = ran to completion (regs[0] holds the result), other
// codes are reserved for "deopt back to interpreter" in later
// stages.
//
// We're using i64 instead of u64 for the return type because
// Cranelift's I64 type is signed-ish-but-actually-bag-of-bits for
// our purposes. moof's Value is u64 internally — for stage 0 we
// store it as a raw bit pattern in a register slot.
//
// The JITModule must outlive every function it produces — we hold
// it in `Jit` and never drop it during a session.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use moof_core::value::Value;

/// A live JIT compilation context. Owns the cranelift module,
/// the function builder context, and the module's data context.
/// Consumers add functions one at a time and get back fn pointers
/// that remain valid as long as `Jit` lives.
pub struct Jit {
    /// Builds and stores the Cranelift IR.
    builder_ctx: FunctionBuilderContext,
    /// Pre-allocated codegen context — reused across compiles to
    /// avoid resetting the IR allocator.
    ctx: codegen::Context,
    /// The module owns the produced machine code.
    module: JITModule,
}

impl Jit {
    pub fn new() -> Result<Self, String> {
        // ISA-detect for the host. Cranelift's "native" backend
        // picks features (SSE2/AVX/etc) so the produced code uses
        // whatever the host supports.
        let mut flag_builder = settings::builder();
        // PIC is fine; the JIT handles relocations regardless.
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
        })
    }

    /// Stage-0 smoke test: produce a function `extern "C" fn() -> i64`
    /// that returns the given constant. Returns the function pointer
    /// transmuted to a typed callable.
    ///
    /// This is the proof-of-pipeline: if this works end-to-end, we
    /// know cranelift is loaded, the ISA is reachable, codegen runs,
    /// and we can call into produced machine code from Rust.
    pub fn compile_constant_returner(&mut self, value: i64) -> Result<extern "C" fn() -> i64, String> {
        // Reset ctx for a fresh compile.
        self.module.clear_context(&mut self.ctx);

        // Function signature: `() -> i64`.
        self.ctx.func.signature.returns.push(AbiParam::new(types::I64));

        // Build the function body.
        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);

            let val = builder.ins().iconst(types::I64, value);
            builder.ins().return_(&[val]);

            builder.finalize();
        }

        // Declare + define the function in the module, then
        // finalize so the machine code becomes callable.
        let func_id = self.module.declare_function(
            &format!("moof_const_{}", value),
            Linkage::Local,
            &self.ctx.func.signature,
        ).map_err(|e| format!("declare: {e}"))?;
        self.module.define_function(func_id, &mut self.ctx)
            .map_err(|e| format!("define: {e}"))?;
        self.module.finalize_definitions()
            .map_err(|e| format!("finalize: {e}"))?;

        // Transmute the raw fn pointer to the typed signature.
        let code_ptr = self.module.get_finalized_function(func_id);
        // SAFETY: signature matches what we declared above; the
        // module keeps the code alive for as long as `self` lives.
        let typed: extern "C" fn() -> i64 = unsafe { std::mem::transmute(code_ptr) };
        Ok(typed)
    }

    /// Stage-0 second smoke test: a function that takes a pointer to
    /// a slice of Value (one register) and stores `value` into it.
    /// Mirrors what stage 1's actual codegen will do — write through
    /// a pointer the caller passes in, return a status code.
    ///
    ///   extern "C" fn(regs: *mut u64, n: usize) -> i64
    ///   regs[0] = value
    ///   return 0
    pub fn compile_constant_writer(&mut self, value: Value)
        -> Result<extern "C" fn(*mut u64, usize) -> i64, String>
    {
        self.module.clear_context(&mut self.ctx);

        let ptr_ty = self.module.target_config().pointer_type();
        self.ctx.func.signature.params.push(AbiParam::new(ptr_ty));        // regs
        self.ctx.func.signature.params.push(AbiParam::new(types::I64));    // n_regs (unused for now)
        self.ctx.func.signature.returns.push(AbiParam::new(types::I64));   // status

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);

            let regs_ptr = builder.block_params(block)[0];
            // store value at regs[0]
            let val_const = builder.ins().iconst(types::I64, value.to_bits() as i64);
            builder.ins().store(MemFlags::trusted(), val_const, regs_ptr, 0);
            // return 0 (ok)
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);

            builder.finalize();
        }

        let func_id = self.module.declare_function(
            &format!("moof_writer_{:x}", value.to_bits()),
            Linkage::Local,
            &self.ctx.func.signature,
        ).map_err(|e| format!("declare: {e}"))?;
        self.module.define_function(func_id, &mut self.ctx)
            .map_err(|e| format!("define: {e}"))?;
        self.module.finalize_definitions()
            .map_err(|e| format!("finalize: {e}"))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        let typed: extern "C" fn(*mut u64, usize) -> i64 =
            unsafe { std::mem::transmute(code_ptr) };
        Ok(typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage 0 acceptance: cranelift pipeline produces callable
    /// machine code that returns the value we asked for.
    #[test]
    fn smoke_constant_returner() {
        let mut jit = Jit::new().expect("init jit");
        let f = jit.compile_constant_returner(42).expect("compile");
        assert_eq!(f(), 42);
    }

    #[test]
    fn smoke_constant_returner_negative() {
        let mut jit = Jit::new().expect("init jit");
        let f = jit.compile_constant_returner(-1).expect("compile");
        assert_eq!(f(), -1);
    }

    /// Multiple compiles in one Jit — each gets its own fn pointer
    /// that stays valid alongside the others.
    #[test]
    fn smoke_two_functions() {
        let mut jit = Jit::new().expect("init jit");
        let f1 = jit.compile_constant_returner(7).expect("f1");
        let f2 = jit.compile_constant_returner(13).expect("f2");
        assert_eq!(f1(), 7);
        assert_eq!(f2(), 13);
        // f1 still works after f2 was added
        assert_eq!(f1(), 7);
    }

    /// Stage 0 acceptance for the pointer-passing shape that stage 1
    /// will actually use: jitted code writes a Value into a regs
    /// slot the caller hands it.
    #[test]
    fn smoke_constant_writer() {
        let mut jit = Jit::new().expect("init jit");
        let val = Value::integer(123);
        let f = jit.compile_constant_writer(val).expect("compile");
        let mut regs = [0u64; 4];
        let status = f(regs.as_mut_ptr(), regs.len());
        assert_eq!(status, 0);
        assert_eq!(Value::from_bits(regs[0]), val);
        // other regs untouched
        assert_eq!(regs[1], 0);
    }
}
