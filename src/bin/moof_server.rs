/// moof-server: generic fabric host.
///
/// Boots a fabric, loads extension dylibs, listens on a unix socket.
/// The server doesn't know about any language. Extensions bring that.
///
/// Usage:
///   moof-server [--load path.dylib]... [--socket path] [--token TOKEN]
///
/// If no --load is given, tries to load libmoof_lang from the default path.

use std::io::{self, BufRead, Write as IoWrite};
use std::os::unix::net::UnixListener;
use moof_fabric::wire::{self, Request, Response, WireArg};
use moof_fabric::{Value, HeapObject};
use moof_server::Server;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut socket_path = "/tmp/moof.sock".to_string();
    let mut extensions: Vec<String> = Vec::new();
    let mut tokens: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => { i += 1; socket_path = args[i].clone(); }
            "--load" => { i += 1; extensions.push(args[i].clone()); }
            "--token" => { i += 1; tokens.push(args[i].clone()); }
            _ => { eprintln!("unknown arg: {}", args[i]); std::process::exit(1); }
        }
        i += 1;
    }

    // ── Boot ──
    let mut server = Server::new();

    // Register IO system vat handlers (Console, Filesystem, Clock)
    moof_server::io::register_system_handlers(&mut server);

    // Pre-bind IO capabilities in the global symbol table so extensions can reference them
    // (e.g., moof-lang's bootstrap defines `print` as [Console writeLine:])
    {
        let console_id = server.system.Console;
        let fs_id = server.system.Filesystem;
        let clock_id = server.system.Clock;
        // These will be available to any environment that inherits from root
        // Extensions create root envs that will pick these up
    }

    eprintln!("MOOF server");

    // Load extensions
    if extensions.is_empty() {
        // Default: try to load moof-lang from the build directory
        let default_paths = [
            "target/debug/libmoof_lang.dylib",
            "target/release/libmoof_lang.dylib",
            "target/debug/libmoof_lang.so",
            "target/release/libmoof_lang.so",
        ];
        for path in &default_paths {
            if std::path::Path::new(path).exists() {
                extensions.push(path.to_string());
                break;
            }
        }
    }

    for ext_path in &extensions {
        eprint!("  loading {}... ", ext_path);
        match moof_server::extension::load_extension(&mut server, ext_path) {
            Ok(()) => eprintln!("ok"),
            Err(e) => { eprintln!("FAILED: {}", e); std::process::exit(1); }
        }
    }

    // Register auth tokens
    for token in &tokens {
        let all_caps: Vec<String> = server.system.by_name.keys().cloned().collect();
        server.add_token(token, all_caps);
    }

    eprintln!("  heap: {} objects, {} symbols",
        server.fabric().heap.len(), server.fabric().heap.symbol_count());
    eprintln!("  system vats: {}", server.system.by_name.keys()
        .filter(|k| !k.starts_with("__"))
        .cloned().collect::<Vec<_>>().join(", "));

    // ── Listen ──
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("cannot bind socket");
    eprintln!("  listening: {}", socket_path);
    eprintln!();

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => { eprintln!("!! accept: {}", e); continue; }
        };

        eprintln!("connection...");

        let msg_bytes = match wire::read_message(&mut stream) {
            Ok(b) => b,
            Err(e) => { eprintln!("!! read: {}", e); continue; }
        };

        let token = match Request::decode(&msg_bytes) {
            Some(Request::Connect { token }) => token,
            _ => { eprintln!("!! expected Connect"); continue; }
        };

        // Approval
        eprint!("  approve (token: '{}')? [y/n] ", token);
        io::stderr().flush().unwrap();
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer).unwrap();
        if !answer.trim().starts_with('y') {
            eprintln!("  DENIED");
            let _ = wire::write_message(&mut stream, &Response::Error("denied".into()).encode());
            continue;
        }

        let conn = server.connect_all();

        // Find the moof root env (if moof-lang extension loaded it)
        let sentinel_id = server.system.by_name.get("__moof_env").copied();
        let env_id = sentinel_id.and_then(|sid| {
            let sym = server.fabric().heap.symbol_lookup_only("__moof_root_env")?;
            match server.fabric().heap.slot_get(sid, sym) {
                Value::Object(id) => Some(id),
                _ => None,
            }
        }).map(|root_env| {
            let child = server.fabric().heap.alloc_env(Some(root_env));
            conn.bind_capabilities(server.fabric(), child);
            child
        });

        eprintln!("  APPROVED: vat {} ({} caps{})",
            conn.vat_id, conn.capabilities.len(),
            if env_id.is_some() { ", moof-lang" } else { "" });

        // Send connected + env
        let resp = Response::Connected {
            vat_id: conn.vat_id,
            capabilities: conn.capabilities.clone(),
        };
        if wire::write_message(&mut stream, &resp.encode()).is_err() { continue; }

        if let Some(eid) = env_id {
            let _ = wire::write_message(&mut stream, &Response::Ok(Value::Object(eid)).encode());
        } else {
            let _ = wire::write_message(&mut stream, &Response::Ok(Value::Nil).encode());
        }

        // ── Message loop ──
        loop {
            server.tick();

            let msg_bytes = match wire::read_message(&mut stream) {
                Ok(b) => b,
                Err(_) => { eprintln!("  [vat {}] disconnected", conn.vat_id); break; }
            };

            let request = match Request::decode(&msg_bytes) {
                Some(r) => r,
                None => {
                    let _ = wire::write_message(&mut stream, &Response::Error("bad request".into()).encode());
                    continue;
                }
            };

            let response = handle_request(&mut server, &conn, env_id, request);

            if wire::write_message(&mut stream, &response.encode()).is_err() { break; }
        }
    }
}

