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

- **typeclasses → protocols.** haskell's `Eq`, `Ord`, `Functor`,
  `Foldable`, `Traversable` — specify required methods, derive
  the rest. moof's protocols are this but as objects. conform
  to Comparable (implement `<`), get seven methods free. conform
  to Iterable (implement `each:`), get thirty.

- **effects are capabilities.** if your vat doesn't hold a ref
  to the Console object, you can't do IO. haskell's IO monad
  made concrete — not a type constraint but an object constraint.

- **pattern matching.** `match` as a derived form from vau.
  destructure objects by shape, arrays by contents, hashmaps
  by keys. match on protocol conformance.

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

### protocols: the type system

protocols are the contracts of the objectspace. a protocol says
"if you can do X, i'll give you Y for free." protocols are
objects (of course).

```
(def Comparable (protocol
  requires: '(<)
  provides:
    (>       (fn (other) [other < self]))
    (<=      (fn (other) (not [other < self])))
    (>=      (fn (other) (not [self < other])))
    (=       (fn (other) (and [self <= other] [self >= other])))
    (min:    (fn (other) (if [self < other] self other)))
    (max:    (fn (other) (if [self < other] other self)))
    (clamp:to: (fn (lo hi) [[self max: lo] min: hi]))))
```

implement one handler (`<`), get seven for free. the provided
handlers are mixed into the conforming object's handler table
when you conform.

```
(conform Point Comparable
  <: (fn (other) [self.x < other.x]))
; Point now has <, >, <=, >=, =, min:, max:, clamp:to:

[[{ Point x: 3 } min: { Point x: 7 }] x]  ; => 3
```

you can override any provided handler if the default isn't right:

```
(conform Point Comparable
  <:  (fn (other) [self.x < other.x])
  =:  (fn (other) (and [self.x = other.x] [self.y = other.y])))
; custom = instead of the default derived from <
```

**conformance is nominal + structural.** you explicitly conform
(`(conform Foo Protocol ...)`), which checks that required handlers
are present and mixes in provided ones. but you can also check
structural conformance: `[obj responds: Iterable]` returns true
if the object has all the required handlers, whether or not it
formally conformed.

**protocols are used everywhere:**

- **pattern matching.** match on protocol conformance:

  ```
  (match obj
    (Printable [obj describe])
    (Iterable  [obj toArray])
    (_         "unknown"))
  ```

- **handler signatures.** document expected protocols:

  ```
  (def sort (fn (coll) ; coll : Iterable & Comparable
    ...))
  ```

- **the agent.** the agent discovers capabilities through
  protocols — `[obj protocols]` returns the list.

- **the query model.** Queryable is a protocol. conform to it
  and get `where:`, `select:`, `groupBy:`, etc.

- **the canvas.** Renderable is a protocol. conform and your
  objects appear on the canvas.

#### the standard protocols

these are the protocols that come with the image. each one has a
small set of required handlers and a large set of provided ones.
implementing the minimum gets you the maximum.

**Printable** — how things present themselves.

```
requires: describe
provides: toString, toDebugString, print:
```

every object conforms to Printable via Object's default
`describe` handler. override it for custom display.

**Comparable** — ordering.

```
requires: <
provides: >, <=, >=, =, min:, max:, clamp:to:,
          between:and:
```

**Numeric** — arithmetic.

```
requires: +, -, *, negate
provides: abs, sign, zero?, positive?, negative?,
          /, %  (default / and % via repeated subtraction — override for performance)
```

**Hashable** — identity for collections.

```
requires: hash
provides: (enables use as HashMap key)
```

**Iterable** — the big one. this is ruby's Enumerable.

```
requires: each:
provides: map:, filter:, reject:, fold:inject:, reduce:,
          any:, every:, none:, count, count:,
          find:, findIndex:,
          first, last, isEmpty,
          toArray, toList,
          flat, flatMap:,
          zip:, zip:with:,
          take:, drop:, takeWhile:, dropWhile:,
          min, max, minBy:, maxBy:, sort, sortBy:,
          sum, product,
          groupBy:, partition:,
          each:withIndex:,
          join:, join
```

implement `each:` and you get ~30 collection operations. this
is how ruby made Enumerable the most-used module in the
language. one handler, thirty for free.

```
(conform Point Iterable
  each:: (fn (block) (do [block call: @x] [block call: @y])))

[{ Point x: 3 y: 4 } sum]     ; => 7
[{ Point x: 3 y: 4 } toArray] ; => [3, 4]
```

