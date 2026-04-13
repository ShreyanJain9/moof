# effects and vats: the unified model

## the insight

the vat is not a concurrency primitive. the vat is the
**universal effect boundary**. the REPL is a vat. Console is a
vat. Clock is a vat. every capability is its own vat.

every effectful operation is a cross-vat send. every cross-vat
send returns an **Act** — the type of effectful computation.
pure code never sees one.

## pure vs effectful: the total split

```
PURE                                EFFECTFUL
───────────────────────             ───────────────────────
synchronous                         eventual (Act)
deterministic                       depends on the world
can memoize                         cannot memoize
can parallelize safely              must sequence
can rewrite / optimize              must preserve effect order
can content-address                 identity-dependent
no capability refs                  holds vat refs
lives INSIDE a vat turn             IS the cross-vat boundary
```

pure code: `[3 + 4] → 7`. immediate. in your vat.
effectful: `[console readLine] → Act<String>`. cross-vat send.

the split isn't convention — it's architecture. if your code
holds no vat refs, it's pure. the runtime can prove this.

## what the split buys you

**referential transparency.** same inputs → same outputs. always.
no capability refs = no way to cheat.

**automatic memoization.** pure function + same args = cached result.

**content-addressed compilation.** same AST + same closed-over values
= same result. hash it. cache the bytecode. cache the RESULT.

**safe parallelism.** pure computations can run anywhere. nothing
to race on.

**speculative execution.** evaluate pure branches while waiting for
an Act to resolve.

**serializable closures.** pure closures close over values, not vat
refs. serialize them, send them anywhere.

**time-travel debugging.** pure computation is deterministic. only
record Act boundaries.

**reactive recomputation.** change an input → recompute everything
downstream (it's pure, so it's safe). crucial for notebooks.

**vat serialization.** snapshot heap + env + pending Acts, move to
another machine, resume. capability refs become far refs. live
migration for free.

## Act: the one effectful type

Act is what you get when you talk to the outside world.

```
Act a = "a computation that will produce an `a` after effects happen"
```

Act is NOT Promise. Promise is "someone will fill this in later."
Act is "this is a description of something that will happen."

**Acts are inspectable data.** an Act is an object:
`{ target: Console, selector: 'println:, args: ("hello") }`.
you can inspect it, rewrite it, log it, replay it, mock it.
this is algebraic effects as objects. essential for self-hosting,
essential for debugging, essential for the live environment.

```moof
Act
  flatMap:    ; chain another Act-producing function
  map:        ; transform the result purely
  recover:    ; handle errors
  inspect     ; return the Act's description as data
  describe    ; "<Act: println: on Console>"
```

capability info is implicit — inspect the target vat to know
what kind of effect this is. no need for `Act<IO>` sub-types.

## do-notation: composing Acts

moof's `do` already means "sequence expressions." extend it:
when `<-` appears, it becomes monadic composition over Acts.

```moof
; old do — still works, unchanged
(do [x print] [y print] [x + y])

; new do — monadic binding
(do (name <- [console readLine])
    (time <- [clock now])
    [console println: [name ++ " at " ++ [time describe]]])

; desugars to:
[[console readLine] flatMap: |name|
  [[clock now] flatMap: |time|
    [console println: [name ++ " at " ++ [time describe]]]]]
```

backwards compatible. no `<-`, old behavior. with `<-`, monadic.

### pure bindings in do-blocks

use `let` for pure bindings inside a do-block (same as haskell):

```moof
(do (x <- [console readLine])
    (let ((y [x ++ " world"])))
    [console println: y])
```

`<-` waits for an Act. `let` is immediate pure binding. `:=` is
local rebinding within the vat — also pure/local.

### compound Acts

a do-block produces a compound Act — a closure chain where each
step's Act is constructed from the previous step's result:

```moof
(do (x <- act1) (y <- [f x]) [g y])
; = [act1 flatMap: |x| [[f x] flatMap: |y| [g y]]]
```

the scheduler unwinds and executes one step at a time.

## Flatmappable: the universal composition protocol

```moof
(def Flatmappable (protocol
  requires: '(flatMap: map:)
  provides: '(then: sequence: traverse:with: ...)))
```

anything conforming gets do-notation:

```moof
(conform Act Flatmappable)       ; effect chains
(conform Option Flatmappable)    ; nil-safe chains
(conform Cons Flatmappable)      ; list comprehensions
(conform Result Flatmappable)    ; error chains
```

same `do`, different types:

```moof
; effect chain
(do (user <- [api fetchUser: 42])
    (posts <- [api fetchPosts: user.id])
    { user: user posts: posts })

; nil-safe chain
(do (addr <- [user at: 'address])
    (zip  <- [addr at: 'zip])
    [zip toInteger])

; list comprehension (flatMap = concatMap)
(do (x <- xs) (y <- ys) [x + y])
```

## error handling: no try/catch

errors propagate through the monadic chain. Act carries success
or failure. `flatMap:` short-circuits on error. pure code uses
Result values directly. no TryCatch opcode. simpler runtime.

```moof
; monadic error propagation — chain fails at first error
; errors should carry rich info: source, context, stack
(do (data <- [file read: path])
    (parsed <- [json parse: data])
    [store save: 'key value: parsed])

; explicit recovery
[[file read: path] recover: |e| "default value"]

; pure code: Result values + match
(match [parse input]
  { Ok val }    [process val]
  { Error msg } [report msg])
```

