// The bytecode interpreter: register-based VM with frame stack.
//
// ONE opcode loop. Closure calls push frames, returns pop them.
// No duplicated opcode handling. Fuel counting and TCO built in.

use std::sync::Arc;
use moof_core::dispatch;
use moof_core::heap::Heap;
use crate::lang::compiler::{ClosureDesc, CompileResult};
use crate::opcodes::{Chunk, Op};
use moof_core::value::Value;

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
    /// Shared bytecode + constant pool. Holding an Arc here means
    /// push_closure_frame is one refcount bump per call instead of
    /// cloning a few hundred bytes of code/constants.
    chunk: Arc<Chunk>,
    desc_base: usize,
    result_reg: u8,
    saved_env: Option<u32>,
    saved_lex: Option<u32>,

    /// Lazy per-call env. Some(_) only when the closure was pushed
    /// down the fast path (compile-time `needs_env = false`): nothing
    /// in the body would observe a heap-materialized env, so we
    /// stashed the data needed to build one and skipped the
    /// allocation. `materialize_current_frame` reifies it on demand —
    /// triggered when a callee is a vau (whose `$e` must be a real
    /// env) or any opcode the static analysis missed.
    ///
    /// $env stays *semantically* an object; the heap allocation just
    /// gets postponed until something looks at it.
    lazy_env: Option<LazyEnvData>,
}

struct LazyEnvData {
    /// closure.scope at the call site — parent for the materialized env.
    scope_val: Value,
    /// (name, value) pairs as make_env wants them: params (with the
    /// trailing $env binding) followed by captures.
    bindings: Vec<(u32, Value)>,
}

pub struct VM {
    frames: Vec<Frame>,
    closure_descs: Vec<ClosureDesc>,
    pub fuel: u64,      // 0 = unlimited
    /// Stack of "currently active" source records — the source text
    /// of the outermost top-level form whose eval is in progress.
    /// Op::Eval reads the top of this stack so closures created via
    /// runtime macro expansion (vau operatives) inherit the outer
    /// form's source. Push on eval_result_with_source, pop at end.
    active_sources: Vec<Option<moof_core::source::ClosureSource>>,
}

impl VM {
    pub fn new() -> Self {
        VM {
            frames: Vec::new(),
            closure_descs: Vec::new(),
            fuel: 0,
            active_sources: Vec::new(),
        }
    }

    pub fn add_closure_desc(&mut self, desc: ClosureDesc) {
        self.closure_descs.push(desc);
    }

    /// Replace all closure descs (used on image load).
    pub fn set_closure_descs(&mut self, descs: Vec<ClosureDesc>) {
        self.closure_descs = descs;
    }

    pub fn closure_descs_ref(&self) -> &[ClosureDesc] {
        &self.closure_descs
    }

    /// Execute a chunk, returning the result. Takes Arc<Chunk> so
    /// the bytecode is shared with the Frame, not cloned.
    pub fn execute(&mut self, heap: &mut Heap, chunk: Arc<Chunk>, _env: Value) -> Result<Value, String> {
        let regs = vec![Value::NIL; chunk.num_regs as usize + 1];
        self.frames.push(Frame {
            regs,
            pc: 0,
            chunk,
            desc_base: self.current_desc_base(),
            result_reg: 0,
            saved_env: None,
            saved_lex: None,
            lazy_env: None,
        });
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
    ///
    /// `unpacked` is the flat list of arg values (receiver + send args
    /// already merged, or the pre-unpacked contents of a `call:` arg
    /// list). Callers build this Vec directly — we avoid round-tripping
    /// through a moof-heap cons chain, which was the dominant per-call
    /// allocation in tight recursion.
    fn push_closure_frame(
        &mut self,
        heap: &mut Heap,
        closure_val: Value,
        code_idx: usize,
        unpacked: Vec<Value>,
        result_reg: u8,
    ) -> Result<(), String> {
        if code_idx >= self.closure_descs.len() {
            return Err(format!("closure code_idx {} out of bounds (have {})", code_idx, self.closure_descs.len()));
        }

        // grab Arc handles from the desc — refcount bumps, no allocation.
        let chunk = Arc::clone(&self.closure_descs[code_idx].chunk);
        let closure_desc_base = self.closure_descs[code_idx].desc_base;
        let capture_local_regs = Arc::clone(&self.closure_descs[code_idx].capture_local_regs);
        let rest_reg = self.closure_descs[code_idx].rest_param_reg;
        let arity = chunk.arity as usize;
        let param_names = Arc::clone(&self.closure_descs[code_idx].param_names);
        let capture_names = Arc::clone(&self.closure_descs[code_idx].capture_names);
        let needs_env = self.closure_descs[code_idx].needs_env;

        let captures_from_obj = heap.closure_captures(closure_val);

        let mut regs = vec![Value::NIL; chunk.num_regs as usize + 16];

        let n_before_env = arity.saturating_sub(1);
        let env_val = Value::nursery(heap.env);

        // params + captures into registers (compile-time-resolved
        // accesses don't depend on the env being heap-materialized).
        for i in 0..n_before_env.min(unpacked.len()) {
            regs[i] = unpacked[i];
        }
        if arity > 0 {
            regs[arity - 1] = env_val;
        }
        if let Some(rest_r) = rest_reg {
            let rest_args: Vec<Value> = unpacked.iter().skip(n_before_env).copied().collect();
            regs[rest_r as usize] = heap.list(&rest_args);
        }
        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
            if i < capture_local_regs.len() {
                let reg = capture_local_regs[i] as usize;
                if reg < regs.len() { regs[reg] = *val; }
            }
        }

        // Build the (name, value) pairs that *would* go into a real
        // env. Used eagerly (slow path) or stashed as lazy data (fast
        // path).
        let mut bindings: Vec<(u32, Value)> = Vec::with_capacity(param_names.len() + capture_names.len());
        for (i, &name) in param_names.iter().enumerate().take(arity) {
            if i < n_before_env && i < unpacked.len() {
                bindings.push((name, unpacked[i]));
            } else if i == n_before_env {
                bindings.push((name, env_val));
            }
        }
        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
            if i < capture_names.len() {
                bindings.push((capture_names[i], *val));
            }
        }

