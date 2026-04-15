use moof::scheduler::Scheduler;
use moof::store::Store;
use std::path::Path;

/// Count unbalanced brackets/parens/braces across the entire input.
/// Positive = more openers than closers (need more input).
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

const STORE_PATH: &str = ".moof/store";

pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    // create the scheduler
    let mut sched = Scheduler::new(100_000);

    // vat 0: init vat (the rust runtime — bare, no bootstrap)
    let _init_vat_id = sched.spawn_bare_vat();

    // spawn capability vats and collect their FarRef info
    let capabilities = moof::plugins::default_capabilities();
    let mut cap_refs: Vec<(String, u32, u32)> = Vec::new();
    for cap in &capabilities {
        let (vat_id, obj_id) = sched.spawn_capability(cap.as_ref());
        cap_refs.push((cap.name().to_string(), vat_id, obj_id));
    }

    // the REPL vat (just a regular vat with bootstrap)
    let repl_vat_id = sched.spawn_vat();

    // give the REPL far references to all capabilities
    for (name, vat_id, obj_id) in &cap_refs {
        let farref = sched.create_farref(repl_vat_id, *vat_id, *obj_id);
        let sym = sched.vat_mut(repl_vat_id).heap.intern(name);
        sched.vat_mut(repl_vat_id).heap.env_def(sym, farref);
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
        if trimmed.starts_with("(load-plugin ") && trimmed.ends_with(')') {
            let path_str = trimmed.strip_prefix("(load-plugin ").unwrap()
                .strip_suffix(')').unwrap().trim().trim_matches('"');
            match sched.load_plugin(std::path::Path::new(path_str)) {
                Ok((name, vat_id, obj_id)) => {
                    let farref = sched.create_farref(repl_vat_id, vat_id, obj_id);
                    let sym = sched.vat_mut(repl_vat_id).heap.intern(&name);
                    sched.vat_mut(repl_vat_id).heap.env_def(sym, farref);
                    eprintln!("  loaded plugin '{name}' (vat {vat_id})");
                }
                Err(e) => eprintln!("  ~ plugin error: {e}"),
            }
            continue;
        }

        // accumulate multi-line input when brackets are unbalanced
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

        // eval in the REPL vat, then drain all pending cross-vat work
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
                            // drain cross-vat work after each expression
                            sched.drain();

                            let vat = sched.vat_mut(repl_vat_id);
                            // display: try [val show], fall back to display_value
                            let show_sym = vat.heap.intern("show");
                            let displayed = match vat.vm.send_message(&mut vat.heap, val, show_sym, &[]) {
                                Ok(show_val) => {
                                    if let Some(id) = show_val.as_any_object() {
                                        if let moof::object::HeapObject::Text(s) = vat.heap.get(id) {
                                            s.clone()
                                        } else {
                                            vat.heap.display_value(val)
                                        }
                                    } else {
                                        vat.heap.display_value(val)
                                    }
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
    }

    // save to LMDB
    if let Ok(store) = Store::open(Path::new(STORE_PATH)) {
        let vat = sched.vat(repl_vat_id);
        match store.save_all(
            vat.heap.objects_ref(),
            vat.heap.symbols_ref(),
            vat.heap.env,
            vat.vm.closure_descs_ref(),
        ) {
            Ok(()) => eprintln!("  image saved to LMDB ({} objects)", vat.heap.object_count()),
            Err(e) => eprintln!("  ~ save failed: {e}"),
        }
    }

    println!("\n  the circle closes. moof.\n");
}