**Indexable** — positional access.

```
requires: at:, length
provides: first, last, isEmpty, slice:to:,
          indexOf:, contains:, reverse
includes Iterable (each: derived from at: + length)
```

conform to Indexable and you also get Iterable for free —
`each:` is derived from `at:` and `length`. protocol inclusion
means conforming to Indexable automatically conforms to
Iterable.

**Callable** — anything invocable with `()` syntax.

```
requires: call:
provides: compose:, andThen:, curry, partial:
```

blocks, lambdas, any object with `call:` — all conform.

**Serializable** — persistence and wire transfer.

```
requires: serialize:
provides: deserialize:, clone, deepClone
```

**Renderable** — canvas display.

```
requires: render:
provides: bounds, position, moveTo:
```

**Queryable** — the query model.

```
requires: (nothing — default implementations from Iterable)
provides: where:, select:, orderBy:, groupBy:,
          join:on:equals:, aggregate:,
          distinct, limit:, offset:
includes Iterable
```

Queryable builds on Iterable. any Iterable is already
Queryable. the provided handlers implement relational
operations in terms of `each:`, `filter:`, `map:`, etc. the
query model isn't magic — it's just protocols.

**Observable** — reactive updates.

```
requires: (nothing — default state tracking)
provides: onChange:, watch:, unwatch:, notify:
```

slot mutation triggers `onChange:` observers. the canvas uses
this to re-render when objects change.

#### protocol inclusion

protocols can include other protocols:

```
(def Indexable (protocol
  includes: (list Iterable)
  requires: '(at: length)
  provides:
    (each: (fn (block)
      (let ((i 0))
        (while [i < [self length]]
          [block call: [self at: i]]
          (<- i [i + 1])))))
    ...))
```

conforming to Indexable automatically conforms to Iterable
(via the derived `each:`). conforming to Comparable + Iterable
gives you sortable collections. protocols compose.

#### asking about protocols

```
[obj protocols]              ; => (Printable Comparable Iterable ...)
[obj conforms: Iterable]     ; => true
[obj responds: Iterable]     ; structural check (has the handlers?)
[Iterable conformers]        ; all objects that conform
[Iterable required]          ; => (each:)
[Iterable provided]          ; => (map: filter: fold:inject: ...)
```

protocols are objects. you can inspect them, query them, extend
them. the agent uses `[obj protocols]` and `[Protocol required]`
to understand what an object can do.

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

prototypes and the protocols they conform to:

```
Object                    Printable
  Nil                     Printable
  Boolean (True, False)   Printable, Hashable
  Number                  Printable, Comparable, Numeric, Hashable
    Integer               + Iterable (times:)
    Float
  Symbol                  Printable, Hashable
  Cons                    Printable, Iterable, Indexable
  String                  Printable, Comparable, Hashable, Indexable, Iterable
  Bytes                   Indexable
  Array                   Printable, Iterable, Indexable, Queryable
  HashMap                 Printable, Iterable
  Stream                  Iterable
  Block                   Callable
  Environment
  Vat                     Printable
  Promise                 Printable
  Membrane
  Facet
  Mirror                  Printable, Iterable
  Error                   Printable
  Continuation            Printable
  Canvas                  Renderable, Observable
  Protocol                Printable, Iterable
```

every one of these is an object. every one responds to messages.
primitive values (nil, bool, int, float, symbol) are NaN-boxed
immediates — 8 bytes, no heap allocation. but they delegate to
their prototype for behavior. their protocol conformances give
them rich default behavior from minimal implementations.

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
the data. the messages ARE the queries. every Iterable is
automatically Queryable — `where:`, `groupBy:`, `orderBy:` are
provided handlers from the Queryable protocol. implement `each:`
and you get a query language.

the runtime knows about shapes and slots — it can optimize
`where:` to a slot-offset comparison, not a hash lookup + method
call.

not bolted on. emergent from protocols + fixed-shape objects.

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

## liveness

this is where moof stops being a language and becomes an
environment. everything is introspectable. everything is
modifiable at runtime. the system is never "stopped."

### mirrors: safe reflection

a Mirror is a reflective handle on any object. read-only by
default. the browser, the agent, and the debugger all work
through mirrors — they never touch objects directly.

