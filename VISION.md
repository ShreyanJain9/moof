# moof v2: the real redesign

> what if we actually meant "not a programming language"?

---

## the diagnosis

v1 said "moof is not a programming language" and then built a programming
language. a good one! bytecode VM, operatives, prototype objects, module
system. but at the end of the day you type s-expressions into a REPL and
get values back. that's a programming language.

the design doc's own section 10 describes Browser, Inspector, Workspace,
Transcript, Debugger — a full live environment where objects are
clickable, editable, wirable. none of that exists. what exists is a
text REPL. the gap between the vision and the artifact is the actual
problem, not the implementation quality.

so the question isn't "how do we build a better VM." the question is:
**what does it feel like to live inside a moof image?**

---

## the thesis

moof is a **place**. not a language, not a tool, not an app.

you open moof and you're *somewhere*. there are objects around you.
you can look at them, touch them, talk to them. some of them are
data (a number, a string, a list). some of them are behavior (a
handler, a method, a rule). some of them are alive (an agent, a
process, a subscription). the boundary between these categories is
blurry on purpose.

the keyboard is one way in. but it's not the primary way. the
primary way is *pointing and speaking* — clicking on objects in a
spatial browser, and telling an AI agent what you want. the REPL
is the power-user escape hatch, not the front door.

this changes everything about the implementation priorities.

---

## what changes

### 1. the browser is the primary interface, not the REPL

v1 built the VM first and treated the browser as a nice-to-have.
v2 inverts this: **the browser is the first thing that works.**

the browser shows the objectspace as a spatial graph. objects are
nodes. references are edges. you click an object, you see its
slots and handlers. you click a handler, you see its source. you
can edit a slot value inline. you can drag a reference from one
object to another.

this means the browser isn't an afterthought bolted onto the VM.
the browser IS the environment. the VM exists to make the browser's
objects come alive.

implementation: egui. not a TUI, not a web app. a native window
that feels like a tool you reach for, not a website you visit.

### 2. the agent is a co-inhabitant, not an API client

v1 had an MCP server that let an AI poke at the image from outside.
that's the wrong topology. the agent should be *inside* the image,
in its own vat, with its own objects, seeing the same objectspace
you see in the browser.

when you're looking at an object in the browser and you say "make
this sortable," the agent sees the same object, creates a handler,
and you watch it appear in real time. the agent's work is visible,
auditable, and reversible because it's just objects in the image.

the agent isn't called via MCP. the agent IS an object. it responds
to messages. it has a mailbox. it has capabilities (faceted
references). you can inspect the agent's memory, its pending
actions, its history — because those are objects too.

implementation: the agent's vat runs an LLM tool-use loop. its
"tools" are the handlers on the objects it holds references to.
every tool call is a message send. every message send goes through
the membrane. the membrane can log, rate-limit, require approval,
or deny. this is capability security doing real work.

### 3. persistence is structural, not serialized

v1 agonized over persistence: binary snapshots? source files?
WAL? four rewrites. the core issue: treating the heap as a thing
that needs to be "saved" — as if it were a document.

v2 approach: **the heap IS the database.** not "backed by a
database" — IS one. every mutation is a transaction. there is no
"save" operation because there is no unsaved state. if the process
crashes, you lose nothing. if the power goes out, you lose nothing.

this isn't LMDB as a storage backend for a traditional heap. this
is the heap *being* an LMDB environment. objects don't live in
memory and get flushed to disk. objects live in the memory-mapped
file and get paged into RAM by the OS on demand.

the implication: startup is instantaneous (mmap, done). there is
no bootstrap. there is no "loading modules." the objects are already
there. the first time you start, you build the image; after that,
you just open it.

### 4. vau gets a reality check

vau is beautiful. vau is also an optimization cliff. every call
site that might receive unevaluated arguments is a call site the
compiler can't reason about. the v1 design doc waves at "inline
caching and operative specialization at the JIT level" but that
JIT will take person-years to build and might never happen.

here's the uncomfortable truth: 95% of operative usage is `if`,
`let`, `when`, `unless`, `cond`, `match`, `while` — forms that
are defined once and never redefined. they don't NEED to be
operatives at runtime. they need to be operatives at DESIGN time
(so the language stays minimal and the user has compiler-level
power) but they can compile to normal control flow.

