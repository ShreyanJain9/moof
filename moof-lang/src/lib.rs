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

/// Set up the moof shell on a fabric: register type invokers, create root env.
/// IO capabilities are registered separately on the server's system vats.
pub fn setup(fabric: &mut Fabric) -> SetupResult {
    let mut native = NativeInvoker::new();

    // Register type conventions (Integer +, String length, etc.)
    conventions::register(fabric, &mut native);

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

    SetupResult { root_env }
}

pub struct SetupResult {
    pub root_env: u32,
}

// ── Extension entry point ──
// When loaded as a dylib by the server, this function is called.
// It registers moof-lang: BytecodeInvoker, type conventions, IO handlers,
// bootstrap, and sets up a root environment.

/// The dylib entry point. Called by the server when loading this extension.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moof_extension_init(server_ptr: *mut moof_server::Server) {
    let server = &mut *server_ptr;

    // Register language shell
    let result = setup(server.fabric());
    let root_env = result.root_env;

    // Register IO on system vats
    io::register_system_handlers(server);

    // print for bootstrap
    let _ = eval(server.fabric(),
        "(def print (lambda (x) [Console writeLine: x]))", root_env);

    // Load bootstrap from lib/
    let bootstrap_path = std::path::PathBuf::from("lib/bootstrap.moof");
    if bootstrap_path.exists() {
        if let Ok(source) = std::fs::read_to_string(&bootstrap_path) {
            let body = skip_module_header(&source);
            match eval(server.fabric(), body, root_env) {
                Ok(_) => eprintln!("  [moof-lang] bootstrap loaded"),
                Err(e) => eprintln!("  [moof-lang] bootstrap failed: {}", e),
            }
        }
    }

    // Store root_env on the server for client connections
    // We use a well-known slot on the fabric's first object
    let root_env_sym = server.fabric().intern("__moof_root_env");
    let sentinel = server.fabric().create_object(moof_fabric::Value::Nil);
    server.fabric().heap.slot_set(sentinel, root_env_sym, moof_fabric::Value::Object(root_env));
    server.system.by_name.insert("__moof_env".to_string(), sentinel);

    eprintln!("  [moof-lang] registered (BytecodeInvoker + {} type handlers)",
        server.fabric().heap.len());
}

fn skip_module_header(source: &str) -> &str {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in source.char_indices() {
        if escape { escape = false; continue; }
        if in_string { if ch == '\\' { escape = true; } else if ch == '"' { in_string = false; } continue; }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => { depth -= 1; if depth == 0 { return source[i+1..].trim_start(); } }
            _ => {}
        }
    }
    source
}
