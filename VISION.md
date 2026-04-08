# moof v2

> a persistent, concurrent objectspace with capability security
> and a lisp-shaped surface syntax.

---

## what moof is

moof is a runtime. like the BEAM is erlang's runtime, moof is
moof's runtime. it's not a "language-agnostic substrate" and it's
not a "pluggable fabric." it knows what it is. it knows how to
run code. the surface syntax is separate — but the computational
model (objects, messaging, vats, operatives, persistence) is the
runtime. inseparable.

the things that make moof moof:

1. **everything is an object.** integers, strings, booleans,
   functions, environments, errors, the module system, the agent.
   objects have slots (public data, fixed at creation) and handlers
   (public behavior, extensible after creation). handlers delegate
   through prototype chains. slots don't.

2. **the only operation is send.** `[obj selector: arg]` is the
   one thing the VM does. function calls, arithmetic, slot access,
   control flow — all message sends. `(f x)` is `[f call: x]`.
   `[3 + 4]` is a send to an integer with selector `+`.
   `obj.x` is `[obj slotAt: 'x]` — even slot access is a send.

3. **vats are the unit of concurrency.** a vat is a single-threaded
   event loop that owns a set of objects. within a vat, sends are
   synchronous. across vats, sends are asynchronous and return
   promises. this is E's model. this is erlang's model. no shared
   mutable state. no locks. ever.

4. **a reference is a capability.** if you hold a reference to an
   object, you can send it messages. if you don't, the object
   doesn't exist in your world. there is no global namespace, no
   ambient authority, no way to conjure a reference from a name.
   this is E's capability model, applied everywhere.

5. **the image persists.** when you define an object, it survives
   restarts. there is no "save." there is no "load." the objects
   are just there. the image is the program is the data is the
   state.

6. **vau gives user code compiler power.** an operative receives
   its arguments unevaluated and the caller's environment as a
   first-class value. `if`, `let`, `while`, `match` are library
   functions, not special forms. the user has the same expressive
   power as the compiler.

---

## the debts

every design choice comes from somewhere. these are the systems
moof is stealing from and what specifically it takes:

### erlang / BEAM

the most important influence on v2.