```
(def m [Mirror on: pt])

[m slots]                ; => {x: 3, y: 4}
[m handlers]             ; => (describe, distanceTo:, magnitude)
[m parent]               ; => <Mirror on: Point>
[m protocols]            ; => (Printable, Comparable)
[m source: 'magnitude]   ; => "(fn () [[@x * @x] + [@y * @y]])"
[m bytecode: 'magnitude] ; => <Bytes: 24 instructions>
[m vat]                  ; => <Mirror on: Vat#0>
[m shape]                ; => (x y) — the slot names
[m persistent?]          ; => true (in LMDB, not nursery)
```

mirrors are objects. they conform to Printable and Iterable
(iterate over slots). the canvas renders mirrors as inspector
panels. the agent uses mirrors to understand objects before
modifying them.

**writable mirrors.** `[Mirror writable: pt]` returns a mirror
that can also modify. modification goes through the membrane
(if one is active), so the agent's writable mirrors still get
audited.

```
(def wm [Mirror writable: pt])
[wm setSlot: 'x to: 99]
[wm addHandler: 'magnitude with: (fn () ...)]
[wm removeHandler: 'magnitude]
[wm setParent: NewPoint]
[wm conform: Renderable with: (render:: (fn (c) ...))]
```

### everything is inspectable

there is no hidden state in the objectspace. anything the VM
knows, you can ask about:

**objects:**

```
[obj slotNames]          ; => (x y)
[obj handlerNames]       ; => (describe distanceTo: magnitude)
[obj parent]             ; => Point
[obj protocols]          ; => (Printable Comparable)
[obj conforms: Iterable] ; => false
[obj identity]           ; => 47 (the object's unique ID)
[obj vat]                ; => <Vat#0>
[obj persistent?]        ; => true
[obj describe]           ; => "(3, 4)"
[obj interface]          ; => handler signatures + docs
```

**handlers:**

```
[obj source: 'magnitude]  ; => the source AST
[obj bytecode: 'magnitude]; => the compiled bytecode blob
[obj arity: 'magnitude]   ; => 0
[obj handlerOf: 'magnitude]; => the lambda/native object itself
```

**environments:**

```
[env bindings]           ; => ((x . 3) (y . 4))
[env parent]             ; => <enclosing env>
[env depth]              ; => 2 (nesting level)
```

**vats:**

```
[vat objects]            ; => all objects in this vat
[vat mailbox]            ; => pending messages
[vat fuel]               ; => remaining reductions
[vat capabilities]       ; => faceted references
[vat status]             ; => 'running, 'suspended, 'dead
```

**protocols:**

```
[Protocol all]           ; => every protocol in the image
[Iterable required]      ; => (each:)
[Iterable provided]      ; => (map: filter: fold:inject: ...)
[Iterable conformers]    ; => all objects that conform
[Iterable includes]      ; => ()
[Queryable includes]     ; => (Iterable)
```

**the compiler and evaluator:**

```
[Compiler current]       ; => the live compiler object
[Compiler parse: "(+ 1 2)"]  ; => AST
[Compiler compile: ast]  ; => bytecode blob
[Compiler analyze: ast]  ; => stability classification

[Evaluator current]      ; => the live evaluator
[Evaluator eval: ast in: env] ; => result
```

### everything is modifiable

handler modification is live and immediate:

```
; change how Points describe themselves — every Point instantly affected
[Point handle: 'describe with: (fn () (str "Point(" @x ", " @y ")"))]

; add a protocol conformance at runtime
(conform Integer Iterable
  each:: (fn (block) [self times: block]))

; change a protocol's provided methods
[Iterable addProvided: 'tally with: (fn () [self fold: 0 inject: |acc _| [acc + 1]])]

; change the parent chain
[pt setParent: Point3D]
```

**handler replacement is atomic.** the old handler stays active for
any in-progress sends. the next send uses the new handler. this is
erlang's hot code swapping semantics.

**prototype modification propagates instantly.** add a handler to
Point, every existing Point-child gains it on the next send. no
restart. no cache invalidation (the inline caches check handler
identity, not prototype version).

### code is data — source manipulation from within moof

the AST is cons cells. cons cells are objects. therefore source
code is objects. you can read it, walk it, transform it, and
write it back — all from within moof. this is the homoiconicity
payoff, made real.

**every handler carries its source.** not just bytecode — the
original AST and the human-readable source text (with comments,
formatting, whitespace) both live on the handler object as slots:

```
(def mag [pt handlerOf: 'magnitude])

[mag source]       ; => "(fn () [[@x * @x] + [@y * @y]])"
[mag ast]          ; => the live cons-cell AST
[mag bytecode]     ; => the compiled bytecode blob
[mag sourceText]   ; => source with original formatting + comments
```

