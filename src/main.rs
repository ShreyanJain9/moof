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
use persistence::image;

fn image_dir() -> PathBuf {
    PathBuf::from(".moof")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gui_mode = args.iter().any(|a| a == "--gui");
    let mcp_mode = args.iter().any(|a| a == "--mcp");
    let seed_mode = args.iter().any(|a| a == "--seed");

    let mut vm = VM::new();
    let root_env = vm.heap.alloc_env(None);
    bootstrap_env(&mut vm, root_env);

    let root_sym = vm.heap.intern("*root-env*");
    vm.env_define_helper(root_env, root_sym, Value::Object(root_env));
    vm.root_env = Some(root_env);

    let img_dir = image_dir();
    let mut module_loader: Option<modules::loader::ModuleLoader> = None;

    if !seed_mode && image::image_exists(&img_dir) {
        // ── Load from image directory ──
        match image::load_manifest(&img_dir) {
            Ok(manifest) => {
                let mod_dir = image::modules_dir(&img_dir);
                match modules::loader::ModuleLoader::discover(&mod_dir, &mut vm.heap) {
                    Ok(mut loader) => {
                        loader.image_dir = img_dir.clone();
                        match load_modules_sequenced(&mut loader, &mut vm, root_env) {
                            Ok(()) => {
                                loader.merge_into_root(&mut vm, root_env);
                                let n = loader.graph.modules.len();
                                eprintln!("(image loaded: {} modules, hash {}...)",
                                    n, &manifest.global_hash[..12]);
                                module_loader = Some(loader);
                            }
                            Err(e) => {
                                eprintln!("!! Module loading failed: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("!! Module discovery failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("!! Manifest error: {}", e);
                eprintln!("!! Try --seed to re-initialize from lib/");
                std::process::exit(1);
            }
        }
    } else {
        // ── Seed from lib/ ──
        let lib_dir = find_lib_dir();
        if !lib_dir.exists() {
            eprintln!("!! No image found and no lib/ directory to seed from");
            std::process::exit(1);
        }

        eprintln!("(seeding image from {})", lib_dir.display());

        // Copy lib/ files into .moof/modules/
        match image::seed_from_directory(&lib_dir, &img_dir) {
            Ok(files) => eprintln!("(copied {} files)", files.len()),
            Err(e) => {
                eprintln!("!! Seed failed: {}", e);
                std::process::exit(1);
            }
        }

        // Now discover from the image directory
        let mod_dir = image::modules_dir(&img_dir);
        match modules::loader::ModuleLoader::discover(&mod_dir, &mut vm.heap) {
            Ok(mut loader) => {
                loader.image_dir = img_dir.clone();
                match load_modules_sequenced(&mut loader, &mut vm, root_env) {
                    Ok(()) => {
                        loader.merge_into_root(&mut vm, root_env);

                        // Auto-checkpoint after seed
                        match loader.save_image() {
                            Ok(hash) => eprintln!("(seeded image: {} modules, hash {}...)",
                                loader.graph.modules.len(), &hash[..12]),
                            Err(e) => eprintln!("!! Save failed: {}", e),
                        }

                        module_loader = Some(loader);
                    }
                    Err(e) => {
                        eprintln!("!! Module loading failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("!! Module discovery failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    if gui_mode {
        println!("MOOF — launching System Browser...");
        gui::browser::run_browser(vm, root_env);
        return;
    }

    if mcp_mode {
        match eval_source(&mut vm, root_env, "(mcp-serve)", "<mcp>") {
            Ok(_) => {}
            Err(e) => {
                eprintln!("!! MCP server failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    println!("MOOF — Moof Open Objectspace Fabric");
    println!("clarus the dogcow lives again");
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    // REPL
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
                // Save on exit
                if let Some(ref loader) = module_loader {
                    match loader.save_image() {
                        Ok(hash) => eprintln!("(saved, hash {}...)", &hash[..12]),
                        Err(e) => eprintln!("!! Save failed: {}", e),
                    }
                }
                println!("\nmoof.");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() && buffer.is_empty() { continue; }

                buffer.push_str(&line);

                if !brackets_balanced(&buffer) {
                    continue;
                }

                let input = buffer.trim().to_string();
                buffer.clear();

                if input.is_empty() { continue; }

                // ── REPL commands ──

                if input == "(checkpoint)" || input == "(save)" {
                    if let Some(ref loader) = module_loader {
                        match loader.save_image() {
                            Ok(hash) => eprintln!("(saved, hash {}...)", &hash[..12]),
                            Err(e) => eprintln!("!! {}", e),
                        }
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
                    }
                    continue;
                }

                if input.starts_with("(module-source ") {
                    let name = input[15..input.len()-1].trim();
                    if let Some(ref loader) = module_loader {
                        if let Some(source) = loader.source_texts.get(name) {
                            println!("{}", source);
                        } else {
                            eprintln!("!! unknown module: {}", name);
                        }
                    }
                    continue;
                }

                if input.starts_with("(module-edit ") {
                    let name = input[13..input.len()-1].trim().to_string();
                    if let Some(ref mut loader) = module_loader {
                        match handle_module_edit(&name, loader, &mut vm, root_env) {
                            Ok(()) => eprintln!("(reloaded {})", name),
                            Err(e) => eprintln!("!! {}", e),
                        }
                    }
                    continue;
                }

                if input.starts_with("(module-reload ") {
                    let name = input[15..input.len()-1].trim();
                    if let Some(ref mut loader) = module_loader {
                        // Re-read source from disk (in case externally edited)
                        let mod_path = image::modules_dir(&img_dir)
                            .join(format!("{}.moof", name));
                        if let Ok(new_source) = std::fs::read_to_string(&mod_path) {
                            loader.source_texts.insert(name.to_string(), new_source);
                        }
                        match loader.reload(name, &mut vm, root_env) {
                            Ok(()) => eprintln!("(reloaded {})", name),
                            Err(e) => eprintln!("!! {}", e),
                        }
                    }
                    continue;
                }

                if input.starts_with("(module-remove ") {
                    let name = input[15..input.len()-1].trim();
                    if let Some(ref mut loader) = module_loader {
                        match loader.remove(name) {
                            Ok(removed) => {
                                for sym_name in &removed {
                                    let sym = vm.heap.intern(sym_name);
                                    vm.env_define_helper(root_env, sym, Value::Nil);
                                }
                                eprintln!("(removed {} — unbound {} symbols)", name, removed.len());
                            }
                            Err(e) => eprintln!("!! {}", e),
                        }
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
                    }
                    continue;
                }

                if input == "(export-modules)" || input.starts_with("(export-modules ") {
                    let target = if input.starts_with("(export-modules ") {
                        PathBuf::from(input[16..input.len()-1].trim())
                    } else {
                        PathBuf::from("lib")
                    };
                    match image::export_to_directory(&img_dir, &target) {
                        Ok(n) => eprintln!("(exported {} modules to {})", n, target.display()),
                        Err(e) => eprintln!("!! {}", e),
                    }
                    continue;
                }

                if input == "(import-modules)" || input.starts_with("(import-modules ") {
                    let source_dir = if input.starts_with("(import-modules ") {
                        PathBuf::from(input[16..input.len()-1].trim())
                    } else {
                        find_lib_dir()
                    };
                    match image::seed_from_directory(&source_dir, &img_dir) {
                        Ok(files) => {
                            eprintln!("(imported {} files — restart to reload)", files.len());
                        }
                        Err(e) => eprintln!("!! {}", e),
                    }
                    continue;
                }

                // ── Evaluate expression ──
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

/// Handle (module-edit name) — open $EDITOR, re-evaluate on save.
fn handle_module_edit(
    name: &str,
    loader: &mut modules::loader::ModuleLoader,
    vm: &mut VM,
    root_env: u32,
) -> Result<(), String> {
    let source = loader.source_texts.get(name)
        .ok_or_else(|| format!("unknown module: {}", name))?
        .clone();

    // Write to temp file
    let tmp_path = std::env::temp_dir().join(format!("{}.moof", name));
    std::fs::write(&tmp_path, &source)
        .map_err(|e| format!("cannot write temp file: {}", e))?;

    // Open $EDITOR
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new(&editor)
        .arg(&tmp_path)
        .status()
        .map_err(|e| format!("cannot open editor '{}': {}", editor, e))?;

    if !status.success() {
        return Err("editor exited with error".into());
    }

    // Read back
    let new_source = std::fs::read_to_string(&tmp_path)
        .map_err(|e| format!("cannot read temp file: {}", e))?;
    let _ = std::fs::remove_file(&tmp_path);

    if new_source == source {
        return Err("no changes".into());
    }

    // Update module source, re-parse header, re-evaluate
    loader.update_module_source(name, new_source, vm, root_env)
        .map_err(|e| format!("reload failed: {}", e))?;

    // Save the updated module to disk
    loader.save_module(name)
        .map_err(|e| format!("save failed: {}", e))?;

    Ok(())
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

fn eval_line(vm: &mut VM, env_id: u32, input: &str) -> Result<Value, String> {
    eval_source(vm, env_id, input, "<repl>")
}

pub fn eval_source(vm: &mut VM, env_id: u32, input: &str, source_name: &str) -> Result<Value, String> {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize()
        .map_err(|e| format!("{}: lex error: {}", source_name, e))?;

    let mut parser = Parser::new(tokens);
    let exprs = parser.parse_all(&mut vm.heap)
        .map_err(|e| format!("{}: parse error: {}", source_name, e))?;

    if exprs.is_empty() {
        return Ok(Value::Nil);
    }

    let mut result = Value::Nil;
    for expr in exprs {
        let mut compiler = Compiler::new();
        let chunk = compiler.compile_expr(&mut vm.heap, expr)?;
        let chunk_id = vm.heap.alloc_chunk(chunk);
        result = vm.execute(chunk_id, env_id)?;
    }
    Ok(result)
}

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
    pub fn env_define_helper(&mut self, env_id: u32, sym: u32, val: Value) {
        self.heap.env_define(env_id, sym, val);
    }
}

fn register_type_prototypes(vm: &mut VM, env_id: u32) {
    vm::natives::register_all_natives(vm, env_id);
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

/// Find the lib/ directory for seeding.
fn find_lib_dir() -> PathBuf {
    let cwd_path = PathBuf::from("lib");
    if cwd_path.exists() {
        return cwd_path;
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap();
        let exe_path = exe_dir.join("lib");
        if exe_path.exists() { return exe_path; }
        let parent_path = exe_dir.join("../../lib");
        if parent_path.exists() { return parent_path; }
    }
    cwd_path
}
