// The REPL: one interface among several, not a privileged layer.
//
// Boot orchestration lives in `crate::boot`. The REPL does one thing:
// gives the user an interactive moof prompt. A --script runner is a
// sibling that lives in `crate::shell::script`. Any other interface
// (a network socket, a tui, a morph-backed editor) should be a
// sibling too — never a privileged fork of this module.

use moof::boot::BootedSystem;
use moof::manifest::Manifest;
use moof::store::Store;
use std::path::Path;

/// Count unbalanced brackets/parens/braces for multi-line input.
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

/// Run the REPL. Boots the system using the manifest, spawns a vat
/// for the user to type into, grants it the [grants] repl caps,
/// then loops on readline. On exit, saves the image.
pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    let manifest = load_manifest();
    let mut sys = BootedSystem::boot(manifest);
    let grants = sys.manifest.grants.get("repl").cloned().unwrap_or_default();
    let repl_vat_id = sys.spawn_with_caps(&grants);

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

        if handle_repl_command(trimmed, &mut sys, repl_vat_id) { continue; }

        // accumulate multi-line input
        let mut input = trimmed.to_string();
        loop {
            if bracket_depth(&input) <= 0 { break; }
            let cont = match rl.readline("  ... ") {
                Ok(l) => l,
                Err(_) => break,
            };
            let _ = rl.add_history_entry(&cont);
            input.push('\n');
            input.push_str(&cont);
        }

        eval_and_print(&mut sys, repl_vat_id, &input);
        safepoint_gc(&mut sys, repl_vat_id);
    }

    // save image at exit
    save_on_exit(&sys, repl_vat_id);
    println!("\n  the circle closes. moof.\n");
}

/// Load the manifest, falling back to defaults if it's missing.
fn load_manifest() -> Manifest {
    match Path::new(MANIFEST_PATH).exists() {
        true => match Manifest::load(Path::new(MANIFEST_PATH)) {
            Ok(m) => { eprintln!("  manifest: {MANIFEST_PATH}"); m }
            Err(e) => { eprintln!("  ~ manifest error: {e}, using defaults"); Manifest::default() }
        },
        false => { eprintln!("  no manifest, using defaults"); Manifest::default() }
    }
}

/// Handle REPL meta-commands like `(plugins)` and `(reload ...)`.
/// Returns true if the line was a command (skip normal eval).
fn handle_repl_command(trimmed: &str, sys: &mut BootedSystem, repl_vat_id: u32) -> bool {
    if trimmed == "(plugins)" {
        if sys.scheduler.loaded_plugins.is_empty() && sys.cap_refs.is_empty() {
            println!("  no capabilities loaded");
        } else {
            for cap in &sys.cap_refs {
                println!("  {} (vat {})", cap.name, cap.vat_id);
            }
        }
        return true;
    }
    if trimmed.starts_with("(reload ") && trimmed.ends_with(')') {
        let path_str = trimmed.strip_prefix("(reload ").unwrap()
            .strip_suffix(')').unwrap().trim().trim_matches('"');
        match std::fs::read_to_string(path_str) {
            Ok(source) => {
                match sys.eval(repl_vat_id, &source) {
                    Ok(_) => eprintln!("  reloaded {path_str}"),
                    Err(e) => eprintln!("  ~ error in {path_str}: {e}"),
                }
            }
            Err(_) => eprintln!("  ~ file not found: {path_str}"),
        }
        return true;
    }
    false
}

/// Parse, compile, evaluate, and print. Uses `show` to render the
/// result when available; falls back to the heap's display form.
fn eval_and_print(sys: &mut BootedSystem, repl_vat_id: u32, input: &str) {
    let vat = sys.scheduler.vat_mut(repl_vat_id);

    let tokens = match moof_lang::lang::lexer::tokenize(input) {
        Ok(t) => t,
        Err(e) => { eprintln!("  ~ lex: {e}"); return; }
    };
    let mut parser = moof_lang::lang::parser::Parser::new(&tokens, &mut vat.heap);
    let exprs = match parser.parse_all() {
        Ok(e) => e,
        Err(e) => { eprintln!("  ~ parse: {e}"); return; }
    };

    for expr in &exprs {
        let vat = sys.scheduler.vat_mut(repl_vat_id);
        match moof_lang::lang::compiler::Compiler::compile_toplevel(&vat.heap, *expr) {
            Ok(result) => match vat.vm.eval_result(&mut vat.heap, result) {
                Ok(val) => {
                    sys.drain();
                    let vat = sys.scheduler.vat_mut(repl_vat_id);
                    let show_sym = vat.heap.intern("show");
                    let displayed = match vat.vm.send_message(&mut vat.heap, val, show_sym, &[]) {
                        Ok(show_val) => match show_val.as_any_object().and_then(|id| vat.heap.get_string(id)) {
                            Some(s) => s.to_string(),
                            None => vat.heap.display_value(val),
                        },
                        Err(_) => vat.heap.display_value(val),
                    };
                    println!("  {displayed}");
                }
                Err(e) => eprintln!("  ~ {e}"),
            },
            Err(e) => eprintln!("  ~ compile: {e}"),
        }
    }
}

/// After an expression finishes, if moof code requested a GC,
/// collect now with VM roots + closure-desc constants included.
fn safepoint_gc(sys: &mut BootedSystem, repl_vat_id: u32) {
    let vat = sys.scheduler.vat_mut(repl_vat_id);
    if !vat.heap.gc_requested { return; }

    let extra: Vec<moof_core::Value> = vat.vm.closure_descs_ref().iter()
        .flat_map(|d| d.chunk.constants.iter().map(|b| moof_core::Value::from_bits(*b)))
        .collect();
    vat.heap.gc_requested = false;
    let stats = vat.heap.gc(&extra);
    eprintln!("  ~ gc: freed {} slots ({} live / {} total)",
        stats.freed, stats.live, stats.before);
}

fn save_on_exit(sys: &BootedSystem, repl_vat_id: u32) {
    let store_path = &sys.manifest.image.path;
    if let Ok(store) = Store::open(Path::new(store_path)) {
        let vat = sys.scheduler.vat(repl_vat_id);
        match store.save_all(&vat.heap, vat.vm.closure_descs_ref()) {
            Ok(()) => eprintln!("  image saved ({} objects)", vat.heap.object_count()),
            Err(e) => eprintln!("  ~ save failed: {e}"),
        }
    }
}
