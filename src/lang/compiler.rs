// Compiler: cons-cell ASTs → register bytecode.
//
// Walks the AST (cons cells in the heap) and emits bytecode into Chunks.
// Handles special forms: def, quote, send, if, fn, %dot, %block, %object-literal.

use crate::heap::Heap;
use crate::object::HeapObject;
use crate::opcodes::{Chunk, Op};
use crate::value::Value;

pub struct ClosureDesc {
    pub chunk: Chunk,
    pub param_names: Vec<u32>,
    pub is_operative: bool,
    pub capture_names: Vec<u32>,
    pub capture_parent_regs: Vec<u8>,  // which parent registers to read
    pub capture_local_regs: Vec<u8>,   // which local registers to write captures into
    pub capture_values: Vec<Value>,
    pub desc_base: usize,
    pub rest_param_reg: Option<u8>,  // if set, extra args go here as a list
}

pub struct CompileResult {
    pub chunk: Chunk,
    pub closure_descs: Vec<ClosureDesc>,
}

pub struct Compiler<'a> {
    heap: &'a Heap,
    chunk: Chunk,
    next_reg: u8,
    pub closure_descs: Vec<ClosureDesc>,
    locals: Vec<(u32, u8)>,
    captures: Vec<(u32, u8, u8)>, // (symbol_id, parent_reg, local_reg)
    parent_locals: Vec<(u32, u8)>, // locals from the enclosing compiler (includes its captures)
}

impl<'a> Compiler<'a> {
    pub fn new(heap: &'a Heap, name: &str) -> Self {
        Compiler {
            heap,
            chunk: Chunk::new(name, 0, 0),
            next_reg: 0,
            closure_descs: Vec::new(),
            locals: Vec::new(),
            captures: Vec::new(),
            parent_locals: Vec::new(),
        }
    }

    fn alloc_reg(&mut self) -> u8 {
        let r = self.next_reg;
        self.next_reg += 1;
        if self.next_reg > self.chunk.num_regs {
            self.chunk.num_regs = self.next_reg;
        }
        r
    }

    fn find_local(&mut self, sym_id: u32) -> Option<u8> {
        // search own locals first
        if let Some((_, r)) = self.locals.iter().rev().find(|(s, _)| *s == sym_id) {
            return Some(*r);
        }
        // check if already captured
        if let Some((_, _, lr)) = self.captures.iter().find(|(s, _, _)| *s == sym_id) {
            return Some(*lr);
        }
        // check parent locals — if found, add as a capture
        if let Some((_, parent_reg)) = self.parent_locals.iter().rev().find(|(s, _)| *s == sym_id) {
            let parent_reg = *parent_reg;
            let local_reg = self.alloc_reg();
            self.captures.push((sym_id, parent_reg, local_reg));
            self.locals.push((sym_id, local_reg));
            return Some(local_reg);
        }
        None
    }

    /// Build parent_locals for a sub-compiler. Includes both this compiler's
    /// locals AND any parent_locals entries that aren't already captured.
    /// Forces intermediate captures so grandparent variables are accessible.
    fn build_sub_parent_locals(&mut self) -> Vec<(u32, u8)> {
        // force-capture all parent_locals entries into our own locals
        // so they're available in our frame for the sub to capture from
        for i in 0..self.parent_locals.len() {
            let (sym, _parent_reg) = self.parent_locals[i];
            // check if we already have this as a local or capture
            let already_local = self.locals.iter().any(|(s, _)| *s == sym);
            if !already_local {
                let parent_reg = self.parent_locals[i].1;
                let local_reg = self.alloc_reg();
                self.captures.push((sym, parent_reg, local_reg));
                self.locals.push((sym, local_reg));
            }
        }
        self.locals.clone()
    }

    fn add_sym_const(&mut self, sym_id: u32) -> u16 {
        self.chunk.add_constant(Value::symbol(sym_id).to_bits())
    }

    fn emit_load_const(&mut self, dst: u8, val: Value) {
        let idx = self.chunk.add_constant(val.to_bits());
        self.chunk.emit(Op::LoadConst, dst, (idx >> 8) as u8, idx as u8);
    }

    /// Compile an expression, placing the result in `dst`.
    pub fn compile_expr(&mut self, expr: Value, dst: u8) -> Result<(), String> {
        // primitives
        if expr.is_nil() { self.chunk.emit(Op::LoadNil, dst, 0, 0); return Ok(()); }
        if expr.is_true() { self.chunk.emit(Op::LoadTrue, dst, 0, 0); return Ok(()); }
        if expr.is_false() { self.chunk.emit(Op::LoadFalse, dst, 0, 0); return Ok(()); }
        if expr.is_integer() {
            let n = expr.as_integer().unwrap();
            if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
                let bytes = (n as i16).to_be_bytes();
                self.chunk.emit(Op::LoadInt, dst, bytes[0], bytes[1]);
            } else {
                self.emit_load_const(dst, expr);
            }
            return Ok(());
        }
        if expr.is_float() { self.emit_load_const(dst, expr); return Ok(()); }