v2 approach: **vau is a compile-time construct by default.**

the compiler knows which operatives are "stable" (bound at the
top level, never reassigned). stable operatives are expanded at
compile time, like macros — but they're defined with vau syntax,
so they have full access to the AST and the environment. the
result is normal bytecode: branches, loops, calls.

for the rare case where you actually want a runtime operative
(metaprogramming, DSL construction, reflective tower), you mark
it `(vau/dynamic ...)`. this tells the compiler "yes, really,
reify the environment at this call site." this is the slow path.
it's there when you need it. you almost never need it.

this means `(if cond then else)` compiles to a conditional jump,
not a function call. `(let ((x 1)) body)` compiles to a local
binding, not an environment allocation. the 95% case is fast.
the 5% case is possible.

### 5. the object model simplifies

v1 has: Object, Cons, String, Bytes, Environment, Lambda,
Operative, BytecodeChunk, NativeFunction. nine heap object types.
that's too many. most of them are just Objects with specific slots.

v2 has three heap types:

```
Object  { parent, slots, handlers }
Cons    { car, cdr }
Blob    { bytes }
```

that's it. strings are Blobs with a type tag. bytecode is a Blob.
environments are Objects (bindings are slots, parent env is parent).
lambdas are Objects (code is a Blob slot, params is a slot).
native functions are Objects (name is a symbol slot).

the dispatch system doesn't need to know about any of this. it
sees handlers (values in the handler table) and asks the registered
invokers "can you run this?" the bytecode invoker knows that a
handler pointing to an Object with a `code` slot is a lambda. the
native invoker knows that a handler pointing to a symbol is a
native. they coexist.

### 6. the wire protocol is MCP

v1 defined a custom binary wire protocol. why? MCP already exists.
it's JSON-RPC over stdio or HTTP. every AI tool already speaks it.

v2's external protocol IS MCP. the fabric speaks MCP natively.
when a remote client connects, it gets an MCP session. `tools/list`
returns the handlers on the objects in its vat. `tools/call` is a
message send. `resources/list` returns objects with `describe`.
`resources/read` returns slot data.

this means: every moof image is an MCP server out of the box.
claude desktop can connect to it. any MCP client can connect to it.
the protocol for "AI agent talks to moof" and "remote moof talks to
moof" and "custom app talks to moof" is the same protocol.

for in-process communication (the browser, the REPL), we skip the
JSON serialization and call directly. but the API surface is the
same: send a message, get a result.

### 7. the surface syntax is just one frontend

v1's syntax is good. keep it. `()` `[]` `{}`, keywords, dot
access, `@self`, `'quote`. it's readable and it's elegant.

but the syntax is not the environment. the syntax is one way to
produce ASTs. the browser is another (click "add handler," type
the body). the agent is another (it constructs ASTs from tool
calls). a hypothetical lua or python frontend is another.

v2 makes this real by putting the parser in the `lang` crate and
keeping it out of the `fabric` and `shell` crates entirely. the
browser and the agent don't parse s-expressions. they construct
objects directly.

---

## the new architecture

```
                         moof process
 ┌──────────────────────────────────────────────────────────┐
 │                                                          │
 │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
 │  │  browser      │  │  repl        │  │  mcp server  │   │
 │  │  (egui)       │  │  (readline)  │  │  (stdio/tcp) │   │
 │  │               │  │              │  │              │   │
 │  │  spatial graph │  │  text in/out │  │  json-rpc    │   │
 │  │  click-edit   │  │  parse-eval  │  │  tool calls  │   │
 │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘   │
 │         │                 │                 │            │
 │         ▼                 ▼                 ▼            │
 │  ┌─────────────────────────────────────────────────┐     │
 │  │               moof-lang                          │     │
 │  │  lexer -> parser -> analyzer -> compiler -> vm   │     │
 │  │                                                   │     │
 │  │  the moof language. one frontend among many.      │     │
 │  │  registers BytecodeInvoker with the fabric.       │     │
 │  └─────────────────────┬───────────────────────────┘     │
 │                        │                                  │
 │                        ▼                                  │
 │  ┌─────────────────────────────────────────────────┐     │
 │  │               moof-fabric                        │     │
 │  │                                                   │     │
 │  │  the substrate. language-agnostic.                │     │
 │  │                                                   │     │
 │  │  ┌─────────┐ ┌─────────┐ ┌───────┐ ┌─────────┐  │     │
 │  │  │ objects  │ │  send   │ │ vats  │ │  store  │  │     │
 │  │  │ (3 types)│ │ dispatch│ │ sched │ │ (LMDB)  │  │     │
 │  │  └─────────┘ └─────────┘ └───────┘ └─────────┘  │     │
 │  │                                                   │     │
 │  │  no bytecode. no ASTs. no syntax. no language.    │     │
 │  └───────────────────────────────────────────────────┘     │
 │                                                          │
 └──────────────────────────────────────────────────────────┘
```

