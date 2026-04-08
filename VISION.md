# moof v2: the reimagination

> if i were rebuilding moof from nothing, knowing everything the v1 journey
> taught us, this is what i would build.

---

## what v1 got right (keep these)

these are load-bearing ideas. they survive the rewrite:

- **six kernel primitives.** `vau`, `send`, `def`, `quote`, `cons`, `eq`. this is the right set.
- **objects and messaging as the only abstraction.** `(f a b c)` is `[f call: a b c]`. no separate function type.
- **slots vs handlers.** the distinction between private storage and public behavior is the right encapsulation model.
- **prototype delegation for handlers only.** slots are per-object, behavior inherits.
- **the three bracket species.** `()` for calls, `[]` for sends, `{}` for objects. beautiful syntax.
- **the fabric is language-agnostic.** the substrate doesn't know about any language.
- **capability security via vats, membranes, facets.** references are capabilities.
- **rust as the implementation language.** the borrow checker earns its keep at the GC boundary.

## what v1 got wrong (change these)

### 1. persistence should have been designed first, not retrofitted four times

v2 approach: **the heap is an LMDB database from day zero.** not bincode, not files, not WAL-over-arena. LMDB gives us:
- memory-mapped reads (zero-copy, instant startup)
- ACID transactions (no corruption, no partial writes)
- concurrent readers (browser/inspector can read while the VM runs)
- the OS page cache IS the cache (no double-buffering)

every object is a key-value entry. the key is its object ID. the value is its serialized slots, handlers, and type tag. symbol table is a separate LMDB database in the same environment. this means:
- startup is `mmap` the file. done. no deserialization.
- checkpoint is `commit`. done. no serialization walk.
- GC is a compaction transaction that copies live objects to a new database.

### 2. the interpreter is too complex

the v1 interpreter is 986 lines — the largest file in the codebase. it mixes bytecode execution, environment manipulation, native dispatch, tail call optimization, and error handling in one monolithic `run()` loop.

v2 approach: **register-based bytecode, not stack-based.** fewer opcodes, simpler dispatch. the interpreter becomes a simple loop over a flat instruction array. environment access is compile-time indexed (de bruijn indices), not runtime name lookup. this eliminates the entire environment-as-linked-list overhead.

### 3. vau needs a compilation strategy from day one

v1 acknowledged that vau makes optimization hard and punted to "future JIT." v2 faces it head-on:

**two-phase compilation.** first pass: analyze every `vau` call site. if the operative is statically known (bound to a `def` at compile time and never reassigned), inline it. this covers 95% of uses — `if`, `let`, `when`, `unless`, `cond`, `match`, `while` are all statically known operatives. second pass: for the remaining 5% (truly dynamic operatives), emit a slow path that reifies the environment. the compiler tracks "is this binding stable?" as a simple flag.

this means `(if cond then else)` compiles to a conditional branch, not a call to an operative that might do anything. `vau` keeps its full power for metaprogramming; hot paths get real compilation.

### 4. the module system should be simpler

v1's module system has dependency graphs, topological sorting, sandboxed environments, and a complex bootstrap ordering problem. for what is essentially "evaluate these definitions in order."

v2 approach: **modules are just objects with ordered definition lists.** no dependency graph. no topological sort. load order is explicit in the image. a module's environment is just an object with bindings as slots. "importing" a module means getting a reference to that object. "sandboxing" means the module object's parent chain doesn't include IO capabilities.

the module system is ~50 lines of moof, not ~500 lines of rust.

### 5. the wire protocol should be the fabric's native tongue

v1 has a binary wire protocol that's mostly unused — in-process calls bypass it. the MCP server is a separate stdio mode.

v2 approach: **everything goes through the message protocol, even in-process.** the wire format is the serialization format is the persistence format. an object's on-disk representation is the same bytes that would be sent over the wire. this means:
- remote objects are transparent (send goes over the wire instead of to local heap)
- federation is just "connect to another fabric and send messages"
- the MCP server is a protocol adapter, not a separate mode

### 6. the GC should be incremental

v1's "GC at save time" means the heap grows without bound during a session. for a system meant to run for days/weeks, this is fatal.

