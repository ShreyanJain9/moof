# servers: actors in the objectspace

## the idea

a server is an object that lives in its own vat. that's it.

moof objects already have state (slots) and behavior (handlers).
a vat already provides isolation (private heap), sequential
message processing (one at a time), and an effect boundary
(cross-vat sends return Acts). put an object in a vat and you
have an actor.

`defserver` is the bridge. it takes something that looks like
an object literal and gives it its own vat. you get a FarRef
back. sends to the FarRef become cross-vat messages, processed
one at a time, returning Acts. the server's state is private.
its capabilities are explicit. it's the actor model, naturally.

## defining a server

```moof
(defserver Counter (console)
  "A counter with logging."

  count: 0

  [increment] (do
    (:= @count [@count + 1])
    @count)

  [decrement] (do
    (:= @count [@count - 1])
    @count)

  [get] @count

  [reset] (:= @count 0)

  [log]
    [console println: (str "count: " [@count describe])])
```

the parts:

- `Counter` — the server name. becomes a constructor.
- `(console)` — required capabilities. must be provided at
  construction time.
- `"A counter..."` — docstring. queryable at runtime.
- `count: 0` — a slot with a default value. mutable via `:=`.
  private to the server's vat.
- `[increment]`, `[get]`, etc. — handlers. same syntax as
  object literal handlers. these are the server's public API.

the body IS an object literal. the only additions are the name,
the capability list, and the docstring. if you know how to write
moof objects, you know how to write servers.

## construction

```moof
(do (c <- (Counter { console: console }))
    [c increment]
    [c increment]
    (n <- [c get])
    [console println: (str "final count: " [n describe])])
```

`(Counter { console: console })` — call the constructor with a
config object. the config's slots are matched against the
capability declaration `(console)`. missing keys = error.

the constructor returns an Act. the Act resolves to a FarRef
pointing at the server object in its vat.

### why a config object?

capabilities are named, not positional. order doesn't matter.
self-documenting at the call site. scales to many capabilities
without combinatorial keyword messages:

```moof
(do (g <- (GameEngine {
       console: console
       clock:   clock
       store:   store
       network: network }))
    [g start])
```

### init

if the server defines an `[init]` handler, it runs after
construction, inside the server's vat:

```moof
(defserver Counter (console)
  count: 0

  [init] (do
    [console println: "counter started"]
    (:= @count 0))

  [increment] ...)
```

`[init]` can do IO (it has capability refs). its return value
is ignored — construction always resolves to the FarRef.

## the actor model

servers are actors. not "inspired by" — literally are.

```
actor model           moof server
─────────────         ─────────────────────
actor                 server object in its own vat
mailbox               vat message queue
receive               handler dispatch
state                 slots (private to the vat)
send                  cross-vat message → Act
sequential processing one handler at a time (vat is single-threaded)
location transparency FarRef (same interface local or remote)
become                change prototype delegation
```

### sequential processing

the vat processes one message at a time. no concurrency within
a server. this means:

```moof
; these are sequenced — second increment sees first's result
(do [counter increment]
    [counter increment]
    [counter get])
; → Act<2>
```

no locks. no races. the vat's single-threaded execution is the
synchronization mechanism.

### become

an actor can change its behavior for future messages. in moof,
this is prototype delegation:

```moof
(def Locked {
  [increment] (error "counter is locked")
  [unlock] [self become: Unlocked]})

(def Unlocked {
  [increment] (do (:= @count [@count + 1]) @count)
  [lock] [self become: Locked]})

(defserver Counter ()
  Unlocked   ; delegate to Unlocked initially
  count: 0)
```

## lifecycle

### stopping

every server responds to `[stop]`. defserver generates a default
that marks the vat Dead. override it for cleanup:

```moof
(defserver Logger (console store)
  entries: nil

  [log: msg] (do
    (:= @entries (cons msg @entries))
    nil)

  [stop] (do
    [store save: 'log value: @entries]
    [console println: "logger flushed and stopped"]))
```

after `[stop]` runs, the vat is marked Dead. pending messages
are dropped. pending Acts on this vat resolve as failed with
a "vat stopped" error.

`[server stop]` returns an Act that resolves when the stop
handler completes.

### crash

if a handler raises an error and there's no `recover:`:

1. the error propagates to the caller's Act (resolves as failed)
2. the vat stays alive — one bad message shouldn't kill the server
3. the server continues processing the next message

this is a deliberate choice: handlers are isolated from each
other. a crash in `[increment]` doesn't affect the next
`[get]`. state may be inconsistent, but the server is still
responsive.

for "let it crash" erlang-style, you'd override this:

```moof
(defserver StrictCounter ()
  count: 0
  [increment] (do (:= @count [@count + 1]) @count)

  ; any handler error → stop the server
  [handleError: e] [self stop])
```

### supervision (future)

a supervisor is a server that monitors other servers and
restarts them on crash:

```moof
(defserver CounterSupervisor (console)
  counter: nil

  [init] (do
    (:= @counter (Counter { console: console }))
    nil)

  [restart] (do
    [console println: "restarting counter"]
    (:= @counter (Counter { console: console }))
    nil))
```

