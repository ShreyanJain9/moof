mod runtime;
mod vm;
mod reader;
mod compiler;

use std::io::{self, Write, BufRead};
use std::path::PathBuf;
use vm::exec::VM;
use reader::lexer::Lexer;
use reader::parser::Parser;
use compiler::compile::Compiler;
use runtime::value::{Value, HeapObject};

fn main() {
    let mut vm = VM::new();

    // Create the root environment
    let root_env = vm.heap.alloc_env(None);

    // Bootstrap: bind kernel primitives, then load the MOOF standard library
    bootstrap_env(&mut vm, root_env);

    // Find and load bootstrap.moof from the lib/ directory
    let bootstrap_path = find_bootstrap();
    match std::fs::read_to_string(&bootstrap_path) {
        Ok(source) => {
            match eval_source(&mut vm, root_env, &source, &bootstrap_path.display().to_string()) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("!! Bootstrap failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("!! Cannot read {}: {}", bootstrap_path.display(), e);
            std::process::exit(1);
        }
    }

    // Store the lib path in the environment so (load) can find files
    let lib_dir = bootstrap_path.parent().unwrap().to_string_lossy().to_string();
    let lib_dir_sym = vm.heap.intern("*lib-path*");
    let lib_dir_val = vm.heap.alloc_string(&lib_dir);
    vm.env_define_helper(root_env, lib_dir_sym, lib_dir_val);

    // Store the root env id so the VM can access it for load
    vm.root_env = Some(root_env);

    // Register type prototypes (defined in bootstrap.moof)
    register_type_prototypes(&mut vm, root_env);

    println!("MOOF — Moof Open Objectspace Fabric");
    println!("clarus the dogcow lives again");
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    // REPL with multi-line support
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = String::new();

    loop {
        if buffer.is_empty() {
            print!("moof> ");
        } else {
            print!("  ... ");
        }
        stdout.flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!("\nmoof.");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() && buffer.is_empty() { continue; }

                buffer.push_str(&line);

                // Check if brackets are balanced
                if !brackets_balanced(&buffer) {
                    continue; // need more input
                }

                let input = buffer.trim().to_string();
                buffer.clear();

                if input.is_empty() { continue; }

                match eval_line(&mut vm, root_env, &input) {
                    Ok(val) => {
                        let formatted = vm.format_value(val);
                        println!("=> {}", formatted);
                    }
                    Err(e) => {
                        println!("!! {}", e);
                    }
                }
            }
            Err(e) => {
                println!("!! Read error: {}", e);
                break;
            }
        }
    }
}

/// Find bootstrap.moof: check ./lib/, then next to the binary.
fn find_bootstrap() -> PathBuf {
    // Try relative to CWD first
    let cwd_path = PathBuf::from("lib/bootstrap.moof");
    if cwd_path.exists() {
        return cwd_path;
    }
    // Try relative to the binary
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap();
        let exe_path = exe_dir.join("lib/bootstrap.moof");
        if exe_path.exists() {
            return exe_path;
        }
        // Try one level up (for cargo run where binary is in target/debug/)
        let parent_path = exe_dir.join("../../lib/bootstrap.moof");
        if parent_path.exists() {
            return parent_path;
        }
    }
    // Default — will produce a clear error
    cwd_path
}

fn eval_line(vm: &mut VM, env_id: u32, input: &str) -> Result<Value, String> {
    eval_source(vm, env_id, input, "<repl>")
}

pub fn eval_source(vm: &mut VM, env_id: u32, input: &str, source_name: &str) -> Result<Value, String> {
    // Lex
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize()
        .map_err(|e| format!("{}: lex error: {}", source_name, e))?;

    // Parse
    let mut parser = Parser::new(tokens);
    let exprs = parser.parse_all(&mut vm.heap)
        .map_err(|e| format!("{}: parse error: {}", source_name, e))?;

    if exprs.is_empty() {
        return Ok(Value::Nil);
    }

    // Compile and execute each expression, return the last result
    let mut result = Value::Nil;
    for expr in exprs {
        let mut compiler = Compiler::new();
        let chunk = compiler.compile_expr(&mut vm.heap, expr)?;
        let chunk_id = vm.heap.alloc_chunk(chunk);
        result = vm.execute(chunk_id, env_id)?;
    }
    Ok(result)
}