### the fabric (~800 lines)

three object types. one operation (send). one store (LMDB).
one scheduler (vats + mailboxes). one extension point
(HandlerInvoker trait).

```rust
enum HeapObject {
    Object { parent: Value, slots: Vec<(u32, Value)>, handlers: Vec<(u32, Value)> },
    Cons { car: Value, cdr: Value },
    Blob(Vec<u8>),
}
```

values are NaN-boxed: 8 bytes, zero allocation for nil/bool/int/
float/symbol/objref. the store is LMDB. persistence is free —
every mutation is a transaction.

### the language (~1500 lines)

lexer, parser, analyzer, compiler, bytecode invoker. the analyzer
is the key new piece: it classifies vau call sites as static or
dynamic, enabling real compilation of control flow.

register-based bytecode. de bruijn indices for environment access
(no runtime name lookup). inline caches on send sites.

### the browser (~1500 lines, the big new piece)

egui application. shows the objectspace as a navigable graph.
objects are panels. slots and handlers are rows. references are
clickable links. values are inline-editable.

features:
- **object inspector**: slots, handlers, parent chain, history
- **graph view**: objects as nodes, references as edges
- **eval bar**: type an expression, see the result
- **agent panel**: the AI agent's view, pending actions, approvals
- **transcript**: log of all message sends, filterable

the browser reads the LMDB store directly (concurrent reader).
mutations go through the fabric's send path. the browser never
blocks the VM.

### the agent (~500 lines of glue)

an LLM tool-use loop running in its own vat. its tools are
derived from the `interface` handlers on the objects it can see.
every tool call is a message send. the membrane logs everything.

the agent doesn't have its own protocol. it uses the same send()
as everything else. the difference is: its vat has faceted
references (read-only, or requiring human approval for writes).

### the shell (~300 lines)

readline REPL. parse, compile, send, print. the simplest frontend.
also: MCP stdio server for external AI clients.

---

## the bootstrap story

1. first run: empty LMDB. fabric creates the root object.
2. moof-lang registers its BytecodeInvoker.
3. moof-lang evaluates `lib/bootstrap.moof`: defines the six
   kernel forms, derives lambda/if/let/etc, creates type
   prototypes (Integer, String, etc), creates the standard
   library.
