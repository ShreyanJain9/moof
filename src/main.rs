mod runtime;
mod vm;
mod reader;
mod compiler;
mod persistence;
mod modules;
mod tui;
mod ffi;
mod gui;

use std::io::{self, Write, BufRead};
use std::path::PathBuf;
use vm::exec::VM;
use reader::lexer::Lexer;
use reader::parser::Parser;
use compiler::compile::Compiler;
use runtime::value::{Value, HeapObject};
use persistence::snapshot::{self, Image};
use persistence::wal;

fn image_dir() -> PathBuf {
    PathBuf::from(".moof")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gui_mode = args.iter().any(|a| a == "--gui");
    let mcp_mode = args.iter().any(|a| a == "--mcp");
    let image_mode = args.iter().any(|a| a == "--image");

    let mut vm;
    let root_env;
    let mut module_loader: Option<modules::loader::ModuleLoader> = None;

    if image_mode {
        // ── Legacy: load from binary image ──
        let img_dir = image_dir();
        if snapshot::image_exists(&img_dir) {
            match snapshot::load_image(&img_dir) {
                Ok(image) => {
                    vm = VM::new();
                    vm.heap = runtime::heap::Heap::from_image(image.objects, image.symbol_names);

                    match wal::replay_wal(&img_dir) {
                        Ok(entries) => {
                            if !entries.is_empty() {
                                eprintln!("(replaying {} WAL entries)", entries.len());
                                vm.heap.replay_wal(&entries);
                            }
                        }
                        Err(e) => eprintln!("!! WAL replay warning: {}", e),
                    }

                    let root_sym = vm.heap.intern("*root-env*");
                    root_env = match vm.env_lookup_helper(0, root_sym) {
                        Ok(Value::Object(id)) => id,
                        _ => 0,
                    };
                    vm.root_env = Some(root_env);
                    register_type_prototypes(&mut vm, root_env);

                    eprintln!("(image loaded: {} objects, {} symbols)",
                        vm.heap.len(), vm.heap.symbol_count());
                }
                Err(e) => {
                    eprintln!("!! Image load failed: {}", e);
                    eprintln!("!! Falling back to bootstrap");
                    let (v, r) = bootstrap_fresh();
                    vm = v;
                    root_env = r;
                }
            }
        } else {
            let (v, r) = bootstrap_fresh();
            vm = v;
            root_env = r;
        }

        // Attach WAL for durability in image mode
        match wal::WalWriter::open(&img_dir) {
            Ok(wal_writer) => vm.heap.set_wal(wal_writer),
            Err(e) => eprintln!("!! WAL init warning: {}", e),
        }
    } else {
        // ── Default: source-level module system ──
        vm = VM::new();
        root_env = vm.heap.alloc_env(None);
        bootstrap_env(&mut vm, root_env);

        let lib_dir = find_bootstrap().parent().unwrap().to_path_buf();

        // Store root env reference
        let root_sym = vm.heap.intern("*root-env*");
        vm.env_define_helper(root_env, root_sym, Value::Object(root_env));
        vm.root_env = Some(root_env);

        // Discover and load modules
        match modules::loader::ModuleLoader::discover(&lib_dir, &mut vm.heap) {
            Ok(mut loader) => {
                match load_modules_sequenced(&mut loader, &mut vm, root_env) {
                    Ok(()) => {
                        loader.merge_into_root(&mut vm, root_env);

                        let module_count = loader.graph.modules.len();
                        eprintln!("(loaded {} modules, {} objects, {} symbols)",
                            module_count, vm.heap.len(), vm.heap.symbol_count());

                        module_loader = Some(loader);
                    }
                    Err(e) => {
                        eprintln!("!! Module loading failed: {}", e);
                        eprintln!("!! Falling back to legacy bootstrap");
                        let bootstrap_path = find_bootstrap();
                        if let Ok(source) = std::fs::read_to_string(&bootstrap_path) {
                            let body = strip_module_header(&source);
                            let _ = eval_source(&mut vm, root_env, body,
                                &bootstrap_path.display().to_string());
                        }
                        register_type_prototypes(&mut vm, root_env);
                        load_stdlib(&mut vm, root_env);
                    }
                }
            }
            Err(e) => {
                eprintln!("!! Module discovery failed: {}", e);
                eprintln!("!! Falling back to legacy bootstrap");
                let bootstrap_path = find_bootstrap();
                if let Ok(source) = std::fs::read_to_string(&bootstrap_path) {
                    let body = strip_module_header(&source);
                    let _ = eval_source(&mut vm, root_env, body,
                        &bootstrap_path.display().to_string());
                }
                register_type_prototypes(&mut vm, root_env);
                load_stdlib(&mut vm, root_env);
            }
        }
    }

    if gui_mode {
        println!("MOOF — launching System Browser...");
        if image_mode { save_image(&vm); }
        gui::browser::run_browser(vm, root_env);
        return;
    }

    if mcp_mode {
        // MCP server mode: run the mcp-serve function over stdio
        match eval_source(&mut vm, root_env, "(mcp-serve)", "<mcp>") {
            Ok(_) => {}
            Err(e) => {
                eprintln!("!! MCP server failed: {}", e);
                std::process::exit(1);
            }
        }
        if image_mode { save_image(&vm); }
        return;
    }

    println!("MOOF — Moof Open Objectspace Fabric");
    println!("clarus the dogcow lives again");
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    // REPL with multi-line support
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = String::new();
    let mut last_checkpoint_size = vm.heap.len();

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
                if image_mode { save_image(&vm); }
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

                // REPL-level commands
                if input == "(checkpoint)" || input == "(save)" {
                    if image_mode {
                        save_image(&vm);
                    } else {
                        eprintln!("(source is truth — no binary image to save)");
                        eprintln!("(your .moof files ARE the canonical state)");
                    }
                    continue;
                }
                if input == "(browse)" {
                    let _ = tui::inspector::run_inspector(&vm.heap, None);
                    continue;
                }
                if input.starts_with("(browse ") {
                    let arg_source = &input[8..input.len()-1];
                    match eval_line(&mut vm, root_env, arg_source) {
                        Ok(val) => { let _ = tui::inspector::run_inspector(&vm.heap, Some(val)); }
                        Err(e) => println!("!! {}", e),
                    }
                    continue;
                }

                // Module system commands
                if input == "(modules)" {
                    if let Some(ref loader) = module_loader {
                        let mods = loader.list_modules();
                        for (name, requires, provides) in mods {
                            let req_str = if requires.is_empty() {
                                "(kernel)".to_string()
                            } else {
                                requires.join(", ")
                            };
                            eprintln!("  {} [requires: {}] [provides: {} symbols]",
                                name, req_str, provides.len());
                        }
                    } else {
                        eprintln!("(module system not active — use without --image)");
                    }
                    continue;
                }
                if input.starts_with("(module-reload ") {
                    let name = input[15..input.len()-1].trim();
                    if let Some(ref mut loader) = module_loader {
                        match loader.reload(name, &mut vm, root_env) {
                            Ok(()) => eprintln!("(reloaded {})", name),
                            Err(e) => eprintln!("!! {}", e),
                        }
                    } else {
                        eprintln!("(module system not active)");
                    }
                    continue;
                }
                if input.starts_with("(module-remove ") {
                    let name = input[15..input.len()-1].trim();
                    if let Some(ref mut loader) = module_loader {
                        match loader.remove(name) {
                            Ok(removed_symbols) => {
                                // Unbind removed symbols from root env
                                for sym_name in &removed_symbols {
                                    let sym = vm.heap.intern(sym_name);
                                    vm.env_define_helper(root_env, sym, Value::Nil);
                                }
                                eprintln!("(removed {} — unbound {} symbols)", name, removed_symbols.len());
                            }
                            Err(e) => eprintln!("!! {}", e),
                        }
                    } else {
                        eprintln!("(module system not active)");
                    }
                    continue;
                }
                if input.starts_with("(module-exports ") {
                    let name = input[16..input.len()-1].trim();
                    if let Some(ref loader) = module_loader {
                        if let Some(exports) = loader.exports.get(name) {
                            for (sym, _val) in exports {
                                eprintln!("  {}", sym);
                            }
                        } else {
                            eprintln!("!! unknown module: {}", name);
                        }
                    } else {
                        eprintln!("(module system not active)");
                    }
                    continue;
                }

                match eval_line(&mut vm, root_env, &input) {
                    Ok(val) => {
                        let formatted = vm.format_value(val);
                        println!("=> {}", formatted);
                    }
                    Err(e) => {
                        println!("!! {}", e);
                    }
                }

                // Auto-checkpoint only in image mode
                if image_mode && vm.heap.len() > last_checkpoint_size + 5000 {
                    save_image(&vm);
                    eprintln!("(auto-saved)");
                    last_checkpoint_size = vm.heap.len();
                }
            }
            Err(e) => {
                println!("!! Read error: {}", e);
                break;
            }
        }
    }
}

