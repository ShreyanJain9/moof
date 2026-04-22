// REPL — an Interface. One peer consumer of System among many.
//
// The repl owns nothing structural: it doesn't spawn vats, it doesn't
// hold capabilities, it doesn't orchestrate boot. System hands it a
// vat id; the repl types in, parses, compiles, eval's in that vat,
// prints the result. That's it.
//
// The phase-1 path: this whole file moves to moof code (or a plugin)
// once vat 0's System defserver can drive it via Acts.

use moof::system::{Interface, System};
use moof::manifest::Manifest;
use std::path::Path;

const MANIFEST_PATH: &str = "moof.toml";

pub fn run() {
    println!();
    println!("  .  *  .        m o o f        .  *  .");
    println!("       ~ a living objectspace ~");
    println!("    clarus the dogcow lives again");

    let manifest = load_manifest();
    let mut sys = System::boot(manifest);
    let mut repl = ReplInterface::new();
    sys.run(&mut repl);
    println!("\n  the circle closes. moof.\n");
}

fn load_manifest() -> Manifest {
    match Path::new(MANIFEST_PATH).exists() {
        true => match Manifest::load(Path::new(MANIFEST_PATH)) {
            Ok(m) => { eprintln!("  manifest: {MANIFEST_PATH}"); m }
            Err(e) => { eprintln!("  ~ manifest error: {e}, using defaults"); Manifest::default() }
        },
        false => { eprintln!("  no manifest, using defaults"); Manifest::default() }
    }
}

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

pub struct ReplInterface;

impl ReplInterface {
    pub fn new() -> Self { ReplInterface }
}

impl Default for ReplInterface {
    fn default() -> Self { Self::new() }
}

impl Interface for ReplInterface {
    fn name(&self) -> &str { "repl" }

    fn required_caps(&self) -> Vec<&str> {
        // the repl would like all four stdlib caps; System filters
        // against the manifest's [grants.repl] list, so any of these
        // that aren't in the manifest are silently dropped.
        vec!["console", "clock", "file", "random"]
    }

    fn run(&mut self, sys: &mut System, vat_id: u32) -> i32 {
        println!();
        let Ok(mut rl) = rustyline::DefaultEditor::new() else {
            eprintln!("  ~ readline init failed");
            return 1;
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

            if handle_command(trimmed, sys, vat_id) { continue; }

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

            eval_and_print(sys, vat_id, &input);
            safepoint_gc(sys, vat_id);
        }
        0
    }
}

/// REPL meta-commands. Returns true if the line was a command.
fn handle_command(trimmed: &str, sys: &mut System, vat_id: u32) -> bool {
    if trimmed == "(plugins)" {
        let caps: Vec<&str> = sys.capability_names();
        if caps.is_empty() {
            println!("  no capabilities loaded");
        } else {
            for name in &caps { println!("  {name}"); }
        }
        return true;
    }
    if trimmed == "(vats)" {
        println!("  user vats spawned by System:");
        for (id, caps) in sys.user_vats() {
            println!("    vat {id} — caps: [{}]", caps.join(", "));
        }
        return true;
    }
    if trimmed.starts_with("(reload ") && trimmed.ends_with(')') {
        let path_str = trimmed.strip_prefix("(reload ").unwrap()
            .strip_suffix(')').unwrap().trim().trim_matches('"');
        match std::fs::read_to_string(path_str) {
            Ok(source) => match sys.eval(vat_id, &source) {
                Ok(_) => eprintln!("  reloaded {path_str}"),
                Err(e) => eprintln!("  ~ error in {path_str}: {e}"),
            },
            Err(_) => eprintln!("  ~ file not found: {path_str}"),
        }
        return true;
    }
    false
}

fn eval_and_print(sys: &mut System, vat_id: u32, input: &str) {
    let vat = sys.vat_mut(vat_id);

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
        let vat = sys.vat_mut(vat_id);
        match moof_lang::lang::compiler::Compiler::compile_toplevel(&vat.heap, *expr) {
            Ok(result) => match vat.vm.eval_result(&mut vat.heap, result) {
                Ok(val) => {
                    sys.drain();
                    let vat = sys.vat_mut(vat_id);
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

fn safepoint_gc(sys: &mut System, vat_id: u32) {
    let vat = sys.vat_mut(vat_id);
    if !vat.heap.gc_requested { return; }

    let extra: Vec<moof_core::Value> = vat.vm.closure_descs_ref().iter()
        .flat_map(|d| d.chunk.constants.iter().map(|b| moof_core::Value::from_bits(*b)))
        .collect();
    vat.heap.gc_requested = false;
    let stats = vat.heap.gc(&extra);
    eprintln!("  ~ gc: freed {} slots ({} live / {} total)",
        stats.freed, stats.live, stats.before);
}
