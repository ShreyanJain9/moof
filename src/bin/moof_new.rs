/// moof — boots a server, connects the REPL as a root vat.

use std::io::{self, Write, BufRead};
use std::path::PathBuf;
use moof_fabric::{Value, HeapObject};
use moof_server::Server;

fn main() {
    // ── Boot the server ──
    let mut server = Server::new();

    // Register the moof language shell
    let setup = moof_lang::setup(server.fabric());
    let root_env = setup.root_env;

    // Register IO handlers on the system vat objects.
    // These are handlers on the console/fs/clock objects that the
    // system vats own. When a user vat sends a message to these objects,
    // the handler executes.
    moof_lang::io::register_system_handlers(&mut server);

    // print must exist before bootstrap (bootstrap's println wraps it)
    match moof_lang::eval(server.fabric(),
        "(def print (lambda (x) [console writeLine: x]))", root_env) {
        Ok(_) => {}
        Err(e) => eprintln!("!! print setup: {}", e),
    }

    // Load bootstrap
    let bootstrap_path = PathBuf::from("lib/bootstrap.moof");
    if bootstrap_path.exists() {
        match std::fs::read_to_string(&bootstrap_path) {
            Ok(source) => {
                let body = skip_module_header(&source);
                match moof_lang::eval(server.fabric(), body, root_env) {
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

    // ── Connect the REPL as root ──
    let mut repl = server.connect_root();
    repl.set_root_env(root_env);

    // Bind capabilities into the REPL's environment
    repl.bind_capabilities(server.fabric(), root_env);

    eprintln!("MOOF — on the fabric");
    eprintln!("vat {} connected as {:?}", repl.vat_id, repl.role);
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    // ── REPL loop ──
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = String::new();

    loop {
        server.tick();

        if buffer.is_empty() { print!("moof> "); } else { print!("  ... "); }
        stdout.flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => { println!("\nmoof."); break; }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() && buffer.is_empty() { continue; }
                buffer.push_str(&line);
                if !brackets_balanced(&buffer) { continue; }
                let input = buffer.trim().to_string();
                buffer.clear();
                if input.is_empty() { continue; }

                let env = repl.root_env.unwrap_or(0);
                match moof_lang::eval(server.fabric(), &input, env) {
                    Ok(val) => println!("=> {}", format_value(server.fabric(), val)),
                    Err(e) => println!("!! {}", e),
                }
            }
            Err(e) => { println!("!! {}", e); break; }
        }
    }
}

fn skip_module_header(source: &str) -> &str {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in source.char_indices() {
        if escape { escape = false; continue; }
        if in_string { if ch == '\\' { escape = true; } else if ch == '"' { in_string = false; } continue; }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => { depth -= 1; if depth == 0 { return source[i+1..].trim_start(); } }
            _ => {}
        }
    }
    source
}

fn brackets_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut in_comment = false;
    let mut escape = false;
    for ch in s.chars() {
        if in_comment { if ch == '\n' { in_comment = false; } continue; }
        if escape { escape = false; continue; }
        if in_string { if ch == '\\' { escape = true; } else if ch == '"' { in_string = false; } continue; }
        match ch { '"' => in_string = true, ';' => in_comment = true, '(' | '[' | '{' => depth += 1, ')' | ']' | '}' => depth -= 1, _ => {} }
    }
    depth <= 0
}

fn format_value(fabric: &moof_fabric::Fabric, val: Value) -> String {
    match val {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::False => "false".into(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'{}", fabric.symbol_name(id)),
        Value::Object(id) => match fabric.heap.get(id) {
            HeapObject::String(s) => format!("\"{}\"", s),
            HeapObject::Cons { .. } => format_list(fabric, val),
            HeapObject::Object { .. } => {
                if let Some(tag_sym) = fabric.heap.symbol_lookup_only("type-tag") {
                    let tag = fabric.heap.slot_get(id, tag_sym);
                    if let Value::Symbol(s) = tag {
                        let name = fabric.symbol_name(s);
                        match name {
                            "lambda" => return "<lambda>".into(),
                            "operative" => return "<operative>".into(),
                            _ => return format!("<{}>", name),
                        }
                    }
                }
                format!("<object #{}>", id)
            }
            HeapObject::Bytes(b) => format!("<bytes {}>", b.len()),
            HeapObject::Environment { .. } => format!("<env #{}>", id),
        },
    }
}

fn format_list(fabric: &moof_fabric::Fabric, val: Value) -> String {
    let mut parts = Vec::new();
    let mut current = val;
    while let Value::Object(id) = current {
        match fabric.heap.get(id) {
            HeapObject::Cons { car, cdr } => { parts.push(format_value(fabric, *car)); current = *cdr; }
            _ => { parts.push(format!(". {}", format_value(fabric, current))); break; }
        }
    }
    if !current.is_nil() && !matches!(current, Value::Object(_)) {
        parts.push(format!(". {}", format_value(fabric, current)));
    }
    format!("({})", parts.join(" "))
}