v2 approach: **incremental mark-sweep with LMDB.** objects are born in a nursery (a small in-memory arena). survivors get promoted to LMDB. the nursery is collected frequently (every N allocations). LMDB objects are collected lazily (mark from roots, sweep in a background transaction). because LMDB is copy-on-write, the sweep doesn't block reads.

## the new architecture

```
moof-v2/
  crates/
    fabric/          the substrate (~800 lines)
      value.rs       tagged values (8 bytes each)
      store.rs       LMDB-backed object store
      dispatch.rs    send() — handler lookup + delegation
      gc.rs          incremental nursery + lazy LMDB sweep
      vat.rs         capability domains + scheduler

    lang/            the moof language (~1500 lines)
      lexer.rs       tokenizer
      parser.rs      cons-cell AST builder
      analyze.rs     vau-aware analysis pass (NEW)
      compiler.rs    AST → register bytecode
      vm.rs          bytecode interpreter (register machine)
      bootstrap.rs   seed the kernel forms

    shell/           the interactive surface (~400 lines)
      repl.rs        readline + eval + print
      inspect.rs     TUI object browser
      mcp.rs         MCP protocol adapter

  lib/
    bootstrap.moof   the kernel library
    core.moof        object model + standard types
    io.moof          file/network/process wrappers

  src/
    main.rs          CLI entry point
```

### fabric: the substrate

the fabric knows about five things:
1. **values** — nil, true, false, integers, floats, symbols, object references
2. **objects** — parent + slots + handlers, stored in LMDB
3. **send** — handler lookup, delegation, doesNotUnderstand
4. **vats** — capability domains with mailboxes
5. **GC** — incremental collection across nursery and store

it does NOT know about: bytecode, ASTs, environments, closures, s-expressions, modules, source code, strings-as-objects (strings are a value type in the fabric).

```rust
// the entire value type
#[repr(u8)]
enum Tag { Nil, True, False, Int, Float, Sym, Obj }

#[derive(Copy, Clone)]
struct Value(u64);  // NaN-boxed: floats inline, ints inline up to 48 bits

// the entire object type
struct Object {
    parent: Value,
    slots: SmallVec<[(u32, Value); 4]>,    // symbol → value
    handlers: SmallVec<[(u32, Value); 4]>,  // selector → handler
}

// the only operation
fn send(store: &mut Store, receiver: Value, selector: u32, args: &[Value]) -> Result<Value>;
```

NaN-boxing puts the most common values (nil, bools, small ints, floats, symbols) in a single 8-byte word with no heap allocation. objects are the only thing that lives in the store.

### lang: the moof language

the language crate is a plugin. it registers a `BytecodeInvoker` that the fabric's `send()` calls when it encounters a handler whose value is a bytecode object.

**key change: the analysis pass.** between parsing and compilation, `analyze.rs` walks the AST and classifies every `vau` use:

```
static-known:  the operative is a def'd binding that is never reassigned
               → inline the operative's expansion at compile time
               → `if`, `let`, `when`, `cond`, `match`, `while` all hit this path

dynamic:       the operative could be anything
               → emit a slow path that captures the caller's environment
               → rare in practice, essential for metaprogramming
```

**register-based bytecode:**

```
LOAD_CONST  r0, #42        ; load constant
LOAD_LOCAL  r1, 0, 2       ; load from env frame 0, slot 2 (de bruijn)
SEND        r2, r0, #add, [r1]  ; [r0 add: r1] → r2
RETURN      r2
```

fewer instructions, each does more. no operand stack to manage. the compiler assigns virtual registers; a simple linear-scan allocator maps them to frame slots.

### shell: the interactive surface

the shell is thin. it owns:
- **repl**: readline, parse, compile, send `[env eval: bytecode]`, print result
- **inspect**: TUI browser using ratatui. reads objects from the LMDB store directly (zero-copy via mmap). can browse while the VM is running because LMDB supports concurrent readers.
- **mcp**: protocol adapter that translates MCP JSON-RPC into fabric sends

the shell does NOT own: the module system, the type hierarchy, the standard library. those are moof code in the image.

## what this enables

