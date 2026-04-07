# Plan: Server as a Separate Process

## The shape

```
Terminal 1 (server):              Terminal 2 (repl):
$ moof-server                     $ moof-repl
  Fabric booted (247 objects)       Connecting to /tmp/moof.sock...
  System vats: Console(0),          [awaiting approval]
    Filesystem(1), Clock(2)
  Listening: /tmp/moof.sock       Server: approve? [y/n] y
                                  
  [vat 3] connected (repl)         Connected as vat 3
  [vat 3] eval: [1 + 2]            (Console, Filesystem, Clock)
  [vat 3] => 3
                                    moof> [1 + 2]
                                    => 3
```

## Wire protocol

Unix domain socket. Line-delimited JSON. Each message is one line.

### Client → Server

```json
{"op": "connect", "token": "optional-auth-token"}
{"op": "eval", "source": "(+ 1 2)"}
{"op": "send", "receiver": 123, "selector": "writeLine:", "args": [{"String": "hello"}]}
{"op": "disconnect"}
```

### Server → Client

```json
{"op": "connected", "vat_id": 3, "capabilities": ["Console", "Filesystem", "Clock"]}
{"op": "result", "value": {"Integer": 3}}
{"op": "error", "message": "doesNotUnderstand: foo on obj#42"}
{"op": "prompt", "message": "approve connection from 'repl'?"}
{"op": "output", "text": "hello"}  // forwarded from Console
```

## Value serialization

Values serialize to JSON naturally:
- `Nil` → `null`
- `True/False` → `true/false`
- `Integer(n)` → `{"Integer": n}`
- `Float(f)` → `{"Float": f}`
- `Symbol(id)` → `{"Symbol": "name"}`
- `Object(id)` → `{"Object": id}` (opaque reference — can't be forged)
- Strings → `{"String": "text"}`
- Lists → `{"List": [...]}`

## Binaries

### moof-server

```rust
fn main() {
    let mut server = Server::new();
    moof_lang::setup(&mut server);
    register_io_handlers(&mut server);
    load_bootstrap(&mut server);
    
    let socket = UnixListener::bind("/tmp/moof.sock")?;
    eprintln!("Listening: /tmp/moof.sock");
    
    loop {
        // Accept connection
        let (stream, _) = socket.accept()?;
        
        // Read connect message
        let msg = read_json_line(&stream);
        
        // Prompt server operator for approval
        eprint!("Approve connection? [y/n] ");
        if read_approval() {
            let conn = server.connect_all(); // or token-based
            send_json_line(&stream, connected_msg(&conn));
            
            // Handle messages from this client
            handle_client(&mut server, conn, stream);
        }
    }
}
```

### moof-repl

```rust
fn main() {
    let stream = UnixStream::connect("/tmp/moof.sock")?;
    send_json_line(&stream, connect_msg());
    
    let response = read_json_line(&stream);
    // Wait for approval...
    
    println!("Connected as vat {}", response.vat_id);
    
    // REPL loop: send eval messages, print results
    loop {
        let input = read_line("moof> ");
        send_json_line(&stream, eval_msg(&input));
        let result = read_json_line(&stream);
        println!("=> {}", format_result(&result));
    }
}
```

## Implementation order

1. Add serde_json to dependencies
2. Define the protocol types (Message, Response enums)
3. Value serialization (to/from JSON)
4. moof-server binary (socket listener + approval flow)
5. moof-repl binary (socket client + REPL)
6. Console output forwarding (server captures println, sends to client)

## The hard part: Console output forwarding

When the REPL evals `(println "hello")`, the Console system vat
writes to stdout. But stdout is the SERVER's stdout, not the client's.

Fix: Console's writeLine: handler sends an "output" message BACK to
the originating vat's connection stream. This requires the Console
to know which client triggered the output — which means the message
needs to carry a "reply-to" channel or the console needs to buffer
output per-vat.

Simpler for v1: Console writes to the server's stdout and the client
sees it there. True output forwarding comes later.

## What this enables

- Multiple REPLs connected to the same image simultaneously
- GUI browser as a separate process
- AI agent connecting as a separate process
- Remote connections (TCP instead of Unix socket)
- The substrate IS a service, not an application
