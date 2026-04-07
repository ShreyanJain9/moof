# Plan: Moof as Shared Data Substrate

## The shape

```
┌─────────────────────────────────────────────────────┐
│ Frontends (connect to the backend)                   │
│                                                      │
│  moof-repl     moof-gui     moof-mcp    your-app   │
│  (uses         (tui/egui    (AI agent    (rust,     │
│   moof-lang)    browser)     tools)      python,    │
│                                           whatever) │
└─────┬────────────┬────────────┬────────────┬────────┘
      │            │            │            │
      ▼            ▼            ▼            ▼
┌─────────────────────────────────────────────────────┐
│ moof-server (the backend)                            │
│                                                      │
│  connection protocol: in-process Rust API            │
│  (future: MCP over stdio, unix socket, TCP)          │
│                                                      │
│  ┌───────────────────────────────────────────────┐   │
│  │ moof-conventions                              │   │
│  │                                               │   │
│  │ Standard object protocols that all frontends  │   │
│  │ agree on. NOT a language — a set of contracts │   │
│  │ for how objects describe themselves, how       │   │
│  │ types work, how modules are organized.        │   │
│  │                                               │   │
│  │ - Object root proto (describe, interface,     │   │
│  │   slotAt:, slotNames, handlerNames, parent)   │   │
│  │ - Type protos (Integer, String, Cons, etc.)   │   │
│  │ - Module protocol (Modules registry)          │   │
│  │ - Introspection protocol                      │   │
│  │ - Capability protocol (Membrane, Facet)       │   │
│  └───────────────────────────────────────────────┘   │
│                                                      │
│  ┌───────────────────────────────────────────────┐   │
│  │ moof-fabric                                   │   │
│  │                                               │   │
│  │ The kernel. Knows about:                      │   │
│  │ - Objects (parent + slots + handler table)    │   │
│  │ - Symbols (interned names)                    │   │
│  │ - Message send (handler lookup + delegation)  │   │
│  │ - Scheduling (vats, mailboxes, turns)         │   │
│  │ - Persistence (save/load the heap)            │   │
│  │                                               │   │
│  │ Does NOT know about:                          │   │
│  │ - Any programming language                    │   │
│  │ - Bytecode, ASTs, closures, environments      │   │
│  │ - S-expressions, brackets, keywords           │   │
│  │ - Modules, definitions, source code           │   │
│  └───────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## The crates

### moof-fabric

The substrate. ~1000 lines.

```rust
// The only types in the heap
enum HeapObject {
    Object { parent: Value, slots: Vec<(u32, Value)>, handlers: Vec<(u32, Value)> },
    Cons { car: Value, cdr: Value },
    String(String),
    Bytes(Vec<u8>),  // opaque data — bytecode, compiled code, images, whatever
}

// The only values
enum Value {
    Nil, True, False,
    Integer(i64), Float(f64),
    Symbol(u32),      // interned name
    Object(u32),      // heap reference
}

