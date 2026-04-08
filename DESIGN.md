# moof v2: deep design

> the hard questions that VISION.md waves at.

---

## 1. what does "browser-first" actually mean for implementation order?

the temptation is to build the VM first because it's familiar.
resist. the browser IS the deliverable. the VM is infrastructure.

**the minimum viable browser needs:**

1. an LMDB store with objects in it
2. a way to read those objects (slots, handlers, parent)
3. a way to render them as panels in egui
4. a way to navigate references (click an object ref → open it)
5. a way to edit a slot value inline

that's it. no VM. no language. no bytecode. just a database of
objects and a GUI that shows them.

**then** you add an eval bar that can parse and compile expressions.
**then** you add handler execution. **then** you add the agent.

the browser's existence forces the right architecture: the store
must support concurrent reads (LMDB does), the object format must
be self-describing (slots and handlers have symbol keys), and the
mutation path must be transactional (LMDB does).

if you build the VM first, you'll build a REPL-shaped VM and then
struggle to make the browser fit. if you build the browser first,
the VM naturally serves it.

## 2. the LMDB tradeoff: what we gain and what we lose

### we gain

- **instant startup**: mmap, done. no deserialization.
- **crash safety**: ACID transactions. power loss loses nothing.
- **concurrent readers**: the browser reads while the VM writes.
- **no custom serialization**: LMDB handles the bytes.
- **no GC pressure**: the OS manages the page cache.

### we lose

- **mutation overhead**: every slot write is an LMDB transaction.
  this is ~1μs per write. for a VM that does millions of mutations
  per second (environment bindings, stack frames, temporaries),
  this is fatal.

### the solution: the nursery

hot mutable state (environments, stack frames, temporaries) lives
in a traditional in-memory arena — the "nursery." this is a plain
`Vec<HeapObject>` indexed by u32, exactly like v1's heap. it's
fast. it's not persistent. it doesn't need to be.

persistent state (user-created objects, handlers, module
definitions, the type hierarchy) lives in LMDB. these objects are
created infrequently and read often — the perfect LMDB workload.

the Value type distinguishes nursery refs from store refs:

```rust
// tag 5 = store object (LMDB)
// tag 6 = nursery object (in-memory arena)
```

send() checks the tag and dispatches to the right backing store.
promotion from nursery to store happens when an object is "anchored"
— assigned as a slot value on a store object, or explicitly
promoted by the user.

this is essentially a generational collector where gen0 is the
nursery arena and gen1 is LMDB. "collection" of the nursery is
trivial: when a vat's turn ends, sweep the nursery for anything
not referenced by a store object. everything else is garbage.

### what about strings?

strings are Blobs. short strings (< 64 bytes) could be inlined
in the Value via a separate tag, like many VMs do. but NaN-boxing
only gives us 48 bits of payload — 6 bytes. not enough.

for v2: strings are Blob objects. they go to the nursery for
temporaries (intermediate string operations) and to LMDB for
persistent strings (names, source text, user data). the nursery
GC collects temporary strings aggressively.

if string allocation becomes a bottleneck: add a string interning
table (like the symbol table but for strings). most programs
create the same strings repeatedly. interning makes them free
after the first allocation.

## 3. the vau question, really

VISION.md says "vau is a compile-time construct by default." let's
be precise about what that means.

### the two faces of vau

**face 1: syntax extension.** `if`, `let`, `when`, `cond`, `match`,
`while`, `and`, `or` — these are defined as vau operatives in
bootstrap.moof. they receive unevaluated arguments and the caller's
environment. but they ALWAYS do the same thing: evaluate their
arguments in predictable patterns (conditional branches, sequential
bindings, short-circuit logic). they never inspect the environment.
they never do anything surprising.

these are macros. they should compile like macros.

**face 2: metaprogramming.** a vau that inspects the environment,
that does different things depending on what bindings exist, that
constructs new code at runtime, that implements a domain-specific
language by rewriting its arguments. THIS is what vau is for.
this is the reflective tower. this is the 3-Lisp heritage.

### the compilation strategy

the compiler maintains a **stability table**: a map from binding
names to their stability class.

```
stable:    bound at the top level, never reassigned, body is
           analyzable (doesn't use $env in surprising ways).
           → expanded at compile time.

unstable:  reassigned, or passed as a value, or explicitly
           marked dynamic.
           → compiled as a runtime operative call (slow path).

unknown:   not yet analyzed.
           → treated as unstable (conservative).
```