        let scope_val = closure_val.as_any_object()
            .and_then(|cid| heap.get(cid).slot_get(heap.sym_scope))
            .unwrap_or(Value::NIL);

        let (saved_env, saved_lex, lazy_env) = if closure_val.as_any_object().is_some() {
            if needs_env {
                // Slow path: materialize immediately. The body has
                // Eval / CurrentEnv / DefGlobal / MakeClosure and will
                // observe heap.env.
                let new_env = heap.make_env(scope_val, bindings.iter().map(|(n,_)|*n).collect(), bindings.iter().map(|(_,v)|*v).collect());
                let new_env_id = new_env.as_any_object()
                    .ok_or("push_closure_frame: make_env returned non-object")?;
                let p_env = heap.env;
                let p_lex = heap.lexical_scope;
                heap.env = new_env_id;
                heap.lexical_scope = new_env_id;
                (Some(p_env), Some(p_lex), None)
            } else {
                // Fast path: skip make_env. heap.env points at the
                // closure's static :scope so GetGlobal still walks
                // chains to the root namespace. Stash bindings in
                // `lazy_env` so we can materialize on demand if a
                // callee turns out to need them (e.g. a vau reading
                // $e for `eval`).
                let target = scope_val.as_any_object().unwrap_or(heap.env);
                let p_env = heap.env;
                let p_lex = heap.lexical_scope;
                heap.env = target;
                heap.lexical_scope = target;
                (Some(p_env), Some(p_lex), Some(LazyEnvData { scope_val, bindings }))
            }
        } else { (None, None, None) };

