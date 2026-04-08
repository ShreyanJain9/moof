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
    locals: Vec<(u32, u8)>, // (symbol_id, register) — local variable bindings
}

impl<'a> Compiler<'a> {
    pub fn new(heap: &'a Heap, name: &str) -> Self {
        Compiler {
            heap,
            chunk: Chunk::new(name, 0, 0),
            next_reg: 0,
            closure_descs: Vec::new(),
            locals: Vec::new(),
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

    fn find_local(&self, sym_id: u32) -> Option<u8> {
        // search locals from the end (most recent binding wins)
        self.locals.iter().rev().find(|(s, _)| *s == sym_id).map(|(_, r)| *r)
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
            if name == "send" || name == "%dot" || name == "%block" || name == "%object-literal" || name == "%eventual-send" {
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
                    let sub_result = sub.finish();

                    let desc = ClosureDesc {
                        chunk: sub_result.chunk,
                        param_names: param_syms,
                    };
                    // pull up any nested closure descs
                    self.closure_descs.extend(sub_result.closure_descs);
                    let idx = self.closure_descs.len();
                    self.closure_descs.push(desc);
                    // emit MakeClosure with index
                    self.chunk.emit(Op::MakeClosure, dst, (idx >> 8) as u8, idx as u8);

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
                    // generic applicative call: (f a b c) → send call: to f
                    return self.compile_call(expr, dst);
                }
            }
        }

        // head is not a symbol — it's an expression. compile as a call.
        self.compile_call(expr, dst)
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
    // create the Object prototype (root of all delegation)
    let object_proto = heap.make_object(Value::NIL);
    heap.type_protos[5] = object_proto; // object type

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

    let describe_handler = heap.register_native("__int_describe", |_heap, receiver, _args| {
        Ok(receiver) // integers describe as themselves
    });
    let describe_sym = heap.intern("describe");
    heap.get_mut(int_id).handler_set(describe_sym, describe_handler);
}