**read and manipulate the AST:**

```
(def tree [mag ast])
; tree is: (fn () (send (send ...) + (send ...)))
; it's cons cells. walk it, transform it, build new trees.

[tree car]         ; => fn
[tree cdr car]     ; => ()  (params)
[tree cdr cdr car] ; => the body expression

; build a new AST from scratch
(def new-body `[[@x * @x] + [@y * @y] + [@z * @z]])
(def new-ast `(fn () ,new-body))
```

**recompile and install:**

```
; replace a handler from its AST
[pt handle: 'magnitude with: (eval new-ast)]

; or from source text
[pt handle: 'magnitude withSource: "(fn () [[@x * @x] + [@y * @y] + [@z * @z]])"]
```

`handle:withSource:` parses, compiles, installs, AND stores
the source text on the handler — so the round-trip is
lossless. comments survive.

**programmatic code generation:**

```
; generate accessors for all slots on a prototype
(def make-accessors (fn (proto)
  [proto slotNames each: |name|
    [proto handle: name
      withSource: (str "(fn () @" name ")")]]))

(make-accessors Point)
; Point now has handlers 'x and 'y that return the slot values
```

**the agent modifies source.** when the agent adds a handler,
it constructs source text (not raw ASTs — source text is what
it's good at), and `handle:withSource:` does the parse-compile-
install cycle. the source text is stored, so you can inspect
what the agent wrote on the canvas, see the actual code, edit
it, and reinstall.

```
; agent constructs this string:
"(fn (other)
  (let ((dx [@x - other.x])
        (dy [@y - other.y]))
    [[[dx * dx] + [dy * dy]] sqrt]))"

; installed via:
[Point handle: 'distanceTo: withSource: that-string]

; later, inspect it:
[Point sourceText: 'distanceTo:]
; => the exact string the agent wrote, formatting preserved
```

**the canvas edits source.** when you click "edit handler" on
the canvas, it opens the source text in an inline editor. when
you save, it calls `handle:withSource:` — parse, compile,
install, store. the handler is live immediately.

**code transformation as a library.** because the AST is cons
cells, you can write code that transforms code:

```
; add tracing to every handler on an object
(def add-tracing (fn (obj)
  [obj handlerNames each: |sel|
    (let ((orig-ast [[obj handlerOf: sel] ast]))
      [obj handle: sel with:
        (eval `(fn args
          [Console println: (str ">> " ,sel " called")]
          (let ((result (apply ,(eval orig-ast) args)))
            [Console println: (str "<< " ,sel " => " result)]
            result)))])]))

(add-tracing pt)
; every send to pt now prints entry/exit traces
```

this is where `vau` and homoiconicity meet: you manipulate
code as data, generate new code, compile and install it, all
at runtime, all from within moof. no external tools. no
restarting. the image modifies itself.

### errors are objects

when a send fails (doesNotUnderstand, type error, assertion
failure), the error is an object:

```
Error {
  selector: 'magnitude
  receiver: <Mirror on: obj>
  args: ()
  message: "does not understand 'magnitude'"
  continuation: <Continuation>
  vat: <Vat#0>
  timestamp: 1712534400
}
```

the continuation is a first-class object representing the
suspended computation. you can inspect it:

```
[error continuation]         ; => <Continuation>
[error continuation frames]  ; => stack frames as objects
[error continuation env]     ; => the environment at the error

(let ((frame [[error continuation frames] first]))
  [frame selector]           ; => 'magnitude
  [frame receiver]           ; => the object that failed
  [frame locals]             ; => local bindings
  [frame source])            ; => source location
```

### fix and proceed

this is smalltalk's greatest UX idea. when an error occurs in
a vat, the vat is **suspended, not killed.** the continuation
object is live. you can:

1. **inspect** — look at the stack, the environment, the
   receiver, the args. understand what went wrong.

2. **fix** — add the missing handler, fix the broken handler,
   change a slot value. the fix is live — you're modifying the
   actual objects involved.

3. **proceed** — resume the suspended continuation. the send
   retries with the fixed handler. the computation continues
   from where it left off.

```
; an error occurred: pt doesn't understand 'magnitude'
; the vat is suspended. the error is in the transcript.