        // symbol reference → check locals first, then globals
        if expr.is_symbol() {
            let sym_id = expr.as_symbol().unwrap();
            let name = self.heap.symbol_name(sym_id);
            // skip well-known internal symbols
            if name == "send" || name == "%dot" || name == "%object-literal" || name == "%eventual-send" || name == "%table-literal" {
                self.emit_load_const(dst, expr);
            } else if let Some(reg) = self.find_local(sym_id) {
                // local variable — just copy from its register
                if reg != dst {
                    self.chunk.emit(Op::Move, dst, reg, 0);
                }
            } else {
                // global variable
                let idx = self.add_sym_const(sym_id);
                self.chunk.emit(Op::GetGlobal, dst, (idx >> 8) as u8, idx as u8);
            }
            return Ok(());
        }

        // must be a heap object — string literal or cons cell (a form)
        let id = expr.as_any_object().ok_or("compile: unexpected value")?;

        match self.heap.get(id) {
            HeapObject::Text(_) => {
                self.emit_load_const(dst, expr);
                return Ok(());
            }
            HeapObject::Pair(_, _) => {}
            _ => {
                self.emit_load_const(dst, expr);
                return Ok(());
            }
        }

        // it's a list form — check the head
        let car = self.heap.car(id);
        let cdr_val = self.heap.cdr(id);