## concurrency: Act combinators

concurrency is described, not performed. the scheduler decides
how to parallelize.

```moof
; concurrent execution (needs destructuring!)
(do ((user posts) <- [Act all: (list
        [api fetchUser: 42]
        [api fetchPosts: 42])])
    { user: user posts: posts })

; race — first to finish wins
(do (result <- [Act race: (list
        [fast-api fetch: id]
        [slow-api fetch: id])])
    result)
```

## the architecture

```
┌─── Console vat ──────────┐
│ handles: println: readLine│     capability vats own
│ (wraps native stdout/in)  │     the outside world
└──────────┬───────────────┘
           │ cross-vat send → Act
┌─── Clock vat ────────────┐        │
│ handles: now measure:     │        │
│ (wraps native clock)      │        │
└──────────┬───────────────┘        │
           │                         │
┌─── Store vat ────────────┐        │
│ handles: save: load:      │        │
│ (wraps LMDB)              │        │
└──────────┬───────────────┘        │
           │                         │
┌─── REPL vat ──────────────────────┤
│                                    │
│  has refs to: Console, Clock,      │
│  Store, Network, Canvas, ...       │
│                                    │
│  ┌─ pure computation ──────────┐  │
│  │ [3 + 4]         → 7        │  │
│  │ [list map: f]   → list'    │  │
│  │ no Acts, no vat refs       │  │
│  │ memoizable, parallelizable │  │
│  └─────────────────────────────┘  │
│                                    │
│  effectful:                        │
│  [console println: "hi"]          │
│    → cross-vat send to Console    │
│    → returns Act<nil>              │
│                                    │
│  (do (name <- [console readLine]) │
│      (time <- [clock now])        │
│      [console println:            │
│        [name ++ " @ " ++ ...]])   │
│    → chains three Acts            │
└────────────────────────────────────┘
```

## the init vat / bootstrap

the rust runtime IS semantically a vat — the **init vat**. it:
1. creates capability vats (Console, Clock, Store, etc.) — mostly
   native code behind a vat interface
2. spawns the REPL vat with refs to those capability vats
3. runs the scheduler

everything above the init vat is moof objects. the init vat is
the only thing that touches raw native code directly.

## Env and Vat

each vat has one env. each env belongs to one vat. tightly linked.

- env bindings are a Table slot (not parent object — all envs
  delegate to Env prototype for behavior)
- `get` walks the parent env chain
- `set` is always local
- closures create child envs with parent refs
- `def` mutates the current vat's env (local, not an Act)
- `let` creates a child env (also local)

## the capability hierarchy

```
Capability (root protocol)
  ├── IO           println: readLine print: flush
  ├── Time         now measure: sleep:
  ├── Randomness   next nextIn: shuffle: seed:
  ├── Persistence  save: load: query: delete:
  ├── Network      fetch: listen: connect:
  ├── Canvas       render: onClick: onDrag:
  ├── Native       call: (raw FFI — most dangerous)
  └── Env          lookup: define: bindings
```

each is a protocol. each is a vat. each can be wrapped
(membrane), mocked, restricted, or withheld entirely.

## spawning

```moof
; spawn with explicit capabilities
[Vat spawn: |console clock|
  (do [console println: "started"]
      (now <- [clock now])
      [console println: [now describe]])]

; spawn pure — no capabilities
[Vat spawn: || [42 * 2]]

; spawn with membrane-wrapped capabilities
[Vat spawn: |(logged console) clock|
  [console println: "this gets logged"]]
```

the spawner decides what capabilities the child gets.
principle of least authority.

## the rules

1. **all effects are cross-vat sends.** no exceptions.
2. **every capability is a vat.** Console, Clock, Store — all vats.
3. **cross-vat sends return Act.** the one effectful type.
4. **Acts are inspectable data.** rewrite, log, replay, mock.
5. **pure code has no vat refs.** guaranteed by architecture.
6. **do + <- composes Acts.** monadic notation, backwards compatible.
7. **Flatmappable unifies composition.** Acts, Options, Lists, Results.
8. **no try/catch.** errors propagate monadically. recover explicitly.
9. **the spawner controls capabilities.** principle of least authority.
10. **the REPL is a vat.** the init vat (rust runtime) is also a vat.

## remaining design work

**mutation + reactivity**: `:=` and `at:put:` are local to the vat.
Observable protocol notifies watchers. reactive system is within-vat.
notebook model: cells = env bindings, dependencies tracked via
Observable, recomputation is pure, only rendering crosses a boundary.
needs more fleshing out.

**destructuring in do-blocks**: `(do ((x y) <- [Act all: ...]) ...)`
needs pattern destructuring for Act results. ties into the pattern
matching story.

**pure vat as sandbox**: a vat with zero capabilities can compute,
create objects, define functions — but cannot talk to the world.
unit of deterministic replay, memoization, content-addressing.
could be the basis for speculative execution and safe parallelism.

**Act fusion/optimization**: consecutive sends to the same capability
vat could be batched. the scheduler could reorder independent Acts.
since Acts are data, an optimizer can inspect and transform them.
