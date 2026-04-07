pub mod lexer;
pub mod parser;
pub mod opcodes;
pub mod compiler;
pub mod interpreter;
pub mod conventions;

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
// ── Extension entry point ──
// When loaded as a dylib by the server, this registers the moof
// language shell: BytecodeInvoker, type conventions, bootstrap.
// IO is NOT registered here — that's the server's concern.

/// The dylib entry point.
#[unsafe(no_mangle)]
pub extern "C" fn moof_extension_init(server_ptr: *mut moof_server::Server) {
    let server = unsafe { &mut *server_ptr };

    // Register language shell (BytecodeInvoker + type conventions)
    let result = setup(server.fabric());
    let root_env = result.root_env;

    // Bind system vat capabilities into root env so bootstrap can use them
    let caps: Vec<(String, u32)> = server.system.by_name.iter()
        .filter(|(k, _)| !k.starts_with("__"))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    for (name, obj_id) in caps {
        let sym = server.fabric().intern(&name);
        server.fabric().heap.env_define(root_env, sym, Value::Object(obj_id));
    }

    // print for bootstrap (Console is now bound in root_env)
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

    // Store root_env for client connections
    let root_env_sym = server.fabric().intern("__moof_root_env");
    let sentinel = server.fabric().create_object(Value::Nil);
    server.fabric().heap.slot_set(sentinel, root_env_sym, Value::Object(root_env));
    server.system.by_name.insert("__moof_env".to_string(), sentinel);

    // Register eval hook so the server can dispatch eval: without knowing about moof-lang
    server.eval_hook = Some(Box::new(|fabric, source, env_id| {
        eval(fabric, source, env_id)
    }));

    eprintln!("  [moof-lang] registered");
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