        if let Some(sym_id) = car.as_symbol() {
            let name = self.heap.symbol_name(sym_id).to_string();
            // is this symbol still bound to its bootstrap value?
            let stable = !self.heap.rebound.contains(&sym_id);
            match name.as_str() {
                // ── KERNEL FORMS (always compiled, never overridable) ──
                "quote" => {
                    // (quote x) → load x as constant
                    let arg = self.first_arg(cdr_val)?;
                    self.emit_load_const(dst, arg);
                    return Ok(());
                }

                "quasiquote" => {
                    // (quasiquote form) — build AST with unquote interpolation
                    let arg = self.first_arg(cdr_val)?;
                    self.compile_quasiquote(arg, dst)?;
                    return Ok(());
                }

                "unquote" => {
                    // (unquote expr) — only valid inside quasiquote
                    return Err("unquote outside of quasiquote".into());
                }

                "send" => {
                    // (send receiver 'selector args...)
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("send: need at least receiver and selector".into());
                    }

                    let recv_reg = self.alloc_reg();
                    self.compile_expr(items[1], recv_reg)?;

                    // selector is (quote sym) — extract the sym
                    let sel_val = self.extract_quoted(items[2])?;
                    let sel_sym = sel_val.as_symbol().ok_or("send: selector must be a symbol")?;
                    let sel_const = self.add_sym_const(sel_sym);

                    // compile args
                    let mut arg_regs = Vec::new();
                    for i in 3..items.len() {
                        let r = self.alloc_reg();
                        self.compile_expr(items[i], r)?;
                        arg_regs.push(r);
                    }

                    // emit SEND dst, recv, sel_const
                    self.chunk.emit(Op::Send, dst, recv_reg, sel_const as u8);
                    // emit nargs + arg registers (packed into next 4 bytes)
                    let nargs = arg_regs.len() as u8;
                    let a0 = arg_regs.first().copied().unwrap_or(0);
                    let a1 = arg_regs.get(1).copied().unwrap_or(0);
                    let a2 = arg_regs.get(2).copied().unwrap_or(0);
                    self.chunk.code.push(nargs);
                    self.chunk.code.push(a0);
                    self.chunk.code.push(a1);
                    self.chunk.code.push(a2);

                    return Ok(());
                }

                "def" => {
                    // (def name value) → compile value, then DEF_GLOBAL
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 {
                        return Err("def: need name and value".into());
                    }
                    let name_sym = items[1].as_symbol()
                        .ok_or("def: name must be a symbol")?;
                    // compile the value into dst
                    self.compile_expr(items[2], dst)?;
                    // emit DEF_GLOBAL
                    let idx = self.add_sym_const(name_sym);
                    self.chunk.emit(Op::DefGlobal, (idx >> 8) as u8, idx as u8, dst);
                    return Ok(());
                }

                "do" if stable => {
                    // (do expr1 expr2 ... exprN) → compile all, return last
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 2 {
                        self.chunk.emit(Op::LoadNil, dst, 0, 0);
                        return Ok(());
                    }
                    for i in 1..items.len() - 1 {
                        let tmp = self.alloc_reg();
                        self.compile_expr(items[i], tmp)?;
                    }
                    self.compile_expr(*items.last().unwrap(), dst)?;
                    return Ok(());
                }

                "vau" => {
                    // (vau (params) $env body...) or (vau rest-param $env body...)
                    // creates an operative — receives unevaluated args + caller env
                    // params bind to the raw AST args, $env binds to caller env
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 4 {
                        return Err("vau: need (params) $env body".into());
                    }
                    let (positional, rest_param) = self.heap.extract_params(items[1]);
                    let mut param_syms: Vec<u32> = positional;

                    // $env param (items[2]) — must start with $
                    let env_sym = items[2].as_symbol()
                        .ok_or("vau: env param must be a symbol")?;
                    let env_name = self.heap.symbol_name(env_sym);
                    if !env_name.starts_with('$') {
                        return Err("vau: env param must start with $".into());
                    }
                    param_syms.push(env_sym); // env is last param
                    let arity = param_syms.len() as u8;

                    // compile body — force-capture ancestor variables
                    let mut sub = Compiler::new(self.heap, "<vau>");
                    sub.parent_locals = self.build_sub_parent_locals();
                    sub.chunk.arity = arity;
                    for &sym in &param_syms {
                        let reg = sub.alloc_reg();
                        sub.locals.push((sym, reg));
                    }
                    // rest param gets extra args as a list (before env)
                    let rest_reg = if let Some(rest_sym) = rest_param {
                        let reg = sub.alloc_reg();
                        sub.locals.push((rest_sym, reg));
                        Some(reg)
                    } else { None };
                    let body_dst = sub.alloc_reg();
                    if items.len() == 4 {
                        sub.compile_expr(items[3], body_dst)?;
                    } else {
                        for i in 3..items.len() - 1 {
                            let tmp = sub.alloc_reg();
                            sub.compile_expr(items[i], tmp)?;
                        }
                        sub.compile_expr(*items.last().unwrap(), body_dst)?;
                    }
                    sub.chunk.emit(Op::Return, body_dst, 0, 0);
                    sub.chunk.optimize_tail_calls();
                    let capture_names: Vec<u32> = sub.captures.iter().map(|(s, _, _)| *s).collect();
                    let capture_parent_regs: Vec<u8> = sub.captures.iter().map(|(_, r, _)| *r).collect();
                    let capture_local_regs: Vec<u8> = sub.captures.iter().map(|(_, _, lr)| *lr).collect();
                    let sub_result = sub.finish();

                    let sub_descs_offset = self.closure_descs.len();
                    let n_sub_descs = sub_result.closure_descs.len();
                    self.closure_descs.extend(sub_result.closure_descs);

                    let mut chunk = sub_result.chunk;
                    if n_sub_descs > 0 {
                        let mut pc = 0;
                        while pc + 3 < chunk.code.len() {
                            if crate::opcodes::Op::from_u8(chunk.code[pc]) == Some(crate::opcodes::Op::MakeClosure) {
                                let old = u16::from_be_bytes([chunk.code[pc + 2], chunk.code[pc + 3]]);
                                let new_idx = old + sub_descs_offset as u16;
                                chunk.code[pc + 2] = (new_idx >> 8) as u8;
                                chunk.code[pc + 3] = new_idx as u8;
                            }
                            pc += 4;
                            if pc >= 4 {
                                let prev = crate::opcodes::Op::from_u8(chunk.code[pc - 4]);
                                if prev == Some(crate::opcodes::Op::Send) || prev == Some(crate::opcodes::Op::TailCall) {
                                    pc += 4;
                                }
                            }
                        }
                    }

                    let desc = ClosureDesc {
                        chunk,
                        param_names: param_syms,
                        is_operative: true,
                        capture_names,
                        capture_parent_regs,
                        capture_local_regs,
                        capture_values: Vec::new(), desc_base: 0, rest_param_reg: rest_reg,
                    };
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);
                    return Ok(());
                }

                "fn" | "lambda" if stable => {
                    // (fn (params...) body...) → compile body as a sub-chunk, emit MakeClosure
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("fn: need params and body".into());
                    }
                    // extract param names (with optional rest param)
                    let (positional, rest_param) = self.heap.extract_params(items[1]);
                    let arity = positional.len() as u8;

                    let mut sub = Compiler::new(self.heap, "<fn>");
                    sub.parent_locals = self.build_sub_parent_locals();
                    sub.chunk.arity = arity;
                    for &sym in &positional {
                        let reg = sub.alloc_reg();
                        sub.locals.push((sym, reg));
                    }
                    // rest param gets its own register (filled by VM)
                    let rest_reg = if let Some(rest_sym) = rest_param {
                        let reg = sub.alloc_reg();
                        sub.locals.push((rest_sym, reg));
                        Some(reg)
                    } else { None };
                    let body_dst = sub.alloc_reg();
                    // if multiple body exprs, wrap in do
                    if items.len() == 3 {
                        sub.compile_expr(items[2], body_dst)?;
                    } else {
                        // multiple body expressions — compile each, return last
                        for i in 2..items.len() - 1 {
                            let tmp = sub.alloc_reg();
                            sub.compile_expr(items[i], tmp)?;
                        }
                        sub.compile_expr(*items.last().unwrap(), body_dst)?;
                    }
                    sub.chunk.emit(Op::Return, body_dst, 0, 0);
                    // peephole: replace Send+Return with TailCall
                    sub.chunk.optimize_tail_calls();
                    let capture_names: Vec<u32> = sub.captures.iter().map(|(s, _, _)| *s).collect();
                    let capture_parent_regs: Vec<u8> = sub.captures.iter().map(|(_, r, _)| *r).collect();
                    let capture_local_regs: Vec<u8> = sub.captures.iter().map(|(_, _, lr)| *lr).collect();
                    let sub_result = sub.finish();