// The only operation
fn send(heap: &mut Heap, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String>;
```

The fabric provides:
- `Heap` — arena of objects + symbol table
- `send()` — handler lookup, delegation, doesNotUnderstand
- `Scheduler` — vats, mailboxes, fuel-based turns
- `Persistence` — save/load the heap as bincode
- `HandlerInvoker` trait — shells register how to call their handlers

Critically, `send()` finds a handler value in the handler table, then calls
the registered `HandlerInvoker` for that handler's type. The fabric doesn't
know how to execute handlers — it delegates to whoever registered the invoker.

```rust
pub trait HandlerInvoker: Send {
    /// Can this invoker handle this handler value?
    fn can_invoke(&self, heap: &Heap, handler: Value) -> bool;
    /// Invoke the handler with receiver and args
    fn invoke(&self, ctx: &mut InvokeContext, handler: Value, 
              receiver: Value, args: &[Value]) -> Result<Value, String>;
}
```

A `NativeInvoker` (always present) handles handlers that are registered
Rust closures (looked up by name in a registry). The moof shell adds a
`BytecodeInvoker` that runs Lambda/Operative bodies. A Python bridge adds
a `PythonInvoker`. They coexist.

### moof-conventions

Standard protocols. Depends on moof-fabric. ~500 lines.

Registers the Object root prototype, type prototypes (Integer, Float, etc.),
and standard handlers (describe, interface, slotAt:, etc.) as native closures.

This is where all the current natives.rs code goes — but reorganized as
"conventions" rather than "language features." The Integer + handler doesn't
need a language. It's a convention: integers respond to +.

Also defines:
- The type hierarchy (Object → Number → Integer/Float, etc.)
- The module protocol (how Modules/ModuleImage/Definition work)
- The introspection protocol (handlerNames, parent, etc.)
- The capability protocol (Membrane, Facet)

### moof-server

A running fabric instance. Depends on moof-fabric + moof-conventions. ~300 lines.

The server has NO special frontend API. When a frontend connects, it gets
a **vat**. All interaction is message-passing through that vat. The vat's
capabilities determine what the frontend can do.

```rust
pub struct Server {
    fabric: Fabric,     // heap + scheduler
    image_path: PathBuf,
}

impl Server {
    pub fn new() -> Self;
    pub fn load(path: &Path) -> Result<Self, String>;
    pub fn save(&self) -> Result<(), String>;
    
    /// A frontend connects. Gets a vat with the given capabilities.
    /// Capabilities are faceted references to objects in the heap.
    pub fn connect(&mut self, capabilities: Vec<Value>) -> VatId;
    
    /// Frontend enqueues a message on its vat's mailbox.
    pub fn enqueue(&mut self, vat: VatId, msg: Message);
    
    /// Frontend polls for resolved results.
    pub fn poll_result(&mut self, vat: VatId) -> Option<Value>;
    
    /// Run one scheduler tick (poll extensions, deliver messages, run turns).
    pub fn tick(&mut self);
}
```

Every frontend is a vat. Every operation is a message. Capability security
is just "what facets does this vat hold?"

```
repl-vat:    (Facet wrap: root-env allow: '(eval: lookup: define:to:))
             → full power REPL

agent-vat:   (Facet wrap: root-env allow: '(eval: lookup:))
             → can read and eval, writes go through review queue

browser-vat: raw read access (shared heap) + eventual sends for writes

guest-vat:   (Facet wrap: sandbox-env allow: '(eval:))
             → sandboxed evaluation only
```

Revoke a facet → the vat loses access. Mid-session. No restart.

The wire protocol for out-of-process frontends is just serialized Messages.
One protocol for everything — MCP, custom apps, remote images, federation.
```

### moof-lang

The moof language. Depends on moof-fabric. ~2000 lines.

- Reader (lexer + parser for s-expressions, brackets, braces, blocks)
- Compiler (AST → bytecode stored as Bytes heap objects)
- BytecodeInvoker (implements HandlerInvoker for Lambda/Operative)
- Bootstrap (loads the kernel: fn, if, let, cond, etc.)

The moof shell registers itself with a server:

```rust
pub struct MoofShell;

impl MoofShell {
    pub fn register(server: &mut Server) {
        // Register BytecodeInvoker
        // Register bootstrap natives (fn, if, etc. if not already in image)
        // Set up the moof language environment
    }
    
    pub fn eval(server: &mut Server, source: &str, env: u32) -> Result<Value, String> {
        // Lex → parse → compile → send to bytecode invoker
    }
}
```

### moof (the binary)

The CLI application. Depends on everything. ~200 lines.

```rust
fn main() {
    let mut server = if image_exists() {
        Server::load(&image_path())
    } else {
        let mut s = Server::new();
        MoofShell::register(&mut s);
        MoofShell::bootstrap(&mut s, &lib_dir());
        s.save();
        s
    };
    
    MoofShell::register(&mut server);  // re-register invokers (not in image)
    
    match mode {
        Mode::Repl => repl::run(&mut server),
        Mode::Mcp => mcp::run(&mut server),
        Mode::Gui => gui::run(&mut server),
    }
}
```

## What this changes

### HeapObject simplifies from 8 variants to 4

Gone: Lambda, Operative, BytecodeChunk, Environment, NativeFunction.

A Lambda is now an Object with slots `{params, body, def_env, source}` and
a handler table entry `{call: <bytecode-invoker-tag>}`. The moof shell's
BytecodeInvoker knows how to read these slots and execute the body.

An Environment is now an Object with a `bindings` slot (or maybe just
regular slots — every binding is a slot). The moof shell knows how to
use environments for scoping.

A NativeFunction is now an Object with a `name` slot. The NativeInvoker
looks up the closure by name and calls it.

### The image format becomes language-agnostic

The image contains Objects, Strings, Cons cells, and Bytes. No Lambda or
Operative or BytecodeChunk. A bytecode body is just a Bytes blob. The
fabric doesn't interpret it — the moof shell does.

This means a Python bridge could store Python source as a String in a
handler's slots, and its PythonInvoker compiles and runs it on demand.

### Message dispatch becomes pluggable

Currently: `call_value` pattern-matches on HeapObject variant to decide
how to invoke. New: `send()` finds the handler value, then asks each
registered HandlerInvoker "can you handle this?" The first one that says
yes gets to invoke it.

### The REPL is just another frontend

Currently: the REPL loop is in main.rs, hardcoded to use the moof
language. New: the REPL connects to the server as a frontend, gets a
vat, and uses MoofShell::eval to evaluate expressions. The GUI and MCP
server connect the same way.

### Multiple languages can coexist

A Python bridge registers its invoker. Python handlers and moof handlers
live in the same heap. A moof handler can send a message to an object
whose handler is implemented in Python. They're all just objects.

## Migration

### Kill the image

The current image contains Lambda/Operative/BytecodeChunk heap objects
that don't exist in the new architecture. We start fresh.

### Preserve the source

lib/*.moof has all module sources. The bootstrap rebuilds from those.

### Phased approach

1. Create moof-fabric as a workspace crate (move heap, value, dispatch)
2. Create moof-conventions (move natives, type protos)
3. Create moof-lang (move reader, compiler, interpreter)
4. Refactor HeapObject (Lambda/Operative → Object with slots)
5. Implement HandlerInvoker trait
6. Create moof-server
7. Refactor main.rs to use the new architecture
8. Kill old image, reseed from lib/

## What this enables (the payoff)

- **Any language as a shell.** Python, Lua, Wasm, Rust closures — all first-class.
- **The fabric IS the database.** Applications store state as objects. No ORM.
- **Frontends are peers.** The REPL, GUI, MCP agent, custom apps — all equal.
- **The image is the truth.** Language-agnostic. Survives shell changes.
- **"Not a programming language."** Finally, literally true.
