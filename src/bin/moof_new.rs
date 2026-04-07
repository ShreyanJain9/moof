/// moof — the new binary on the fabric substrate.
///
/// Creates a fabric, registers the moof language shell, boots from
/// lib/ sources or the image, runs the REPL.

use std::io::{self, Write, BufRead};
use std::path::PathBuf;
use moof_fabric::{Fabric, Value};

fn main() {
    let mut fabric = Fabric::new();
    let root_env = moof_lang::setup(&mut fabric);

    // Try loading bootstrap from lib/
    let bootstrap_path = PathBuf::from("lib/bootstrap.moof");
    if bootstrap_path.exists() {
        match std::fs::read_to_string(&bootstrap_path) {
            Ok(source) => {
                // Skip the module header — find the first blank line after it
                let body = skip_module_header(&source);
                match moof_lang::eval(&mut fabric, body, root_env) {
                    Ok(_) => eprintln!("(loaded bootstrap)"),
                    Err(e) => {
                        eprintln!("!! bootstrap failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => eprintln!("!! cannot read bootstrap: {}", e),
        }
    } else {
        eprintln!("(no lib/bootstrap.moof — running with bare fabric)");
    }

    // Bind some useful native functions
    bind_io_natives(&mut fabric, root_env);

    println!("MOOF — on the fabric");
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
                println!("\nmoof.");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() && buffer.is_empty() { continue; }

                buffer.push_str(&line);

                if !brackets_balanced(&buffer) { continue; }

                let input = buffer.trim().to_string();
                buffer.clear();

                if input.is_empty() { continue; }

                match moof_lang::eval(&mut fabric, &input, root_env) {
                    Ok(val) => {
                        let formatted = format_value(&fabric, val);
                        println!("=> {}", formatted);
                    }
                    Err(e) => println!("!! {}", e),
                }
            }
            Err(e) => {
                println!("!! Read error: {}", e);
                break;
            }
        }
    }
}

fn skip_module_header(source: &str) -> &str {
    // Find the end of the (module ...) form
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in source.char_indices() {
        if escape { escape = false; continue; }
        if in_string {
            if ch == '\\' { escape = true; }
            else if ch == '"' { in_string = false; }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    // Skip past this form + any trailing whitespace
                    let rest = &source[i + 1..];
                    return rest.trim_start();
                }
            }
            _ => {}
        }
    }
    source // no header found, use whole source
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

fn format_value(fabric: &Fabric, val: Value) -> String {
    match val {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::False => "false".into(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'{}", fabric.symbol_name(id)),
        Value::Object(id) => {
            use moof_fabric::HeapObject;
            match fabric.heap.get(id) {
                HeapObject::String(s) => format!("\"{}\"", s),
                HeapObject::Cons { .. } => format_list(fabric, val),
                HeapObject::Object { .. } => {
                    // Check for type-tag
                    if let Some(tag_sym) = fabric.heap.symbol_lookup_only("type-tag") {
                        let tag = fabric.heap.slot_get(id, tag_sym);
                        if let Value::Symbol(s) = tag {
                            let name = fabric.symbol_name(s);
                            if name == "lambda" { return "<lambda>".into(); }
                            if name == "operative" { return "<operative>".into(); }
                        }
                    }
                    format!("<object #{}>", id)
                }
                HeapObject::Bytes(b) => format!("<bytes {} bytes>", b.len()),
                HeapObject::Environment { .. } => format!("<env #{}>", id),
            }
        }
    }
}

fn format_list(fabric: &Fabric, val: Value) -> String {
    let mut parts = Vec::new();
    let mut current = val;
    while let Value::Object(id) = current {
        match fabric.heap.get(id) {
            moof_fabric::HeapObject::Cons { car, cdr } => {
                parts.push(format_value(fabric, *car));
                current = *cdr;
            }
            _ => {
                parts.push(format!(" . {}", format_value(fabric, current)));
                break;
            }
        }
    }
    if !current.is_nil() && !matches!(current, Value::Object(_)) {
        parts.push(format!(" . {}", format_value(fabric, current)));
    }
    format!("({})", parts.join(" "))
}

fn bind_io_natives(fabric: &mut Fabric, root_env: u32) {
    use moof_fabric::{NativeInvoker, HeapObject};

    // println: write a value to stdout
    // This is a moof-level function, not a native handler.
    // We bind it as a native in the env for convenience.
    let handler_id = NativeInvoker::make_handler(&mut fabric.heap, "io:println");
    let sym = fabric.intern("__println");
    fabric.heap.env_define(root_env, sym, Value::Object(handler_id));

    // We need to register the function in the NativeInvoker too
    // But the invoker is already registered... we need to access it.
    // For now, just make println a bootstrap definition.
    // The bootstrap defines println in terms of write-line.
}
