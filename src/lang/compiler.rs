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
    pub desc_base: usize,  // the desc_base when this closure was compiled
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
    parent_locals: Vec<(u32, u8)>, // locals from the enclosing compiler
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
            match name.as_str() {
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

                "do" => {
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
                    // (vau (params) $env body...)
                    // creates an operative — receives unevaluated args + caller env
                    // params bind to the raw AST args, $env binds to caller env
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 4 {
                        return Err("vau: need (params) $env body".into());
                    }
                    let params = self.heap.list_to_vec(items[1]);
                    let mut param_syms: Vec<u32> = params.iter()
                        .map(|p| p.as_symbol().ok_or("vau: param must be a symbol"))
                        .collect::<Result<_, _>>()?;

                    // $env param (items[2]) — must start with $
                    let env_sym = items[2].as_symbol()
                        .ok_or("vau: env param must be a symbol")?;
                    let env_name = self.heap.symbol_name(env_sym);
                    if !env_name.starts_with('$') {
                        return Err("vau: env param must start with $".into());
                    }
                    param_syms.push(env_sym); // env is last param
                    let arity = param_syms.len() as u8;

                    // compile body
                    let mut sub = Compiler::new(self.heap, "<vau>");
                    sub.parent_locals = self.locals.clone();
                    sub.chunk.arity = arity;
                    for &sym in &param_syms {
                        let reg = sub.alloc_reg();
                        sub.locals.push((sym, reg));
                    }
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
                    let capture_names: Vec<u32> = sub.captures.iter().map(|(s, _, _)| *s).collect();
                    let capture_parent_regs: Vec<u8> = sub.captures.iter().map(|(_, r, _)| *r).collect();
                    let capture_local_regs: Vec<u8> = sub.captures.iter().map(|(_, _, lr)| *lr).collect();
                    let sub_result = sub.finish();

                    let desc = ClosureDesc {
                        chunk: sub_result.chunk,
                        param_names: param_syms,
                        is_operative: true,
                        capture_names,
                        capture_parent_regs,
                        capture_local_regs,
                        capture_values: Vec::new(), desc_base: 0,
                    };
                    self.closure_descs.extend(sub_result.closure_descs);
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);
                    return Ok(());
                }

                "%block" => {
                    // (%block (params) body) — compiles exactly like fn
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 { return Err("block: need params and body".into()); }
                    let params = self.heap.list_to_vec(items[1]);
                    let param_syms: Vec<u32> = params.iter()
                        .map(|p| p.as_symbol().ok_or("block: param must be a symbol"))
                        .collect::<Result<_, _>>()?;
                    let arity = param_syms.len() as u8;
                    let mut sub = Compiler::new(self.heap, "<block>");
                    sub.parent_locals = self.locals.clone();
                    sub.chunk.arity = arity;
                    for &sym in &param_syms {
                        let reg = sub.alloc_reg();
                        sub.locals.push((sym, reg));
                    }
                    let body_dst = sub.alloc_reg();
                    sub.compile_expr(items[2], body_dst)?;
                    sub.chunk.emit(Op::Return, body_dst, 0, 0);
                    let capture_names: Vec<u32> = sub.captures.iter().map(|(s, _, _)| *s).collect();
                    let capture_parent_regs: Vec<u8> = sub.captures.iter().map(|(_, r, _)| *r).collect();
                    let capture_local_regs: Vec<u8> = sub.captures.iter().map(|(_, _, lr)| *lr).collect();
                    let sub_result = sub.finish();
                    let desc = ClosureDesc {
                        chunk: sub_result.chunk,
                        param_names: param_syms,
                        is_operative: false,
                        capture_names,
                        capture_parent_regs,
                        capture_local_regs,
                        capture_values: Vec::new(), desc_base: 0,
                    };
                    self.closure_descs.extend(sub_result.closure_descs);
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);
                    return Ok(());
                }

                "fn" | "lambda" => {
                    // (fn (params...) body...) → compile body as a sub-chunk, emit MakeClosure
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("fn: need params and body".into());
                    }
                    // extract param names
                    let params = self.heap.list_to_vec(items[1]);
                    let param_syms: Vec<u32> = params.iter()
                        .map(|p| p.as_symbol().ok_or("fn: param must be a symbol"))
                        .collect::<Result<_, _>>()?;
                    let arity = param_syms.len() as u8;

                    // compile body as a sub-chunk
                    let mut sub = Compiler::new(self.heap, "<fn>");
                    sub.parent_locals = self.locals.clone();
                    sub.chunk.arity = arity;
                    // allocate registers for params and register them as locals
                    for &sym in &param_syms {
                        let reg = sub.alloc_reg();
                        sub.locals.push((sym, reg));
                    }
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
                    let capture_names: Vec<u32> = sub.captures.iter().map(|(s, _, _)| *s).collect();
                    let capture_parent_regs: Vec<u8> = sub.captures.iter().map(|(_, r, _)| *r).collect();
                    let capture_local_regs: Vec<u8> = sub.captures.iter().map(|(_, _, lr)| *lr).collect();
                    let sub_result = sub.finish();

                    let desc = ClosureDesc {
                        chunk: sub_result.chunk,
                        param_names: param_syms,
                        is_operative: false,
                        capture_names,
                        capture_parent_regs,
                        capture_local_regs,
                        capture_values: Vec::new(), desc_base: 0,
                    };
                    // pull up any nested closure descs
                    self.closure_descs.extend(sub_result.closure_descs);
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    // emit MakeClosure with index
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);

                    return Ok(());
                }

                "let" => {
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

                "while" => {
                    // (while cond body...) → loop: if !cond break; body; goto loop
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("while: need condition and body".into());
                    }

                    let loop_start = self.chunk.offset();

                    // compile condition
                    let cond_reg = self.alloc_reg();
                    self.compile_expr(items[1], cond_reg)?;

                    // jump out if false
                    let exit_jump = self.chunk.offset();
                    self.chunk.emit(Op::JumpIfFalse, cond_reg, 0, 0);

                    // compile body
                    for i in 2..items.len() {
                        let tmp = self.alloc_reg();
                        self.compile_expr(items[i], tmp)?;
                    }

                    // jump back to loop start
                    let back_offset = (loop_start as i16) - (self.chunk.offset() as i16) - 4;
                    let back_bytes = back_offset.to_be_bytes();
                    self.chunk.emit(Op::Jump, back_bytes[0], back_bytes[1], 0);

                    // patch exit jump
                    let exit_target = self.chunk.offset();
                    let exit_offset = (exit_target as i16) - (exit_jump as i16) - 4;
                    let exit_bytes = exit_offset.to_be_bytes();
                    self.chunk.code[exit_jump + 2] = exit_bytes[0];
                    self.chunk.code[exit_jump + 3] = exit_bytes[1];

                    // while returns nil
                    self.chunk.emit(Op::LoadNil, dst, 0, 0);
                    return Ok(());
                }

                "cons" => {
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 { return Err("cons: need car and cdr".into()); }
                    let car_reg = self.alloc_reg();
                    let cdr_reg = self.alloc_reg();
                    self.compile_expr(items[1], car_reg)?;
                    self.compile_expr(items[2], cdr_reg)?;
                    self.chunk.emit(Op::Cons, dst, car_reg, cdr_reg);
                    return Ok(());
                }

                "eq" => {
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 { return Err("eq: need two args".into()); }
                    let a_reg = self.alloc_reg();
                    let b_reg = self.alloc_reg();
                    self.compile_expr(items[1], a_reg)?;
                    self.compile_expr(items[2], b_reg)?;
                    self.chunk.emit(Op::Eq, dst, a_reg, b_reg);
                    return Ok(());
                }

                "eval" => {
                    // (eval expr) — compile and execute expr at runtime
                    // expr must be an AST (cons cells from quote or quasiquote)
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 2 { return Err("eval: need one arg".into()); }
                    self.compile_expr(items[1], dst)?;
                    // emit EVAL opcode — the VM handles compilation + execution
                    self.chunk.emit(Op::Eval, dst, dst, 0);
                    return Ok(());
                }

                ":=" => {
                    // (:= name value) — mutate a local or global binding
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 {
                        return Err("<-: need name and value".into());
                    }
                    let name_sym = items[1].as_symbol()
                        .ok_or("<-: target must be a symbol")?;
                    self.compile_expr(items[2], dst)?;
                    if let Some(reg) = self.find_local(name_sym) {
                        // mutate local
                        self.chunk.emit(Op::Move, reg, dst, 0);
                    } else {
                        // mutate global
                        let idx = self.add_sym_const(name_sym);
                        self.chunk.emit(Op::DefGlobal, (idx >> 8) as u8, idx as u8, dst);
                    }
                    return Ok(());
                }

                "if" => {
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
                        // no else branch — patch jump-to-else, result is nil
                        let end = self.chunk.offset();
                        self.chunk.emit(Op::LoadNil, dst, 0, 0);
                        let past = self.chunk.offset();
                        let delta = (past - 4 as usize) as i16 - jump_to_else as i16 - 4;
                        let bytes = delta.to_be_bytes();
                        self.chunk.code[jump_to_else + 2] = bytes[0];
                        self.chunk.code[jump_to_else + 3] = bytes[1];
                    }

                    return Ok(());
                }

                "%table-literal" => {
                    // (%table-literal (seq...) (k1 v1 k2 v2...))
                    let items = self.heap.list_to_vec(expr);
                    if items.len() != 3 {
                        return Err("table literal: need seq and kv lists".into());
                    }

                    let seq_items = self.heap.list_to_vec(items[1]);
                    let kv_items = self.heap.list_to_vec(items[2]);
                    let nseq = seq_items.len();
                    let nmap = kv_items.len() / 2;

                    // compile all seq values into registers
                    let mut seq_regs = Vec::with_capacity(nseq);
                    for item in &seq_items {
                        let r = self.alloc_reg();
                        self.compile_expr(*item, r)?;
                        seq_regs.push(r);
                    }

                    // compile all kv pairs into registers
                    let mut kv_regs = Vec::with_capacity(kv_items.len());
                    for item in &kv_items {
                        let r = self.alloc_reg();
                        self.compile_expr(*item, r)?;
                        kv_regs.push(r);
                    }

                    // emit MakeTable dst, nseq, nmap
                    self.chunk.emit(Op::MakeTable, dst, nseq as u8, nmap as u8);

                    // trailing data: seq regs, then kv regs (padded to 4-byte alignment)
                    let total_regs = seq_regs.len() + kv_regs.len();
                    let mut trailing = Vec::with_capacity(total_regs);
                    trailing.extend_from_slice(&seq_regs);
                    trailing.extend_from_slice(&kv_regs);
                    // pad to multiple of 4
                    while trailing.len() % 4 != 0 {
                        trailing.push(0);
                    }
                    self.chunk.code.extend_from_slice(&trailing);

                    return Ok(());
                }

                "%object-literal" => {
                    // (%object-literal parent (name1 name2...) val1 val2...)
                    let items = self.heap.list_to_vec(expr);
                    if items.len() < 3 {
                        return Err("object literal: need parent and slot names".into());
                    }

                    // compile parent
                    let parent_reg = self.alloc_reg();
                    self.compile_expr(items[1], parent_reg)?;

                    // get slot names from the list
                    let name_list = self.heap.list_to_vec(items[2]);
                    let nslots = name_list.len();

                    // compile slot values
                    let mut val_regs = Vec::with_capacity(nslots);
                    for i in 0..nslots {
                        let r = self.alloc_reg();
                        self.compile_expr(items[3 + i], r)?;
                        val_regs.push(r);
                    }

                    // emit MAKE_OBJ dst, parent, nslots
                    self.chunk.emit(Op::MakeObj, dst, parent_reg, nslots as u8);

                    // emit slot name/value pairs as trailing data
                    for i in 0..nslots {
                        let name_sym = self.extract_quoted(name_list[i])?
                            .as_symbol().ok_or("object literal: slot name must be a symbol")?;
                        let name_const = self.add_sym_const(name_sym);
                        self.chunk.code.push((name_const >> 8) as u8);
                        self.chunk.code.push(name_const as u8);
                        self.chunk.code.push(val_regs[i]);
                        self.chunk.code.push(0); // padding to 4 bytes
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

                _ => {
                    // check if this is a known operative
                    if self.heap.operatives.contains(&sym_id) {
                        return self.compile_operative_call(expr, sym_id, dst);
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
            crate::object::HeapObject::General { .. } => {
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

        // env placeholder (nil for now — real first-class envs later)
        let env_reg = self.alloc_reg();
        self.chunk.emit(Op::LoadNil, env_reg, 0, 0);

        // call the operative with (args-list, env)
        // operative's params are (user-params... $env)
        // we pass: the args list as first param, env as second
        // the operative body destructures from the args list
        // ... actually, the operative expects individual params, not a list.
        // let's pass the args as individual values (quoted) + env as last

        // simpler: just pass the raw AST values + nil env
        // the operative's params match 1:1 with the args
        // rebuild as contiguous registers:
        // [func_reg, arg1_quoted, arg2_quoted, ..., env_reg]
        let base = self.next_reg;
        let func_reg2 = self.alloc_reg();
        self.chunk.emit(Op::Move, func_reg2, func_reg, 0);

        let mut n_params = 0;
        for i in 1..items.len() {
            let r = self.alloc_reg();
            self.emit_load_const(r, items[i]); // raw AST, not compiled
            n_params += 1;
        }
        // env as last param
        let env_r = self.alloc_reg();
        self.chunk.emit(Op::LoadNil, env_r, 0, 0);
        n_params += 1;

        self.chunk.emit(Op::Call, dst, func_reg2, n_params as u8);
        Ok(())
    }

    fn compile_call(&mut self, expr: Value, dst: u8) -> Result<(), String> {
        let items = self.heap.list_to_vec(expr);
        if items.is_empty() { return Err("empty call".into()); }

        // allocate a contiguous window: [func, arg0, arg1, ...]
        let window_start = self.next_reg;
        let func_reg = self.alloc_reg();
        let nargs = items.len() - 1;
        let mut arg_regs = Vec::with_capacity(nargs);
        for _ in 0..nargs {
            arg_regs.push(self.alloc_reg());
        }

        // compile func and args into the contiguous window
        self.compile_expr(items[0], func_reg)?;
        for i in 0..nargs {
            self.compile_expr(items[i + 1], arg_regs[i])?;
        }

        // emit CALL dst, func, nargs — args must be in func+1..func+nargs
        self.chunk.emit(Op::Call, dst, func_reg, nargs as u8);

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

/// Register type prototypes and native handlers on the heap.
pub fn register_type_protos(heap: &mut Heap) {
    // pre-intern symbols used by the compiler's defmethod
    heap.intern("self");
    // create the Object prototype (root of all delegation)
    let object_proto = heap.make_object(Value::NIL);
    heap.type_protos[5] = object_proto; // object type
    let obj_id = object_proto.as_any_object().unwrap();

    // Object: slotAt:
    let slot_at_handler = heap.register_native("__obj_slotAt", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("slotAt: receiver not an object")?;
        let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt: arg must be a symbol")?;
        Ok(heap.get(id).slot_get(name).unwrap_or(Value::NIL))
    });
    let slot_at_sym = heap.sym_slot_at;
    heap.get_mut(obj_id).handler_set(slot_at_sym, slot_at_handler);

    // Object: slotAt:put:
    let slot_at_put_handler = heap.register_native("__obj_slotAtPut", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("slotAt:put: receiver not an object")?;
        let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt:put: arg0 must be a symbol")?;
        let val = args.get(1).copied().unwrap_or(Value::NIL);
        heap.get_mut(id).slot_set(name, val);
        Ok(val)
    });
    let slot_at_put_sym = heap.sym_slot_at_put;
    heap.get_mut(obj_id).handler_set(slot_at_put_sym, slot_at_put_handler);

    // Object: parent
    let parent_handler = heap.register_native("__obj_parent", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("parent: not an object")?;
        Ok(heap.get(id).parent())
    });
    let parent_sym = heap.sym_parent;
    heap.get_mut(obj_id).handler_set(parent_sym, parent_handler);

    // Object: slotNames
    let slot_names_handler = heap.register_native("__obj_slotNames", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("slotNames: not an object")?;
        let names = heap.get(id).slot_names();
        let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
        Ok(heap.list(&syms))
    });
    let slot_names_sym = heap.sym_slot_names;
    heap.get_mut(obj_id).handler_set(slot_names_sym, slot_names_handler);

    // Object: handlerNames
    let handler_names_handler = heap.register_native("__obj_handlerNames", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("handlerNames: not an object")?;
        let names = heap.get(id).handler_names();
        let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
        Ok(heap.list(&syms))
    });
    let handler_names_sym = heap.sym_handler_names;
    heap.get_mut(obj_id).handler_set(handler_names_sym, handler_names_handler);

    // Object: handle:with:
    let handle_with_handler = heap.register_native("__obj_handleWith", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("handle:with: not an object")?;
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("handle:with: selector must be a symbol")?;
        let handler = args.get(1).copied().ok_or("handle:with: need handler value")?;
        heap.get_mut(id).handler_set(sel, handler);
        Ok(receiver)
    });
    let handle_with_sym = heap.intern("handle:with:");
    heap.get_mut(obj_id).handler_set(handle_with_sym, handle_with_handler);

    // Object: describe
    let describe_handler = heap.register_native("__obj_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    let describe_sym = heap.sym_describe;
    heap.get_mut(obj_id).handler_set(describe_sym, describe_handler);

    // Integer prototype
    let int_proto = heap.make_object(object_proto);
    heap.type_protos[2] = int_proto;

    // register native handlers for integer arithmetic
    let add_handler = heap.register_native("__int_add", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("+ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("+ : arg not an integer")?;
        Ok(Value::integer(a + b))
    });
    let int_id = int_proto.as_any_object().unwrap();
    let plus_sym = heap.intern("+");
    heap.get_mut(int_id).handler_set(plus_sym, add_handler);

    let sub_handler = heap.register_native("__int_sub", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("- : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("- : arg not an integer")?;
        Ok(Value::integer(a - b))
    });
    let minus_sym = heap.intern("-");
    heap.get_mut(int_id).handler_set(minus_sym, sub_handler);

    let mul_handler = heap.register_native("__int_mul", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("* : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("* : arg not an integer")?;
        Ok(Value::integer(a * b))
    });
    let mul_sym = heap.intern("*");
    heap.get_mut(int_id).handler_set(mul_sym, mul_handler);

    let div_handler = heap.register_native("__int_div", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("/ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("/ : arg not an integer")?;
        if b == 0 { return Err("division by zero".into()); }
        Ok(Value::integer(a / b))
    });
    let div_sym = heap.intern("/");
    heap.get_mut(int_id).handler_set(div_sym, div_handler);

    let lt_handler = heap.register_native("__int_lt", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("< : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("< : arg not an integer")?;
        Ok(Value::boolean(a < b))
    });
    let lt_sym = heap.intern("<");
    heap.get_mut(int_id).handler_set(lt_sym, lt_handler);

    let gt_handler = heap.register_native("__int_gt", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("> : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("> : arg not an integer")?;
        Ok(Value::boolean(a > b))
    });
    let gt_sym = heap.intern(">");
    heap.get_mut(int_id).handler_set(gt_sym, gt_handler);

    let eq_handler = heap.register_native("__int_eq", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("= : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("= : arg not an integer")?;
        Ok(Value::boolean(a == b))
    });
    let eq_sym = heap.intern("=");
    heap.get_mut(int_id).handler_set(eq_sym, eq_handler);

    let h = heap.register_native("__int_gte", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or(">= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or(">= : arg not int")?;
        Ok(Value::boolean(a >= b))
    });
    let gte_sym = heap.intern(">=");
    heap.get_mut(int_id).handler_set(gte_sym, h);

    let h = heap.register_native("__int_lte", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("<= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("<= : arg not int")?;
        Ok(Value::boolean(a <= b))
    });
    let lte_sym = heap.intern("<=");
    heap.get_mut(int_id).handler_set(lte_sym, h);

    let h = heap.register_native("__int_mod", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("% : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("% : arg not int")?;
        if b == 0 { return Err("modulo by zero".into()); }
        Ok(Value::integer(a % b))
    });
    let mod_sym = heap.intern("%");
    heap.get_mut(int_id).handler_set(mod_sym, h);

    let h = heap.register_native("__int_negate", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("negate: not int")?;
        Ok(Value::integer(-a))
    });
    let neg_sym = heap.intern("negate");
    heap.get_mut(int_id).handler_set(neg_sym, h);

    let describe_handler = heap.register_native("__int_describe", |_heap, receiver, _args| {
        Ok(receiver)
    });
    let describe_sym = heap.intern("describe");
    heap.get_mut(int_id).handler_set(describe_sym, describe_handler);

    // -- Nil prototype (type_protos[0]) --
    let nil_proto = heap.make_object(object_proto);
    heap.type_protos[0] = nil_proto;
    let nil_id = nil_proto.as_any_object().unwrap();

    let h = heap.register_native("__nil_describe", |heap, _receiver, _args| {
        Ok(heap.alloc_string("nil"))
    });
    heap.get_mut(nil_id).handler_set(describe_sym, h);

    // -- Boolean prototype (type_protos[1]) --
    let bool_proto = heap.make_object(object_proto);
    heap.type_protos[1] = bool_proto;
    let bool_id = bool_proto.as_any_object().unwrap();

    let h = heap.register_native("__bool_not", |_heap, receiver, _args| {
        Ok(Value::boolean(!receiver.is_truthy()))
    });
    let not_sym = heap.intern("not");
    heap.get_mut(bool_id).handler_set(not_sym, h);

    let h = heap.register_native("__bool_describe", |heap, receiver, _args| {
        let s = if receiver.is_true() { "true" } else { "false" };
        Ok(heap.alloc_string(s))
    });
    heap.get_mut(bool_id).handler_set(describe_sym, h);

    let h = heap.register_native("__bool_if_true_false", |_heap, receiver, args| {
        let true_val = args.first().copied().unwrap_or(Value::NIL);
        let false_val = args.get(1).copied().unwrap_or(Value::NIL);
        Ok(if receiver.is_truthy() { true_val } else { false_val })
    });
    let if_sym = heap.intern("ifTrue:ifFalse:");
    heap.get_mut(bool_id).handler_set(if_sym, h);

    // -- Float prototype (type_protos[3]) --
    let float_proto = heap.make_object(object_proto);
    heap.type_protos[3] = float_proto;
    let float_id = float_proto.as_any_object().unwrap();

    let h = heap.register_native("__float_add", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("+ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("+ : arg not numeric")?;
        Ok(Value::float(a + b))
    });
    heap.get_mut(float_id).handler_set(plus_sym, h);

    let h = heap.register_native("__float_sub", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("- : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("- : arg not numeric")?;
        Ok(Value::float(a - b))
    });
    heap.get_mut(float_id).handler_set(minus_sym, h);

    let h = heap.register_native("__float_mul", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("* : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("* : arg not numeric")?;
        Ok(Value::float(a * b))
    });
    heap.get_mut(float_id).handler_set(mul_sym, h);

    let h = heap.register_native("__float_div", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("/ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("/ : arg not numeric")?;
        if b == 0.0 { return Err("division by zero".into()); }
        Ok(Value::float(a / b))
    });
    heap.get_mut(float_id).handler_set(div_sym, h);

    let h = heap.register_native("__float_lt", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("< : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("< : arg not numeric")?;
        Ok(Value::boolean(a < b))
    });
    heap.get_mut(float_id).handler_set(lt_sym, h);

    let h = heap.register_native("__float_gt", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("> : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("> : arg not numeric")?;
        Ok(Value::boolean(a > b))
    });
    heap.get_mut(float_id).handler_set(gt_sym, h);

    let h = heap.register_native("__float_eq", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("= : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("= : arg not numeric")?;
        Ok(Value::boolean(a == b))
    });
    heap.get_mut(float_id).handler_set(eq_sym, h);

    let h = heap.register_native("__float_gte", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or(">= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or(">= : arg not numeric")?;
        Ok(Value::boolean(a >= b))
    });
    heap.get_mut(float_id).handler_set(gte_sym, h);

    let h = heap.register_native("__float_lte", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("<= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("<= : arg not numeric")?;
        Ok(Value::boolean(a <= b))
    });
    heap.get_mut(float_id).handler_set(lte_sym, h);

    let h = heap.register_native("__float_sqrt", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("sqrt: not numeric")?;
        Ok(Value::float(a.sqrt()))
    });
    let sqrt_sym = heap.intern("sqrt");
    heap.get_mut(float_id).handler_set(sqrt_sym, h);

    let h = heap.register_native("__float_floor", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("floor: not numeric")?;
        Ok(Value::float(a.floor()))
    });
    let floor_sym = heap.intern("floor");
    heap.get_mut(float_id).handler_set(floor_sym, h);

    let h = heap.register_native("__float_ceil", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("ceil: not numeric")?;
        Ok(Value::float(a.ceil()))
    });
    let ceil_sym = heap.intern("ceil");
    heap.get_mut(float_id).handler_set(ceil_sym, h);

    let h = heap.register_native("__float_round", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("round: not numeric")?;
        Ok(Value::float(a.round()))
    });
    let round_sym = heap.intern("round");
    heap.get_mut(float_id).handler_set(round_sym, h);

    let h = heap.register_native("__float_to_integer", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("toInteger: not numeric")?;
        Ok(Value::integer(a as i64))
    });
    let to_int_sym = heap.intern("toInteger");
    heap.get_mut(float_id).handler_set(to_int_sym, h);

    let h = heap.register_native("__float_describe", |heap, receiver, _args| {
        let a = receiver.as_float().ok_or("describe: not numeric")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    });
    heap.get_mut(float_id).handler_set(describe_sym, h);

    // -- Cons prototype (type_protos[6]) --
    let cons_proto = heap.make_object(object_proto);
    heap.type_protos[6] = cons_proto;
    let cons_id = cons_proto.as_any_object().unwrap();

    let h = heap.register_native("__cons_car", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("car: not a cons")?;
        Ok(heap.car(id))
    });
    let car_sym = heap.intern("car");
    heap.get_mut(cons_id).handler_set(car_sym, h);

    let h = heap.register_native("__cons_cdr", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("cdr: not a cons")?;
        Ok(heap.cdr(id))
    });
    let cdr_sym = heap.intern("cdr");
    heap.get_mut(cons_id).handler_set(cdr_sym, h);

    let h = heap.register_native("__cons_length", |heap, receiver, _args| {
        let mut count = 0i64;
        let mut cur = receiver;
        while let Some(id) = cur.as_any_object() {
            match heap.get(id) {
                HeapObject::Pair(_, cdr) => { count += 1; cur = *cdr; }
                _ => break,
            }
        }
        Ok(Value::integer(count))
    });
    let length_sym = heap.intern("length");
    heap.get_mut(cons_id).handler_set(length_sym, h);

    let h = heap.register_native("__cons_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(cons_id).handler_set(describe_sym, h);

    // -- String prototype (type_protos[7]) --
    let str_proto = heap.make_object(object_proto);
    heap.type_protos[7] = str_proto;
    let str_id = str_proto.as_any_object().unwrap();

    let h = heap.register_native("__str_length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a string")?;
        let s = heap.get_string(id).ok_or("length: not a Text object")?;
        Ok(Value::integer(s.len() as i64))
    });
    heap.get_mut(str_id).handler_set(length_sym, h);

    let h = heap.register_native("__str_at", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at: not a string")?;
        let s = heap.get_string(id).ok_or("at: not a Text object")?;
        let idx = args.first().and_then(|v| v.as_integer()).ok_or("at: arg not an integer")? as usize;
        let ch = s.chars().nth(idx).map(|c| c.to_string()).unwrap_or_default();
        Ok(heap.alloc_string(&ch))
    });
    let at_sym = heap.intern("at:");
    heap.get_mut(str_id).handler_set(at_sym, h);

    let h = heap.register_native("__str_concat", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("++: not a string")?;
        let a = heap.get_string(id).ok_or("++: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let b = if let Some(bid) = arg.as_any_object() {
            heap.get_string(bid).map(|s| s.to_string()).unwrap_or_else(|| heap.format_value(arg))
        } else {
            heap.format_value(arg)
        };
        Ok(heap.alloc_string(&format!("{}{}", a, b)))
    });
    let concat_sym = heap.intern("++");
    heap.get_mut(str_id).handler_set(concat_sym, h);

    let h = heap.register_native("__str_substring_to", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("substring:to: not a string")?;
        let s = heap.get_string(id).ok_or("substring:to: not a Text object")?;
        let from = args.first().and_then(|v| v.as_integer()).ok_or("substring:to: arg0 not int")? as usize;
        let to = args.get(1).and_then(|v| v.as_integer()).ok_or("substring:to: arg1 not int")? as usize;
        let chars: Vec<char> = s.chars().collect();
        let end = to.min(chars.len());
        let start = from.min(end);
        let sub: String = chars[start..end].iter().collect();
        Ok(heap.alloc_string(&sub))
    });
    let substr_sym = heap.intern("substring:to:");
    heap.get_mut(str_id).handler_set(substr_sym, h);

    let h = heap.register_native("__str_split", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("split: not a string")?;
        let s = heap.get_string(id).ok_or("split: not a Text object")?.to_string();
        let delim_arg = args.first().copied().unwrap_or(Value::NIL);
        let did = delim_arg.as_any_object().ok_or("split: arg not a string")?;
        let delim = heap.get_string(did).ok_or("split: arg not a Text object")?.to_string();
        let parts: Vec<Value> = s.split(&delim).map(|p| heap.alloc_string(p)).collect();
        Ok(heap.list(&parts))
    });
    let split_sym = heap.intern("split:");
    heap.get_mut(str_id).handler_set(split_sym, h);

    let h = heap.register_native("__str_trim", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("trim: not a string")?;
        let s = heap.get_string(id).ok_or("trim: not a Text object")?.trim().to_string();
        Ok(heap.alloc_string(&s))
    });
    let trim_sym = heap.intern("trim");
    heap.get_mut(str_id).handler_set(trim_sym, h);

    let h = heap.register_native("__str_contains", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("contains: not a string")?;
        let s = heap.get_string(id).ok_or("contains: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let nid = arg.as_any_object().ok_or("contains: arg not a string")?;
        let needle = heap.get_string(nid).ok_or("contains: arg not a Text object")?;
        Ok(Value::boolean(s.contains(needle)))
    });
    let contains_sym = heap.intern("contains:");
    heap.get_mut(str_id).handler_set(contains_sym, h);

    let h = heap.register_native("__str_starts_with", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("startsWith: not a string")?;
        let s = heap.get_string(id).ok_or("startsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let pid = arg.as_any_object().ok_or("startsWith: arg not a string")?;
        let prefix = heap.get_string(pid).ok_or("startsWith: arg not a Text object")?;
        Ok(Value::boolean(s.starts_with(prefix)))
    });
    let starts_sym = heap.intern("startsWith:");
    heap.get_mut(str_id).handler_set(starts_sym, h);

    let h = heap.register_native("__str_ends_with", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("endsWith: not a string")?;
        let s = heap.get_string(id).ok_or("endsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let sid = arg.as_any_object().ok_or("endsWith: arg not a string")?;
        let suffix = heap.get_string(sid).ok_or("endsWith: arg not a Text object")?;
        Ok(Value::boolean(s.ends_with(suffix)))
    });
    let ends_sym = heap.intern("endsWith:");
    heap.get_mut(str_id).handler_set(ends_sym, h);

    let h = heap.register_native("__str_to_upper", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toUpper: not a string")?;
        let s = heap.get_string(id).ok_or("toUpper: not a Text object")?;
        Ok(heap.alloc_string(&s.to_uppercase()))
    });
    let upper_sym = heap.intern("toUpper");
    heap.get_mut(str_id).handler_set(upper_sym, h);

    let h = heap.register_native("__str_to_lower", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toLower: not a string")?;
        let s = heap.get_string(id).ok_or("toLower: not a Text object")?;
        Ok(heap.alloc_string(&s.to_lowercase()))
    });
    let lower_sym = heap.intern("toLower");
    heap.get_mut(str_id).handler_set(lower_sym, h);

    let h = heap.register_native("__str_to_integer", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toInteger: not a string")?;
        let s = heap.get_string(id).ok_or("toInteger: not a Text object")?;
        let n: i64 = s.trim().parse().map_err(|_| format!("toInteger: cannot parse '{}'", s))?;
        Ok(Value::integer(n))
    });
    heap.get_mut(str_id).handler_set(to_int_sym, h);

    let h = heap.register_native("__str_describe", |_heap, receiver, _args| {
        Ok(receiver) // strings describe as themselves
    });
    heap.get_mut(str_id).handler_set(describe_sym, h);

    // -- Table prototype (type_protos[9]) --
    let table_proto = heap.make_object(object_proto);
    heap.type_protos[9] = table_proto;
    let table_id = table_proto.as_any_object().unwrap();

    // Table: at:
    let h = heap.register_native("__table_at", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        match heap.get(id) {
            HeapObject::Table { seq, map } => {
                // try integer index into seq first
                if let Some(idx) = key.as_integer() {
                    if idx >= 0 && (idx as usize) < seq.len() {
                        return Ok(seq[idx as usize]);
                    }
                }
                // then check map (content equality for strings)
                for (k, v) in map {
                    if heap.values_equal(*k, key) { return Ok(*v); }
                }
                Ok(Value::NIL)
            }
            _ => Err("at: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(at_sym, h);

    // Table: at:put:
    let at_put_sym = heap.intern("at:put:");
    let h = heap.register_native("__table_at_put", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at:put: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        let val = args.get(1).copied().unwrap_or(Value::NIL);
        // find existing key index using content equality (before mutable borrow)
        let existing_idx = match heap.get(id) {
            HeapObject::Table { map, .. } => {
                map.iter().position(|(k, _)| heap.values_equal(*k, key))
            }
            _ => return Err("at:put: not a Table".into()),
        };
        match heap.get_mut(id) {
            HeapObject::Table { seq, map } => {
                if let Some(idx) = key.as_integer() {
                    if idx >= 0 && (idx as usize) < seq.len() {
                        seq[idx as usize] = val;
                        return Ok(val);
                    }
                }
                if let Some(pos) = existing_idx {
                    map[pos].1 = val;
                } else {
                    map.push((key, val));
                }
                Ok(val)
            }
            _ => Err("at:put: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(at_put_sym, h);

    // Table: push:
    let push_sym = heap.intern("push:");
    let h = heap.register_native("__table_push", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("push: not a table")?;
        let val = args.first().copied().unwrap_or(Value::NIL);
        match heap.get_mut(id) {
            HeapObject::Table { seq, .. } => {
                seq.push(val);
                Ok(val)
            }
            _ => Err("push: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(push_sym, h);

    // Table: length
    let h = heap.register_native("__table_length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a table")?;
        match heap.get(id) {
            HeapObject::Table { seq, .. } => Ok(Value::integer(seq.len() as i64)),
            _ => Err("length: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(length_sym, h);

    // Table: keys
    let keys_sym = heap.intern("keys");
    let h = heap.register_native("__table_keys", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("keys: not a table")?;
        let keys: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(k, _)| *k).collect(),
            _ => return Err("keys: not a Table".into()),
        };
        Ok(heap.list(&keys))
    });
    heap.get_mut(table_id).handler_set(keys_sym, h);

    // Table: values
    let values_sym = heap.intern("values");
    let h = heap.register_native("__table_values", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("values: not a table")?;
        let vals: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(_, v)| *v).collect(),
            _ => return Err("values: not a Table".into()),
        };
        Ok(heap.list(&vals))
    });
    heap.get_mut(table_id).handler_set(values_sym, h);

    // Table: describe
    let h = heap.register_native("__table_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(table_id).handler_set(describe_sym, h);

    // Table: contains:
    let h = heap.register_native("__table_contains", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("contains: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        match heap.get(id) {
            HeapObject::Table { seq, map } => {
                for v in seq {
                    if heap.values_equal(*v, key) { return Ok(Value::TRUE); }
                }
                for (k, _) in map {
                    if heap.values_equal(*k, key) { return Ok(Value::TRUE); }
                }
                Ok(Value::FALSE)
            }
            _ => Err("contains: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(contains_sym, h);

    // Table: remove:
    let remove_sym = heap.intern("remove:");
    let h = heap.register_native("__table_remove", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("remove: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        let pos = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().position(|(k, _)| heap.values_equal(*k, key)),
            _ => return Err("remove: not a Table".into()),
        };
        match heap.get_mut(id) {
            HeapObject::Table { map, .. } => {
                if let Some(pos) = pos {
                    let (_, val) = map.remove(pos);
                    Ok(val)
                } else {
                    Ok(Value::NIL)
                }
            }
            _ => Err("remove: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(remove_sym, h);

    // -- register all prototypes as globals so they're accessible by name --
    let obj_sym = heap.intern("Object");
    heap.globals.insert(obj_sym, object_proto);
    let int_sym = heap.intern("Integer");
    heap.globals.insert(int_sym, int_proto);
    let nil_sym = heap.intern("Nil");
    heap.globals.insert(nil_sym, nil_proto);
    let bool_sym = heap.intern("Boolean");
    heap.globals.insert(bool_sym, bool_proto);
    let float_sym = heap.intern("Float");
    heap.globals.insert(float_sym, float_proto);
    let cons_sym = heap.intern("Cons");
    heap.globals.insert(cons_sym, cons_proto);
    let string_sym = heap.intern("String");
    heap.globals.insert(string_sym, str_proto);
    let table_sym = heap.intern("Table");
    heap.globals.insert(table_sym, table_proto);

    // -- global utility natives --

    // print: outputs a value and returns it
    let print_handler = heap.register_native("__print", |heap, _receiver, args| {
        let val = args.first().copied().unwrap_or(Value::NIL);
        println!("{}", heap.format_value(val));
        Ok(val)
    });
    let print_sym = heap.intern("print");
    heap.globals.insert(print_sym, print_handler);

    // println: like print but adds newline context
    let println_handler = heap.register_native("__println", |heap, _receiver, args| {
        let val = args.first().copied().unwrap_or(Value::NIL);
        println!("{}", heap.format_value(val));
        Ok(Value::NIL)
    });
    let println_sym = heap.intern("println");
    heap.globals.insert(println_sym, println_handler);

    // type-of: returns a symbol for the type
    let typeof_handler = heap.register_native("__typeof", |heap, _receiver, args| {
        let val = args.first().copied().unwrap_or(Value::NIL);
        let name = if val.is_nil() { "Nil" }
            else if val.is_bool() { "Boolean" }
            else if val.is_integer() {
                if val.as_integer().unwrap() < 0 { "Fn" } else { "Integer" }
            }
            else if val.is_float() { "Float" }
            else if val.is_symbol() { "Symbol" }
            else if let Some(id) = val.as_any_object() {
                match heap.get(id) {
                    HeapObject::General { .. } => "Object",
                    HeapObject::Pair(_, _) => "Cons",
                    HeapObject::Text(_) => "String",
                    HeapObject::Buffer(_) => "Bytes",
                    HeapObject::Table { .. } => "Table",
                }
            } else { "Unknown" };
        Ok(Value::symbol(heap.intern(name)))
    });
    let typeof_sym = heap.intern("type-of");
    heap.globals.insert(typeof_sym, typeof_handler);
}
