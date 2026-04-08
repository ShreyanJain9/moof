//! Compiler: cons-cell AST → register bytecode.
//!
//! Walks the AST (which is cons cells in the store) and emits bytecode
//! into Chunk objects. The compiler tracks register allocation with a
//! simple bump allocator per function scope.

use moof_fabric::{Store, Value};

use crate::opcodes::{Chunk, Op};

/// Compiler state for a single function scope.
pub struct Compiler<'a> {
    store: &'a Store,
    chunk: Chunk,
    next_reg: u8,
}

impl<'a> Compiler<'a> {
    pub fn new(store: &'a Store, name: &str, arity: u8) -> Self {
        Compiler {
            store,
            chunk: Chunk::new(name, arity),
            next_reg: arity, // first N registers are params
        }
    }

    /// Allocate a register.
    fn alloc_reg(&mut self) -> u8 {
        let r = self.next_reg;
        self.next_reg += 1;
        if self.next_reg > self.chunk.num_registers {
            self.chunk.num_registers = self.next_reg;
        }
        r
    }

    /// Compile an expression, placing the result in `dst`.
    pub fn compile_expr(&mut self, expr: Value, dst: u8) -> Result<(), String> {
        if expr.is_nil() {
            self.chunk.emit(Op::LoadNil, dst, 0, 0);
            return Ok(());
        }
        if expr.is_true() {
            self.chunk.emit(Op::LoadTrue, dst, 0, 0);
            return Ok(());
        }
        if expr.is_false() {
            self.chunk.emit(Op::LoadFalse, dst, 0, 0);
            return Ok(());
        }
        if expr.is_integer() || expr.is_float() {
            let idx = self.chunk.add_constant(expr.to_bits());
            let [hi, lo] = idx.to_be_bytes();
            self.chunk.emit(Op::LoadConst, dst, hi, lo);
            return Ok(());
        }
        if expr.is_symbol() {
            // symbol reference → load from environment
            // for now, emit as a constant load (the VM will resolve it)
            let idx = self.chunk.add_constant(expr.to_bits());
            let [hi, lo] = idx.to_be_bytes();
            self.chunk.emit(Op::LoadConst, dst, hi, lo);
            return Ok(());
        }

        // must be an object — either a cons cell (list/form) or a string
        let obj_id = expr
            .as_object()
            .ok_or_else(|| "expected object in compile_expr".to_string())?;

        // check if it's a string
        if let Ok(_s) = self.store.get_string_owned(obj_id) {
            let idx = self.chunk.add_constant(expr.to_bits());
            let [hi, lo] = idx.to_be_bytes();
            self.chunk.emit(Op::LoadConst, dst, hi, lo);
            return Ok(());
        }

        // must be a cons cell — a list form
        let car = self.store.car(obj_id)?;
        let _cdr = self.store.cdr(obj_id)?;

        // check for special forms by head symbol
        if let Some(sym_id) = car.as_symbol() {
            let name = self.store.symbol_name(sym_id)?;
            match name.as_str() {
                "quote" => {
                    // (quote x) → load x as constant
                    let arg = self.store.car(
                        self.store
                            .cdr(obj_id)?
                            .as_object()
                            .ok_or("quote: missing arg")?,
                    )?;
                    let idx = self.chunk.add_constant(arg.to_bits());
                    let [hi, lo] = idx.to_be_bytes();
                    self.chunk.emit(Op::LoadConst, dst, hi, lo);
                    return Ok(());
                }
                "def" => {
                    // (def name value) → compile value, then DEF
                    // TODO: proper implementation
                    self.chunk.emit(Op::LoadNil, dst, 0, 0);
                    return Ok(());
                }
                "send" => {
                    // (send receiver 'selector args...)
                    // TODO: proper implementation
                    self.chunk.emit(Op::LoadNil, dst, 0, 0);
                    return Ok(());
                }
                _ => {
                    // generic function call: (f a b c)
                    // TODO: compile f and args, emit CALL
                    self.chunk.emit(Op::LoadNil, dst, 0, 0);
                    return Ok(());
                }
            }
        }

        // fallback: generic call
        self.chunk.emit(Op::LoadNil, dst, 0, 0);
        Ok(())
    }

    /// Finish compilation, return the chunk.
    pub fn finish(mut self) -> Chunk {
        self.chunk.emit(Op::Return, 0, 0, 0);
        self.chunk
    }
}
