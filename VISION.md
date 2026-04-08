# moof v2

> a persistent, concurrent objectspace with capability security
> and a lisp-shaped surface syntax.

---

## what moof is

moof is a runtime. like the BEAM is erlang's runtime, moof is
moof's runtime. the surface syntax is separate — but the
computational model is the runtime. inseparable.

the things that make moof moof:

1. **everything is an object.** integers, strings, booleans,
   cons cells, arrays, hashmaps, lambdas, vats, the canvas, the
   agent — all objects. objects have slots (public data, fixed at
   creation) and handlers (public behavior, extensible anytime).
   handlers delegate through prototype chains. slots don't.

   the VM has optimized internal representations for common
   shapes — a cons cell is stored as two values, not a full slot
   array. but semantically, it's an object. you send it messages.
   it delegates to the Cons prototype. there are no second-class
   citizens.

2. **the only operation is send.** `[obj selector: arg]` is the
   one thing the VM does. function calls, arithmetic, slot access,
   control flow — all message sends. `(f x)` is `[f call: x]`.
   `[3 + 4]` is a send to an integer with selector `+`.
   `obj.x` is `[obj slotAt: 'x]` — even slot access is a send.

3. **vats are the unit of concurrency — and they're objects.**
   a vat is a single-threaded event loop. within a vat, sends are
   synchronous. across vats, sends are eventual and return
   promises. vats are objects: `[myVat spawn: ...]`,
   `[myVat capabilities]`, `[myVat kill]`. the scheduler is an
   object. no shared mutable state. no locks. ever.

4. **a reference is a capability.** if you hold a reference to an
   object, you can send it messages. if you don't, the object
   doesn't exist in your world. there is no global namespace, no
   ambient authority. IO is a capability: if your vat doesn't
   hold a reference to Console, you physically cannot print.

5. **the image persists.** when you create an object, it survives
   restarts. LMDB — crash-safe, concurrent readers, instant
   startup via mmap. there is no "save." the objects are just
   there.

6. **vau gives user code compiler power.** an operative receives
   its arguments unevaluated and the caller's environment as a
   first-class value. `if`, `let`, `while`, `match` are library
   functions, not special forms.

---

## the debts

### erlang / BEAM