- **processes everywhere.** in erlang, everything runs in a
  process. processes are cheap (thousands, millions). they
  communicate by message passing. they don't share memory. if
  one crashes, the others don't.

  in moof: vats are processes. objects live in vats. cross-vat
  sends are async messages. a vat crash is isolated. the
  scheduler is preemptive (fuel-based, like BEAM's reductions).

- **let it crash.** erlang doesn't try to prevent errors. it
  lets processes crash and restarts them from a known-good state.
  supervisors monitor processes and restart them on failure.

  in moof: `doesNotUnderstand:` is not a bug — it's a message.
  a vat can crash without taking down the image. supervisor
  objects can monitor vats and restart them. the image persists
  through crashes.

- **hot code swapping.** in erlang, you can replace a module's
  code while the system is running. the old code keeps serving
  existing calls; new calls use the new code.

  in moof: handlers are just slots on prototype objects. change
  a handler, and the next send uses the new code. no restart.
  the image is always live.

- **distribution is transparent.** in erlang, sending a message
  to a process on another node looks the same as sending to a
  local process. the runtime handles serialization.

  in moof: a reference to an object in a remote image looks
  like a local reference. send goes over the network. the vat
  model already has async messaging — remote is just "more async."

### E language (mark miller)

the capability security model and the concurrency model.

- **near refs and far refs.** a near ref is to an object in your
  vat — sends are synchronous, immediate. a far ref is to an
  object in another vat — sends are asynchronous, return promises.

  in moof: same-vat sends use `[obj selector: arg]` (synchronous).
  cross-vat sends use `[obj <- selector: arg]` (eventual, returns
  a promise). the syntax makes the distinction visible.

- **promise pipelining.** in E, `x <- foo() <- bar()` sends
  `bar` to the *promise* of `x.foo()`, not to the resolved value.
  the messages queue up and execute in order when the promise
  resolves. this avoids round-trip latency.

  in moof: `[[obj <- foo] <- bar: x]` pipelines. the second send
  attaches to the promise of the first. the scheduler delivers
  them in order.

- **membranes.** a membrane wraps an object graph and intercepts
  all sends crossing the boundary. used for: logging, access
  control, revocation, attenuation.

  in moof: membranes are objects with `on-send:` handlers. wrap
  any object. intercept any message. log, allow, deny, transform.
  the agent lives behind a membrane. always.

- **facets.** a facet is a restricted view of an object that
  exposes only named selectors. `[obj facet: '(read: list:)]`
  gives you a read-only view.

  in moof: facets compose with membranes. the agent gets faceted
  references wrapped in a logging membrane. it can do exactly
  what the facets allow, and every action is recorded.

### haskell

not the type system. the thinking about effects.

- **effects are visible.** in haskell, a function that does IO
  has `IO` in its type. you can't accidentally do IO in pure code.
  effects are tracked, not hidden.

  in moof: IO is a capability. if your vat doesn't hold a
  reference to the Console or Filesystem object, you can't do IO.
  period. capabilities are haskell's IO monad made concrete — not
  a type-level constraint but an object-level one. you physically
  cannot print unless someone gave you the printer.

- **laziness.** haskell evaluates expressions only when needed.
  infinite data structures are fine.

  in moof: streams. `[integers from: 1]` returns a lazy infinite
  stream. `[[integers from: 1] take: 10]` materializes the first
  10. streams are objects with a `next` handler that computes on
  demand. not pervasive laziness (that's too hard to reason about)
  but explicit lazy objects where you want them.

- **pattern matching.** haskell's case expressions destructure
  data cleanly.

  in moof: `match` as a derived form from vau. pattern matching
  on message arguments, on object structure, on type. this is
  where vau earns its keep — `match` is a library function that
  inspects its arguments unevaluated and compiles to efficient
  dispatch.

### ruby

the vibes.

- **everything is an object.** `3.times { |i| puts i }` — the
  integer 3 is an object, `times` is a method, the block is a
  closure. no primitives. no special cases.

  moof takes this literally. `[3 times: { :i [Console println: i] }]`
  is a message send to an integer. the integer's `times:` handler
  iterates and calls the block. no special syntax. no special case.

- **blocks.** ruby's blocks are closures you pass to methods.
  `array.each { |x| process(x) }`. the method receives the block
  and calls it when ready.

  in moof: blocks are objects with a `call:` handler. `{ :x [x + 1] }`
  is an object. you pass it as an argument. the receiving handler
  sends `call:` to it. blocks close over their environment.

- **method_missing.** in ruby, if an object doesn't have a method,
  `method_missing` is called. you can override it. proxies,
  delegation, DSLs — all built on this.

  in moof: `doesNotUnderstand:`. same idea. when handler lookup
  fails (including the full delegation chain), the receiver gets
  a `doesNotUnderstand:` message with the selector and args. the
  default handler raises an error. override it for proxies,
  forwarding, dynamic dispatch, whatever.

- **open classes.** in ruby, you can reopen any class and add
  methods. `class Integer; def prime?; ...; end; end`.

  in moof: handlers are slots on prototype objects. add a handler
  to Integer's prototype and every integer gains that behavior.
  `[Integer handle: 'prime? with: (fn (self) ...)]`. no ceremony.
  the image is always malleable.

### self

the object model and the live environment.

- **prototypes, not classes.** objects delegate to other objects.
  no class/metaclass distinction. create an object by cloning
  another and modifying it.

  in moof: same. `{ Point x: 3 y: 4 }` creates an object with
  Point as parent. Point delegates to Object. no classes anywhere.

- **the live environment.** self had an IDE where objects were
  visual things you could click on, inspect, modify. the
  environment was the language was the IDE.

  in moof: the browser. objects are panels. click, inspect, edit.
  the agent lives there too.

---

## the runtime

### values

NaN-boxed, 8 bytes each. no heap allocation for common values.

```
nil, true, false          — singleton tags
integer (48-bit signed)   — inline in the NaN payload
float (64-bit)            — the non-NaN bit patterns
symbol (32-bit interned)  — inline in the NaN payload
object reference (32-bit) — inline in the NaN payload
```

everything that isn't an object is a tagged immediate. this means
arithmetic never allocates. comparisons never allocate. symbol
lookup never allocates.

### objects

three kinds of heap entity:

```
Object  { parent: Value, slots: [(sym, val)], handlers: [(sym, val)] }
Cons    { car: Value, cdr: Value }
Blob    { tag: u8, bytes: [u8] }
```

**Object**: the universal container. parent enables delegation.

- **slots are public data.** anyone with a reference can read
  any slot via `[obj slotAt: 'x]` (or the sugar `obj.x`).
  slots are writable via `[obj slotAt: 'x put: v]`.

- **slots are fixed at creation.** `{ Point x: 3 y: 4 }` creates
  an object with exactly two slots: `x` and `y`. you cannot add
  a slot `z` later. the *shape* of an object is immutable. only
  the *values* in those slots can change.

- **handlers are extensible.** you can add handlers to any object
  at any time. `[pt handle: 'magnitude with: (fn () ...)]` adds
  a new handler. handlers delegate through the parent chain.

this split is the core of the object model:

```
slots    = data.     public. fixed set. values mutable.
handlers = behavior. public. open set.  delegated via parent.
```

**why fixed slots?** four reasons:

1. **shapes are known.** the VM can compute slot offsets at
   creation time. `obj.x` is an array index, not a hash lookup.
   this is what V8's hidden classes buy you, but we get it for
   free from the language semantics.

2. **objects are self-describing.** an object's slot names are
   part of its identity. `{ x: 3 y: 4 }` and `{ x: 3 z: 4 }`
   are different shapes. pattern matching on shapes is natural:
   `(match obj ({ x: _ y: _ } ...) ({ r: _ theta: _ } ...))`.

3. **serialization is trivial.** the shape is known, the slot
   count is fixed. write the values in order. no dynamic field
   discovery.

4. **reasoning is possible.** if slots can't appear or disappear,
   you can look at an object and know what data it has. no
   spooky action at a distance. no `addSlot:` in a handler
   somewhere silently changing an object's shape.

**handlers are extensible because behavior should be open.** you
should be able to say "all Points can now compute magnitude" by
adding a handler to the Point prototype. this is ruby's open
classes. this is the whole point of a live environment — you
evolve behavior without restarting.

**an environment is an Object.** bindings are slots (fixed at
scope creation — a `let` with 3 bindings creates an object with
3 slots). parent env is the parent. de bruijn indices become
slot offsets. this is clean and it means the VM never does name
lookup at runtime.

**a lambda is an Object.** code blob in a slot, params in a slot,
captured env reference in a slot. there is no separate Lambda heap
type — it's just an Object with a `call:` handler.

**Cons**: pairs. the AST is cons lists. argument lists are cons
lists. everything sequential is cons cells.

**Blob**: opaque bytes with a type tag. strings (tag 0), bytecode
chunks (tag 1), raw bytes (tag 2). the VM knows how to interpret
bytecode blobs. everything else is opaque data.

### send

the heart. the only operation.

```
send(receiver, selector, args) → result

1. if receiver is an object:
   a. look in receiver's handler table for selector
   b. if not found, recurse on receiver's parent (delegation)
   c. depth limit: 256 levels

2. if receiver is a primitive (int, float, symbol, bool, nil):
   a. look in the type prototype's handler table
   b. Integer prototype has +, -, *, /, etc.
   c. String prototype has length, at:, etc.

3. if handler found:
   a. if handler is a bytecode object → execute in VM
   b. if handler is a native → call the rust closure
   c. handler receives (self, args...)

4. if no handler found:
   a. send doesNotUnderstand: to receiver with selector + args
   b. default doesNotUnderstand: raises an error
```

there is no "HandlerInvoker trait." the VM knows what bytecode is
and knows what native closures are. those are the two kinds of
handler. if we ever need a third kind (wasm? python?), we add it
to the VM. the VM is not a plugin host — it's the runtime.

**slot access is a send.** `obj.x` desugars to `[obj slotAt: 'x]`.
the default `slotAt:` handler on Object does a direct offset read
(fast path). but because it's a send, membranes can intercept it.
a faceted reference can deny slot reads. the capability model is
complete — there is no back door around it.

the VM optimizes the common case: if the receiver is a plain object
(no membrane, no custom `slotAt:`), slot access compiles to a
direct offset read. the optimization is invisible. the semantics
are always "it's a send."

### the VM

register-based bytecode. the interpreter is a loop over a flat
instruction array. each instruction is 4 bytes: opcode + 3 operands.

the key opcodes:

```
LOAD_CONST    r, const    — load a constant into register r
LOAD_LOCAL    r, depth, slot — load from enclosing scope (de bruijn)
STORE_LOCAL   depth, slot, r — store into enclosing scope
SEND          dst, recv, sel, nargs — message send
CALL          dst, func, nargs — applicative call (sugar for send call:)
TAIL_CALL     func, nargs  — replace current frame
JUMP          offset       — unconditional
JUMP_FALSE    r, offset    — conditional
MAKE_OBJECT   dst, parent  — create object
SET_SLOT      obj, sym, val
SET_HANDLER   obj, sym, val
CLOSURE       dst, code    — capture current environment
RETURN        r
```

environments are Objects on the heap. closures capture a reference
to the defining environment's Object. de bruijn indices mean the
compiler resolves variable names at compile time — the VM never
does name lookup.

**tail calls are real.** `TAIL_CALL` replaces the current frame.
recursive loops don't grow the stack. this is not optional — it's
how `while`, `loop`, `map`, `fold` work without stack overflow.

### vats

every vat is a single-threaded event loop. objects belong to
exactly one vat. sends within a vat are synchronous (direct call).
sends across vats are eventual (message queued to target vat's
mailbox, returns a promise).

```
Vat {
    id: u32,
    objects: Set<ObjectId>,  — which objects live here
    mailbox: Queue<Message>, — pending incoming messages
    capabilities: [Value],   — faceted refs given at creation
    fuel: u32,               — reductions before preemption
}
```

the scheduler runs vats round-robin with fuel-based preemption
(like BEAM's reduction counting). each vat gets N sends per tick.
when fuel runs out, the scheduler moves to the next vat. this
means a runaway computation in one vat doesn't starve others.

**spawn.** `(spawn (fn () ...))` creates a new vat, runs the
function in it, returns a far reference. the new vat has only
the capabilities explicitly passed to it.

**eventual sends.** `[obj <- selector: arg]` enqueues a message
on the target's vat and returns a promise. the promise resolves
when the message is processed.

**promise pipelining.** `[[obj <- foo] <- bar: x]` sends `bar:`
to the promise of `foo`. when `foo` resolves, `bar:` is delivered
to the result. no explicit `.then()` chaining.

### vau and compilation

vau is a feature of the language, compiled by the compiler,
executed by the VM. it's not a mystery to the runtime.

the compiler classifies every operative call site:

**static operatives** — the binding is top-level, never reassigned,
and the body follows a known pattern (evaluate some args, branch,
bind, loop). the compiler expands these at compile time. `if`
becomes a conditional jump. `let` becomes local bindings. `while`
becomes a loop. `fn` becomes a closure. 95% of vau usage.

**dynamic operatives** — the binding could change, or the body
does genuinely dynamic things with the captured environment. the
compiler emits a full operative call: push unevaluated args as a
cons list, push the caller's environment object, call the
operative. 5% of usage.

the `vau` form itself always works — you can always write
`(vau (args) $env body)` and it will do the right thing. the
optimization is transparent. you don't need to annotate anything.
the compiler figures out which operatives are static by analyzing
the code.

(for the rare case where the compiler gets it wrong — an operative
that looks static but actually depends on runtime environment
state — the compiler inserts a guard check. if the guard fails,
it falls back to the dynamic path. this is speculative
optimization, same as JIT inline caches.)

### persistence

the image persists. LMDB.

v1 rewrote persistence four times because it kept trying to defer
the decision. the lesson isn't "start simple" — it's "commit to
something and stop rewriting." LMDB is the right something:

- **instant startup.** `mmap()`. done. no deserialization. the
  first `send()` touches the pages it needs; the OS loads them
  on demand.

- **crash safety.** ACID transactions. power loss loses nothing.
  no WAL to build, no corruption to handle, no recovery logic.
  LMDB does all of this.

- **concurrent readers.** the browser can read the object graph
  while the VM is mutating it. the agent can read while the
  REPL is running. readers never block writers. this is a hard
  requirement for a live environment with a browser — you can't
  lock the heap every time you repaint a panel.

- **no custom serialization.** LMDB stores bytes. we store
  bincode-serialized objects. the serialization format is simple
  because object shapes are fixed (the slot set is known at
  creation, we just serialize the values in order).

**the mutation cost.** LMDB writes are ~1μs each. that's fine for
persistent state (user objects, handlers, definitions). it's NOT
fine for VM temporaries (environments, stack frames, intermediate
values). solution: **the nursery.**

hot mutable state lives in a traditional in-memory arena — a
`Vec<HeapObject>` indexed by u32. environments created by `let`,
`fn`, etc. go here. they're fast. they're not persistent. they
don't need to be.

persistent state lives in LMDB. user-created objects, prototype
handlers, module definitions. these are created infrequently and
read often — the perfect LMDB workload.

the Value type distinguishes the two:

```
tag 5 = store object  (LMDB, persistent, crash-safe)
tag 6 = nursery object (in-memory, ephemeral, fast)
```

`send()` checks the tag and reads from the right place. promotion
from nursery to store happens when an object is "anchored" —
assigned to a slot on a store object, explicitly persisted, or
returned from a vat's turn.

this is a generational scheme. gen0 = nursery (RAM). gen1 = store
(LMDB). "GC" of the nursery = at the end of a vat turn, anything
not reachable from a store root is dead. simple. fast. no pause.

### the browser

egui. objects as panels. click to navigate. edit slots inline.
eval bar at the bottom. transcript of message sends.

the browser is a vat. it holds references to the objects it's
showing. it sends messages to read slot values and handler names.
it's not special — it's a normal vat with read-mostly capabilities.

the browser is important but it's not a prerequisite for the VM.
the REPL comes first (it's simpler). the browser comes second
(it's the real interface). the agent comes third (it needs the
browser for the approval UI).

### the agent

an LLM tool-use loop running in a vat. its tools are derived from
the handlers on the objects it can see (the `interface` protocol).
every tool call is a message send through a membrane. the membrane
logs, allows, requires approval, or denies.

the agent's memory is objects in its vat. inspectable, editable,
deletable. the agent is just another user of the objectspace —
a user with limited permissions and full auditability.

### the syntax

unchanged from v1. it's good.

```
(f a b c)            ; applicative call → [f call: a b c]
[obj selector: arg]  ; message send
{ Parent x: 10 }    ; object literal
'symbol              ; quoted symbol
obj.x                ; slot access
@x                   ; self slot access
{ :x [x + 1] }      ; block (closure)
```

the parser lives in its own module. it produces cons-cell ASTs.
the compiler turns those into bytecode. the VM executes the
bytecode. the browser and the agent don't go through the parser —
they construct objects directly.

---

## the crate structure

```
moof/
  src/
    value.rs        NaN-boxed values (8 bytes, 7 tags)
    object.rs       Object, Cons, Blob — the three heap types
    store.rs        LMDB-backed persistent object store
    nursery.rs      in-memory arena for VM temporaries
    dispatch.rs     send() — handler lookup + delegation + execution
    vm.rs           bytecode interpreter
    vat.rs          vats, scheduler, promises

    lang/
      lexer.rs      tokenizer
      parser.rs     cons-cell AST builder
      analyzer.rs   vau stability classification
      compiler.rs   AST → register bytecode

    shell/
      repl.rs       readline REPL
      browser.rs    egui object browser
      agent.rs      LLM tool-use loop
      mcp.rs        MCP protocol adapter

    main.rs         CLI entry point
```

one crate. not three. the "fabric is separate from the language"
split was architecture astronautics. the VM IS the fabric. the
compiler IS part of the system. splitting them into separate crates
bought nothing except indirection.

the module structure within the crate is clean enough. `src/` is
the runtime (objects, dispatch, VM, vats, persistence). `src/lang/`
is the language frontend (parser, compiler). `src/shell/` is the
interactive surface (REPL, browser, agent). clear boundaries
without crate-level ceremony.

---

## what we're keeping from v1

- the six kernel primitives (vau, send, def, quote, cons, eq)
- the surface syntax (three bracket species, keywords, sugar)
- prototype delegation for behavior
- capability security (vats, membranes, facets)
- the bytecode approach (compile, don't interpret trees)
- the introspection protocol (describe, interface, source)

## what we're killing from v1

- the HandlerInvoker abstraction (the VM knows what it runs)
- the three-crate split (one crate, clear modules)
- the module system as a rust-side graph solver (modules are objects)
- source projection (the image is the artifact)
- the custom binary wire protocol (MCP for external, direct for internal)
- nine heap object types (three: Object, Cons, Blob)
- private slots (slots are public data now)
- mutable slot sets (shape is fixed at creation)

## what's new in v2

- **fixed-shape objects** with public slots (data) and open handlers (behavior)
- **LMDB persistence** with nursery arena for VM temporaries
- vat/scheduler model taken seriously (erlang-style concurrency)
- eventual sends + promise pipelining (E-style async)
- capabilities as the IO model (haskell-style effect tracking, but concrete)
- vau stability analysis (compile-time expansion of static operatives)
- the browser as a first-class interface (self-style live environment)
- the agent as a vat co-inhabitant (not an API client)
- register-based bytecode (simpler than v1's stack machine)
- NaN-boxed values (8 bytes, zero-alloc for primitives)
- slot access as send (membranes intercept everything)

---

## implementation order

### phase 1: the runtime

NaN-boxed values. LMDB store. nursery arena. the three heap types
with fixed-shape objects. `send()` with handler lookup, delegation,
slot-access-as-send. type prototypes for Integer, Float, String.
tests: create objects, send messages, persist across restarts.

### phase 2: the language

lexer, parser, analyzer, compiler, bytecode VM. parse `[3 + 4]`,
compile it, execute it. the bootstrap runs. `if`, `let`, `fn`,
`while`, `match` all work. the REPL works.

### phase 3: vats

the scheduler. spawn. eventual sends. promises. fuel-based
preemption. multiple vats running concurrently. nursery GC at
turn boundaries.

### phase 4: the browser

egui. object panels with public slots and handlers. navigation.
eval bar. transcript. concurrent reads from LMDB while VM runs.

### phase 5: the agent

LLM tool-use loop. membranes. facets. approval queue.

---

*moof is a runtime for a world where objects are alive,
messages are the only operation, capabilities are the only
security, and the image never dies.*

*clarus lives.*