fn handle_request(server: &mut Server, conn: &moof_server::Connection, env_id: Option<u32>, request: Request) -> Response {
    match request {
        Request::Send { receiver, selector, args } => {
            let resolved: Vec<Value> = args.iter().map(|a| match a {
                WireArg::Val(v) => *v,
                WireArg::Str(s) => server.fabric().alloc_string(s),
            }).collect();

            // Intercept eval: on environments → moof_lang::eval (if loaded)
            if selector == "eval:" {
                if let HeapObject::Environment { .. } = server.fabric().heap.get(receiver) {
                    let source = match resolved.first().copied().unwrap_or(Value::Nil) {
                        Value::Object(sid) => match server.fabric().heap.get(sid) {
                            HeapObject::String(s) => s.clone(),
                            _ => return Response::Error("eval: arg must be string".into()),
                        },
                        _ => return Response::Error("eval: arg must be string".into()),
                    };
                    // Use moof_lang if available
                    match moof_lang::eval(server.fabric(), &source, receiver) {
                        Ok(val) => {
                            eprintln!("  [vat {}] eval => {:?}", conn.vat_id, val);
                            return Response::Ok(val);
                        }
                        Err(e) => {
                            eprintln!("  [vat {}] eval !! {}", conn.vat_id, e);
                            return Response::Error(e);
                        }
                    }
                }
            }

            let sel_sym = server.intern(&selector);
            match server.fabric().send(Value::Object(receiver), sel_sym, &resolved) {
                Ok(val) => {
                    eprintln!("  [vat {}] {} {} => {:?}", conn.vat_id, receiver, selector, val);
                    Response::Ok(val)
                }
                Err(e) => {
                    eprintln!("  [vat {}] {} {} !! {}", conn.vat_id, receiver, selector, e);
                    Response::Error(e)
                }
            }
        }
        Request::Create { parent } => {
            let id = server.fabric().create_object(parent);
            Response::Created(id)
        }
        Request::SlotGet { object, slot } => {
            Response::Ok(server.fabric().get_slot(object, &slot))
        }
        Request::SlotSet { object, slot, value } => {
            server.fabric().set_slot(object, &slot, value);
            Response::Ok(value)
        }
        Request::Intern { name } => {
            Response::Interned(server.intern(&name))
        }
        Request::Disconnect => {
            eprintln!("  [vat {}] disconnect", conn.vat_id);
            Response::Ok(Value::Nil) // caller breaks the loop
        }
        Request::Connect { .. } => {
            Response::Error("already connected".into())
        }
    }
}