### instant startup
LMDB is memory-mapped. "loading the image" is calling `mmap()`. there is no deserialization step. the first `send()` touches the pages it needs; the OS loads them on demand. a 100MB image starts in microseconds.

### concurrent inspection
LMDB readers don't block writers. the TUI browser, MCP server, and AI agents can all read the object graph while the VM is mutating it. no locks, no snapshots, no copying.

### real federation
objects in a remote fabric look like local objects with a network transport. `send()` checks: is this object local? → dispatch locally. is it remote? → serialize the message, send over the wire, deserialize the result. the `Value` type already has a tag for remote references.

### language plurality
the fabric doesn't know about moof-the-language. a lua frontend registers a `LuaInvoker`. a wasm frontend registers a `WasmInvoker`. all coexist. you can have a lua handler call a moof handler call a native handler, all through the same `send()` path.

### real AI participation
an AI agent is a vat. it connects to the fabric, gets faceted references, sends messages. its "memory" is objects in its vat. its "tools" are the handlers on the objects it can see. revoke a facet → the agent loses access, mid-conversation, no restart. the MCP adapter is one line: translate JSON-RPC `tools/call` into `send()`.

## the bootstrap sequence

```
1. fabric boots with an empty LMDB store
2. lang registers BytecodeInvoker
3. lang loads bootstrap.moof:
   - defines vau, send, def, quote, cons, eq as kernel forms
   - derives lambda, if, let, cond, match, while, do, loop
   - defines Object, Integer, Float, String, Cons, etc. prototypes
4. lang loads core.moof:
   - defines Module, Definition, Modules prototypes
   - defines standard collections (Assoc, Stack, Queue, etc.)
   - defines Membrane, Facet, AuditLog
5. shell starts the REPL (or MCP server, or TUI browser)
6. image is committed to LMDB
7. next startup: skip steps 2-4, go straight to 5
```

## migration plan

### phase 0: archive v1
- create `archive/v1` branch from current master
- tag it `v1-final`
- document the state

### phase 1: fabric (~2 days of work)
- `Value` with NaN-boxing
- `Store` backed by LMDB (or heed, the rust wrapper)
- `send()` with handler lookup + delegation
- `HandlerInvoker` trait
- `NativeInvoker` for rust closures
- basic vat + scheduler
- tests

### phase 2: lang (~3 days)
- lexer (port from v1, it's solid)
- parser (port from v1, it's solid)
- analysis pass (new)
- register-based compiler (rewrite)
- register-based interpreter (rewrite)
- bootstrap.moof (port + simplify)
- tests

### phase 3: shell (~1 day)
- REPL with readline
- TUI inspector
- MCP adapter

### phase 4: library (~2 days)
- core.moof: collections, classes, json
- modules.moof: the module system in moof
- membrane.moof: capability security
- system.moof: image management, introspection

### phase 5: parity check
- verify all v1 REPL examples work
- verify MCP server works
- verify persistence works (restart and resume)

## what i'm NOT doing

- **JIT.** not yet. the register-based bytecode is designed to be JIT-friendly (SSA-adjacent), but the JIT is a later project. the analysis pass + inline caching gets us 80% of the performance win.
- **GUI browser.** the TUI inspector is enough for now. the egui browser is nice but not essential.
- **federation.** the architecture supports it but implementing it is a separate project.
- **CRDTs.** same — the store model is compatible but the implementation is future work.

## lines of code estimate

| crate | v1 | v2 estimate | notes |
|---|---|---|---|
| fabric | ~1200 | ~800 | LMDB replaces custom heap+persist+wire |
| lang | ~2700 | ~1500 | register VM simpler than stack VM; analysis pass new but saves elsewhere |
| shell | ~400 | ~400 | roughly the same |
| moof libs | ~1250 | ~800 | simpler module system, less ceremony |
| **total** | **~5550** | **~3500** | 37% smaller |

the reduction comes from: LMDB replacing custom persistence, register VM being more direct, the analysis pass eliminating the vau-overhead code, and the module system being moof-not-rust.

---

*this is my honest reimagination. the v1 journey was essential — every wrong turn taught something. the decisions above aren't "obvious in hindsight"; they're only visible because v1 explored the space. moof v2 is v1's gift to itself.*

*clarus lives. moof.*
