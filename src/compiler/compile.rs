/// The MOOF bytecode compiler.
///
/// Compiles cons-cell ASTs into BytecodeChunks. The compiler handles
/// the 6 kernel forms and the syntactic desugaring (%send, %dot, %object-literal, %do).
///
/// The bytecode is the canonical form (§9.2) — what gets serialized.

use crate::runtime::value::{Value, HeapObject, BytecodeChunk};
use crate::runtime::heap::Heap;
use crate::vm::opcodes::*;

pub struct Compiler {
    code: Vec<u8>,
    constants: Vec<Value>,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            code: Vec::new(),
            constants: Vec::new(),
        }
    }

    /// Add a constant to the pool, return its index.
    fn add_constant(&mut self, val: Value) -> u16 {
        for (i, c) in self.constants.iter().enumerate() {
            if *c == val {
                return i as u16;
            }
        }
        let idx = self.constants.len() as u16;
        self.constants.push(val);
        idx
    }

    fn emit(&mut self, byte: u8) {
        self.code.push(byte);
    }

    fn emit_u16(&mut self, val: u16) {
        self.code.push((val >> 8) as u8);
        self.code.push((val & 0xFF) as u8);
    }

    /// Compile a single expression AST into a bytecode chunk.
    pub fn compile_expr(&mut self, heap: &mut Heap, expr: Value) -> Result<BytecodeChunk, String> {
        self.compile(heap, expr)?;
        self.emit(OP_RETURN);
        Ok(BytecodeChunk {
            code: std::mem::take(&mut self.code),
            constants: std::mem::take(&mut self.constants),
        })
    }

    /// Compile a sequence of expressions (for %do or top-level), keeping only the last value.
    pub fn compile_body(&mut self, heap: &mut Heap, exprs: &[Value]) -> Result<BytecodeChunk, String> {
        for (i, &expr) in exprs.iter().enumerate() {
            self.compile(heap, expr)?;
            if i < exprs.len() - 1 {
                self.emit(OP_POP);
            }
        }
        if exprs.is_empty() {
            self.emit(OP_NIL);
        }
        self.emit(OP_RETURN);
        Ok(BytecodeChunk {
            code: std::mem::take(&mut self.code),
            constants: std::mem::take(&mut self.constants),
        })
    }

    /// Compile a value/expression to bytecode (appending to self.code).
    fn compile(&mut self, heap: &mut Heap, expr: Value) -> Result<(), String> {
        match expr {
            Value::Nil => { self.emit(OP_NIL); Ok(()) }
            Value::True => { self.emit(OP_TRUE); Ok(()) }
            Value::False => { self.emit(OP_FALSE); Ok(()) }
            Value::Integer(_) => {
                let idx = self.add_constant(expr);
                self.emit(OP_CONST);
                self.emit_u16(idx);
                Ok(())
            }
            Value::Symbol(sym) => {
                let idx = self.add_constant(Value::Symbol(sym));
                self.emit(OP_LOOKUP);
                self.emit_u16(idx);
                Ok(())
            }
            Value::Object(id) => {
                match heap.get(id).clone() {
                    HeapObject::Cons { .. } => {
                        self.compile_list(heap, expr)
                    }
                    HeapObject::MoofString(_) => {
                        let idx = self.add_constant(expr);
                        self.emit(OP_CONST);
                        self.emit_u16(idx);
                        Ok(())
                    }
                    _ => {
                        let idx = self.add_constant(expr);
                        self.emit(OP_CONST);
                        self.emit_u16(idx);
                        Ok(())
                    }
                }
            }
        }
    }

    /// Compile a list form (could be a special form or a function call).
    fn compile_list(&mut self, heap: &mut Heap, list: Value) -> Result<(), String> {
        let elements = heap.list_to_vec(list);
        if elements.is_empty() {
            self.emit(OP_NIL);
            return Ok(());
        }

        let head = elements[0];

        if let Value::Symbol(sym) = head {
            let name = heap.symbol_name(sym).to_string();
            match name.as_str() {
                // Kernel primitives
                "vau" => return self.compile_vau(heap, &elements[1..]),
                "def" => return self.compile_def(heap, &elements[1..]),
                "quote" => return self.compile_quote(heap, &elements[1..]),
                "cons" => return self.compile_cons(heap, &elements[1..]),
                "eq" => return self.compile_eq(heap, &elements[1..]),
                // Syntax desugaring
                "%send" => return self.compile_send(heap, &elements[1..]),
                "%block" => return self.compile_block(heap, &elements[1..]),
                "%do" => return self.compile_do(heap, &elements[1..]),
                "%dot" => return self.compile_dot(heap, &elements[1..]),
                "%object-literal" => return self.compile_object_literal(heap, &elements[1..]),
                // Derived forms
                "if" => return self.compile_if(heap, &elements[1..]),
                "lambda" => return self.compile_lambda(heap, &elements[1..]),
                "let" => return self.compile_let(heap, &elements[1..]),
                // Built-in operations
                "eval" => return self.compile_eval(heap, &elements[1..]),
                "print" => return self.compile_print(heap, &elements[1..]),
                "car" => return self.compile_car(heap, &elements[1..]),
                "cdr" => return self.compile_cdr(heap, &elements[1..]),
                "type-of" => return self.compile_type_of(heap, &elements[1..]),
                "list" => return self.compile_list_form(heap, &elements[1..]),
                "set!" => return self.compile_set(heap, &elements[1..]),
                "while" => return self.compile_while(heap, &elements[1..]),
                "load" => return self.compile_load(heap, &elements[1..]),
                "source" => return self.compile_source(heap, &elements[1..]),
                "object" => return self.compile_object(heap, &elements[1..]),
                "handle!" => return self.compile_handle(heap, &elements[1..]),
                _ => {}
            }
        }

        // Generic call: (f a b c)
        self.compile(heap, head)?;
        let args_list = heap.list(&elements[1..]);
        let args_idx = self.add_constant(args_list);
        self.emit(OP_QUOTE);
        self.emit_u16(args_idx);
        self.emit(OP_APPLY);
        Ok(())
    }

    /// Compile (%dot obj 'field) — direct slot access
    fn compile_dot(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("%dot requires object and field".into());
        }
        self.compile(heap, args[0])?; // object
        self.compile(heap, args[1])?; // quoted field symbol
        self.emit(OP_SLOT_GET);
        Ok(())
    }

    /// Compile (%object-literal parent (%slot key val) ... (%method sel params body...) ...)
    fn compile_object_literal(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.is_empty() {
            return Err("%object-literal requires at least a parent".into());
        }

        let parent = args[0];
        let entries = &args[1..];

        // Separate slots and methods
        let slot_tag = heap.intern("%slot");
        let method_tag = heap.intern("%method");

        let mut slots: Vec<(Value, Value)> = Vec::new(); // (key_sym, value_expr)
        let mut methods: Vec<(Value, Value, Vec<Value>)> = Vec::new(); // (sel_sym, params, body_exprs)

        for &entry in entries {
            let parts = heap.list_to_vec(entry);
            if parts.is_empty() { continue; }
            if let Value::Symbol(tag) = parts[0] {
                if tag == slot_tag && parts.len() == 3 {
                    // (%slot key value)
                    slots.push((parts[1], parts[2]));
                } else if tag == method_tag && parts.len() >= 3 {
                    // (%method selector params body...)
                    let sel = parts[1];
                    let params = parts[2];
                    let body = parts[3..].to_vec();
                    methods.push((sel, params, body));
                }
            }
        }

        // Compile parent
        self.compile(heap, parent)?;

        // Compile slots as key-value pairs for OP_MAKE_OBJECT
        for (key, val) in &slots {
            // key is already a symbol value, quote it
            let key_idx = self.add_constant(*key);
            self.emit(OP_QUOTE);
            self.emit_u16(key_idx);
            self.compile(heap, *val)?;
        }
        self.emit(OP_MAKE_OBJECT);
        self.emit(slots.len() as u8);

        // For each method, compile a lambda (with self prepended) and OP_HANDLE
        for (sel, params, body_exprs) in &methods {
            // Push quoted selector symbol
            let sel_idx = self.add_constant(*sel);
            self.emit(OP_QUOTE);
            self.emit_u16(sel_idx);

            // Build lambda with self prepended to params
            let self_sym = Value::Symbol(heap.intern("self"));
            let param_vec = heap.list_to_vec(*params);
            let mut full_params = vec![self_sym];
            full_params.extend(param_vec);
            let param_list = heap.list(&full_params);

            // Compile as lambda
            let mut lambda_args = vec![param_list];
            lambda_args.extend(body_exprs.iter().copied());
            self.compile_lambda(heap, &lambda_args)?;

            self.emit(OP_HANDLE);
        }

        Ok(())
    }

    /// Compile (vau (params) $env body...)
    fn compile_vau(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        self.compile_vau_with_source(heap, args, Value::Nil)
    }

    fn compile_vau_with_source(&mut self, heap: &mut Heap, args: &[Value], source_override: Value) -> Result<(), String> {
        if args.len() < 3 {
            return Err("vau requires (params) $env body".into());
        }
        let params = args[0];
        let env_param = args[1];
        let body_exprs = &args[2..];

        let env_param_sym = match env_param {
            Value::Symbol(s) => s,
            _ => return Err("vau: expected symbol for env parameter".into()),
        };

        let source = if source_override != Value::Nil {
            source_override
        } else {
            let vau_sym = Value::Symbol(heap.intern("vau"));
            let mut src_elems = vec![vau_sym];
            src_elems.extend_from_slice(args);
            heap.list(&src_elems)
        };

        let mut body_compiler = Compiler::new();
        let body_chunk = body_compiler.compile_body(heap, body_exprs)?;
        let body_id = heap.alloc_chunk(body_chunk);

        let params_idx = self.add_constant(params);
        let env_param_idx = self.add_constant(Value::Symbol(env_param_sym));
        let body_idx = self.add_constant(Value::Object(body_id));
        let source_idx = self.add_constant(source);

        self.emit(OP_VAU);
        self.emit_u16(params_idx);
        self.emit_u16(env_param_idx);
        self.emit_u16(body_idx);
        self.emit_u16(source_idx);
        Ok(())
    }

    /// Compile (def name value)
    fn compile_def(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("def requires name and value".into());
        }
        self.compile(heap, args[1])?;
        let name_idx = self.add_constant(args[0]);
        self.emit(OP_DEF);
        self.emit_u16(name_idx);
        Ok(())
    }

    /// Compile (quote x)
    fn compile_quote(&mut self, _heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("quote requires exactly one argument".into());
        }
        let idx = self.add_constant(args[0]);
        self.emit(OP_QUOTE);
        self.emit_u16(idx);
        Ok(())
    }

    /// Compile (cons a b)
    fn compile_cons(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("cons requires exactly two arguments".into());
        }
        self.compile(heap, args[0])?;
        self.compile(heap, args[1])?;
        self.emit(OP_CONS);
        Ok(())
    }

    /// Compile (eq a b)
    fn compile_eq(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("eq requires exactly two arguments".into());
        }
        self.compile(heap, args[0])?;
        self.compile(heap, args[1])?;
        self.emit(OP_EQ);
        Ok(())
    }

    /// Compile (%send receiver selector arg1 arg2 ...)
    fn compile_send(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("%send requires receiver and selector".into());
        }
        let receiver = args[0];
        let selector = args[1];
        let msg_args = &args[2..];

        self.compile(heap, receiver)?;
        for &arg in msg_args {
            self.compile(heap, arg)?;
        }
        let sel_idx = self.add_constant(selector);
        self.emit(OP_SEND);
        self.emit_u16(sel_idx);
        self.emit(msg_args.len() as u8);
        Ok(())
    }

    /// Compile (%block (params) body)
    fn compile_block(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("%block requires params and body".into());
        }
        let params = args[0];
        let body_expr = args[1];

        let block_sym = Value::Symbol(heap.intern("block"));
        let source = heap.list(&[block_sym, params, body_expr]);

        let mut body_compiler = Compiler::new();
        let body_chunk = body_compiler.compile_expr(heap, body_expr)?;
        let body_id = heap.alloc_chunk(body_chunk);

        let params_idx = self.add_constant(params);
        let env_param_sym = heap.intern("$_block_env");
        let env_param_idx = self.add_constant(Value::Symbol(env_param_sym));
        let body_idx = self.add_constant(Value::Object(body_id));
        let source_idx = self.add_constant(source);

        self.emit(OP_VAU);
        self.emit_u16(params_idx);
        self.emit_u16(env_param_idx);
        self.emit_u16(body_idx);
        self.emit_u16(source_idx);
        Ok(())
    }

    /// Compile (%do expr1 expr2 ... exprN) — sequence, returns last value.
    fn compile_do(&mut self, heap: &mut Heap, exprs: &[Value]) -> Result<(), String> {
        for (i, &expr) in exprs.iter().enumerate() {
            self.compile(heap, expr)?;
            if i < exprs.len() - 1 {
                self.emit(OP_POP);
            }
        }
        if exprs.is_empty() {
            self.emit(OP_NIL);
        }
        Ok(())
    }

    /// Compile (if cond then-expr else-expr)
    fn compile_if(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("if requires condition and then-branch".into());
        }
        let condition = args[0];
        let then_branch = args[1];
        let else_branch = if args.len() > 2 { args[2] } else { Value::Nil };

        self.compile(heap, condition)?;

        self.emit(OP_JUMP_IF_FALSE);
        let jump_to_else = self.code.len();
        self.emit_u16(0);

        self.compile(heap, then_branch)?;

        self.emit(OP_JUMP);
        let jump_to_end = self.code.len();
        self.emit_u16(0);

        let else_offset = self.code.len() - (jump_to_else + 2);
        self.code[jump_to_else] = (else_offset >> 8) as u8;
        self.code[jump_to_else + 1] = (else_offset & 0xFF) as u8;

        match else_branch {
            Value::Nil if args.len() <= 2 => self.emit(OP_NIL),
            _ => self.compile(heap, else_branch)?,
        }

        let end_offset = self.code.len() - (jump_to_end + 2);
        self.code[jump_to_end] = (end_offset >> 8) as u8;
        self.code[jump_to_end + 1] = (end_offset & 0xFF) as u8;

        Ok(())
    }

    /// Compile (lambda (params) body...) or (fn (params) body...)
    fn compile_lambda(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("lambda/fn requires (params) and body".into());
        }
        let params = args[0];
        let body_exprs = &args[1..];

        let lambda_sym = Value::Symbol(heap.intern("lambda"));
        let mut src_elems = vec![lambda_sym];
        src_elems.extend_from_slice(args);
        let source = heap.list(&src_elems);

        let mut body_compiler = Compiler::new();
        let body_chunk = body_compiler.compile_body(heap, body_exprs)?;
        let body_id = heap.alloc_chunk(body_chunk);

        let params_idx = self.add_constant(params);
        let body_idx = self.add_constant(Value::Object(body_id));
        let underscore = heap.intern("$_");
        let env_param_idx = self.add_constant(Value::Symbol(underscore));
        let source_idx = self.add_constant(source);

        self.emit(OP_VAU);
        self.emit_u16(params_idx);
        self.emit_u16(env_param_idx);
        self.emit_u16(body_idx);
        self.emit_u16(source_idx);
        Ok(())
    }

    /// Compile (let ((name1 val1) (name2 val2)) body...)
    fn compile_let(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("let requires bindings and body".into());
        }
        let bindings_list = args[0];
        let body_exprs = &args[1..];

        let bindings = heap.list_to_vec(bindings_list);
        let mut params = Vec::new();
        let mut values = Vec::new();
        for binding in &bindings {
            let pair = heap.list_to_vec(*binding);
            if pair.len() != 2 {
                return Err("let binding must be (name value)".into());
            }
            params.push(pair[0]);
            values.push(pair[1]);
        }

        let param_list = heap.list(&params);
        let mut lambda_args = vec![param_list];
        lambda_args.extend_from_slice(body_exprs);

        self.compile_lambda(heap, &lambda_args)?;

        let argc = values.len();
        for val in values {
            self.compile(heap, val)?;
        }
        self.emit(OP_CALL);
        self.emit(argc as u8);
        Ok(())
    }

    /// Compile (eval expr) or (eval expr env)
    fn compile_eval(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        match args.len() {
            1 => {
                self.compile(heap, args[0])?;
                self.emit(OP_EVAL);
                Ok(())
            }
            2 => {
                self.compile(heap, args[1])?;
                self.compile(heap, args[0])?;
                let sel = self.add_constant(Value::Symbol(heap.intern("eval:")));
                self.emit(OP_SEND);
                self.emit_u16(sel);
                self.emit(1u8);
                Ok(())
            }
            _ => Err("eval requires 1 or 2 arguments".into()),
        }
    }

    fn compile_print(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("print requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_PRINT);
        Ok(())
    }

    fn compile_car(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("car requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_CAR);
        Ok(())
    }

    fn compile_cdr(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("cdr requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_CDR);
        Ok(())
    }

    fn compile_type_of(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("type-of requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_TYPE_OF);
        Ok(())
    }

    /// Compile (list a b c ...) → nested cons
    fn compile_list_form(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        for &arg in args.iter() {
            self.compile(heap, arg)?;
        }
        self.emit(OP_NIL);
        for _ in 0..args.len() {
            self.emit(OP_CONS);
        }
        Ok(())
    }

    /// Compile (object) or (object parent) or (object parent 'slot1 val1 'slot2 val2)
    fn compile_object(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.is_empty() {
            self.emit(OP_NIL);
            self.emit(OP_MAKE_OBJECT);
            self.emit(0u8);
            return Ok(());
        }

        self.compile(heap, args[0])?;

        let slot_args = &args[1..];
        if slot_args.len() % 2 != 0 {
            return Err("object: slot args must be key-value pairs".into());
        }
        let slot_count = slot_args.len() / 2;
        for pair in slot_args.chunks(2) {
            self.compile(heap, pair[0])?;
            self.compile(heap, pair[1])?;
        }
        self.emit(OP_MAKE_OBJECT);
        self.emit(slot_count as u8);
        Ok(())
    }

    /// Compile (handle! obj selector handler)
    fn compile_handle(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 3 {
            return Err("handle! requires object, selector, and handler".into());
        }
        self.compile(heap, args[0])?;
        self.compile(heap, args[1])?;
        self.compile(heap, args[2])?;
        self.emit(OP_HANDLE);
        Ok(())
    }

    /// Compile (set! name value) — mutate an existing binding
    fn compile_set(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 2 {
            return Err("set! requires name and value".into());
        }
        self.compile(heap, args[1])?;
        let name_idx = self.add_constant(args[0]);
        self.emit(OP_DEF);
        self.emit_u16(name_idx);
        Ok(())
    }

    /// Compile (while cond body...)
    fn compile_while(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("while requires condition and body".into());
        }
        let condition = args[0];
        let body_exprs = &args[1..];

        let loop_start = self.code.len();

        self.compile(heap, condition)?;

        self.emit(OP_JUMP_IF_FALSE);
        let exit_jump = self.code.len();
        self.emit_u16(0);

        for &expr in body_exprs {
            self.compile(heap, expr)?;
            self.emit(OP_POP);
        }

        self.emit(OP_LOOP_BACK);
        let back_distance = (self.code.len() + 2) - loop_start;
        self.emit_u16(back_distance as u16);

        let exit_offset = self.code.len() - (exit_jump + 2);
        self.code[exit_jump] = (exit_offset >> 8) as u8;
        self.code[exit_jump + 1] = (exit_offset & 0xFF) as u8;

        self.emit(OP_NIL);
        Ok(())
    }

    fn compile_load(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("load requires exactly one argument (path)".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_LOAD);
        Ok(())
    }

    fn compile_source(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("source requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_SOURCE);
        Ok(())
    }
}
