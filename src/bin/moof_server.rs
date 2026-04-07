/// moof-server: boots a fabric, listens on a unix socket.
///
/// The server is the fabric. Clients connect and get vats.
/// moof-lang is pre-loaded: BytecodeInvoker, type conventions, bootstrap.
/// Clients send fabric operations (send, create, slot-get, etc).
/// "eval" is just [env eval: source] — the server handles it via moof-lang.

use std::io::{self, BufRead, Write as IoWrite};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use moof_fabric::wire::{self, Request, Response};
use moof_fabric::Value;
use moof_server::Server;

fn main() {
    let socket_path = std::env::args().nth(1)
        .unwrap_or_else(|| "/tmp/moof.sock".to_string());

    // ── Boot the fabric ──
    let mut server = Server::new();

    // Register moof-lang shell
    let setup = moof_lang::setup(server.fabric());
    let root_env = setup.root_env;

    // Register IO on system vats
    moof_lang::io::register_system_handlers(&mut server);

    // print for bootstrap
    let _ = moof_lang::eval(server.fabric(),
        "(def print (lambda (x) [Console writeLine: x]))", root_env);

    // Load bootstrap
    let bootstrap_path = PathBuf::from("lib/bootstrap.moof");
    if bootstrap_path.exists() {
        if let Ok(source) = std::fs::read_to_string(&bootstrap_path) {
            let body = skip_header(&source);
            match moof_lang::eval(server.fabric(), body, root_env) {
                Ok(_) => eprintln!("(loaded bootstrap)"),
                Err(e) => { eprintln!("!! bootstrap: {}", e); std::process::exit(1); }
            }
        }
    }

    eprintln!("MOOF server");
    eprintln!("  heap: {} objects", server.fabric().heap.len());
    eprintln!("  system vats: Console({}), Filesystem({}), Clock({})",
        server.system.console_vat, server.system.filesystem_vat, server.system.clock_vat);

    // ── Listen ──
    let _ = std::fs::remove_file(&socket_path); // clean up stale socket
    let listener = UnixListener::bind(&socket_path).expect("cannot bind socket");
    eprintln!("  listening: {}", socket_path);
    eprintln!();

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => { eprintln!("!! accept: {}", e); continue; }
        };

        eprintln!("connection from client...");

        // Read connect request
        let msg_bytes = match wire::read_message(&mut stream) {
            Ok(b) => b,
            Err(e) => { eprintln!("!! read: {}", e); continue; }
        };

        let request = match Request::decode(&msg_bytes) {
            Some(Request::Connect { token }) => token,
            _ => { eprintln!("!! expected Connect request"); continue; }
        };

        // Prompt for approval
        eprint!("  approve connection (token: '{}')? [y/n] ", request);
        io::stderr().flush().unwrap();

        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer).unwrap();
        if !answer.trim().starts_with('y') {
            eprintln!("  DENIED");
            let resp = Response::Error("connection denied".into());
            let _ = wire::write_message(&mut stream, &resp.encode());
            continue;
        }

        // Accept: create vat with capabilities
        let conn = server.connect_all();
        let env = {
            // Create a child env of root for this connection
            let child_env = server.fabric().heap.alloc_env(Some(root_env));
            // Bind capabilities
            conn.bind_capabilities(server.fabric(), child_env);
            child_env
        };

        eprintln!("  APPROVED: vat {} ({} capabilities)", conn.vat_id, conn.capabilities.len());

        // Send connected response
        let resp = Response::Connected {
            vat_id: conn.vat_id,
            capabilities: conn.capabilities.clone(),
        };
        if wire::write_message(&mut stream, &resp.encode()).is_err() { continue; }

        // Also send the env id so the client can eval in it
        let resp = Response::Ok(Value::Object(env));
        if wire::write_message(&mut stream, &resp.encode()).is_err() { continue; }

        // ── Handle messages from this client ──
        loop {
            server.tick();

            let msg_bytes = match wire::read_message(&mut stream) {
                Ok(b) => b,
                Err(_) => { eprintln!("  [vat {}] disconnected", conn.vat_id); break; }
            };

            let request = match Request::decode(&msg_bytes) {
                Some(r) => r,
                None => {
                    let resp = Response::Error("malformed request".into());
                    let _ = wire::write_message(&mut stream, &resp.encode());
                    continue;
                }
            };

            let response = match request {
                Request::Send { receiver, selector, args } => {
                    // Resolve WireArgs: allocate strings in the heap
                    let resolved: Vec<Value> = args.iter().map(|a| match a {
                        wire::WireArg::Val(v) => *v,
                        wire::WireArg::Str(s) => server.fabric().alloc_string(s),
                    }).collect();

                    // Intercept eval: on environments — route through moof-lang
                    if selector == "eval:" {
                        if let moof_fabric::HeapObject::Environment { .. } = server.fabric().heap.get(receiver) {
                            // The arg is a heap-allocated string — read it back
                            let source = match resolved.first().copied().unwrap_or(Value::Nil) {
                                Value::Object(sid) => match server.fabric().heap.get(sid) {
                                    moof_fabric::HeapObject::String(s) => s.clone(),
                                    _ => {
                                        Response::Error("eval: arg must be string".into());
                                        continue;
                                    }
                                },
                                _ => {
                                    Response::Error("eval: arg must be string".into());
                                    continue;
                                }
                            };
                            match moof_lang::eval(server.fabric(), &source, receiver) {
                                Ok(val) => {
                                    eprintln!("  [vat {}] eval => {:?}", conn.vat_id, val);
                                    Response::Ok(val)
                                }
                                Err(e) => {
                                    eprintln!("  [vat {}] eval !! {}", conn.vat_id, e);
                                    Response::Error(e)
                                }
                            }
                        } else {
                            let sel_sym = server.intern(&selector);
                            match server.fabric().send(Value::Object(receiver), sel_sym, &resolved) {
                                Ok(val) => Response::Ok(val),
                                Err(e) => Response::Error(e),
                            }
                        }
                    } else {
                        let sel_sym = server.intern(&selector);
                        match server.fabric().send(Value::Object(receiver), sel_sym, &resolved) {
                            Ok(val) => {
                                eprintln!("  [vat {}] send {} {} => {:?}", conn.vat_id, receiver, selector, val);
                                Response::Ok(val)
                            }
                            Err(e) => {
                                eprintln!("  [vat {}] send {} {} !! {}", conn.vat_id, receiver, selector, e);
                                Response::Error(e)
                            }
                        }
                    }
                }
                Request::Create { parent } => {
                    let id = server.fabric().create_object(parent);
                    eprintln!("  [vat {}] create => #{}", conn.vat_id, id);
                    Response::Created(id)
                }
                Request::SlotGet { object, slot } => {
                    let val = server.fabric().get_slot(object, &slot);
                    Response::Ok(val)
                }
                Request::SlotSet { object, slot, value } => {
                    server.fabric().set_slot(object, &slot, value);
                    Response::Ok(value)
                }
                Request::Intern { name } => {
                    let id = server.intern(&name);
                    Response::Interned(id)
                }
                Request::Disconnect => {
                    eprintln!("  [vat {}] disconnected", conn.vat_id);
                    break;
                }
                Request::Connect { .. } => {
                    Response::Error("already connected".into())
                }
            };

            if wire::write_message(&mut stream, &response.encode()).is_err() {
                break;
            }
        }
    }
}

fn skip_header(source: &str) -> &str {
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