        self.frames.push(Frame {
            regs,
            pc: 0,
            chunk,
            desc_base: closure_desc_base,
            result_reg,
            saved_env,
            saved_lex,
            lazy_env,
        });
        Ok(())
    }

    /// If the topmost frame deferred its env allocation, build the
    /// real env now. heap.env / lexical_scope swap to it. Subsequent
    /// reads of $env see the materialized value.
    fn materialize_current_frame(&mut self, heap: &mut Heap) {
        let f = match self.frames.last_mut() {
            Some(f) => f,
            None => return,
        };
        let lazy = match f.lazy_env.take() {
            Some(l) => l,
            None => return,
        };
        // make_env_inline stashes bindings DIRECTLY on the env's
        // HeapObject (one alloc) instead of going through a Table
        // foreign payload (two allocs + indexmap insert per binding).
        // env_get understands both shapes — semantics preserved.
        let new_env = heap.make_env_inline(lazy.scope_val, lazy.bindings);
        if let Some(new_env_id) = new_env.as_any_object() {
            heap.env = new_env_id;
            heap.lexical_scope = new_env_id;
        }
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

            if pc + 3 >= f.chunk.code.len() {
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

            let op = f.chunk.code[pc];
            let a = f.chunk.code[pc + 1];
            let b = f.chunk.code[pc + 2];
            let c = f.chunk.code[pc + 3];
            f.pc += 4;

            let Some(opcode) = Op::from_u8(op) else {
                return Err(format!("unknown opcode: {op}"));
            };

            match opcode {
                Op::LoadConst => {
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let val = Value::from_bits(f.chunk.constants[idx]);
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
                    let saved_env = f.saved_env;
                    let saved_lex = f.saved_lex;
                    self.frames.pop();
                    // closures-carry-env: restore the heap.env and
                    // lexical_scope that were active before this
                    // closure's body started.
                    if let Some(prior) = saved_env { heap.env = prior; }
                    if let Some(prior) = saved_lex { heap.lexical_scope = prior; }
                    if self.frames.len() <= base_depth {
                        // returned from the frame we were called to run
                        return Ok(val);
                    }
                    self.frames.last_mut().unwrap().regs[result_reg as usize] = val;
                    continue;
                }

                Op::Send => {
                    // CONTRACT: Send is 9 bytes — opcode + dst + recv + sel_lo,
                    // followed by 5 trailing bytes: sel_hi, nargs, a0, a1, a2.
                    // sel is a 16-bit constant pool index (low byte in c,
                    // high byte in first trailing slot). nargs is 0..=3;
                    // >3 args use an explicit argument list + `call:`.
                    let dst = a;
                    let recv = f.regs[b as usize];
                    if f.pc + 4 >= f.chunk.code.len() {
                        return Err("send: truncated".into());
                    }
                    let sel_hi = f.chunk.code[f.pc] as usize;
                    let sel_idx = (sel_hi << 8) | (c as usize);
                    let sel_sym = if sel_idx < f.chunk.constants.len() {
                        Value::from_bits(f.chunk.constants[sel_idx]).as_symbol()
                            .ok_or("send: selector constant is not a symbol")?
                    } else {
                        return Err("send: selector constant out of bounds".into());
                    };

                    let nargs = f.chunk.code[f.pc + 1] as usize;
                    let arg_start = f.pc + 2;
                    // nargs is bounded to 3 by the bytecode format
                    // (bigger calls go through `call:` with an arg
                    // list). Stack-buffer + slice avoids a Vec
                    // allocation per Send.
                    let mut send_args_buf = [Value::NIL; 3];
                    let n = nargs.min(3);
                    for i in 0..n {
                        send_args_buf[i] = f.regs[f.chunk.code[arg_start + i] as usize];
                    }
                    let send_args: &[Value] = &send_args_buf[..n];
                    f.pc += 5;

                    // Inline fixnum fast paths. For the common case of
                    // `[int + int]`, `[int < int]`, etc., skip the full
                    // dispatch (proto-cache lookup + native call +
                    // result-marshalling) and do the op right here.
                    // Both operands must be inline i48 fixnums; on any
                    // mismatch (BigInt, Float, non-int) we fall through
                    // to the regular Send path. This is the hottest
                    // path in arithmetic-heavy code (conway's neighbor
                    // count is ~70% Send-on-int).
                    if n == 1 {
                        let a_val = recv;
                        let b_val = send_args[0];
                        if let (Some(x), Some(y)) = (a_val.as_integer(), b_val.as_integer()) {
                            // try the op; if it overflows, fall through
                            // to the dispatch path (which promotes to
                            // BigInt).
                            let fast: Option<Value> =
                                if      sel_sym == heap.sym_plus  { x.checked_add(y).map(Value::integer) }
                                else if sel_sym == heap.sym_minus { x.checked_sub(y).map(Value::integer) }
                                else if sel_sym == heap.sym_mul   { x.checked_mul(y).map(Value::integer) }
                                else if sel_sym == heap.sym_lt    { Some(Value::boolean(x <  y)) }
                                else if sel_sym == heap.sym_le    { Some(Value::boolean(x <= y)) }
                                else if sel_sym == heap.sym_gt    { Some(Value::boolean(x >  y)) }
                                else if sel_sym == heap.sym_ge    { Some(Value::boolean(x >= y)) }
                                else if sel_sym == heap.sym_eq_op { Some(Value::boolean(x == y)) }
                                else { None };
                            if let Some(result) = fast {
                                self.frames.last_mut().unwrap().regs[dst as usize] = result;
                                continue;
                            }
                        }
                    }

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

                    // dispatch: native or closure? natives and closures
                    // both follow the same call protocol: on `[f call: lst]`
                    // where the handler is the receiver (generic invocation),
                    // the arg list unpacks into (first-as-receiver, rest-as-args).
                    let unpacked_for_call: Option<Vec<Value>> =
                        if sel_sym == heap.sym_call && handler == recv {
                            let arg_list = send_args.first().copied().unwrap_or(Value::NIL);
                            Some(heap.list_to_vec(arg_list))
                        } else { None };

                    if dispatch::is_native(heap, handler) {
                        let result = if let Some(unpacked) = unpacked_for_call {
                            // unpacked[0] is the new receiver, the rest args.
                            let new_recv = unpacked.first().copied().unwrap_or(Value::NIL);
                            let new_args = &unpacked[1.min(unpacked.len())..];
                            dispatch::call_native(heap, handler, new_recv, new_args)?
                        } else {
                            dispatch::call_native(heap, handler, recv, &send_args)?
                        };
                        self.frames.last_mut().unwrap().regs[dst as usize] = result;
                    } else if let Some((code_idx, is_operative)) = heap.as_closure(handler) {
                        // If the callee is a vau, its $e will be set
                        // from the *caller's* heap.env. Make sure the
                        // caller has a real per-call env before we
                        // hand it off — otherwise the vau body's
                        // (eval form $e) walks the wrong chain.
                        if is_operative {
                            self.materialize_current_frame(heap);
                        }
                        let unpacked = unpacked_for_call.unwrap_or_else(|| {
                            let mut full = Vec::with_capacity(1 + send_args.len());
                            full.push(recv);
                            full.extend_from_slice(&send_args);
                            full
                        });
                        self.push_closure_frame(heap, handler, code_idx, unpacked, dst)?;
                    } else {
                        return Err(format!("handler is not callable"));
                    }
                }

                Op::TailCall => {
                    // TailCall: same encoding as Send (9 bytes, 16-bit sel);
                    // reuses the current frame for the called closure so
                    // tail-recursion is O(1) in frame depth.
                    let dst = a;
                    let recv = f.regs[b as usize];
                    if f.pc + 4 >= f.chunk.code.len() {
                        return Err("tail_call: truncated".into());
                    }
                    let sel_hi = f.chunk.code[f.pc] as usize;
                    let sel_idx = (sel_hi << 8) | (c as usize);
                    let sel_sym = if sel_idx < f.chunk.constants.len() {
                        Value::from_bits(f.chunk.constants[sel_idx]).as_symbol()
                            .ok_or("tail_call: selector not a symbol")?
                    } else {
                        return Err("tail_call: selector out of bounds".into());
                    };

                    let nargs = f.chunk.code[f.pc + 1] as usize;
                    let arg_start = f.pc + 2;
                    // nargs is bounded to 3 by the bytecode format
                    // (bigger calls go through `call:` with an arg
                    // list). Stack-buffer + slice avoids a Vec
                    // allocation per Send.
                    let mut send_args_buf = [Value::NIL; 3];
                    let n = nargs.min(3);
                    for i in 0..n {
                        send_args_buf[i] = f.regs[f.chunk.code[arg_start + i] as usize];
                    }
                    let send_args: &[Value] = &send_args_buf[..n];
                    f.pc += 5;

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
                    } else if let Some((code_idx, is_operative)) = heap.as_closure(handler) {
                        // tail call to a vau: caller's heap.env will
                        // become the vau's $e. Materialize first so $e
                        // resolves caller's locals by name.
                        if is_operative {
                            self.materialize_current_frame(heap);
                        }
                        let unpacked: Vec<Value> =
                            if sel_sym == heap.sym_call && handler == recv {
                                let arg_list = send_args.first().copied().unwrap_or(Value::NIL);
                                heap.list_to_vec(arg_list)
                            } else {
                                let mut full = Vec::with_capacity(1 + send_args.len());
                                full.push(recv);
                                full.extend_from_slice(&send_args);
                                full
                            };
                        if code_idx >= self.closure_descs.len() {
                            return Err(format!("tail_call: code_idx {} out of bounds", code_idx));
                        }
                        let chunk = Arc::clone(&self.closure_descs[code_idx].chunk);
                        let closure_desc_base = self.closure_descs[code_idx].desc_base;
                        let capture_local_regs = Arc::clone(&self.closure_descs[code_idx].capture_local_regs);
                        let rest_reg = self.closure_descs[code_idx].rest_param_reg;
                        let arity = chunk.arity as usize;
                        let param_names = Arc::clone(&self.closure_descs[code_idx].param_names);
                        let capture_names = Arc::clone(&self.closure_descs[code_idx].capture_names);
                        let needs_env = self.closure_descs[code_idx].needs_env;
                        let captures_from_obj = heap.closure_captures(handler);

                        let f = self.frames.last_mut().unwrap();
                        f.regs.clear();
                        f.regs.resize(chunk.num_regs as usize + 16, Value::NIL);
                        f.pc = 0;
                        f.chunk = chunk;
                        f.desc_base = closure_desc_base;
                        f.lazy_env = None;

                        let n_before_env = arity.saturating_sub(1);
                        let env_val = Value::nursery(heap.env);

                        for i in 0..n_before_env.min(unpacked.len()) {
                            f.regs[i] = unpacked[i];
                        }
                        if arity > 0 {
                            f.regs[arity - 1] = env_val;
                        }
                        if let Some(rest_r) = rest_reg {
                            let rest_args: Vec<Value> = unpacked.iter().skip(n_before_env).copied().collect();
                            f.regs[rest_r as usize] = heap.list(&rest_args);
                        }
                        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
                            if i < capture_local_regs.len() {
                                let reg = capture_local_regs[i] as usize;
                                if reg < f.regs.len() { f.regs[reg] = *val; }
                            }
                        }

                        let mut bindings: Vec<(u32, Value)> = Vec::with_capacity(param_names.len() + capture_names.len());
                        for (i, &name) in param_names.iter().enumerate().take(arity) {
                            if i < n_before_env && i < unpacked.len() {
                                bindings.push((name, unpacked[i]));
                            } else if i == n_before_env {
                                bindings.push((name, env_val));
                            }
                        }
                        for (i, (_, val)) in captures_from_obj.iter().enumerate() {
                            if i < capture_names.len() {
                                bindings.push((capture_names[i], *val));
                            }
                        }

                        if let Some(cid) = handler.as_any_object() {
                            let scope_sym = heap.sym_scope;
                            let scope_val = heap.get(cid).slot_get(scope_sym).unwrap_or(Value::NIL);

                            if needs_env {
                                let names: Vec<u32> = bindings.iter().map(|(n,_)|*n).collect();
                                let vals: Vec<Value> = bindings.iter().map(|(_,v)|*v).collect();
                                let new_env = heap.make_env(scope_val, names, vals);
                                if let Some(new_env_id) = new_env.as_any_object() {
                                    let f = self.frames.last_mut().unwrap();
                                    if f.saved_env.is_none() {
                                        f.saved_env = Some(heap.env);
                                        f.saved_lex = Some(heap.lexical_scope);
                                    }
                                    heap.env = new_env_id;
                                    heap.lexical_scope = new_env_id;
                                }
                            } else {
                                let target = scope_val.as_any_object().unwrap_or(heap.env);
                                let f = self.frames.last_mut().unwrap();
                                if f.saved_env.is_none() {
                                    f.saved_env = Some(heap.env);
                                    f.saved_lex = Some(heap.lexical_scope);
                                }
                                heap.env = target;
                                heap.lexical_scope = target;
                                f.lazy_env = Some(LazyEnvData { scope_val, bindings });
                            }
                        }
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
                        if f.pc + 3 >= f.chunk.code.len() { break; }
                        let nc = u16::from_be_bytes([f.chunk.code[f.pc], f.chunk.code[f.pc + 1]]) as usize;
                        let vr = f.chunk.code[f.pc + 2] as usize;
                        f.pc += 4;
                        let ns = Value::from_bits(f.chunk.constants[nc]).as_symbol()
                            .ok_or("make_obj: slot name not a symbol")?;
                        slot_names.push(ns);
                        slot_values.push(f.regs[vr]);
                    }

                    if clone_parent {
                        // clone: copy parent's slots as defaults, overlay with provided slots.
                        if let Some(pid) = parent.as_any_object() {
                            let parent_slot_names = heap.get(pid).slot_names();
                            let mut merged_names = Vec::new();
                            let mut merged_values = Vec::new();

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
                    let name_sym = Value::from_bits(f.chunk.constants[name_const]).as_symbol()
                        .ok_or("set_slot: name is not a symbol")?;
                    let val = f.regs[c as usize];
                    heap.get_mut(obj_id).slot_set(name_sym, val);
                }

                Op::SetHandler => {
                    let f = self.frames.last_mut().unwrap();
                    let obj_id = f.regs[a as usize].as_any_object()
                        .ok_or("set_handler: not an object")?;
                    let sel_const = b as usize;
                    let sel_sym = Value::from_bits(f.chunk.constants[sel_const]).as_symbol()
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
                        seq.push(f.regs[f.chunk.code[f.pc + i] as usize]);
                    }
                    // collect raw key/val regs first; canonicalize keys via
                    // the heap so String literals become symbol-hashed.
                    let mut pairs: Vec<(Value, Value)> = Vec::with_capacity(nmap);
                    for i in 0..nmap {
                        let ki = nseq + i * 2;
                        let vi = nseq + i * 2 + 1;
                        let key = f.regs[f.chunk.code[f.pc + ki] as usize];
                        let val = f.regs[f.chunk.code[f.pc + vi] as usize];
                        pairs.push((key, val));
                    }
                    f.pc += padded;
                    let mut map: indexmap::IndexMap<Value, Value> = indexmap::IndexMap::with_capacity(nmap);
                    for (k, v) in pairs {
                        map.insert(heap.canonicalize_key(k), v);
                    }
                    f.regs[a as usize] = heap.alloc_table(seq, map);
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
                    let parent_regs = desc.capture_parent_regs.clone();
                    let capture_names = desc.capture_names.clone();

                    // scan bytecode for GetGlobal refs to FarRefs (impure env access)
                    let bytecode = &desc.chunk.code;
                    let constants = &desc.chunk.constants;
                    let farref_proto = heap.lookup_type("FarRef");
                    let mut references_farref = false;
                    if !farref_proto.is_nil() {
                        let mut pc = 0;
                        while pc + 3 < bytecode.len() {
                            if crate::opcodes::Op::from_u8(bytecode[pc]) == Some(crate::opcodes::Op::GetGlobal) {
                                let const_idx = u16::from_be_bytes([bytecode[pc+2], bytecode[pc+3]]) as usize;
                                if const_idx < constants.len() {
                                    let sym_val = Value::from_bits(constants[const_idx]);
                                    if let Some(sym) = sym_val.as_symbol() {
                                        if let Some(val) = heap.env_get(sym) {
                                            if heap.prototype_of(val) == farref_proto {
                                                references_farref = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            pc += 4;
                        }
                    }

                    let f = self.frames.last_mut().unwrap();
                    let cap_pairs: Vec<(u32, Value)> = capture_names.iter().zip(parent_regs.iter())
                        .map(|(&name, &r)| (name, f.regs[r as usize]))
                        .collect();
                    let closure = heap.make_closure(idx, arity, &cap_pairs);
                    // override purity if we found FarRef references in bytecode
                    if references_farref {
                        heap.set_closure_pure(closure, false);
                    }
                    let f = self.frames.last_mut().unwrap();
                    f.regs[a as usize] = closure;
                }

                Op::Wrap => {
                    // Op::Wrap (dst, src, _) — make-applicative on src.
                    // installs `__underlying = src` on a copy of src,
                    // result in dst. fn fast-path emits MakeClosure
                    // (the underlying vau) into a reg, then Op::Wrap
                    // to mark it as applicative. callers seeing a
                    // wrap-output's has-underlying = true emit
                    // eval'd-args dispatch.
                    let f = self.frames.last_mut().unwrap();
                    let target = f.regs[b as usize];
                    let (code_idx, _) = heap.as_closure(target)
                        .ok_or("Wrap: src is not a closure")?;
                    let arity = heap.closure_arity(target).unwrap_or(0);
                    let captures = heap.closure_captures(target);
                    let new_closure = heap.make_closure(code_idx, arity, &captures);
                    heap.set_closure_underlying(new_closure, target);
                    let f = self.frames.last_mut().unwrap();
                    f.regs[a as usize] = new_closure;
                }

                Op::GetGlobal => {
                    let f = self.frames.last_mut().unwrap();
                    let idx = u16::from_be_bytes([b, c]) as usize;
                    let name_sym = Value::from_bits(f.chunk.constants[idx]).as_symbol()
                        .ok_or("get_global: name constant is not a symbol")?;
                    let val = match heap.env_get(name_sym) {
                        Some(v) => v,
                        None => {
                            let msg = format!("unbound: '{}'", heap.symbol_name(name_sym));
                            return Ok(heap.make_error(&msg));
                        }
                    };
                    f.regs[a as usize] = val;
                }

                Op::DefGlobal => {
                    let f = self.frames.last_mut().unwrap();
                    let idx = u16::from_be_bytes([a, b]) as usize;
                    let name_sym = Value::from_bits(f.chunk.constants[idx]).as_symbol()
                        .ok_or("def_global: name constant is not a symbol")?;
                    let val = f.regs[c as usize];
                    if let Some(old) = heap.env_get(name_sym) {
                        if old != val { heap.rebound.insert(name_sym); }
                    }
                    heap.env_def(name_sym, val);
                }

                Op::CurrentEnv => {
                    let f = self.frames.last_mut().unwrap();
                    f.regs[a as usize] = Value::nursery(heap.env);
                }

                Op::Eval => {
                    let ast = self.frames.last().unwrap().regs[b as usize];
                    let env_val = if c != 0 { self.frames.last().unwrap().regs[c as usize] } else { Value::NIL };
                    let saved_frame_depth = self.frames.len();

                    // if ast is an Err, short-circuit — don't try to compile it
                    if Self::is_err_value(heap, ast) {
                        self.frames.last_mut().unwrap().regs[a as usize] = ast;
                        continue;
                    }

                    // env_val handling has two shapes:
                    //
                    //   REAL ENV (has `bindings` slot pointing at a
                    //   Table) → SWAP heap.env. defs land in target.
                    //   closures created during the eval carry target
                    //   as their :__scope and look up free names in
                    //   target's chain at call time, regardless of
                    //   where they're invoked from. real isolation
                    //   for [bundle apply: target].
                    //
                    //   SLOT-SNAPSHOT (no bindings table) → INJECT.
                    //   copy slots into the current heap.env's
                    //   bindings, run, restore. used by vau bodies
                    //   where `$e` carries caller locals that should
                    //   be visible as globals during eval (do-notation,
                    //   defmethod, defserver, etc.).
                    let saved_env = heap.env;
                    let saved_lex = heap.lexical_scope;
                    let mut swapped = false;
                    let saved_target_parent: Option<(u32, Value)> = None;
                    let mut saved_values: Vec<(u32, Option<Value>)> = Vec::new();
                    if let Some(env_id) = env_val.as_any_object() {
                        let bind_sym = heap.find_symbol("bindings");
                        let is_real_env = bind_sym
                            .and_then(|s| heap.get(env_id).slot_get(s))
                            .and_then(|b| b.as_any_object())
                            .map(|bid| heap.is_table(Value::nursery(bid)))
                            .unwrap_or(false);
                        if is_real_env {
                            // Just swap. Don't mutate target.parent —
                            // its real chain is what defines the lookup
                            // path. (a previous design transiently
                            // re-chained target.parent = caller's env
                            // to make "outer names visible during eval"
                            // work, but that creates a cycle whenever
                            // target is an ancestor of caller's env —
                            // e.g. (eval form vat-root) from inside a
                            // nested call. with locals-in-env, the
                            // closure's natural scope chain reaches
                            // outer names without the patch.)
                            heap.env = env_id;
                            heap.lexical_scope = env_id;
                            swapped = true;
                        } else {
                            let slot_names = heap.get(env_id).slot_names();
                            let slot_vals: Vec<Value> = slot_names.iter()
                                .map(|&n| heap.get(env_id).slot_get(n).unwrap_or(Value::NIL))
                                .collect();
                            for (&name, &val) in slot_names.iter().zip(slot_vals.iter()) {
                                saved_values.push((name, heap.env_get(name)));
                                heap.env_def(name, val);
                            }
                        }
                    }

                    let compile_result = match crate::lang::compiler::Compiler::compile_toplevel(heap, ast) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = heap.make_error(&format!("eval compile: {e}"));
                            if swapped {
                                heap.env = saved_env;
                                heap.lexical_scope = saved_lex;
                            }
                            if let Some((env_id, prior)) = saved_target_parent {
                                let par_sym = heap.intern("parent");
                                heap.get_mut(env_id).slot_set(par_sym, prior);
                            }
                            for (name, old_val) in saved_values {
                                match old_val {
                                    Some(v) => { heap.env_def(name, v); }
                                    None => { heap.env_remove(name); }
                                }
                            }
                            self.frames.last_mut().unwrap().regs[a as usize] = err;
                            continue;
                        }
                    };
                    let result = match self.eval_result(heap, compile_result) {
                        Ok(v) => v,
                        Err(e) => heap.make_error(&e),
                    };

                    if swapped {
                        heap.env = saved_env;
                        heap.lexical_scope = saved_lex;
                    }
                    if let Some((env_id, prior)) = saved_target_parent {
                        let par_sym = heap.intern("parent");
                        heap.get_mut(env_id).slot_set(par_sym, prior);
                    }
                    for (name, old_val) in saved_values {
                        match old_val {
                            Some(v) => { heap.env_def(name, v); }
                            None => { heap.env_remove(name); }
                        }
                    }

                    // ensure we're back to the right frame (eval_result may have
                    // pushed/popped frames; on error the stack might differ)
                    while self.frames.len() > saved_frame_depth {
                        self.frames.pop();
                    }
                    if let Some(f) = self.frames.last_mut() {
                        if (a as usize) < f.regs.len() {
                            f.regs[a as usize] = result;
                        }
                    } else {
                        return Ok(result);
                    }
                }

                // DEPRECATED: TryCatch and Throw opcodes are retained in the opcode enum for
                // bytecode compatibility auditing only. The compiler does NOT emit these.
                // The VM unconditionally rejects them at runtime with an error.
                // See docs/core-contract-matrix.md for status.
                Op::TryCatch | Op::Throw => {
                    return Err("try/catch/error removed — use Result values".into());
                }

                _ => return Err(format!("unimplemented opcode: {opcode:?}")),
            }
        }
    }

    /// Recursive dispatch helper (used by Call opcode compatibility and DNU).
    /// For most sends, the frame-based run() loop handles dispatch directly.
    fn dispatch_send(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        // if receiver is an Err, short-circuit UNLESS the Err prototype
        // handles this selector (show, describe, ok?, recover:, then:, etc.)
        if Self::is_err_value(heap, receiver) && selector != heap.sym_dnu {
            // check if Err has a handler for this selector
            if dispatch::lookup_handler(heap, receiver, selector).is_err() {
                return Ok(receiver);
            }
        }
        // if any arg is an Err, short-circuit (poison in args)
        for arg in args {
            if Self::is_err_value(heap, *arg) {
                return Ok(*arg);
            }
        }
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
                // return Err as a moof value, not a Rust error
                Ok(heap.make_error(&err))
            }
        }
    }

    /// Check if a value is a moof Err (PROTO_ERR prototype).
    fn is_err_value(heap: &Heap, val: Value) -> bool {
        let err_proto = heap.lookup_type("Err");
        if err_proto.is_nil() { return false; }
        heap.prototype_of(val) == err_proto
    }

    /// Call a handler recursively (pushes frame, runs, returns result).
    fn call_handler_recursive(&mut self, heap: &mut Heap, handler: Value, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        if dispatch::is_native(heap, handler) {
            match dispatch::call_native(heap, handler, receiver, args) {
                Ok(val) => return Ok(val),
                Err(msg) => return Ok(heap.make_error(&msg)),
            }
        } else if let Some((code_idx, is_operative)) = heap.as_closure(handler) {
            if is_operative {
                self.materialize_current_frame(heap);
            }
            let unpacked: Vec<Value> =
                if selector == heap.sym_call && handler == receiver {
                    let arg_list = args.first().copied().unwrap_or(Value::NIL);
                    heap.list_to_vec(arg_list)
                } else {
                    let mut full = Vec::with_capacity(1 + args.len());
                    full.push(receiver);
                    full.extend_from_slice(args);
                    full
                };
            match self.push_closure_frame(heap, handler, code_idx, unpacked, 0) {
                Ok(()) => self.run(heap),
                Err(msg) => Ok(heap.make_error(&msg)),
            }
        } else {
            Ok(heap.make_error("handler is not callable"))
        }
    }

    /// Public interface: send a message to a value.
    pub fn send_message(&mut self, heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        self.dispatch_send(heap, receiver, selector, args)
    }

    /// Call a closure/callable value with args.
    /// Wraps args in a cons list (as expected by call: dispatch).
    pub fn call_value(&mut self, heap: &mut Heap, callable: Value, args: &[Value]) -> Result<Value, String> {
        let sel = heap.sym_call;
        let arg_list = heap.list(args);
        self.dispatch_send(heap, callable, sel, &[arg_list])
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

    /// Evaluate a CompileResult with a known outer source record.
    /// Pushes `source` onto the active-sources stack so any Op::Eval
    /// invoked during this evaluation (vau macro expansion) produces
    /// closures carrying the same source text.
    pub fn eval_result_with_source(
        &mut self,
        heap: &mut Heap,
        result: CompileResult,
        source: Option<moof_core::source::ClosureSource>,
    ) -> Result<Value, String> {
        self.active_sources.push(source);
        let out = self.eval_result(heap, result);
        self.active_sources.pop();
        out
    }

    /// Evaluate a CompileResult, accumulating closure descs.
    /// Inherits the current active-source from the stack (for nested
    /// Op::Eval invocations). Prefer `eval_result_with_source` from
    /// top-level eval paths so the source is available to any inner
    /// Op::Eval triggered by macro expansion.
    pub fn eval_result(&mut self, heap: &mut Heap, result: CompileResult) -> Result<Value, String> {
        let base_idx = self.closure_descs.len();
        self.closure_descs.extend(result.closure_descs);
        let chunk = result.chunk;
        // inherited source for descs that didn't get one at compile time
        // (produced by Op::Eval from a runtime-constructed AST).
        let inherited = self.active_sources.last().cloned().flatten();
        for i in base_idx..self.closure_descs.len() {
            self.closure_descs[i].desc_base = base_idx;
            if self.closure_descs[i].source.is_none() {
                self.closure_descs[i].source = inherited.clone();
            }
            // mirror source into the heap so native handlers on the
            // Block prototype can read it without VM access.
            if let Some(src) = self.closure_descs[i].source.clone() {
                heap.register_closure_source(i, src);
            }
        }
        // push frame with desc_base set correctly
        let regs = vec![Value::NIL; chunk.num_regs as usize + 1];
        self.frames.push(Frame {
            regs,
            pc: 0,
            chunk,
            desc_base: base_idx,
            result_reg: 0,
            saved_env: None,
            saved_lex: None,
            lazy_env: None,
        });
        self.run(heap)
    }
}

/// Convenience: evaluate a chunk in a fresh VM (for tests).
pub fn eval_chunk(heap: &mut Heap, chunk: &Chunk) -> Result<Value, String> {
    let mut vm = VM::new();
    vm.execute(heap, Arc::new(chunk.clone()), Value::NIL)
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

    #[test]
    fn rejects_trycatch_opcode() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 1;
        chunk.emit(Op::TryCatch, 0, 0, 0);

        let err = eval_chunk(&mut heap, &chunk).unwrap_err();
        assert!(err.contains("try/catch/error removed"));
    }

    #[test]
    fn rejects_throw_opcode() {
        let mut heap = Heap::new();
        let mut chunk = Chunk::new("test", 0, 0);
        chunk.num_regs = 1;
        chunk.emit(Op::Throw, 0, 0, 0);

        let err = eval_chunk(&mut heap, &chunk).unwrap_err();
        assert!(err.contains("try/catch/error removed"));
    }
}