                    // pull up nested closure descs, patching the fn's chunk
                    let sub_descs_offset = self.closure_descs.len();
                    let n_sub_descs = sub_result.closure_descs.len();
                    self.closure_descs.extend(sub_result.closure_descs);

                    // patch MakeClosure indices inside the fn's chunk
                    // they were compiled relative to sub_index 0, but in the
                    // parent they start at sub_descs_offset
                    let mut chunk = sub_result.chunk;
                    if n_sub_descs > 0 {
                        let mut pc = 0;
                        while pc + 3 < chunk.code.len() {
                            if crate::opcodes::Op::from_u8(chunk.code[pc]) == Some(crate::opcodes::Op::MakeClosure) {
                                let old = u16::from_be_bytes([chunk.code[pc + 2], chunk.code[pc + 3]]);
                                let new_idx = old + sub_descs_offset as u16;
                                chunk.code[pc + 2] = (new_idx >> 8) as u8;
                                chunk.code[pc + 3] = new_idx as u8;
                            }
                            pc += 4;
                            // skip Send/TailCall trailing data
                            if pc >= 4 {
                                let prev = crate::opcodes::Op::from_u8(chunk.code[pc - 4]);
                                if prev == Some(crate::opcodes::Op::Send) || prev == Some(crate::opcodes::Op::TailCall) {
                                    pc += 4;
                                }
                            }
                        }
                    }