/// Check if brackets/parens/braces are balanced (for multi-line REPL).
fn brackets_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut in_comment = false;
    let mut escape = false;
    for ch in s.chars() {
        if in_comment {
            if ch == '\n' { in_comment = false; }
            continue;
        }
        if escape { escape = false; continue; }
        if in_string {
            if ch == '\\' { escape = true; }
            else if ch == '"' { in_string = false; }
            continue;
        }
        match ch {
            '"' => in_string = true,
            ';' => in_comment = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

/// Bootstrap the root environment with essential bindings.
fn bootstrap_env(vm: &mut VM, env_id: u32) {
    let pairs: &[(&str, Value)] = &[
        ("nil", Value::Nil),
        ("true", Value::True),
        ("false", Value::False),
    ];
    for &(name, val) in pairs {
        let sym = vm.heap.intern(name);
        vm.env_define_helper(env_id, sym, val);
    }
}

impl VM {
    /// Helper to define in an environment (used by bootstrap).
    pub fn env_define_helper(&mut self, env_id: u32, sym: u32, val: Value) {
        match self.heap.get_mut(env_id) {
            HeapObject::Environment(env) => { env.define(sym, val); }
            _ => panic!("Not an environment"),
        }
    }
}

/// Look up type prototypes from bootstrap and register native handlers.
fn register_type_prototypes(vm: &mut VM, env_id: u32) {
    // (name, setter, native_handlers as (selector, argc_excluding_self))
    let protos: &[(&str, fn(&mut VM) -> &mut Option<u32>, &[(&str, u8)])] = &[
        ("Integer", |vm| &mut vm.proto_integer, &[
            ("+",1), ("-",1), ("*",1), ("/",1), ("%",1),
            ("<",1), (">",1), ("=",1), ("<=",1), (">=",1),
            ("negate",0), ("abs",0), ("toString",0),
        ]),
        ("Boolean", |vm| &mut vm.proto_boolean, &[
            ("not",0), ("and:",1), ("or:",1),
            ("ifTrue:",1), ("ifTrue:ifFalse:",2), ("ifFalse:",1),
            ("toString",0),
        ]),
        ("String", |vm| &mut vm.proto_string, &[
            ("length",0), ("++",1), ("toString",0),
            ("substring:to:",2), ("at:",1), ("indexOf:",1),
            ("split:",1), ("trim",0),
            ("startsWith:",1), ("endsWith:",1), ("contains:",1),
            ("toUpper",0), ("toLower",0),
            ("toSymbol",0), ("toInteger",0),
            ("chars",0), ("replace:with:",2),
        ]),
        ("Cons", |vm| &mut vm.proto_cons, &[
            ("car",0), ("cdr",0), ("toString",0),
        ]),
        ("Nil", |vm| &mut vm.proto_nil, &[
            ("isNil",0), ("toString",0),
        ]),
        ("Symbol", |vm| &mut vm.proto_symbol, &[
            ("toString",0), ("asString",0), ("name",0),
        ]),
        ("Lambda", |vm| &mut vm.proto_lambda, &[
            ("source",0), ("params",0), ("arity",0), ("toString",0),
        ]),
        ("Operative", |vm| &mut vm.proto_operative, &[
            ("source",0), ("params",0), ("envParam",0), ("toString",0),
        ]),
        // Block is gone — blocks are just lambdas now
        ("Environment", |vm| &mut vm.proto_environment, &[
            ("eval:",1), ("lookup:",1), ("set:to:",2), ("toString",0),
        ]),
    ];

    for &(name, ref setter, native_sels) in protos {
        let sym = vm.heap.intern(name);
        if let Ok(Value::Object(id)) = vm.env_lookup_helper(env_id, sym) {
            *setter(vm) = Some(id);
            if !native_sels.is_empty() {
                vm.register_native_handlers(id, env_id, name, native_sels);
            }
        }
    }
}

impl VM {
    fn env_lookup_helper(&self, env_id: u32, sym: u32) -> Result<Value, String> {
        let mut current = Some(env_id);
        while let Some(eid) = current {
            match self.heap.get(eid) {
                HeapObject::Environment(env) => {
                    if let Some(val) = env.lookup_local(sym) {
                        return Ok(val);
                    }
                    current = env.parent;
                }
                _ => return Err("not an env".into()),
            }
        }
        Err("not found".into())
    }
}