/// Run a fresh bootstrap: create VM, load bootstrap.moof, register prototypes.
fn bootstrap_fresh() -> (VM, u32) {
    let mut vm = VM::new();
    let root_env = vm.heap.alloc_env(None);

    bootstrap_env(&mut vm, root_env);

    let bootstrap_path = find_bootstrap();
    match std::fs::read_to_string(&bootstrap_path) {
        Ok(source) => {
            // Skip module header if present
            let body = strip_module_header(&source);
            match eval_source(&mut vm, root_env, body, &bootstrap_path.display().to_string()) {
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

    let lib_dir = bootstrap_path.parent().unwrap().to_string_lossy().to_string();
    let lib_dir_sym = vm.heap.intern("*lib-path*");
    let lib_dir_val = vm.heap.alloc_string(&lib_dir);
    vm.env_define_helper(root_env, lib_dir_sym, lib_dir_val);

    // Store root env reference for image recovery
    let root_sym = vm.heap.intern("*root-env*");
    vm.env_define_helper(root_env, root_sym, Value::Object(root_env));

    vm.root_env = Some(root_env);
    register_type_prototypes(&mut vm, root_env);
    load_stdlib(&mut vm, root_env);

    (vm, root_env)
}

/// Save the heap as a snapshot image, compacting first to remove garbage.
/// Also projects source files for git diffing.
fn save_image(vm: &VM) {
    let image = Image {
        objects: vm.heap.objects().to_vec(),
        symbol_names: vm.heap.symbol_names_ref().to_vec(),
    };

    let root_env = vm.root_env.unwrap_or(0);
    let (compacted, _new_root) = snapshot::compact_image(&image, root_env);
    let before = image.objects.len();
    let after = compacted.objects.len();

    let dir = image_dir();
    match snapshot::save_image(&compacted, &dir) {
        Ok(hash) => eprintln!("(image saved: {} objects (compacted from {}), hash {}...)",
            after, before, &hash[..12]),
        Err(e) => eprintln!("!! Image save failed: {}", e),
    }

    // Project source files for git diffing
    match persistence::source_project::project_source(&vm.heap, root_env, &dir) {
        Ok(()) => {}
        Err(e) => eprintln!("!! Source projection failed: {}", e),
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
        self.heap.env_define(env_id, sym, val);
    }
}

/// Look up type prototypes from bootstrap and register all native handlers.
/// Then load standard library files.
fn register_type_prototypes(vm: &mut VM, env_id: u32) {
    vm::natives::register_all_natives(vm, env_id);
}

/// Load standard library .moof files (after natives are registered).
fn load_stdlib(vm: &mut VM, root_env: u32) {
    let lib_sym = vm.heap.intern("*lib-path*");
    let lib_dir = match vm.env_lookup_helper(root_env, lib_sym) {
        Ok(Value::Object(id)) => match vm.heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return,
        },
        _ => return,
    };

    let libs = ["collections.moof", "classes.moof", "membrane.moof", "json.moof", "mcp.moof"];
    for lib in &libs {
        let path = format!("{}/{}", lib_dir, lib);
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                // Skip module header if present
                let body = strip_module_header(&source);
                match eval_source(vm, root_env, body, &path) {
                    Ok(_) => {}
                    Err(e) => eprintln!("!! Loading {}: {}", lib, e),
                }
            }
            Err(_) => {} // library not found — skip silently
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

/// Load modules in topo order, registering type prototypes after bootstrap.
fn load_modules_sequenced(
    loader: &mut modules::loader::ModuleLoader,
    vm: &mut VM,
    root_env: u32,
) -> Result<(), String> {
    let order = loader.load_order()?;

    for name in &order {
        loader.load_one(name, vm)?;

        // After bootstrap: merge its exports into root env and register
        // native handlers so subsequent modules can use toSymbol, +, etc.
        if name == "bootstrap" {
            if let Some(exports) = loader.exports.get("bootstrap") {
                for (sym_name, val) in exports {
                    let sym = vm.heap.intern(sym_name);
                    vm.env_define_helper(root_env, sym, *val);
                }
            }
            register_type_prototypes(vm, root_env);
        }
    }

    Ok(())
}

/// Strip a (module ...) header from source if present, returning the body.
fn strip_module_header(source: &str) -> &str {
    let trimmed = source.trim_start();
    if !trimmed.starts_with("(module ") {
        return source;
    }
    // Find the matching close paren
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let offset = source.len() - trimmed.len();
    for (i, ch) in trimmed.char_indices() {
        if escape { escape = false; continue; }
        if in_string {
            if ch == '\\' { escape = true; }
            else if ch == '"' { in_string = false; }
            continue;
        }
        match ch {
            '"' => in_string = true,
            ';' => {
                // skip to end of line — but char_indices doesn't let us skip,
                // so we just ignore ; in this simple scanner (module headers
                // shouldn't contain comments)
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[offset + i + 1..];
                }
            }
            _ => {}
        }
    }
    source
}
