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

    let mod_dir = image::modules_dir(&img_dir);
    let need_seed = seed_mode || !mod_dir.exists();

    if need_seed {
        // ── Seed from lib/ into .moof/modules/ ──
        let lib_dir = find_lib_dir();
        if !lib_dir.exists() {
            eprintln!("!! No image found and no lib/ directory to seed from");
            std::process::exit(1);
        }
        eprintln!("(seeding image from {})", lib_dir.display());
        match image::seed_from_directory(&lib_dir, &img_dir) {
            Ok(files) => eprintln!("(copied {} files)", files.len()),
            Err(e) => {
                eprintln!("!! Seed failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    // ── Discover and load modules from .moof/modules/ ──
    // Always re-discover from disk (manifest is rebuilt on save).
    // This is robust: stale manifests, missing manifests, external edits — all handled.
    {
        match modules::loader::ModuleLoader::discover(&mod_dir, &mut vm.heap) {
            Ok(mut loader) => {
                loader.image_dir = img_dir.clone();
                match load_modules_sequenced(&mut loader, &mut vm, root_env) {
                    Ok(()) => {
                        loader.merge_into_root(&mut vm, root_env);

                        // Populate Modules registry with loaded module info
                        populate_modules_registry(&loader, &mut vm, root_env);

                        // Set up workspace
                        setup_workspace(&mut loader, &mut vm, root_env);

                        // Save manifest (rebuild from current state)
                        match loader.save_image() {
                            Ok(hash) => {
                                let n = loader.graph.modules.len();
                                if need_seed {
                                    eprintln!("(seeded image: {} modules, hash {}...)", n, &hash[..12]);
                                } else {
                                    eprintln!("(loaded {} modules, hash {}...)", n, &hash[..12]);
                                }
                            }
                            Err(e) => eprintln!("!! manifest save: {}", e),
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

                // (define-in module-name def-source...)
                // e.g. (define-in collections (def my-fn (fn (x) [x + 1])))
                if input.starts_with("(define-in ") {
                    if let Some(ref mut loader) = module_loader {
                        match parse_define_in(&input) {
                            Ok((module_name, def_name, def_source)) => {
                                match loader.define_in(
                                    &module_name, &def_name, &def_source,
                                    &mut vm, root_env,
                                ) {
                                    Ok(val) => {
                                        let formatted = vm.format_value(val);
                                        println!("=> {}", formatted);
                                        eprintln!("(defined {} in {})", def_name, module_name);
                                    }
                                    Err(e) => eprintln!("!! {}", e),
                                }
                            }
                            Err(e) => eprintln!("!! {}", e),
                        }
                    }
                    continue;
                }

                // (module-create name (requires dep1 dep2))
                if input.starts_with("(module-create ") {
                    if let Some(ref mut loader) = module_loader {
                        match parse_module_create(&input, &mut vm.heap) {
                            Ok((name, requires, unrestricted)) => {
                                match loader.create_module(
                                    &name, &requires, unrestricted, &mut vm, root_env,
                                ) {
                                    Ok(()) => eprintln!("(created module: {})", name),
                                    Err(e) => eprintln!("!! {}", e),
                                }
                            }
                            Err(e) => eprintln!("!! {}", e),
                        }
                    }
                    continue;
                }

                // (which-module symbol-name)
                if input.starts_with("(which-module ") {
                    let sym_name = input[14..input.len()-1].trim();
                    if let Some(ref loader) = module_loader {
                        match loader.which_module(sym_name) {
                            Some(module_name) => eprintln!("  {} is defined in {}", sym_name, module_name),
                            None => eprintln!("  {} is not in any module", sym_name),
                        }
                    }
                    continue;
                }

                // ── Evaluate expression ──
                match eval_line(&mut vm, root_env, &input) {
                    Ok(val) => {
                        let formatted = vm.format_value(val);
                        println!("=> {}", formatted);

                        // Autosave (def ...) and (defmethod ...) forms to workspace
                        let trimmed_input = input.trim();
                        if trimmed_input.starts_with("(def ") || trimmed_input.starts_with("(defmethod ") {
                            if let Some(ref mut loader) = module_loader {
                                if loader.graph.modules.contains_key("workspace") {
                                    if let Ok(def_name) = extract_def_name(trimmed_input) {
                                        match loader.define_in("workspace", &def_name, trimmed_input, &mut vm, root_env) {
                                            Ok(_) => {}
                                            Err(e) => eprintln!("!! workspace autosave: {}", e),
                                        }
                                    }
                                }
                            }
                        }
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

/// Populate the Modules object with info about all loaded modules.
fn populate_modules_registry(
    loader: &modules::loader::ModuleLoader,
    vm: &mut VM,
    root_env: u32,
) {
    if let Ok(order) = loader.load_order() {
        for name in &order {
            if name == "modules" { continue; }

            let desc = match loader.graph.modules.get(name) {
                Some(d) => d,
                None => continue,
            };
            let env_id = loader.loaded_envs.get(name).copied().unwrap_or(0);

            // Build provides and requires as moof list expressions
            let provides_str = desc.provides.iter()
                .map(|p| format!("\"{}\"", p.replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(" ");
            let requires_str = desc.requires.iter()
                .map(|r| format!("\"{}\"", r))
                .collect::<Vec<_>>()
                .join(" ");

            // Register via eval — source text is placeholder, set directly after
            let expr = format!(
                "[Modules register: \"{}\" source: \"\" provides: (list {}) requires: (list {}) env: {}]",
                name.replace('"', "\\\""),
                provides_str,
                requires_str,
                env_id,
            );

            match eval_source(vm, root_env, &expr, "<register>") {
                Ok(module_val) => {
                    // Set the actual source text directly via heap slot
                    if let Value::Object(mod_id) = module_val {
                        if let Some(source) = loader.source_texts.get(name) {
                            let source_sym = vm.heap.intern("source");
                            let source_val = vm.heap.alloc_string(source);
                            vm.heap.set_slot(mod_id, source_sym, source_val);
                        }
                    }
                }
                Err(e) => eprintln!("!! register {}: {}", name, e),
            }
        }
    }
}

/// Set up the workspace module for REPL autosave.
fn setup_workspace(
    loader: &mut modules::loader::ModuleLoader,
    vm: &mut VM,
    root_env: u32,
) {
    // Check if workspace already exists
    if loader.graph.modules.contains_key("workspace") {
        return;
    }

    // If there's a workspace.moof file, it'll be loaded by the module system.
    // If not, we create one dynamically.
    let workspace_path = image::modules_dir(&image_dir())
        .join("workspace.moof");

    if !workspace_path.exists() {
        // Get all module names for the requires list
        let all_modules: Vec<String> = loader.graph.modules.keys()
            .filter(|n| n.as_str() != "workspace")
            .cloned()
            .collect();

        let requires_str = all_modules.join(" ");
        let source = format!(
            "(module workspace\n  (requires {})\n  (unrestricted)\n  (provides))\n\n; workspace — REPL definitions autosave here\n",
            requires_str
        );

        if let Err(e) = std::fs::write(&workspace_path, &source) {
            eprintln!("!! workspace create failed: {}", e);
            return;
        }
    }

    // Load the workspace module
    let workspace_source = match std::fs::read_to_string(&workspace_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("!! workspace read failed: {}", e); return; }
    };

    match modules::loader::parse_header(&workspace_source, &workspace_path, &mut vm.heap) {
        Ok(desc) => {
            // Add to graph (need to rebuild)
            let mut all_descs: Vec<modules::ModuleDescriptor> = loader.graph.modules.values().cloned().collect();
            all_descs.push(desc);
            match modules::graph::ModuleGraph::build(all_descs) {
                Ok(new_graph) => loader.graph = new_graph,
                Err(e) => { eprintln!("!! workspace graph: {}", e); return; }
            }

            // Load it
            if let Err(e) = loader.load_one("workspace", vm) {
                eprintln!("!! workspace load: {}", e);
                return;
            }

            // Merge workspace exports into root
            if let Some(exports) = loader.exports.get("workspace") {
                for (sym_name, val) in exports {
                    let sym = vm.heap.intern(sym_name);
                    vm.env_define_helper(root_env, sym, *val);
                }
            }
        }
        Err(e) => eprintln!("!! workspace parse: {}", e),
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

/// Parse (define-in module-name (def name ...)) or (define-in module-name (defmethod ...))
/// Returns (module_name, def_name, full_def_source)
fn parse_define_in(input: &str) -> Result<(String, String, String), String> {
    // (define-in MODULE_NAME REST...)
    // Strip outer parens: "define-in MODULE_NAME REST..."
    let inner = &input[1..input.len()-1]; // strip ( and )
    let inner = inner.strip_prefix("define-in ").ok_or("expected (define-in ...)")?;
    let inner = inner.trim_start();

    // Find module name (first token)
    let space = inner.find(|c: char| c.is_whitespace())
        .ok_or("expected module name and definition")?;
    let module_name = inner[..space].to_string();
    let def_source = inner[space..].trim().to_string();

    // Extract the defined name from the def source
    let def_name = extract_def_name(&def_source)?;

    Ok((module_name, def_name, def_source))
}

/// Extract the name being defined from a (def NAME ...) or (defmethod OBJ NAME ...) form.
fn extract_def_name(source: &str) -> Result<String, String> {
    let trimmed = source.trim();
    if trimmed.starts_with("(def ") && !trimmed.starts_with("(defmethod ") {
        // (def NAME ...)
        let after = &trimmed[5..];
        let end = after.find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(after.len());
        Ok(after[..end].to_string())
    } else if trimmed.starts_with("(defmethod ") {
        // (defmethod OBJ NAME ...)
        let after = &trimmed[11..];
        let first_space = after.find(|c: char| c.is_whitespace())
            .ok_or("expected (defmethod OBJ NAME ...)")?;
        let rest = after[first_space..].trim_start();
        let end = rest.find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(rest.len());
        Ok(rest[..end].to_string())
    } else if trimmed.starts_with("(handle! ") {
        // (handle! OBJ 'SELECTOR ...) — use the selector as the name
        // This is trickier; just use a generic name
        Ok("_handler_".to_string())
    } else {
        Err(format!("cannot extract name from: {}", &trimmed[..trimmed.len().min(40)]))
    }
}

/// Parse (module-create name (requires dep1 dep2))
fn parse_module_create(input: &str, heap: &mut crate::runtime::heap::Heap) -> Result<(String, Vec<String>, bool), String> {
    let inner = &input[1..input.len()-1]; // strip outer parens
    let inner = inner.strip_prefix("module-create ").ok_or("expected (module-create ...)")?;
    let inner = inner.trim_start();

    // Parse as a moof expression for clean handling
    let full = format!("(module-create {})", inner);
    let mut lexer = Lexer::new(&full);
    let tokens = lexer.tokenize().map_err(|e| format!("lex error: {}", e))?;
    let mut parser = Parser::new(tokens);
    let exprs = parser.parse_all(heap).map_err(|e| format!("parse error: {}", e))?;

    if exprs.is_empty() {
        return Err("empty module-create".into());
    }

    let elements = heap.list_to_vec(exprs[0]);
    // elements[0] = module-create, elements[1] = name, elements[2..] = clauses

    if elements.len() < 2 {
        return Err("(module-create) missing name".into());
    }

    let name = match elements[1] {
        Value::Symbol(sym) => heap.symbol_name(sym).to_string(),
        _ => return Err("module name must be a symbol".into()),
    };

    let mut requires = Vec::new();
    let mut unrestricted = false;

    for &element in &elements[2..] {
        if let Value::Symbol(sym) = element {
            if heap.symbol_name(sym) == "unrestricted" {
                unrestricted = true;
                continue;
            }
        }
        let sub = heap.list_to_vec(element);
        if sub.is_empty() { continue; }
        if let Value::Symbol(sym) = sub[0] {
            let kw = heap.symbol_name(sym).to_string();
            if kw == "requires" {
                for &item in &sub[1..] {
                    if let Value::Symbol(s) = item {
                        requires.push(heap.symbol_name(s).to_string());
                    }
                }
            }
        }
    }

    Ok((name, requires, unrestricted))
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
