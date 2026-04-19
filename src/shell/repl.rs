use moof::manifest::Manifest;
use moof::scheduler::Scheduler;
use moof::store::Store;
use std::path::Path;

/// Count unbalanced brackets/parens/braces.
fn bracket_depth(s: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut in_comment = false;
    for c in s.chars() {
        if c == '\n' { in_comment = false; continue; }
        if in_comment { continue; }
        if escape { escape = false; continue; }
        if c == '\\' && in_string { escape = true; continue; }
        if c == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        if c == ';' { in_comment = true; continue; }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

const MANIFEST_PATH: &str = "moof.toml";

pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    // load manifest (or use defaults)
    let manifest = match Path::new(MANIFEST_PATH).exists() {
        true => match Manifest::load(Path::new(MANIFEST_PATH)) {
            Ok(m) => { eprintln!("  manifest: {MANIFEST_PATH}"); m }
            Err(e) => { eprintln!("  ~ manifest error: {e}, using defaults"); Manifest::default() }
        },
        false => { eprintln!("  no manifest, using defaults"); Manifest::default() }
    };

    let mut sched = Scheduler::new(100_000);

    // vat 0: init vat (bare)
    let _init_vat_id = sched.spawn_bare_vat();

    // spawn capability vats from manifest
    let mut cap_refs: Vec<(String, u32, u32)> = Vec::new();
    for (name, spec) in &manifest.capabilities {
        if let Some(builtin_name) = Manifest::is_builtin(spec) {
            if let Some(cap) = moof::plugins::builtin_capability(builtin_name) {
                let (vat_id, obj_id) = sched.spawn_capability(cap.as_ref());
                cap_refs.push((name.clone(), vat_id, obj_id));
            } else {
                eprintln!("  ~ unknown builtin capability: {builtin_name}");
            }
        } else {
            // external plugin dylib
            match sched.load_plugin(Path::new(spec)) {
                Ok((loaded_name, vat_id, obj_id)) => {
                    cap_refs.push((name.clone(), vat_id, obj_id));
                    eprintln!("  loaded plugin '{loaded_name}'");
                }
                Err(e) => eprintln!("  ~ plugin error for '{name}': {e}"),
            }
        }
    }

    // spawn REPL vat — type plugins registered from manifest
    let repl_vat_id = sched.spawn_vat_with_manifest(&manifest);

    // load source files from manifest
    for source_path in &manifest.sources.files {
        if let Ok(source) = std::fs::read_to_string(source_path) {
            let vat = sched.vat_mut(repl_vat_id);
            match vat.eval_source(&source) {
                Ok(_) => eprintln!("  loaded {source_path}"),
                Err(e) => { eprintln!("  ~ error in {source_path}: {e}"); return; }
            }
        } else {
            eprintln!("  ~ source not found: {source_path}");
        }
    }

    // grant capabilities to REPL based on manifest [grants]
    let repl_grants = manifest.grants.get("repl")
        .cloned().unwrap_or_default();
    for (name, vat_id, obj_id) in &cap_refs {
        if repl_grants.contains(name) {
            let farref = sched.create_farref(repl_vat_id, *vat_id, *obj_id);
            let sym = sched.vat_mut(repl_vat_id).heap.intern(name);
            sched.vat_mut(repl_vat_id).heap.env_def(sym, farref);
        }
    }

    println!();

    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => { eprintln!("readline: {e}"); return; }
    };

    loop {
        let line = match rl.readline("\u{2728} ") {
            Ok(line) => { let _ = rl.add_history_entry(&line); line }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => { eprintln!("readline: {e}"); break; }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // REPL commands
        if trimmed == "(plugins)" {
            if sched.loaded_plugins.is_empty() && cap_refs.is_empty() {
                println!("  no capabilities loaded");
            } else {
                for (name, vat_id, _) in &cap_refs {
                    println!("  {name} (vat {vat_id})");
                }
            }
            continue;
        }
        if trimmed.starts_with("(reload ") && trimmed.ends_with(')') {
            let path_str = trimmed.strip_prefix("(reload ").unwrap()
                .strip_suffix(')').unwrap().trim().trim_matches('"');
            if let Ok(source) = std::fs::read_to_string(path_str) {
                let vat = sched.vat_mut(repl_vat_id);
                match vat.eval_source(&source) {
                    Ok(_) => eprintln!("  reloaded {path_str}"),
                    Err(e) => eprintln!("  ~ error in {path_str}: {e}"),
                }
            } else {
                eprintln!("  ~ file not found: {path_str}");
            }
            continue;
        }

        // accumulate multi-line input
        let mut input = trimmed.to_string();
        loop {
            let depth = bracket_depth(&input);
            if depth <= 0 { break; }
            let cont = match rl.readline("  ... ") {
                Ok(l) => l,
                Err(_) => break,
            };
            let _ = rl.add_history_entry(&cont);
            input.push('\n');
            input.push_str(&cont);
        }

        // eval
        let tokens = match moof::lang::lexer::tokenize(&input) {
            Ok(t) => t,
            Err(e) => { eprintln!("  ~ lex: {e}"); continue; }
        };

        let vat = sched.vat_mut(repl_vat_id);
        let mut parser = moof::lang::parser::Parser::new(&tokens, &mut vat.heap);
        let exprs = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => { eprintln!("  ~ parse: {e}"); continue; }
        };

        for expr in &exprs {
            let vat = sched.vat_mut(repl_vat_id);
            match moof::lang::compiler::Compiler::compile_toplevel(&vat.heap, *expr) {
                Ok(result) => {
                    match vat.vm.eval_result(&mut vat.heap, result) {
                        Ok(val) => {
                            sched.drain();
                            let vat = sched.vat_mut(repl_vat_id);
                            let show_sym = vat.heap.intern("show");
                            let displayed = match vat.vm.send_message(&mut vat.heap, val, show_sym, &[]) {
                                Ok(show_val) => {
                                    if let Some(id) = show_val.as_any_object() {
                                        if let moof::object::HeapObject::Text(s) = vat.heap.get(id) {
                                            s.clone()
                                        } else { vat.heap.display_value(val) }
                                    } else { vat.heap.display_value(val) }
                                }
                                Err(_) => vat.heap.display_value(val),
                            };
                            println!("  {displayed}");
                        }
                        Err(e) => eprintln!("  ~ {e}"),
                    }
                }
                Err(e) => eprintln!("  ~ compile: {e}"),
            }
        }

        // safepoint: frames are empty after the expression finishes.
        // if moof code requested a GC via [Vat requestGc], run it now.
        let vat = sched.vat_mut(repl_vat_id);
        if vat.heap.gc_requested {
            vat.heap.gc_requested = false;
            let stats = vat.heap.gc(&[]);
            eprintln!("  ~ gc: freed {} slots ({} live / {} total)",
                stats.freed, stats.live, stats.before);
        }
    }

    // save image
    let store_path = &manifest.image.path;
    if let Ok(store) = Store::open(Path::new(store_path)) {
        let vat = sched.vat(repl_vat_id);
        match store.save_all(
            vat.heap.objects_ref(),
            vat.heap.symbols_ref(),
            vat.heap.env,
            vat.vm.closure_descs_ref(),
        ) {
            Ok(()) => eprintln!("  image saved ({} objects)", vat.heap.object_count()),
            Err(e) => eprintln!("  ~ save failed: {e}"),
        }
    }

    println!("\n  the circle closes. moof.\n");
}
