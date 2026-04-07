pub mod lexer;
pub mod parser;
pub mod opcodes;
pub mod compiler;
pub mod interpreter;
pub mod conventions;
pub mod io;

use moof_fabric::{Fabric, Value, NativeInvoker};

/// Evaluate a moof expression string in the given environment.
pub fn eval(fabric: &mut Fabric, source: &str, env_id: u32) -> Result<Value, String> {
    let mut lex = lexer::Lexer::new(source);
    let tokens = lex.tokenize().map_err(|e| format!("lex: {}", e))?;
    let mut parser = parser::Parser::new(tokens);
    let exprs = parser.parse_all(&mut fabric.heap).map_err(|e| format!("parse: {}", e))?;

    if exprs.is_empty() {
        return Ok(Value::Nil);
    }

    let mut result = Value::Nil;
    for expr in exprs {
        let mut comp = compiler::Compiler::new();
        let chunk = comp.compile_expr(&mut fabric.heap, expr)?;
        let chunk_id = chunk.store_in(&mut fabric.heap);
        result = interpreter::eval_chunk(fabric, chunk_id, env_id)?;
    }
    Ok(result)
}

/// Set up the moof shell on a fabric: register invokers, create root env.
/// Returns (root_env, io_capabilities).
pub fn setup(fabric: &mut Fabric) -> SetupResult {
    let mut native = NativeInvoker::new();

    // Register type conventions (Integer +, String length, etc.)
    conventions::register(fabric, &mut native);

    // Create IO capability objects
    let io_caps = io::create_capabilities(fabric, &mut native);

    // Register all invokers
    fabric.register_invoker(Box::new(native));
    fabric.register_invoker(Box::new(interpreter::BytecodeInvoker));

    // Create root environment
    let root_env = fabric.heap.alloc_env(None);

    // Bind fundamental values
    let nil_sym = fabric.intern("nil");
    fabric.heap.env_define(root_env, nil_sym, Value::Nil);
    let true_sym = fabric.intern("true");
    fabric.heap.env_define(root_env, true_sym, Value::True);
    let false_sym = fabric.intern("false");
    fabric.heap.env_define(root_env, false_sym, Value::False);

    SetupResult { root_env, io: io_caps }
}

pub struct SetupResult {
    pub root_env: u32,
    pub io: io::IoCapabilities,
}