proper supervision trees (one-for-one, one-for-all, rest-for-one)
are future work. the architecture supports it — supervisors are
just servers that hold FarRefs to children.

## compute vats vs server vats

two kinds of vat spawning:

**compute vats** — `[Vat spawn: || expr]`. run a computation,
return the result, vat goes Dead. one-shot. used for parallelism.

```moof
(do (x <- [Vat spawn: || [expensive-computation data]])
    [process x])
```

**server vats** — `(defserver ...)`. create an object, vat stays
alive, FarRef returned. persistent. used for stateful services.

```moof
(do (c <- (Counter { console: console }))
    [c increment]  ; vat stays alive between sends
    [c increment])
```

the scheduler marks compute vats Dead after their result is
copied. server vats stay alive until `[stop]` or crash.

## capabilities

capabilities are FarRefs passed at construction time. the server
declares what it needs:

```moof
(defserver MyServer (console clock store)
  ...)
```

this means:
- the constructor requires a config with `console`, `clock`,
  and `store` keys
- inside handlers, these names are bound to FarRefs
- the server can only do IO through these refs
- withhold a capability = the server can't use it

### principle of least authority

the spawner decides what the child gets:

```moof
; full access
(MyServer { console: console clock: clock store: store })

; read-only store (wrap in a membrane)
(MyServer { console: console
            clock: clock
            store: (read-only store) })

; no store at all — constructor error
(MyServer { console: console clock: clock })
; → error: missing required capability 'store'
```

membranes (wrappers that intercept sends) are future work but
the architecture supports them naturally — a membrane is just
a FarRef that logs/filters/transforms messages.

### testing

servers are trivially testable because capabilities are injected:

```moof
; in production
(Counter { console: real-console })

; in tests — mock console records sends
(def mock { messages: nil
  [println: msg] (:= @messages (cons msg @messages)) })
(Counter { console: mock })
```

no test frameworks, no dependency injection libraries. pass a
different object. that's it.

## config objects and destructuring

server construction uses config objects — moof's native named
parameter pattern:

```moof
{ console: console  clock: clock  store: store }
```

this is an object literal. keys are symbols. values are
anything. order doesn't matter. it's the general solution to
"pass a bundle of named things."

inside defserver, the vau destructures the config: extracts
each required capability by name and binds it as a local.
conceptually:

```moof
; what defserver generates (roughly):
(fn (config)
  (let ((console [config slotAt: 'console])
        (clock   [config slotAt: 'clock]))
    ... spawn vat, create object ...))
```

### a general destructuring form (future)

config objects motivate a general destructuring bind:

```moof
(let-slots { x y z } config
  [x + y + z])
```

or pattern matching on object shape:

```moof
(match config
  { host: h port: p } [connect h p]
  { url: u }          [parse-url u])
```

these are language features that benefit everything, not just
servers. design work for the surface syntax phase.

## introspection

servers are objects. they respond to the same queries as any
moof object:

```moof
[counter doc]           ; → "A counter with logging."
[counter handlerNames]  ; → (increment decrement get reset log stop)
[counter slotNames]     ; → (count)
```

the FarRef transparently proxies these — they're just messages.
the agent (LLM in a vat) can discover what a server does by
querying its handlers and docs.

## the defserver vau

`defserver` is a vau operative. it:

1. parses the capability list and body
2. defines a constructor function in the current environment
3. the constructor:
   a. validates the config object against required capabilities
   b. spawns a new vat
   c. creates the server object inside the vat (with capability
      refs bound as locals)
   d. registers the object as the vat's root (for message dispatch)
   e. runs `[init]` if defined
   f. returns an Act that resolves to a FarRef

this is implemented as a moof vau, not compiler magic. you can
inspect it, override it, wrap it. `(def defserver ...)` is just
a binding. the server pattern is a library, not a language feature.

## example: a key-value store server

```moof
(defserver KVStore (console)
  "A persistent key-value store with logging."

  data: {}

  [get: key]
    [[@data slotAt: key] or: nil]

  [put: key value: val] (do
    (:= @data [[@data clone] slotAt: key put: val])
    [console println: (str "stored " [key describe])]
    val)

  [keys]
    [@data slotNames]

  [size]
    [[@data slotNames] length]

  [stop]
    [console println: (str "kvstore stopped (" [self size] " keys)")])
```

usage:

```moof
(do (kv <- (KVStore { console: console }))
    [kv put: 'name value: "moof"]
    [kv put: 'version value: 2]
    (name <- [kv get: 'name])
    [console println: name]
    [kv stop])
```

## summary

1. **a server is an object in its own vat.** that's the whole idea.
2. **defserver body = object literal.** slots, handlers, delegation.
3. **capabilities are explicit.** config object, named, validated.
4. **construction returns Act<FarRef>.** async by nature.
5. **one message at a time.** vat is the synchronization primitive.
6. **[stop] ends the vat.** override for cleanup. default provided.
7. **crashes are isolated.** one bad handler doesn't kill the server.
8. **servers are testable.** inject mock capabilities. done.
9. **defserver is a vau.** library, not language. inspectable, overridable.
