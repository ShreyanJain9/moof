/// The MOOF bytecode compiler.
///
/// Compiles cons-cell ASTs into BytecodeChunks. The compiler handles
/// the 6 kernel forms and the syntactic desugaring (%send, %block, %do).
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
        // Reuse existing constant if possible
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
                // A bare symbol is a variable lookup
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
                        // String literal — push as constant
                        let idx = self.add_constant(expr);
                        self.emit(OP_CONST);
                        self.emit_u16(idx);
                        Ok(())
                    }
                    _ => {
                        // Other heap objects — push as constant
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

        // Check if head is a known special form
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
                // Derived forms (compiled for efficiency)
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
                // set! for mutating env bindings
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
        // We use OP_APPLY so that operatives get unevaluated args at runtime.
        // Compile f, then push a quoted list of the raw arg ASTs.
        self.compile(heap, head)?;
        let args_list = heap.list(&elements[1..]);
        let args_idx = self.add_constant(args_list);
        self.emit(OP_QUOTE);
        self.emit_u16(args_idx);
        self.emit(OP_APPLY);
        Ok(())
    }

    /// Compile (vau (params) $env body...) — stores original AST for source introspection
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

        // Build source AST: (vau params env_param body...)
        let source = if source_override != Value::Nil {
            source_override
        } else {
            let vau_sym = Value::Symbol(heap.intern("vau"));
            let mut src_elems = vec![vau_sym];
            src_elems.extend_from_slice(args);
            heap.list(&src_elems)
        };

        // Compile the body into its own chunk
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
        let name = args[0];
        let value = args[1];

        // Compile the value expression
        self.compile(heap, value)?;

        // Emit def
        let name_idx = self.add_constant(name);
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

        // Compile receiver
        self.compile(heap, receiver)?;
        // Compile each argument
        for &arg in msg_args {
            self.compile(heap, arg)?;
        }
        // Emit send with selector
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

        // Build source AST
        let block_sym = Value::Symbol(heap.intern("block"));
        let source = heap.list(&[block_sym, params, body_expr]);

        // Compile body into its own chunk
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
    /// Derived form — but compiled specially for efficiency.
    fn compile_if(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("if requires condition and then-branch".into());
        }
        let condition = args[0];
        let then_branch = args[1];
        let else_branch = if args.len() > 2 { args[2] } else { Value::Nil };

        // Compile condition
        self.compile(heap, condition)?;

        // Jump over then-branch if false
        self.emit(OP_JUMP_IF_FALSE);
        let jump_to_else = self.code.len();
        self.emit_u16(0); // placeholder

        // Compile then-branch
        self.compile(heap, then_branch)?;

        // Jump over else-branch
        self.emit(OP_JUMP);
        let jump_to_end = self.code.len();
        self.emit_u16(0); // placeholder

        // Patch jump-to-else
        let else_offset = self.code.len() - (jump_to_else + 2);
        self.code[jump_to_else] = (else_offset >> 8) as u8;
        self.code[jump_to_else + 1] = (else_offset & 0xFF) as u8;

        // Compile else-branch
        match else_branch {
            Value::Nil if args.len() <= 2 => self.emit(OP_NIL),
            _ => self.compile(heap, else_branch)?,
        }

        // Patch jump-to-end
        let end_offset = self.code.len() - (jump_to_end + 2);
        self.code[jump_to_end] = (end_offset >> 8) as u8;
        self.code[jump_to_end + 1] = (end_offset & 0xFF) as u8;

        Ok(())
    }

    /// Compile (lambda (params) body...)
    /// Sugar for (wrap (vau (params) $_ body...)) — evaluates args before passing.
    fn compile_lambda(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("lambda requires (params) and body".into());
        }
        let params = args[0];
        let body_exprs = &args[1..];

        // Build source AST: (lambda params body...)
        let lambda_sym = Value::Symbol(heap.intern("lambda"));
        let mut src_elems = vec![lambda_sym];
        src_elems.extend_from_slice(args);
        let source = heap.list(&src_elems);

        // Compile the body into its own chunk
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

        // Desugar let into lambda application:
        // (let ((a 1) (b 2)) body) → ((lambda (a b) body) 1 2)
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

        // Build a lambda with the params
        let param_list = heap.list(&params);
        let mut lambda_args = vec![param_list];
        lambda_args.extend_from_slice(body_exprs);

        // Compile as lambda
        self.compile_lambda(heap, &lambda_args)?;

        // Compile args and call
        let argc = values.len();
        for val in values {
            self.compile(heap, val)?;
        }
        self.emit(OP_CALL);
        self.emit(argc as u8);
        Ok(())
    }

    /// Compile (eval expr) or (eval expr env)
    /// One-arg form: evaluate in the current environment
    /// Two-arg form: evaluate in the given environment — [env eval: expr]
    fn compile_eval(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        match args.len() {
            1 => {
                self.compile(heap, args[0])?;
                self.emit(OP_EVAL);
                Ok(())
            }
            2 => {
                // (eval expr env) → [env eval: expr]
                // Compile env (receiver), then expr (arg), then send eval:
                self.compile(heap, args[1])?; // env
                self.compile(heap, args[0])?; // expr
                let sel = self.add_constant(Value::Symbol(heap.intern("eval:")));
                self.emit(OP_SEND);
                self.emit_u16(sel);
                self.emit(1u8);
                Ok(())
            }
            _ => Err("eval requires 1 or 2 arguments".into()),
        }
    }

    /// Compile (print expr)
    fn compile_print(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("print requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_PRINT);
        Ok(())
    }

    /// Compile (car expr)
    fn compile_car(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("car requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_CAR);
        Ok(())
    }

    /// Compile (cdr expr)
    fn compile_cdr(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("cdr requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_CDR);
        Ok(())
    }

    /// Compile (type-of expr)
    fn compile_type_of(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("type-of requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_TYPE_OF);
        Ok(())
    }

    /// Compile (list a b c ...) → nested cons: (cons a (cons b (cons c nil)))
    fn compile_list_form(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        // Desugar to nested cons calls at compile time.
        // (list) → nil
        // (list a) → (cons a nil) → compile(a), OP_NIL, OP_CONS
        // (list a b) → (cons a (cons b nil)) → compile(a), compile(b), OP_NIL, OP_CONS, OP_CONS
        // Pattern: compile all args left to right, push nil, then CONS n times.
        // Stack trace for (list a b c):
        //   compile(a) → [a]
        //   compile(b) → [a, b]
        //   compile(c) → [a, b, c]
        //   OP_NIL     → [a, b, c, nil]
        //   OP_CONS    → [a, b, (c . nil)]     — pops nil(cdr), c(car)
        //   OP_CONS    → [a, (b . (c . nil))]   — pops (c.nil)(cdr), b(car)
        //   OP_CONS    → [(a . (b . (c . nil)))] — correct!
        for &arg in args.iter() {
            self.compile(heap, arg)?;
        }
        self.emit(OP_NIL);
        for _ in 0..args.len() {
            self.emit(OP_CONS);
        }
        Ok(())
    }

    /// Compile (object) or (object parent) or (object parent slot1: val1 slot2: val2)
    fn compile_object(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        // First arg is parent (or nil)
        if args.is_empty() {
            self.emit(OP_NIL); // no parent
            self.emit(OP_MAKE_OBJECT);
            self.emit(0u8);
            return Ok(());
        }

        // Compile parent
        self.compile(heap, args[0])?;

        // Remaining args are key-value pairs: (object parent #x 10 #y 20)
        let slot_args = &args[1..];
        if slot_args.len() % 2 != 0 {
            return Err("object: slot args must be key-value pairs".into());
        }
        let slot_count = slot_args.len() / 2;
        for pair in slot_args.chunks(2) {
            // key should be a symbol
            self.compile(heap, pair[0])?;
            self.compile(heap, pair[1])?;
        }
        self.emit(OP_MAKE_OBJECT);
        self.emit(slot_count as u8);
        Ok(())
    }

    /// Compile (handle! obj selector handler)
    /// e.g. (handle! my-point #distanceTo: (lambda (self other) ...))
    fn compile_handle(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 3 {
            return Err("handle! requires object, selector, and handler".into());
        }
        self.compile(heap, args[0])?; // object
        self.compile(heap, args[1])?; // selector (should be a symbol)
        self.compile(heap, args[2])?; // handler (lambda/operative)
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

    /// Compile (while cond body...) — loop while condition is truthy
    fn compile_while(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() < 2 {
            return Err("while requires condition and body".into());
        }
        let condition = args[0];
        let body_exprs = &args[1..];

        // loop_start:
        let loop_start = self.code.len();

        // Compile condition
        self.compile(heap, condition)?;

        // Jump past body if false
        self.emit(OP_JUMP_IF_FALSE);
        let exit_jump = self.code.len();
        self.emit_u16(0); // placeholder

        // Compile body (discard results)
        for &expr in body_exprs {
            self.compile(heap, expr)?;
            self.emit(OP_POP);
        }

        // Jump back to loop_start
        // We need a backwards jump. OP_JUMP adds offset to ip, but we need to go backwards.
        // Use a separate mechanism: encode as jump to absolute position via relative offset.
        // Current ip after this jump instruction will be code.len() + 3.
        // We want to jump to loop_start.
        // OP_JUMP sets ip = ip + 3 + offset. We need ip + 3 + offset = loop_start.
        // offset = loop_start - (ip + 3). If loop_start < ip + 3, this is negative.
        // Our jump offsets are u16 (unsigned). We need a backward jump opcode.
        // For now: use OP_JUMP with a special encoding. Let's add OP_LOOP.
        // Actually, simpler: just emit a JUMP_BACK that subtracts.
        // Let's use a constant for the absolute target and the EVAL mechanism...
        // Or just add OP_LOOP_BACK.

        // Simplest: encode backward jump distance as u16 from current position.
        self.emit(OP_LOOP_BACK);
        let back_distance = (self.code.len() + 2) - loop_start;
        self.emit_u16(back_distance as u16);

        // Patch exit jump
        let exit_offset = self.code.len() - (exit_jump + 2);
        self.code[exit_jump] = (exit_offset >> 8) as u8;
        self.code[exit_jump + 1] = (exit_offset & 0xFF) as u8;

        // while returns nil
        self.emit(OP_NIL);
        Ok(())
    }

    /// Compile (load "path.moof")
    fn compile_load(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("load requires exactly one argument (path)".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_LOAD);
        Ok(())
    }

    /// Compile (source lambda-or-operative) → returns AST
    fn compile_source(&mut self, heap: &mut Heap, args: &[Value]) -> Result<(), String> {
        if args.len() != 1 {
            return Err("source requires exactly one argument".into());
        }
        self.compile(heap, args[0])?;
        self.emit(OP_SOURCE);
        Ok(())
    }
}