; fix it:
[Point handle: 'magnitude with: (fn () [[@x * @x] + [@y * @y]])]

; resume:
[error proceed]
; => 25 — the computation continues as if the error never happened
```

this is not "restart from the beginning." this is "resume from
the exact point of failure." the continuation holds the entire
state. the fix is applied to the live objects. the computation
picks up where it left off.

the canvas shows suspended vats with their error objects. you
can fix and proceed from the GUI. the agent can fix and proceed
too (with approval through the membrane).

### doesNotUnderstand: as extension point

`doesNotUnderstand:` is a handler like any other. the default
raises an error. override it for:

```
; forwarding proxy
(def proxy { Object
  target: realObj })
[proxy handle: 'doesNotUnderstand: with:
  (fn (msg) [msg.target send: msg.selector with: msg.args])]

; method synthesis
[MyDSL handle: 'doesNotUnderstand: with:
  (fn (msg)
    (if [msg.selector startsWith: "find_by_"]
      (let ((field [msg.selector substring: 8]))
        [self where: |o| [[o slotAt: field] = [msg.args first]]])
      [Error raise: msg]))]

; now you can say:
[people find_by_name: "alice"]
; doesNotUnderstand catches it, synthesizes a where: query
```

this is ruby's `method_missing` and it's how rails' ActiveRecord
works. in moof it's a message, not a hook — it goes through the
same dispatch, the same membranes, the same audit trail.

### Observable: reactive liveness

the Observable protocol connects mutation to reaction:

```
[pt watch: 'x with: |old new| [Console println: (str "x changed: " old " → " new)]]
[pt slotAt: 'x put: 99]
; prints: "x changed: 3 → 99"
```

the canvas uses Observable to re-render objects when their slots
change. you modify a slot in the REPL → the canvas updates. you
modify a slot through the agent → the canvas updates. no manual
refresh. the objectspace is always live.

Observable works on handlers too:

```
[Point watch: 'handlers with: |sel handler|
  [Console println: (str "Point gained handler: " sel)]]

[Point handle: 'area with: (fn () [@x * @y])]
; prints: "Point gained handler: area"
```

### the compiler and evaluator are objects

the compiler is an object. you can intercept compilation:

```
; add a custom syntax pass
[Compiler addPass: |ast|
  (if [ast is: '(trace ...)]
    `(do [Console println: ,(ast-to-string [ast cdr])]
         ,[ast cdr car])
    ast)]
```

the evaluator is an object. you can intercept evaluation — this
is the reflective tower from the design doc. an environment can
override how expressions are evaluated within it:

```
(def tracing-env [Environment extend: root-env
  eval: (fn (expr env)
    [Console println: (str "eval: " expr)]
    [env parent eval: expr env])])

; now expressions evaluated in tracing-env log themselves
```

this is 3-Lisp / Black territory — the meta-level is just more
objects. not a day-one feature, but the architecture never
forecloses it.

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
- **protocols** — the type system. require handlers, provide
  defaults, mix in on conformance. Iterable gives 30 methods
  from one. protocol-based typing used everywhere.
- **fixed-shape slots** (public data) + open handlers (behavior)
- **Array and HashMap** for runtime-mutable collections
- **query operations** on collections (where, select, groupBy,
  join, aggregate) — objects-as-rows, sends-as-queries, built
  on the Queryable protocol
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
- **mirrors** — safe reflection handles for introspection
- **fix and proceed** — errors suspend vats, not kill them.
  inspect, fix, resume from the exact point of failure
- **errors and continuations as objects** — inspect stack
  frames, environments, resume suspended computations
- **Observable** — reactive slot/handler watching, canvas
  auto-updates on mutation
- **compiler and evaluator as objects** — intercept compilation,
  intercept evaluation, the reflective tower

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

### phase 3: protocols

Protocol object. conform, provides, requires, includes. the
standard suite: Printable, Comparable, Numeric, Hashable,
Iterable (the big one — 30 methods from each:), Indexable,
Callable, Queryable. protocol conformance for all built-in types.
pattern matching on protocol conformance.

### phase 4: vats

vats as objects. scheduler. spawn. eventual sends. promises.
fuel-based preemption. nursery GC at turn boundaries.

### phase 5: data

Array, HashMap with full message interfaces. query operations
via Queryable protocol: where, select, groupBy, orderBy, join,
aggregate. destructuring/pattern matching on all types.

### phase 6: the canvas

egui zoomable infinite canvas. object rendering via `render:`
handlers. navigation, zoom, pan. eval bar. slot editing. handler
browsing. transcript.

### phase 7: the agent

LLM tool-use loop in a vat. membranes. facets. approval queue
on the canvas. agent memory as objects.

---

*everything is an object. the only operation is send. the image
never dies. the canvas is the world. the agent lives here too.*

*clarus lives.*
