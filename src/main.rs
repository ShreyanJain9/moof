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
use persistence::{image, snapshot};

fn image_dir() -> PathBuf {
    PathBuf::from(".moof")
}

fn image_bin_path() -> PathBuf {
    image_dir().join("image.bin")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gui_mode = args.iter().any(|a| a == "--gui");
    let mcp_mode = args.iter().any(|a| a == "--mcp");
    let seed_mode = args.iter().any(|a| a == "--seed");

    let mut vm = VM::new();
    let mut root_env: u32;
    let mut module_loader: Option<modules::loader::ModuleLoader> = None;
    let img_dir = image_dir();
    let bin_path = image_bin_path();

    let mut loaded_from_image = false;

    if !seed_mode && bin_path.exists() {
        match snapshot::load_image(&bin_path) {
            Ok((heap, re, protos)) => {
                vm.heap = heap;
                root_env = re;
                vm.root_env = Some(root_env);
                vm.set_protos(protos);
                // RE-REGISTER NATIVES: closures are not in the image
                register_type_prototypes(&mut vm, root_env);
                // Ensure root-env is bound (may not be in old images)
                let root_sym = vm.heap.intern("root-env");
                vm.env_define_helper(root_env, root_sym, Value::Object(root_env));
                eprintln!("(resumed from image: {} objects, {} symbols)", vm.heap.len(), vm.heap.symbol_count());
                loaded_from_image = true;
            }
            Err(e) => {
                eprintln!("!! Image load failed: {}. Falling back to source load.", e);
                root_env = vm.heap.alloc_env(None);
                bootstrap_env(&mut vm, root_env);
            }
        }
    } else {
        root_env = vm.heap.alloc_env(None);
        bootstrap_env(&mut vm, root_env);
    }

    if !loaded_from_image {
        let root_sym = vm.heap.intern("root-env");
        vm.env_define_helper(root_env, root_sym, Value::Object(root_env));
        vm.root_env = Some(root_env);
    }

    let mod_dir = image::modules_dir(&img_dir);
    let need_seed = seed_mode || (!mod_dir.exists() && !loaded_from_image);

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

    // ── Load modules ──
    if loaded_from_image {
        // Image resume: all ModuleImage + Definition objects are in the heap.
        // The Modules registry is in the heap. No re-evaluation needed.
        // We just need a ModuleLoader with the image_dir for save operations.
        let loader = modules::loader::ModuleLoader::from_image_dir(img_dir.clone());
        module_loader = Some(loader);
    } else {
        // Source load: discover .moof files, parse, compile, eval, register on heap.
        match modules::loader::ModuleLoader::discover(&mod_dir, &mut vm.heap) {
            Ok(mut loader) => {
                loader.image_dir = img_dir.clone();

                match load_modules_sequenced(&mut loader, &mut vm, root_env) {
                    Ok(()) => {
                        loader.merge_into_root(&mut vm, root_env);
                        setup_workspace(&mut loader, &mut vm, root_env);

                        match loader.save_image(&vm) {
                            Ok(hash) => {
                                let n = loader.graph.modules.len();
                                eprintln!("(loaded {} modules, hash {}...)", n, &hash[..12]);
                            }
                            Err(e) => eprintln!("!! manifest save: {}", e),
                        }

                        // Save binary image
                        if let Err(e) = snapshot::save_image(&bin_path, &vm.heap, root_env, vm.get_protos()) {
                            eprintln!("!! image.bin save failed: {}", e);
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
                    let _ = loader.save_image(&vm);
                    let _ = snapshot::save_image(&bin_path, &vm.heap, root_env, vm.get_protos());
                    eprintln!("(saved image)");
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

                // checkpoint/save is now a moof function in system.moof

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

                // Module system commands (most moved to system.moof)
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
                        match loader.remove(name, &mut vm) {
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

                // define-in and module-create are now moof functions in system.moof

                // ── Evaluate expression ──
                match eval_line(&mut vm, root_env, &input) {
                    Ok(val) => {
                        let formatted = vm.format_value(val);
                        println!("=> {}", formatted);

                        // Autosave (def ...) forms to workspace via moof define-in
                        let trimmed_input = input.trim();
                        if trimmed_input.starts_with("(def ") || trimmed_input.starts_with("(defmethod ") {
                            if let Ok(def_name) = extract_def_name(trimmed_input) {
                                let escaped = trimmed_input.replace('\\', "\\\\").replace('"', "\\\"");
                                let autosave_expr = format!(
                                    "(define-in workspace {} \"{}\")",
                                    def_name, escaped
                                );
                                let _ = eval_source(&mut vm, root_env, &autosave_expr, "<autosave>");
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
    loader.save_module(name, vm)
        .map_err(|e| format!("save failed: {}", e))?;

    Ok(())
}

/// Populate the Modules object with info about all loaded modules.
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
            // Merge bootstrap exports into root so that ModuleImage, Definition,
            // Object, etc. are available during subsequent module loads.
            if let Some(exports) = loader.exports.get("bootstrap") {
                for (sym_name, val) in exports {
                    let sym = vm.heap.intern(sym_name);
                    vm.env_define_helper(root_env, sym, *val);
                }
            }
            register_type_prototypes(vm, root_env);

            // Create a stub Modules object with a simple list-based registry.
            // This lets register_module_on_heap register ModuleImage objects
            // before modules.moof defines the real Modules with an Assoc.
            create_stub_modules(vm, root_env);

            // Now re-register bootstrap itself on the heap (it was loaded
            // before ModuleImage/Modules existed).
            if let Some(desc) = loader.graph.modules.get("bootstrap") {
                if let Some(source) = loader.source_texts.get("bootstrap") {
                    let env_id = loader.loaded_envs.get("bootstrap").copied().unwrap_or(0);
                    loader.register_module_on_heap("bootstrap", desc, source, env_id, vm);
                }
            }
        }

        if name == "modules" {
            // modules.moof just loaded — it defines the real Modules object
            // with an Assoc-backed _registry. Merge its exports into root.
            if let Some(exports) = loader.exports.get("modules") {
                for (sym_name, val) in exports {
                    let sym = vm.heap.intern(sym_name);
                    vm.env_define_helper(root_env, sym, *val);
                }
            }

            // Migrate: collect all ModuleImage objects that were registered
            // in the stub, and re-register them in the real Modules.
            migrate_stub_to_real_modules(vm, root_env);
        }
    }

    Ok(())
}

/// Create a stub Modules singleton with a list-backed registry.
/// This exists only until modules.moof loads and defines the real Modules.
fn create_stub_modules(vm: &mut VM, root_env: u32) {
    // Create a GeneralObject to serve as the stub registry.
    // It stores entries as a cons-list of (name_string . mod_id) pairs
    // in an "entries" slot, mimicking the Assoc interface just enough
    // for register_module_on_heap to work.
    let entries_sym = vm.heap.intern("entries");
    let size_sym = vm.heap.intern("size");

    let registry_id = vm.heap.alloc(HeapObject::GeneralObject {
        parent: Value::Nil,
        slots: vec![
            (entries_sym, Value::Nil),
            (size_sym, Value::Integer(0)),
        ],
        handlers: Vec::new(),
    });

    // Handler lambdas are called with (self, ...args).
    // We use [self slotAt: ...] instead of @ since these are standalone lambdas.
    let set_to_sym = vm.heap.intern("set:to:");
    let set_source = "(fn (self key val) (do [self slotAt: 'entries put: (cons (cons key val) [self slotAt: 'entries])] [self slotAt: 'size put: [[self slotAt: 'size] + 1]] val))";
    match eval_source(vm, root_env, set_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(registry_id, set_to_sym, handler_val);
        }
        Err(e) => eprintln!("!! stub registry set:to: failed: {}", e),
    }

    let get_sym = vm.heap.intern("get:");
    let get_source = "(fn (self key) (let ((found (find (fn (pair) [[(car pair)] = key]) [self slotAt: 'entries]))) (if (null? found) nil (cdr found))))";
    match eval_source(vm, root_env, get_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(registry_id, get_sym, handler_val);
        }
        Err(e) => eprintln!("!! stub registry get: failed: {}", e),
    }

    let keys_sym = vm.heap.intern("keys");
    let keys_source = "(fn (self) (map (fn (pair) (car pair)) [self slotAt: 'entries]))";
    match eval_source(vm, root_env, keys_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(registry_id, keys_sym, handler_val);
        }
        Err(e) => eprintln!("!! stub registry keys failed: {}", e),
    }

    let values_sym = vm.heap.intern("values");
    let values_source = "(fn (self) (map (fn (pair) (cdr pair)) [self slotAt: 'entries]))";
    match eval_source(vm, root_env, values_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(registry_id, values_sym, handler_val);
        }
        Err(e) => eprintln!("!! stub registry values failed: {}", e),
    }

    // Create the stub Modules object
    let registry_slot_sym = vm.heap.intern("_registry");
    let modules_id = vm.heap.alloc(HeapObject::GeneralObject {
        parent: Value::Nil,
        slots: vec![
            (registry_slot_sym, Value::Object(registry_id)),
        ],
        handlers: Vec::new(),
    });

    // Add handlers that delegate to registry
    let list_sym = vm.heap.intern("list");
    let list_source = "(fn (self) [[self slotAt: '_registry] keys])";
    match eval_source(vm, root_env, list_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(modules_id, list_sym, handler_val);
        }
        Err(_) => {}
    }

    let named_sym = vm.heap.intern("named:");
    let named_source = "(fn (self name) [[self slotAt: '_registry] get: name])";
    match eval_source(vm, root_env, named_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(modules_id, named_sym, handler_val);
        }
        Err(_) => {}
    }

    let describe_sym = vm.heap.intern("describe");
    let describe_source = "(fn (self) (str \"<Modules (stub) \" [[self slotAt: '_registry] slotAt: 'size] \" loaded>\"))";
    match eval_source(vm, root_env, describe_source, "<stub>") {
        Ok(handler_val) => {
            vm.heap.add_handler(modules_id, describe_sym, handler_val);
        }
        Err(_) => {}
    }

    // Bind Modules in root env
    let modules_sym = vm.heap.intern("Modules");
    vm.env_define_helper(root_env, modules_sym, Value::Object(modules_id));

    // Also bind __stub_modules so migration can find the old entries
    let stub_sym = vm.heap.intern("__stub_modules");
    vm.env_define_helper(root_env, stub_sym, Value::Object(modules_id));
}

/// Migrate ModuleImage objects from the stub Modules to the real Modules.
/// Called after modules.moof loads and defines the real Modules with Assoc.
fn migrate_stub_to_real_modules(vm: &mut VM, root_env: u32) {
    // Read entries from the stub
    let stub_sym = vm.heap.intern("__stub_modules");
    let stub_obj = match vm.env_lookup(root_env, stub_sym) {
        Ok(Value::Object(id)) => id,
        _ => return,
    };

    let registry_val = vm.read_slot(stub_obj, "_registry");
    let entries_val = match registry_val {
        Value::Object(reg_id) => vm.read_slot(reg_id, "entries"),
        _ => return,
    };

    // The real Modules is now bound in root_env
    let modules_sym = vm.heap.intern("Modules");
    let real_modules = match vm.env_lookup(root_env, modules_sym) {
        Ok(Value::Object(id)) if id != stub_obj => id,
        _ => return,
    };

    let real_registry_val = vm.read_slot(real_modules, "_registry");
    let real_registry_id = match real_registry_val {
        Value::Object(id) => id,
        _ => return,
    };

    // Walk the stub entries and re-register each in the real Modules
    let entries = vm.heap.list_to_vec(entries_val);
    let set_to_sym = vm.heap.intern("set:to:");
    for entry in entries {
        if let Value::Object(pair_id) = entry {
            if let HeapObject::Cons { car, cdr } = vm.heap.get(pair_id).clone() {
                let _ = vm.message_send(
                    Value::Object(real_registry_id),
                    set_to_sym,
                    &[car, cdr],
                );
            }
        }
    }
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