                    let desc = ClosureDesc {
                        chunk,
                        param_names: positional,
                        is_operative: false,
                        capture_names,
                        capture_parent_regs,
                        capture_local_regs,
                        capture_values: Vec::new(), desc_base: 0, rest_param_reg: rest_reg,
                    };
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    // emit MakeClosure with index
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);

                    return Ok(());
                }

                "let" if stable => {
                    // (let ((x 1) (y 2)) body...)
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("let: need bindings and body".into());
                    }
                    let bindings_list = self.heap.list_to_vec(items[1]);
                    let saved_locals_len = self.locals.len();

                    // compile each binding and register as local
                    for binding in &bindings_list {
                        let pair = self.heap.list_to_vec(*binding);
                        if pair.len() != 2 {
                            return Err("let: each binding must be (name value)".into());
                        }
                        let name_sym = pair[0].as_symbol()
                            .ok_or("let: binding name must be a symbol")?;
                        let reg = self.alloc_reg();
                        self.compile_expr(pair[1], reg)?;
                        self.locals.push((name_sym, reg));
                    }

                    // compile body expressions, return last
                    if items.len() == 3 {
                        self.compile_expr(items[2], dst)?;
                    } else {
                        for i in 2..items.len() - 1 {
                            let tmp = self.alloc_reg();
                            self.compile_expr(items[i], tmp)?;
                        }
                        self.compile_expr(*items.last().unwrap(), dst)?;
                    }

                    // restore locals (let bindings go out of scope)
                    self.locals.truncate(saved_locals_len);
                    return Ok(());
                }

                // "while" removed — use recursion instead

                "cons" if stable => {
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 { return Err("cons: need car and cdr".into()); }
                    let car_reg = self.alloc_reg();
                    let cdr_reg = self.alloc_reg();
                    self.compile_expr(items[1], car_reg)?;
                    self.compile_expr(items[2], cdr_reg)?;
                    self.chunk.emit(Op::Cons, dst, car_reg, cdr_reg);
                    return Ok(());
                }

                "eq" if stable => {
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 { return Err("eq: need two args".into()); }
                    let a_reg = self.alloc_reg();
                    let b_reg = self.alloc_reg();
                    self.compile_expr(items[1], a_reg)?;
                    self.compile_expr(items[2], b_reg)?;
                    self.chunk.emit(Op::Eq, dst, a_reg, b_reg);
                    return Ok(());
                }

                "eval" if stable => {
                    // (eval expr) or (eval expr env)
                    // expr is an AST. env is an optional environment object.
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 2 { return Err("eval: need at least one arg".into()); }
                    let ast_reg = self.alloc_reg();
                    self.compile_expr(items[1], ast_reg)?;
                    let env_reg = if items.len() >= 3 {
                        let r = self.alloc_reg();
                        self.compile_expr(items[2], r)?;
                        r
                    } else {
                        0 // 0 = no env
                    };
                    self.chunk.emit(Op::Eval, dst, ast_reg, env_reg);
                    return Ok(());
                }

                // ":=" removed — all values are immutable.
                // use let bindings, fold, recursion, or servers for state.

                "if" if stable => {
                    // (if cond then else)
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("if: need condition and then branch".into());
                    }

                    // compile condition
                    let cond_reg = self.alloc_reg();
                    self.compile_expr(items[1], cond_reg)?;

                    // emit jump-if-false to else branch
                    let jump_to_else = self.chunk.offset();
                    self.chunk.emit(Op::JumpIfFalse, cond_reg, 0, 0);

                    // compile then branch
                    self.compile_expr(items[2], dst)?;

                    if items.len() > 3 {
                        // jump over else
                        let jump_over_else = self.chunk.offset();
                        self.chunk.emit(Op::Jump, 0, 0, 0);

                        // patch jump-to-else
                        let else_start = self.chunk.offset();
                        let delta = (else_start as i16) - (jump_to_else as i16) - 4;
                        let bytes = delta.to_be_bytes();
                        self.chunk.code[jump_to_else + 2] = bytes[0];
                        self.chunk.code[jump_to_else + 3] = bytes[1];

                        // compile else branch
                        self.compile_expr(items[3], dst)?;

                        // patch jump-over-else
                        let end = self.chunk.offset();
                        let delta = (end as i16) - (jump_over_else as i16) - 4;
                        let bytes = delta.to_be_bytes();
                        self.chunk.code[jump_over_else + 1] = bytes[0];
                        self.chunk.code[jump_over_else + 2] = bytes[1];
                    } else {
                        // no else branch: jump over nil load, then nil load for false case
                        let jump_over_nil = self.chunk.offset();
                        self.chunk.emit(Op::Jump, 0, 0, 0);

                        // patch jump-to-else → land here (at the LoadNil)
                        let nil_start = self.chunk.offset();
                        let delta = (nil_start as i16) - (jump_to_else as i16) - 4;
                        let bytes = delta.to_be_bytes();
                        self.chunk.code[jump_to_else + 2] = bytes[0];
                        self.chunk.code[jump_to_else + 3] = bytes[1];

                        self.chunk.emit(Op::LoadNil, dst, 0, 0);

                        // patch jump-over-nil
                        let end = self.chunk.offset();
                        let delta2 = (end as i16) - (jump_over_nil as i16) - 4;
                        let bytes2 = delta2.to_be_bytes();
                        self.chunk.code[jump_over_nil + 1] = bytes2[0];
                        self.chunk.code[jump_over_nil + 2] = bytes2[1];
                    }

                    return Ok(());
                }

                "%table-literal" => {
                    // (%table-literal (seq...) (k1 v1 k2 v2...))
                    // compiles to: create empty table, push seq items, at:put: kv pairs
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 {
                        return Err("table literal: need seq and kv lists".into());
                    }

                    let seq_items = self.heap.list_to_vec(items[1]);
                    let kv_items = self.heap.list_to_vec(items[2]);
                    let nseq = seq_items.len();
                    let nmap = kv_items.len() / 2;

                    // for small tables (<=10 entries), use the old MakeTable opcode
                    // for large tables, use incremental at:put: to avoid register overflow
                    if nseq + nmap * 2 <= 20 {
                        let mut seq_regs = Vec::with_capacity(nseq);
                        for item in &seq_items {
                            let r = self.alloc_reg();
                            self.compile_expr(*item, r)?;
                            seq_regs.push(r);
                        }
                        let mut kv_regs = Vec::with_capacity(kv_items.len());
                        for item in &kv_items {
                            let r = self.alloc_reg();
                            self.compile_expr(*item, r)?;
                            kv_regs.push(r);
                        }
                        self.chunk.emit(Op::MakeTable, dst, nseq as u8, nmap as u8);
                        let total_regs = seq_regs.len() + kv_regs.len();
                        let mut trailing = Vec::with_capacity(total_regs);
                        trailing.extend_from_slice(&seq_regs);
                        trailing.extend_from_slice(&kv_regs);
                        while trailing.len() % 4 != 0 { trailing.push(0); }
                        self.chunk.code.extend_from_slice(&trailing);
                    } else {
                        // large table: create empty, then push/put incrementally
                        // this avoids register overflow for large tables
                        self.chunk.emit(Op::MakeTable, dst, 0, 0); // empty table

                        // TODO: sequential items for large tables (not needed for Iterable)
                        // for now, large tables with seq items use the small path

                        // at:put: keyed items using the well-known symbol
                        let at_put_const = self.add_sym_const(self.heap.sym_at_put);
                        let key_reg = self.alloc_reg();
                        let val_reg = self.alloc_reg();
                        let discard_reg = self.alloc_reg(); // don't overwrite dst!
                        for i in 0..nmap {
                            self.compile_expr(kv_items[i * 2], key_reg)?;
                            self.compile_expr(kv_items[i * 2 + 1], val_reg)?;
                            // send at:put: to the table (dst), result goes to discard
                            self.chunk.emit(Op::Send, discard_reg, dst, at_put_const as u8);
                            self.chunk.code.push(2);
                            self.chunk.code.push(key_reg);
                            self.chunk.code.push(val_reg);
                            self.chunk.code.push(0);
                        }
                    }

                    return Ok(());
                }

                "%object-literal" => {
                    // (%object-literal parent (slot-names...) slot-val1... (methods...) (protocols...))
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("object literal: need parent and slot names".into());
                    }

                    // compile parent
                    let parent_reg = self.alloc_reg();
                    self.compile_expr(items[1], parent_reg)?;

                    // get slot names
                    let name_list = self.heap.list_to_vec(items[2]);
                    let nslots = name_list.len();

                    // compile slot values (items[3..3+nslots])
                    let mut val_regs = Vec::with_capacity(nslots);
                    for i in 0..nslots {
                        let r = self.alloc_reg();
                        self.compile_expr(items[3 + i], r)?;
                        val_regs.push(r);
                    }

                    // emit MakeObj — now with clone flag (nslots | 0x80 means clone parent)
                    // the high bit of nslots signals "clone parent slots"
                    self.chunk.emit(Op::MakeObj, dst, parent_reg, (nslots as u8) | 0x80);

                    // emit slot name/value pairs as trailing data
                    for i in 0..nslots {
                        let name_sym = self.extract_quoted(name_list[i])?
                            .as_symbol().ok_or("object literal: slot name must be a symbol")?;
                        let name_const = self.add_sym_const(name_sym);
                        self.chunk.code.push((name_const >> 8) as u8);
                        self.chunk.code.push(name_const as u8);
                        self.chunk.code.push(val_regs[i]);
                        self.chunk.code.push(0);
                    }

                    // methods: items[3+nslots] is the methods list (sel1 fn1 sel2 fn2...)
                    let methods_idx = 3 + nslots;
                    if methods_idx < items.len() {
                        let methods_list = self.heap.list_to_vec(items[methods_idx]);
                        // pairs: (quoted-sel, fn-expr)
                        let mut i = 0;
                        while i + 1 < methods_list.len() {
                            let sel = self.extract_quoted(methods_list[i])?
                                .as_symbol().ok_or("method selector must be a symbol")?;
                            let fn_expr = methods_list[i + 1];

                            // compile the fn expression
                            let handler_reg = self.alloc_reg();
                            self.compile_expr(fn_expr, handler_reg)?;

                            // emit SetHandler on dst
                            let sel_const = self.add_sym_const(sel);
                            self.chunk.emit(Op::SetHandler, dst, sel_const as u8, handler_reg);

                            i += 2;
                        }
                    }

                    // init block: items[3+nslots+1] is the init expressions list
                    // these run with `self` bound to the newly created object
                    let init_idx = 3 + nslots + 1;
                    if init_idx < items.len() {
                        let init_list = self.heap.list_to_vec(items[init_idx]);
                        if !init_list.is_empty() {
                            // temporarily bind `self` to dst for the init block
                            let self_sym = self.heap.find_symbol("self").unwrap_or(0);
                            let saved_locals = self.locals.len();
                            self.locals.push((self_sym, dst));

                            // compile each init expression
                            for init_expr in &init_list {
                                let tmp = self.alloc_reg();
                                self.compile_expr(*init_expr, tmp)?;
                            }

                            // restore locals
                            self.locals.truncate(saved_locals);
                        }
                    }

                    return Ok(());
                }

                "%dot" => {
                    // (%dot obj 'field) → send slotAt: to obj
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 { return Err("%dot: need obj and field".into()); }

                    let recv_reg = self.alloc_reg();
                    self.compile_expr(items[1], recv_reg)?;

                    // the field name is (quote sym), compile it as the arg to slotAt:
                    let field = self.extract_quoted(items[2])?;
                    let field_reg = self.alloc_reg();
                    self.emit_load_const(field_reg, field);

                    let sel_const = self.add_sym_const(self.heap.sym_slot_at);
                    self.chunk.emit(Op::Send, dst, recv_reg, sel_const as u8);
                    self.chunk.code.push(1); // nargs = 1
                    self.chunk.code.push(field_reg);
                    self.chunk.code.push(0);
                    self.chunk.code.push(0);

                    return Ok(());
                }

                // try/catch and error removed — errors are Result values.
                // use { Err message: "..." } for application errors.
                // use recover: on Acts/Results for error handling.

                _ => {
                    // check if this is a known operative (derived from value)
                    if let Some(val) = self.heap.env_get(sym_id) {
                        if let Some((_, true)) = self.heap.as_closure(val) {
                            return self.compile_operative_call(expr, sym_id, dst);
                        }
                    }
                    // generic applicative call: (f a b c) → send call: to f
                    return self.compile_call(expr, dst);
                }
            }
        }

        // head is not a symbol — it's an expression. compile as a call.
        self.compile_call(expr, dst)
    }

    fn compile_quasiquote(&mut self, form: Value, dst: u8) -> Result<(), String> {
        // atoms: load as constants
        if form.is_nil() || form.is_true() || form.is_false()
            || form.is_integer() || form.is_float() || form.is_symbol() {
            self.emit_load_const(dst, form);
            return Ok(());
        }

        let id = form.as_any_object().ok_or("quasiquote: unexpected value")?;
        match self.heap.get(id) {
            crate::object::HeapObject::Text(_) | crate::object::HeapObject::Buffer(_)
            | crate::object::HeapObject::Table { .. } => {
                self.emit_load_const(dst, form);
                return Ok(());
            }
            crate::object::HeapObject::General { .. } |
            crate::object::HeapObject::Closure { .. } |
            crate::object::HeapObject::Environment { .. } => {
                self.emit_load_const(dst, form);
                return Ok(());
            }
            crate::object::HeapObject::Pair(car, cdr) => {
                let car = *car;
                let cdr = *cdr;
                // check for (unquote expr) — evaluate expr normally
                if let Some(sym) = car.as_symbol() {
                    if self.heap.symbol_name(sym) == "unquote" {
                        let arg_id = cdr.as_any_object().ok_or("unquote: missing arg")?;
                        let arg = self.heap.car(arg_id);
                        self.compile_expr(arg, dst)?;
                        return Ok(());
                    }
                    if self.heap.symbol_name(sym) == "unquote-splicing" {
                        // top-level ,@expr — just evaluate
                        let arg_id = cdr.as_any_object().ok_or("unquote-splicing: missing arg")?;
                        let arg = self.heap.car(arg_id);
                        self.compile_expr(arg, dst)?;
                        return Ok(());
                    }
                }
                // check if car is (unquote-splicing expr) — splice into list
                if let Some(car_id) = car.as_any_object() {
                    if let crate::object::HeapObject::Pair(splice_head, splice_rest) = self.heap.get(car_id) {
                        let splice_head = *splice_head;
                        let splice_rest = *splice_rest;
                        if let Some(sym) = splice_head.as_symbol() {
                            if self.heap.symbol_name(sym) == "unquote-splicing" {
                                let arg_id = splice_rest.as_any_object()
                                    .ok_or("unquote-splicing: missing arg")?;
                                let arg = self.heap.car(arg_id);
                                // compile the splice expression (evaluates to a list)
                                let list_reg = self.alloc_reg();
                                self.compile_expr(arg, list_reg)?;
                                // compile the rest of the quasiquoted list
                                let rest_reg = self.alloc_reg();
                                self.compile_quasiquote(cdr, rest_reg)?;
                                // emit [list append: rest] to concatenate
                                let sel_const = self.add_sym_const(
                                    self.heap.find_symbol("append:").unwrap_or(0));
                                self.chunk.emit(Op::Send, dst, list_reg, sel_const as u8);
                                self.chunk.code.push(1); // nargs
                                self.chunk.code.push(rest_reg);
                                self.chunk.code.push(0);
                                self.chunk.code.push(0);
                                return Ok(());
                            }
                        }
                    }
                }
                // recursively quasiquote car and cdr, then cons
                let car_reg = self.alloc_reg();
                let cdr_reg = self.alloc_reg();
                self.compile_quasiquote(car, car_reg)?;
                self.compile_quasiquote(cdr, cdr_reg)?;
                self.chunk.emit(Op::Cons, dst, car_reg, cdr_reg);
                Ok(())
            }
        }
    }

    /// Compile a call to a known operative — quote args, pass as list + env.
    fn compile_operative_call(&mut self, expr: Value, operative_sym: u32, dst: u8) -> Result<(), String> {
        let items = self.heap.list_to_vec(expr);
        // items[0] is the operative name, items[1..] are unevaluated args

        // load the operative itself
        let window_start = self.next_reg;
        let func_reg = self.alloc_reg();
        let func_const = self.add_sym_const(operative_sym);
        self.chunk.emit(Op::GetGlobal, func_reg, (func_const >> 8) as u8, func_const as u8);

        // build a cons list of the unevaluated args as AST data
        // each arg is stored as a constant (it's already a Value — a cons cell or literal)
        let args_reg = self.alloc_reg();
        self.chunk.emit(Op::LoadNil, args_reg, 0, 0);
        for i in (1..items.len()).rev() {
            let arg_reg = self.alloc_reg();
            // store the raw AST as a constant — DON'T compile it
            self.emit_load_const(arg_reg, items[i]);
            self.chunk.emit(Op::Cons, args_reg, arg_reg, args_reg);
        }

        // capture the current local environment as a heap object
        let env_reg = self.alloc_reg();
        let locals_snapshot: Vec<(u32, u8)> = self.locals.clone();
        if locals_snapshot.is_empty() {
            self.chunk.emit(Op::LoadNil, env_reg, 0, 0);
        } else {
            let parent_reg = self.alloc_reg();
            let obj_sym = self.heap.find_symbol("Object").unwrap_or(0);
            let obj_const = self.add_sym_const(obj_sym);
            self.chunk.emit(Op::GetGlobal, parent_reg, (obj_const >> 8) as u8, obj_const as u8);
            let nslots = locals_snapshot.len();
            let mut val_regs = Vec::new();
            for &(_, reg) in &locals_snapshot {
                let r = self.alloc_reg();
                self.chunk.emit(Op::Move, r, reg, 0);
                val_regs.push(r);
            }
            self.chunk.emit(Op::MakeObj, env_reg, parent_reg, nslots as u8);
            for (i, &(sym, _)) in locals_snapshot.iter().enumerate() {
                let name_const = self.add_sym_const(sym);
                self.chunk.code.push((name_const >> 8) as u8);
                self.chunk.code.push(name_const as u8);
                self.chunk.code.push(val_regs[i]);
                self.chunk.code.push(0);
            }
        }

        // call the operative with (args-list, env)
        // operative's params are (user-params... $env)
        // we pass: the args list as first param, env as second
        // the operative body destructures from the args list
        // ... actually, the operative expects individual params, not a list.
        // let's pass the args as individual values (quoted) + env as last

        // build a cons list: (ast-arg1 ast-arg2 ... env)
        // operative params destructure from this list
        let list_reg = self.alloc_reg();
        self.chunk.emit(Op::LoadNil, list_reg, 0, 0);
        // env is last element
        self.chunk.emit(Op::Cons, list_reg, env_reg, list_reg);
        // args in reverse order (so they end up in correct list order)
        for i in (1..items.len()).rev() {
            let r = self.alloc_reg();
            self.emit_load_const(r, items[i]); // raw AST, not compiled
            self.chunk.emit(Op::Cons, list_reg, r, list_reg);
        }

        // emit [operative call: args-list]
        let call_const = self.add_sym_const(self.heap.sym_call);
        self.chunk.emit(Op::Send, dst, func_reg, call_const as u8);
        self.chunk.code.push(1);
        self.chunk.code.push(list_reg);
        self.chunk.code.push(0);
        self.chunk.code.push(0);
        Ok(())
    }

    fn compile_call(&mut self, expr: Value, dst: u8) -> Result<(), String> {
        // (f a b c) → [f call: (cons a (cons b (cons c nil)))]
        // call: takes ONE arg — a list of arguments
        let items = self.heap.list_to_vec(expr);
        if items.is_empty() { return Err("empty call".into()); }

        let recv_reg = self.alloc_reg();
        self.compile_expr(items[0], recv_reg)?;

        // build the args list as a cons chain
        let list_reg = self.alloc_reg();
        self.chunk.emit(Op::LoadNil, list_reg, 0, 0);
        for i in (1..items.len()).rev() {
            let arg_reg = self.alloc_reg();
            self.compile_expr(items[i], arg_reg)?;
            self.chunk.emit(Op::Cons, list_reg, arg_reg, list_reg);
        }

        // emit SEND recv, call:, (the list)
        let call_const = self.add_sym_const(self.heap.sym_call);
        self.chunk.emit(Op::Send, dst, recv_reg, call_const as u8);
        self.chunk.code.push(1); // nargs = 1 (the list)
        self.chunk.code.push(list_reg);
        self.chunk.code.push(0);
        self.chunk.code.push(0);

        Ok(())
    }

    fn first_arg(&self, cdr: Value) -> Result<Value, String> {
        let id = cdr.as_any_object().ok_or("expected argument")?;
        Ok(self.heap.car(id))
    }

    fn extract_quoted(&self, val: Value) -> Result<Value, String> {
        // val should be (quote x) — extract x
        if let Some(id) = val.as_any_object() {
            if let HeapObject::Pair(car, cdr) = self.heap.get(id) {
                if let Some(sym) = car.as_symbol() {
                    if self.heap.symbol_name(sym) == "quote" {
                        if let Some(cdr_id) = cdr.as_any_object() {
                            return Ok(self.heap.car(cdr_id));
                        }
                    }
                }
            }
        }
        // not a quote form, return as-is
        Ok(val)
    }

    fn finish(self) -> CompileResult {
        CompileResult {
            chunk: self.chunk,
            closure_descs: self.closure_descs,
        }
    }

    /// Compile a top-level expression.
    pub fn compile_toplevel(heap: &Heap, expr: Value) -> Result<CompileResult, String> {
        let mut c = Compiler::new(heap, "<toplevel>");
        let dst = c.alloc_reg();
        c.compile_expr(expr, dst)?;
        c.chunk.emit(Op::Return, dst, 0, 0);
        Ok(c.finish())
    }
}

// NOTE: register_type_protos has been moved to src/runtime.rs
// The compiler is now purely AST → bytecode. Runtime init is separate.