- **processes everywhere.** vats are erlang processes. cheap.
  isolated. communicate by async message passing. the scheduler
  is preemptive (fuel-based, like BEAM's reduction counting).

- **let it crash.** a vat can crash without taking down the
  image. supervisor objects monitor vats and restart them.
  `doesNotUnderstand:` is a message, not a fatal error.

- **hot code swapping.** change a handler on a prototype, and
  every object delegating to it gets the new behavior. no restart.

- **distribution is transparent.** a reference to a remote
  object looks local. send goes over the network. the vat model
  already has async messaging — remote is just "more async."

### E language (mark miller)

- **near refs and far refs.** same-vat sends are synchronous.
  cross-vat sends are eventual (return promises). syntax makes
  the distinction visible: `[obj sel]` vs `[obj <- sel]`.

- **promise pipelining.** `[[obj <- foo] <- bar: x]` pipelines.
  no explicit `.then()` chaining.

- **membranes and facets.** intercept all sends crossing a
  boundary. log, allow, deny, transform. the agent lives behind
  a membrane. always.

### haskell

- **effects are capabilities.** if your vat doesn't hold a ref
  to the Console object, you can't do IO. haskell's IO monad
  made concrete — not a type constraint but an object constraint.

- **pattern matching.** `match` as a derived form from vau.
  destructure objects by shape, arrays by contents, hashmaps
  by keys.

- **laziness where you want it.** streams are objects with a
  `next` handler that computes on demand. infinite sequences
  are natural.

### ruby

- **everything is an object.** `[3 times: |i| [Console println: i]]`.
  no primitives. no special cases. the integer 3 responds to
  messages because the Integer prototype has handlers.

- **blocks.** `|x| [x + 1]` is a closure. pass it to a method.
  the method sends `call:` to it. blocks close over their
  environment. blocks are objects.

- **method_missing → doesNotUnderstand:.** proxies, delegation,
  DSLs — all built on handler-not-found interception.

- **open prototypes.** add a handler to Integer and every integer
  gains that behavior. the image is always malleable.

### self

- **prototypes, not classes.** objects delegate to other objects.
  no class/metaclass distinction.

- **the live environment.** an infinite canvas where objects are
  visual things you can see, click, inspect, modify. the
  environment is the language is the IDE.

### SQL / relational model

- **objects as rows.** fixed-shape objects with public slots are
  structurally rows. a collection of same-shaped objects is a
  table.

- **query as messaging.** `where:`, `select:`, `groupBy:`,
  `orderBy:`, `join:on:`, `aggregate:` — query operations as
  message sends on collections. the object model IS the query
  language.

---

## the object model

### everything is Object

there is one semantic type: **Object.** everything is one.

the VM has optimized internal representations for performance,
but the semantics are always "it's an object, it responds to
messages, it delegates to a prototype":

```
Object     — general: parent + named slots + handlers
Cons       — optimized pair: parent Cons, slots car/cdr
String     — optimized text: parent String, internal bytes
Bytes      — optimized blob: parent Bytes, internal bytes
Array      — mutable indexed collection: parent Array
HashMap    — mutable key-value collection: parent HashMap
```

a Cons cell is stored internally as two Values (16 bytes), not
a full slot array. but `[pair car]` is a message send to the
Cons prototype, `[pair describe]` works, `[pair slotNames]`
returns `'(car cdr)`. the optimization is invisible.

### slots: public, fixed, destructurable

an object's **slots** are its data. they are:

- **public.** anyone with a reference can read any slot.
  `obj.x` desugars to `[obj slotAt: 'x]`. it's a send, so
  membranes can intercept it. the VM optimizes the common case
  (no membrane) to a direct offset read.

- **fixed at creation.** `{ Point x: 3 y: 4 }` creates an
  object with exactly two slots: `x` and `y`. you cannot add
  `z` later. the shape is sealed. values are mutable.

- **destructurable.** pattern matching works on shapes:

  ```
  (match obj
    ({ x: x y: y } [Console println: (str "cartesian: " x ", " y)])
    ({ r: r theta: t } [Console println: (str "polar: " r " @ " t)]))
  ```

  arrays and hashmaps destructure too:

  ```
  (match data
    ([first second . rest] ...)    ; array destructure
    ({| "name" => n "age" => a |} ...))  ; hashmap destructure
  ```

**why fixed?** shapes are known at creation → slot access is
an array offset, not a hash lookup. serialization is trivial.
objects are self-describing. V8 spends enormous complexity on
hidden classes to *guess* shapes. we just declare them.

### handlers: open, delegated

an object's **handlers** are its behavior. they are:

- **open.** add handlers anytime: `[pt handle: 'magnitude with: ...]`.
- **delegated.** handler lookup walks the parent chain. add a
  handler to the Point prototype → every Point-child gains it.
- **the interface.** the set of handlers an object responds to
  is its public API. membranes, facets, the agent — all interact
  through handlers.

```
slots    = data.     public. fixed set. values mutable.
handlers = behavior. public. open set.  delegated via parent.
```

### mutable collections: Array and HashMap

fixed-shape objects cover the 90% case. for the other 10% — when
you need runtime-mutable indexed or keyed data — the lang provides
Array and HashMap as built-in object types:

```
(def a [Array of: 1 2 3])
[a push: 4]              ; => [1, 2, 3, 4]
[a at: 0]                ; => 1
[a length]               ; => 4

(def m [HashMap of: "x" 10 "y" 20])
[m at: "x"]              ; => 10
[m at: "z" put: 30]      ; => {x: 10, y: 20, z: 30}
[m keys]                 ; => ["x", "y", "z"]
```

these are objects. they respond to messages. they delegate to the
Array and HashMap prototypes. they're destructurable. they persist
in LMDB. they're just objects with internal storage that the VM
knows how to handle efficiently.

Array and HashMap are how you escape fixed shapes when you
genuinely need dynamic data. but fixed-shape objects remain the
default, the common case, the thing the whole system optimizes for.

### the type hierarchy

```
Object
  Nil
  Boolean (True, False)
  Number
    Integer
    Float
  Symbol
  Cons
  String
  Bytes
  Array
  HashMap
  Stream
  Block        — closure, has call: handler
  Environment  — scope bindings
  Vat          — concurrency domain
  Promise      — eventual result
  Membrane     — send interceptor
  Facet        — restricted view
  Canvas       — the spatial browser surface
```

every one of these is an object. every one responds to messages.
primitive values (nil, bool, int, float, symbol) are NaN-boxed
immediates — 8 bytes, no heap allocation. but they delegate to
their prototype for behavior.

### objects as data: the query model

because objects have fixed, public, named slots, a collection of
same-shaped objects is structurally a table. query operations fall
out naturally as message sends on collections:

```
(def people (list
  { Person name: "alice" age: 30 dept: "eng" }
  { Person name: "bob" age: 25 dept: "design" }
  { Person name: "carol" age: 35 dept: "eng" }))

[people where: |p| [p.age > 28]]
; => (alice, carol)

[people select: '(name dept)]
; => projection — objects with only name and dept slots

[people groupBy: 'dept]
; => { "eng" => (alice, carol), "design" => (bob) }

[people orderBy: 'age]
; => (bob, alice, carol)

[people aggregate: { count: [Count new] avgAge: [Avg on: 'age] }]
; => { count: 3 avgAge: 30 }

; joins
[people join: departments on: 'dept equals: 'name]
```

this isn't an ORM. there's no SQL being generated. the objects ARE
the data. the messages ARE the queries. the runtime knows about
shapes and slots — it can optimize `where:` to a slot-offset
comparison, not a hash lookup + method call.

you get a query language for free from the object model. not bolted
on. emergent.

---

## the syntax

### the three bracket species

```
(f a b c)            ; applicative call → [f call: a b c]
[obj selector: arg]  ; message send
{ Parent x: 10 }     ; object literal
```

`{ }` is **exclusively** for objects. no blocks. no ambiguity.

### blocks

blocks get their own syntax: `|params| body`.

```
|x| [x + 1]                      ; one-arg block
|x y| [x + y]                    ; two-arg block
|| [Console println: "hello"]    ; zero-arg block
```

blocks are objects with a `call:` handler. the syntax is sugar.
a block closes over its lexical environment.

```
[list map: |x| [x * 2]]
[3 times: |i| [Console println: i]]
[condition ifTrue: || "yes" ifFalse: || "no"]
```

this is ruby's block syntax without the braces. it reads
naturally as "the thing you pass to a method."

`(fn (params) body)` still exists for multi-expression lambdas
and named functions:

```
(def greet (fn (name)
  (let ((msg [name ++ " says moof"]))
    [Console println: msg]
    msg)))
```

### sugar

```
'symbol              ; (quote symbol)
obj.x                ; [obj slotAt: 'x]
@x                   ; [self slotAt: 'x]  (inside handlers)
[obj <- sel: arg]    ; eventual send (cross-vat, returns promise)
```

---

## the runtime

### values

NaN-boxed, 8 bytes each.

```
nil, true, false          — singleton tags
integer (48-bit signed)   — inline
float (64-bit)            — non-NaN bit patterns
symbol (32-bit interned)  — inline
object ref (32-bit)       — store or nursery, distinguished by tag
```

two object ref tags:

```
tag 5 = store object  (LMDB, persistent, crash-safe)
tag 6 = nursery object (in-memory, ephemeral, fast)
```

### send

```
send(receiver, selector, args) → result

1. look in receiver's handler table (or type prototype for primitives)
2. walk the parent chain (delegation, depth limit 256)
3. if found: execute (bytecode → VM, native → rust closure)
4. if not found: send doesNotUnderstand: to receiver
```

slot access is a send. `obj.x` → `[obj slotAt: 'x]`. the default
handler does a direct offset read. membranes can intercept it.
the VM optimizes the common case to skip dispatch entirely.

### the VM

register-based bytecode. 4-byte instructions. de bruijn indices
for lexical scope (no runtime name lookup). tail calls are real.
inline caches on send sites.

### vats

vats are objects. `[Vat spawn: |v| ...]` creates one.

```
[myVat capabilities]      ; what refs does this vat hold?
[myVat send: msg]         ; enqueue a message
[myVat kill]              ; terminate
[myVat supervise: child]  ; erlang-style supervision
```

the scheduler is an object too. fuel-based preemption. round-robin.
a runaway vat doesn't starve others.

eventual sends return promises. promises support pipelining.
`[[obj <- foo] <- bar: x]` — `bar:` is sent to the promise of
`foo`, not to the resolved value.

### vau and compilation

the compiler classifies every operative call site:

**static** (95%): the binding is top-level, never reassigned,
body follows a known pattern. expanded at compile time. `if`
becomes a branch. `let` becomes local bindings. `fn` becomes a
closure. zero overhead.

**dynamic** (5%): genuinely depends on runtime environment. the
compiler emits a full operative call with reified args and env.
the slow path. there when you need it.

### persistence: LMDB + nursery

LMDB for persistent state. crash-safe. concurrent readers (the
browser reads while the VM writes). instant startup via mmap.

nursery arena for VM temporaries. environments, stack frames,
intermediate values — too hot for LMDB's ~1μs write cost. stored
in a plain `Vec` indexed by u32.

promotion: when a nursery object is assigned to a slot on a store
object (or explicitly persisted, or returned from a vat turn),
it's promoted to LMDB. nursery GC at turn boundaries: anything
not reachable from a store root is dead.

---

## the canvas

the browser is not a panel layout. it's a **zoomable infinite
canvas** — a 2D spatial objectspace.

every object has a position on the canvas (or is nested inside
another object's visual representation). every object has a
`render:` handler that knows how to draw itself. the default
`render:` shows a card with slot names and values. override it
for custom visualization — a chart, a diagram, a control panel,
whatever.

**zoom in** on an object → see its slots, handlers, parent chain,
delegation graph. edit a slot value inline. add a handler. send
a message from a scratchpad.

**zoom out** → see the object graph. references are edges.
clusters of related objects become visible. the topology of your
objectspace emerges.

**the canvas is an object.** `[Canvas current]` returns it.
`[Canvas pan: { x: 100 y: 50 }]`. `[Canvas zoom: 0.5]`.
`[Canvas objectsInView]` returns what's visible. the canvas
responds to messages like everything else.

**the canvas is a vat.** it reads from LMDB concurrently. it
never blocks the VM. it gets faceted references — read access to
everything, write access through the membrane (which routes
through the eval bar or the agent).

**every object renders itself.** a Number renders as a number.
a String renders as text. a Point renders as a dot on a 2D plane.
a collection of Points renders as a scatter plot. a Person
renders as a card with name, age, role. you define `render:` on
your prototypes. the canvas calls it.

```
[Point handle: 'render: with: (fn (canvas)
  [canvas dot: @x y: @y color: 'blue])]

[PersonCard handle: 'render: with: (fn (canvas)
  [canvas card: (str @name ", " @age)])]
```

the GUI framework IS the object model. there's no separate widget
toolkit. a button is an object with a `click:` handler. a text
field is an object with a `value` slot and an `onChange:` handler.
layout is messaging: `[container add: child at: position]`.

---

## the crate structure

```
moof/
  src/
    value.rs        NaN-boxed values (8 bytes, 7 tags)
    object.rs       the one heap type, optimized variants
    store.rs        LMDB-backed persistent object store
    nursery.rs      in-memory arena for VM temporaries
    dispatch.rs     send() — the one operation
    vm.rs           bytecode interpreter
    vat.rs          vats, scheduler, promises

    lang/
      lexer.rs      tokenizer
      parser.rs     AST builder
      analyzer.rs   vau stability classification
      compiler.rs   AST → register bytecode

    shell/
      repl.rs       readline REPL
      canvas.rs     egui zoomable spatial browser
      agent.rs      LLM tool-use loop in a vat
      mcp.rs        MCP protocol adapter

    main.rs         CLI entry point
```

one crate. the VM IS the runtime. the compiler IS part of the
system. clear module boundaries, no crate-level ceremony.

---

## what we're keeping from v1

- the six kernel primitives (vau, send, def, quote, cons, eq)
- the three bracket species for calls, sends, objects
- prototype delegation for behavior
- capability security (vats, membranes, facets)
- bytecode (compile, don't interpret trees)
- the introspection protocol (describe, interface, source)

## what's new in v2

- **one semantic type: Object.** cons, string, array, hashmap,
  vat, canvas — all objects. VM has optimized representations
  but semantics are uniform
- **fixed-shape slots** (public data) + open handlers (behavior)
- **Array and HashMap** for runtime-mutable collections
- **query operations** on collections (where, select, groupBy,
  join, aggregate) — objects-as-rows, sends-as-queries
- **block syntax**: `|x| expr` — distinct from object literals
- **LMDB + nursery** persistence from day one
- **vats as objects** (spawn, kill, supervise, capabilities)
- **the canvas** — zoomable infinite spatial browser where every
  object renders itself
- **the agent** — LLM in a vat with membraned capabilities
- eventual sends + promise pipelining (E-style)
- fuel-based preemptive scheduling (erlang-style)
- register-based bytecode + vau stability analysis
- NaN-boxed values (8 bytes, zero-alloc for primitives)
- slot access as send (membranes intercept everything)
- pattern matching on object shapes, arrays, hashmaps

## what we're killing from v1

- multiple heap types (one type now: Object)
- private slots
- mutable slot sets
- `{ :x expr }` block syntax (now `|x| expr`)
- the HandlerInvoker abstraction
- the three-crate split
- the module system as a rust-side graph solver
- source projection
- the custom binary wire protocol

---

## implementation order

### phase 1: the runtime

NaN-boxed values. LMDB store + nursery arena. Object with
optimized variants (general, pair, string, bytes, array, map).
`send()` with delegation and slot-access-as-send. type prototypes.
tests.

### phase 2: the language

lexer, parser (with `|x| expr` block syntax), analyzer, compiler,
VM. bootstrap: vau, fn, if, let, while, match. REPL works.

### phase 3: vats

vats as objects. scheduler. spawn. eventual sends. promises.
fuel-based preemption. nursery GC at turn boundaries.

### phase 4: data

Array, HashMap with full message interfaces. query operations
on collections: where, select, groupBy, orderBy, join, aggregate.
destructuring/pattern matching on all collection types.

### phase 5: the canvas

egui zoomable infinite canvas. object rendering via `render:`
handlers. navigation, zoom, pan. eval bar. slot editing. handler
browsing. transcript.

### phase 6: the agent

LLM tool-use loop in a vat. membranes. facets. approval queue
on the canvas. agent memory as objects.

---

*everything is an object. the only operation is send. the image
never dies. the canvas is the world. the agent lives here too.*

*clarus lives.*