4. LMDB commits. bootstrap complete. image exists.
5. every subsequent run: open LMDB. re-register invokers (they're
   rust closures, can't persist). done. no parsing, no compiling.
   the objects are already there.

step 3 takes maybe 200ms. step 5 takes <1ms.

---

## what this feels like

you run `moof` for the first time. a window opens. it shows a
mostly-empty space with a few foundational objects: Object, Integer,
String, Cons, Modules. you click on Integer. you see its handlers:
`+`, `-`, `*`, `/`, `describe`, `asString`, `asFloat`. you click
on `+`. you see its source.

there's an eval bar at the bottom. you type `{ Point x: 3 y: 4 }`.
a new object appears in the graph. you click on it. you see slots
`x: 3`, `y: 4` and parent `Object`. you right-click and select
"add handler." you type `describe` for the selector and
`(str "(" @x ", " @y ")")` for the body. the handler appears.

there's an agent panel on the right. you type "make Point respond
to distanceTo: another point." the agent constructs the handler,
you see it appear in the approval queue, you click approve, and
it's on the object. you test it in the eval bar:
`[pt distanceTo: { Point x: 0 y: 0 }]`. the result appears.

you close the window. you open it tomorrow. everything is exactly
where you left it. there was no save. there was no load. it's just
*there*.

that's what "not a programming language" means.

---

## the uncomfortable questions

### isn't this just smalltalk?

yes and no. smalltalk had the image, the browser, the inspector,
the live environment. moof steals all of that shamelessly.
the differences:

1. **vau instead of classes.** smalltalk's class/metaclass hierarchy
   is replaced by prototypes + operatives. simpler, more uniform,
   and gives user code compiler-level power.

2. **capability security.** smalltalk images are wide open. any
   object can reach any other. moof's vat/membrane/facet model
   means the agent can be a real participant without being a
   security nightmare.

3. **AI-native.** the agent isn't bolted on. it's a first-class
   co-inhabitant of the objectspace. smalltalk had nothing like
   this because it was built in 1980.

4. **persistence is transactional.** smalltalk images are fragile
   binary snapshots. moof's LMDB store is ACID. crash-safe.
   concurrent-reader-safe.

### isn't egui too limited for a real environment?

maybe. but it's the right trade: native, fast, cross-platform, and
we can ship something in weeks instead of months. if egui hits a
wall, the fabric doesn't care — the browser is a frontend, not a
kernel component.

### can you really build this?

the fabric is ~800 lines. the language is ~1500 lines (v1 proved
this works). the browser is ~1500 lines of egui (this is the new
work). the agent glue is ~500 lines.

total: ~4300 lines. v1 was ~5500. this is smaller AND more
ambitious because the browser replaces mountains of REPL command
infrastructure.

### what about the web?

not yet. egui can compile to wasm+webgl. when the desktop version
works, the web version is a recompile, not a rewrite. but the
desktop is first because latency matters for a live environment.

### what about federation?

the fabric's vat model + MCP protocol means "connect to a remote
moof image and send messages to its objects" is architecturally
trivial. an object in a remote vat looks like a local object with
a network transport. this is future work but the architecture
doesn't foreclose it.

---

## implementation order

### phase 1: the fabric (week 1)

NaN-boxed values. LMDB store. three heap object types. send()
with delegation. HandlerInvoker trait. NativeInvoker. basic vats.

deliverable: `cargo test` passes. you can create objects, set
slots and handlers, dispatch messages, and persist across restarts.

### phase 2: the language (week 2)

port the lexer and parser from v1 (they're solid). write the
analyzer (vau classification). write the register compiler. write
the VM. bootstrap the kernel.

deliverable: you can evaluate `[2 + 3]` and get `5`. the full
bootstrap runs. type prototypes work.

### phase 3: the browser (weeks 3-4)

egui window. object inspector. graph view. eval bar. live updates
from the LMDB store.

deliverable: you can click through the objectspace, inspect
objects, edit slot values, evaluate expressions, and see changes
reflected immediately.

### phase 4: the agent (week 5)

LLM tool-use loop in a vat. faceted references. membrane logging.
approval queue in the browser.

deliverable: you can tell the agent "define a handler on this
object" and watch it happen, with approval.

### phase 5: the living image (week 6+)

workspace persistence. module organization (in moof, not rust).
standard library. MCP server. documentation-as-objects.

deliverable: you can use moof as a daily tool for something real
— notes, code sketches, data exploration, agent-assisted
programming.

---

## what we're NOT building (yet)

- JIT compiler (register bytecode is designed for it but it's later)
- federation (architecture supports it, implementation is later)
- CRDTs (ditto)
- web interface (egui to wasm is a future recompile)
- multiple language frontends (the fabric supports it but one
  language is enough for now)

## what we're killing from v1

- the module system as a rust-side graph solver (modules are just
  moof objects with ordered definition lists)
- the TUI inspector (the egui browser replaces it)
- the binary wire protocol (MCP replaces it)
- source projection (the image is the only artifact)
- `__dunder` VM escape hatches (native handlers through the
  proper HandlerInvoker path)
- the custom persistence stack (LMDB replaces snapshot/WAL/GC)

---

*v1 was a programming language that wanted to be an environment.
v2 is an environment that happens to have a programming language
in it.*

*clarus lives. moof.*
