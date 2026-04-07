/// moof-repl: connects to a running moof-server over unix socket.
///
/// The REPL is a thin client. It sends eval requests (as message sends
/// to the root environment) and prints results. Compilation happens
/// on the server.

use std::io::{self, Write as IoWrite, BufRead};
use std::os::unix::net::UnixStream;
use moof_fabric::wire::{self, Request, Response, WireArg};
use moof_fabric::Value;

fn main() {
    let socket_path = std::env::args().nth(1)
        .unwrap_or_else(|| "/tmp/moof.sock".to_string());

    let token = std::env::args().nth(2)
        .unwrap_or_else(|| "repl".to_string());

    // ── Connect to server ──
    eprintln!("connecting to {}...", socket_path);
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("!! cannot connect: {}. Is moof-server running?", e);
            std::process::exit(1);
        }
    };

    // Send connect request
    let req = Request::Connect { token };
    wire::write_message(&mut stream, &req.encode()).unwrap();

    // Read response (might be Connected or Error)
    let resp_bytes = wire::read_message(&mut stream).unwrap();
    let resp = Response::decode(&resp_bytes).expect("bad response");

    let (vat_id, capabilities) = match resp {
        Response::Connected { vat_id, capabilities } => (vat_id, capabilities),
        Response::Error(e) => {
            eprintln!("!! connection denied: {}", e);
            std::process::exit(1);
        }
        _ => {
            eprintln!("!! unexpected response");
            std::process::exit(1);
        }
    };

    // Read the env id
    let resp_bytes = wire::read_message(&mut stream).unwrap();
    let env_id = match Response::decode(&resp_bytes) {
        Some(Response::Ok(Value::Object(id))) => id,
        _ => {
            eprintln!("!! expected env id");
            std::process::exit(1);
        }
    };

    let cap_names: Vec<&str> = capabilities.iter().map(|(n, _)| n.as_str()).collect();
    println!("MOOF — connected to {}", socket_path);
    println!("vat {} ({} capabilities: {})", vat_id, capabilities.len(), cap_names.join(", "));
    println!("Type expressions to evaluate. Ctrl-D to exit.\n");

    // ── REPL loop ──
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = String::new();

    loop {
        if buffer.is_empty() { print!("moof> "); } else { print!("  ... "); }
        stdout.flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // Disconnect
                let _ = wire::write_message(&mut stream, &Request::Disconnect.encode());
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

                // Send [env eval: "source"] to the server
                // The server allocates the string in the heap, then dispatches
                let req = Request::Send {
                    receiver: env_id,
                    selector: "eval:".to_string(),
                    args: vec![WireArg::Str(input.clone())],
                };
                if wire::write_message(&mut stream, &req.encode()).is_err() {
                    eprintln!("!! connection lost");
                    break;
                }

                // Read result
                match wire::read_message(&mut stream) {
                    Ok(resp_bytes) => match Response::decode(&resp_bytes) {
                        Some(Response::Ok(val)) => println!("=> {}", format_value(val)),
                        Some(Response::Error(e)) => println!("!! {}", e),
                        _ => println!("!! unexpected response"),
                    }
                    Err(_) => { eprintln!("!! connection lost"); break; }
                }
            }
            Err(e) => { eprintln!("!! {}", e); break; }
        }
    }
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

fn format_value(val: Value) -> String {
    match val {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::False => "false".into(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'sym#{}", id),
        Value::Object(id) => format!("<object #{}>", id),
    }
}
