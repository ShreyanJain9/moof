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

  [increment]
    (update { count: [@count + 1] }
            [@count + 1])

  [decrement]
    (update { count: [@count - 1] }
            [@count - 1])

  [get] @count

  [reset] (update { count: 0 })

  [log]
    [console println: (str "count: " [@count describe])])
```

the parts:

- `Counter` — the server name. becomes a constructor.
- `(console)` — required capabilities. must be provided at
  construction time via config object.
- `"A counter..."` — docstring. queryable at runtime.
- `count: 0` — a slot with initial value. immutable within
  a handler — state changes only via `update`.
- `[increment]`, `[get]`, etc. — handlers. same syntax as
  object literal handlers. these are the server's public API.

the body IS an object literal. the only additions are the name,
the capability list, and the docstring. if you know how to write
moof objects, you know how to write servers.

## handler return types

handlers are pure functions. they return one of four things:

**plain value** — query, no state change.

```moof
[get] @count
; vat sends @count as reply. state unchanged.
```

**Update** — state transition with optional reply.

```moof
[increment]
  (update { count: [@count + 1] }   ; delta (which slots change)
          [@count + 1])              ; reply to caller

[reset] (update { count: 0 })
; delta applied. reply is nil.
```

`update` creates an Update value — inspectable data, like Act.
the vat examines the return and applies the delta between
messages. the handler itself never mutates anything.

**Act** — effect, no state change.

```moof
[log]
  [console println: @count]
; cross-vat send → Act. vat executes effect, sends result.
```

**Act resolving to Update** — effectful state transition.

```moof
[save] (do
  [store save: 'count value: @count]
  (update { saved: true }))
; effect first, then state change after effect completes.
```

### snapshot semantics

inside a handler, `@slot` reads from an immutable snapshot
of the server's current state. the snapshot is taken when the
handler starts. the handler never sees its own state changes.

```moof
[increment]
  (update { count: [@count + 1] }
          [@count + 1])
; @count is always the PRE-increment value within this handler.
```

handlers are pure: (current-state, message-args) → return value.
no hidden mutation. referentially transparent.

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
self-documenting at the call site. scales to many capabilities:

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
    [console println: "counter started"])

  [increment] ...)
```

`[init]` can do IO. its return value is ignored —
construction always resolves to the FarRef. if `[init]`
returns an Update, the delta is applied before the first
external message.

## the actor model

servers are actors. not "inspired by" — literally are.

```
actor model           moof server
─────────────         ─────────────────────
actor                 server object in its own vat
mailbox               vat message queue
receive               handler dispatch
state                 slots (immutable snapshots, Update deltas)
send                  cross-vat message → Act
sequential processing one handler at a time (vat is single-threaded)
location transparency FarRef (same interface local or remote)
become                change prototype delegation
```

### sequential processing

the vat processes one message at a time. no concurrency within
a server. state transitions are atomic — the delta from one
handler is applied before the next message is processed.

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
  [increment] (update { count: [@count + 1] } [@count + 1])
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

  [log: msg]
    (update { entries: (cons msg @entries) })

  [stop] (do
    [store save: 'log value: @entries]
    [console println: "logger flushed and stopped"]))
```

after `[stop]` runs, the vat is marked Dead. pending messages
are dropped. pending Acts on this vat resolve as failed with
a "vat stopped" error.

### crash behavior

if a handler raises an error:

1. the error propagates to the caller's Act (resolves as failed)
2. the vat stays alive — one bad message shouldn't kill the server
3. no state change (the handler didn't return an Update)
4. the server continues processing the next message

handlers are isolated from each other. a crash in `[increment]`
doesn't affect the next `[get]`.

### supervision (future)

a supervisor is a server that monitors children and restarts
them on crash. architecture supports it — supervisors hold
FarRefs. proper supervision trees are future work.

## compute vats vs server vats

**compute vats** — `[Vat spawn: || expr]`. one-shot. run a
computation, copy result to parent, vat goes Dead.

```moof
(do (x <- [Vat spawn: || [expensive-computation data]])
    [process x])
```

**server vats** — `(defserver ...)`. persistent. object stays
in the vat. FarRef returned. vat stays alive between messages
until `[stop]`.

```moof
(do (c <- (Counter { console: console }))
    [c increment]
    [c increment])
```

## capabilities

capabilities are FarRefs passed at construction via config
object:

```moof
(defserver MyServer (console clock store)
  ...)

(MyServer { console: console clock: clock store: store })
```

this means:
- constructor validates all required caps are present
- inside handlers, cap names are bound to FarRefs
- the server can only do IO through these refs
- withhold a capability → server can't use it
- pass a membrane → server gets filtered access

### testing

servers are trivially testable — inject mock capabilities:

```moof
; in production
(Counter { console: real-console })

; in tests — mock console records sends instead of printing
(def mock { messages: nil
  [println: msg]
    (update { messages: (cons msg @messages) }) })
(Counter { console: mock })
```

no test frameworks. no dependency injection libraries.
pass a different object. that's it.

## introspection

servers are objects. they respond to standard queries via
the FarRef:

```moof
[counter doc]           ; → "A counter with logging."
[counter handlerNames]  ; → (increment decrement get reset log stop)
[counter slotNames]     ; → (count)
```

the agent (LLM in a vat) discovers server APIs by querying
handlers and docs. same mechanism as any moof object.

## the defserver vau

`defserver` is a vau operative, not compiler magic. it:

1. parses the capability list and body
2. defines a constructor function in the current environment
3. the constructor:
   a. validates the config object against required capabilities
   b. spawns a new vat
   c. creates the server object inside the vat (with capability
      refs bound in scope)
   d. registers the object as the vat's root
   e. runs `[init]` if defined
   f. returns an Act that resolves to a FarRef

inspectable, overridable, composable. the server pattern is a
library, not a language feature.

## example: key-value store

```moof
(defserver KVStore (console)
  "A key-value store with logging."

  data: #[]

  [get: key]
    [@data at: key]

  [put: key value: val] (do
    [console println: (str "stored " [key describe])]
    (update { data: [@data at: key put: val] }
            val))

  [keys]
    [@data keys]

  [size]
    [[@data keys] length]

  [stop] (do
    [console println: (str "kvstore stopped (" [[self size] describe] " keys)")]))
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
3. **handlers are pure.** return plain values, Updates, or Acts.
4. **Update is a value.** state deltas are data, not side effects.
5. **capabilities via config object.** named, validated, mockable.
6. **one message at a time.** vat is the synchronization primitive.
7. **[stop] ends the vat.** override for cleanup. default provided.
8. **crashes are isolated.** bad handler → failed Act, server lives on.
9. **defserver is a vau.** library, not language. inspectable, overridable.