at compile time, when the compiler sees `(if cond then else)`:
1. look up `if` in the stability table → stable
2. the compiler KNOWS what `if` does (it analyzed the body)
3. emit: compile `cond`, JUMP_IF_FALSE, compile `then`, JUMP,
   compile `else`

when the compiler sees `(my-dsl x y z)` where `my-dsl` is unstable:
1. look up `my-dsl` → unstable
2. emit: push unevaluated `(x y z)` as a cons list, push current
   environment, CALL_OPERATIVE

the stability analysis runs once, at definition time. redefining
a stable binding invalidates all code that depended on it (rare,
but must be handled — insert a deoptimization check at call sites,
or just recompile).

### what about `(def fn ...)`?

`fn` is defined as a vau operative:

```scheme
(def fn (vau (params . body) $e
  (eval (cons 'lambda (cons params body)) $e)))
```

the stability analyzer sees: `fn` is a vau that always evals a
lambda form in the caller's environment. it never inspects `$e`
beyond passing it to `eval`. it always produces a lambda. this
is stable. the compiler can expand it at compile time: `(fn (x) body)`
compiles directly to a CLOSURE instruction.

### the `vau/dynamic` escape hatch

if you genuinely need a runtime operative:

```scheme
(def my-dsl (vau/dynamic (args) $env
  (if [[$env lookup: 'debug] = true]
    (do (print "DEBUG: " args) (eval args $env))
    (eval args $env))))
```

`vau/dynamic` tells the compiler "don't try to analyze this, it
really does depend on runtime state." the call site will be slow.
that's fine — this is the 5% case.

## 4. the browser in detail

### the object panel

```
┌─ Point (obj#47) ──────────────────────────┐
│                                            │
│  parent: Object (obj#0)         [click →]  │
│                                            │
│  slots:                                    │
│    x: 3                         [edit]     │
│    y: 4                         [edit]     │
│                                            │
│  handlers:                                 │
│    describe  → (fn () ...)      [source]   │
│    distanceTo: → (fn (other) ...)[source]  │
│    +         → (fn (other) ...) [source]   │
│                                            │
│  [+ handler]  [+ slot]  [eval in context]  │
└────────────────────────────────────────────┘
```

clicking a reference (like `parent: Object`) opens that object's
panel beside the current one. you build up a workspace of open
panels. the graph view shows the spatial relationships.

### the eval bar

at the bottom of the window. type an expression, press enter.
the result appears as a panel (if it's an object) or as inline
text (if it's a primitive). the eval bar has history (up arrow)
and completion (tab, using handler names from the focused object).

### the transcript

a scrolling log of all message sends in the current vat. each
entry shows: sender, receiver, selector, args, result, timestamp.
clickable — click a receiver to open its panel. filterable — show
only sends to a specific object, or only sends with a specific
selector.

### the agent panel

shows the agent's vat. its capabilities (what objects it can see).
its pending actions (proposed message sends awaiting approval).
its history (approved/denied actions). a text input for natural
language instructions.

when you type "add a magnitude handler to Point," the agent:
1. inspects Point's slots and handlers
2. constructs a handler: `(fn () [[@x * @x] + [@y * @y]])`
3. proposes: `[Point handle: 'magnitude with: <lambda>]`
4. the proposal appears in the approval queue
5. you click approve (or edit and approve, or deny)
6. the handler appears on Point's panel

## 5. the agent architecture

### the tool-use loop

```
loop {
    // 1. gather context: what objects does the agent see?
    let tools = agent_vat.capabilities
        .flat_map(|obj| obj.interface())  // selector → schema
        .collect();

    // 2. send tools + conversation to the LLM
    let response = llm.chat(messages, tools);

    // 3. execute tool calls as message sends
    for call in response.tool_calls {
        let result = fabric.send(
            call.receiver,
            call.selector,
            &call.args,
        );
        // membrane logs everything
        // if the membrane requires approval, pause here
    }
}
```

the agent's "tools" are derived from `interface` handlers on the
objects in its vat. every object that has an `interface` handler
automatically becomes a tool the agent can use. adding a new
capability to the agent = giving it a reference to a new object.

### the membrane

every message the agent sends goes through a membrane. the membrane
is an object with an `on-send:` handler. it can:

- **log**: record the send in the audit trail (always)
- **allow**: let the send through (for reads, safe operations)
- **require approval**: pause and put the send in the approval
  queue (for mutations, deletions, anything dangerous)
- **deny**: block the send entirely (for operations outside the
  agent's authorized scope)

the membrane's policy is just a moof object. you can inspect it,
edit it, swap it out. "make the agent read-only" = change the
membrane's policy to deny all mutations. "let the agent create
objects but not delete them" = allow `handle:with:` and
`slotAt:put:`, deny `remove:`.

### agent memory

the agent's memory is objects in its vat. when the agent learns
something, it creates an object:

```scheme
{ Memory
  topic: "the user prefers snake_case"
  confidence: 0.9
  created: 1712534400
  source: "explicit instruction" }
```

these memory objects persist in LMDB. the agent can query them.
you can inspect them in the browser. you can delete ones you
don't like. memory is just data — visible, editable, deletable.

## 6. what "living image" means for development workflow

### the daily loop

1. open moof. the browser shows your objectspace.
2. you see where you left off — objects, handlers, data.
3. you tell the agent what you want to build next.
4. the agent proposes changes. you approve, edit, or deny.
5. you test in the eval bar. you inspect results in the browser.
6. you close moof. done. there was no save.

### version control

the LMDB file IS the image. `git` can't diff it meaningfully.
but objects can version themselves:

- every mutation through a membrane can record the old value
- `[obj history]` returns a list of `{old, new, timestamp, author}`
- "undo" = restore the old value from history

for sharing: `[obj export]` produces a self-contained s-expression
that reconstructs the object. import by evaluating it. this is
the round-trip path — not files, but expressions.

for collaboration: federation. your image connects to mine. i give
you a faceted reference to an object. you send messages to it.
the messages go over the network. the object lives in my image.
(this is future work, but the architecture enables it.)

### debugging

when something goes wrong, you don't read a stack trace. you look
at the transcript — the log of message sends. you click on the
failing send. you see the receiver, the selector, the args, the
error. you click on the receiver. you see its state at the time
of the error (if the membrane recorded it). you fix the handler.
you re-send.

this is smalltalk's "fix and proceed" adapted for a message-passing
world. the error doesn't crash the image. it just fails a send.
you fix it and try again.

## 7. the six-week plan (realistic)

### week 1: fabric + store

- NaN-boxed Value (done in v1 skeleton, proven correct)
- LMDB store: create/read/update objects, symbols, cons, blobs
- nursery arena for hot mutable state
- send() with handler lookup + delegation chain
- HandlerInvoker trait + NativeInvoker
- type prototypes for Integer, Float, String (native handlers)
- tests: object CRUD, dispatch, persistence across restarts

### week 2: language

- lexer (port from v1)
- parser (port from v1)
- stability analyzer (new — classify vau call sites)
- register compiler (new — emit register bytecode)
- BytecodeInvoker (new — register VM)
- bootstrap.moof (port from v1, adjust for new compiler)
- test: `[2 + 3]` → `5`, `(def Point { Object x: 3 })` works

### week 3: browser (core)

- egui window with object panel layout
- render an object: slots, handlers, parent link
- navigate: click a reference → open that object's panel
- edit: change a slot value inline
- eval bar: type expression → parse → compile → eval → show result
- transcript: log message sends, show in a panel

### week 4: browser (polish) + agent (scaffold)

- graph view: objects as nodes, references as edges
- keyboard navigation (vim-style? or just tab/arrow)
- agent vat: create a vat with faceted capabilities
- agent tool derivation: interface handlers → LLM tools
- agent loop: send tools + conversation → execute tool calls
- membrane: log all agent sends, require approval for mutations

### week 5: agent (real) + standard library

- approval queue in the browser UI
- agent memory objects
- natural language input in the agent panel
- standard library: collections, json, string methods
- module organization (modules are just objects, not a graph solver)

### week 6: integration + dog-fooding

- use moof for something real (notes? code sketches? data exploration?)
- MCP server (so claude desktop can connect)
- fix everything that breaks
- write documentation as objects in the image

---

## appendix: prior art that matters for v2

| system | what we steal for v2 specifically |
|---|---|
| Self (1991) | the Morphic UI — objects as visual things you click on |
| Squeak (1996) | fix-and-proceed debugging, the browser as primary IDE |
| Hypercard (1987) | non-programmers building things by pointing and clicking |
| Notion/Obsidian | the feeling of "my stuff is just here when I open it" |
| Cursor/Claude Code | AI as a participant in the creative process, not a chatbot |
| LMDB (Symas) | the insight that mmap IS the persistence layer |
| E language (Mark Miller) | vat/membrane/facet, done right, applied to AI agents |

---

*this document will evolve as we build. the VISION says what.
this document says how and why.*
