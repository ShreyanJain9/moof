# throughlines

**type:** concept (meta)

> moof has a lot of surface concepts — Acts, Updates, streams,
> protocols, schemas, URLs, capabilities, vats, membranes,
> delegation, content-addressing. underneath, there are five
> deep patterns that keep recurring. this doc names them. once
> you see them, everything clicks into place.

---

## 1. contexts — "a value, in something"

**the surface concepts**

- `Option` — a value, in "might be absent"
- `Result` — a value, in "might have failed"
- `Cons` — a value, in "sequence of elements"
- `Stream` — a value, in "values arriving over time"
- `Act` — a value, in "a computation still running"
- `Update` — a value, in "a state change accompanying the reply"

**the unifying pattern**

each of these is a **context** — a wrapper around a value that
carries extra structure. the wrapping determines how you compose
computations over the wrapped value.

moof's `Monadic` protocol is the contract for "things you can
chain computations through." every context above conforms. the
single syntax for chaining is `(do ...)`:

```moof
(do
  (user <- [users <- get: 'alice])    ; an Act context
  (addr <- user.address)               ; a possibly-absent (Option)
  (valid <- (validate addr))           ; a possibly-failed (Result)
  [console <- println: valid])         ; another Act
```

one notation — `(do ...)` — handles Act chains, Option chains,
Result chains, Cons comprehensions, Stream pipelines,
Update-merge sequences. same shape, different flavor.

**why this matters**

most languages have five separate features for these five cases:
- JS: Promise.then, array comprehensions, null-coalescing,
  try/catch, generators
- rust: ?-operator, async/await, iterators, Option::map,
  Result::and_then
- each is its own syntax, its own mental model

moof: **one pattern, five instances.** you learn "value-in-
context" once. you compose them the same way. you don't
special-case at every boundary.

**the corollary**

if you find yourself wanting a new way to compose "a value in
some ongoing situation" — stop. can the situation be an Act, a
Result, a Stream, or a custom Monadic? if yes, reuse. if no,
write a new Monadic conformer, not a new composition syntax.

the user said: "stop adding parallel abstractions." this is
why.

---

## 2. constraints — "a claim about a value, at some phase"

**the surface concepts**

- `Protocol` — a claim about what handlers a value responds to
- `Schema` — a claim about what slots a value has, with types
- an **optional static type** — a claim checked at compile time
- a `Capability` — a claim that you can reach something

**the unifying pattern**

each of these is a **constraint** — a declarative assertion
about a value or reference. constraints differ in:

- **what they constrain**: handlers (protocol), slots (schema),
  types/arity (static types), reachability (capability).
- **when they're checked**: runtime dispatch (protocol),
  construction (schema), compile time (static types), send
  time (capability).

but they share: **declarative, first-class, composable,
queryable.** a constraint is a moof value. you can inspect it,
pass it around, combine it (`and`, `or`, `not`), document it.

```moof
; a protocol constraint — handler-level
(defprotocol Iterable (require (fold:with: init f)))

; a schema constraint — slot-level (planned)
(defshape Recipe (requires (title String) (steps Integer)))

; a static-type constraint — compile-time (future)
(declare map: (fn [Iterable a] [fn a b] → [Iterable b]))

; a capability constraint — reachability-level
; implicit: you hold a FarRef → you can send it
```

all four describe what a value IS or CAN DO. the phase-of-check
is an optimization detail, not a kind-difference.

**why this matters**

- today moof has protocols (runtime) and an emergent
  schema-by-habit.
- tomorrow: schemas for data contracts, optional types for
  compile-time guards, capabilities for reach bounds — all
  using the same mental model.
- no haskell/typescript/clojure schism. one concept, four
  phases.

**the corollary**

when you want a new kind of check, first ask: is this a
protocol-level, schema-level, static-type-level, or reach-
level constraint? pick ONE phase; implement within that layer's
machinery; don't invent a new kind of contract.

---

## 3. walks — "a path through a graph of objects"

**the surface concepts**

- **URL resolution** — `moof:/caps/console` is a path through
  the namespace tree.
- **prototype dispatch** — a message walk from the receiver up
  its proto chain.
- **delegation** — walking the chain until a handler is found.
- **federation** (future) — walking across peer boundaries:
  `moof:peer/alice/vats/12`.
- **content-addressing** — walking a hash-indexed DAG of
  immutable values.
- **env lookup** — walking parent scopes until a name is found.

**the unifying pattern**

**everything in moof is a walk through a graph of objects.**
the graph differs:

| walk | graph | step |
|------|-------|------|
| URL resolve | namespace tree | `[table at: segment]` |
| dispatch | proto chain | `obj.proto` |
| env lookup | env parent chain | `env.parent` |
| federation | peer graph | network hop |
| content-addressed fetch | hash DAG | hash→blob lookup |

but the abstraction — *traverse a reference structure until you
find what you're looking for* — is uniform.

one generic operation would suffice for all of them. in practice
we have specialized versions (because of different
retrieval semantics) but they're the SAME move at different
scales.

**why this matters**

plan-9 gave us: "everything has a path." smalltalk gave us:
"messages walk proto chains." git gave us: "content-addressed
hashes walk a DAG." moof integrates all three.

the mental model: **to do anything with a value, you walk to it,
then apply something.** this is the scheme/lisp consmaster's
insight at every scale.

**the corollary**

when you invent a new way to find/address something, the first
question is: *which existing graph is this a walk through?*
almost always the answer is one of: namespace, proto chain, env
chain, or content-DAG. if none fit, you're inventing a sixth
graph — that's a big design decision, justify it.

---

## 4. additive authoring — "building up, never writing over"

**the surface concepts**

- `(def x 3)` — bind x in the current scope (new binding).
- `(defmethod T sel: (...) ...)` — add a handler to a prototype.
- `(conform T P)` — declare T conforms to protocol P.
- `[obj with: { x: 99 }]` — produce a new object with x
  overridden (old object unchanged).
- `(update { x: 99 } reply)` — a server's delta (applied
  atomically between messages).
- halo-authored-handler (future) — click to add a handler via
  canvas gesture.

**the unifying pattern**

**every authoring gesture ADDS. none writes over in place.**

- bindings add to an environment (or shadow in a new scope).
- conformances add to a protocol's conformer list + a type's
  handler set.
- handlers add to a prototype.
- Updates add to a server's slot values *atomically, producing
  a new state*.
- `with:` returns a new object, the old one untouched.

you can't mutate a slot in place. you can't rebind a name in a
locked scope. you can't delete a handler mid-send. every
change is an ADDITION to the state the system knows about. the
old state doesn't vanish — it just stops being current.

**why this matters**

- time-travel is free: every old state is reachable.
- audit is free: every change is an event.
- replay is free: apply the events in order, get the state.
- sharing is safe: old references stay valid even after new
  changes.
- live editing is safe: in-flight computation holds old
  handlers; new computation uses new ones.

erlang's hot-code-swap and lisp's REPL redefine work for the
same reason: everything is ADDITIVE under the hood. moof
commits to this top-to-bottom.

**the corollary**

if you're designing a moof feature and you find yourself wanting
to MUTATE something — stop. what you really want is an ADDITION
that makes the old version non-current. figure out what the new
version's identity is. compose it.

---

## 5. canonical form — "a value knows its shape everywhere"

**the surface concepts**

- **content-addressing** — value → canonical bytes → hash.
- **equality** — two values equal iff canonical forms match.
- **hashing** — the Hashable protocol uses the canonical form.
- **cross-vat copy** — serialize canonical, deserialize on the
  other side.
- **image save** — canonical bytes in the blob store.
- **federation** — send a hash; peer hydrates from its cache if
  it has the bytes.

**the unifying pattern**

**every immutable moof value has ONE canonical byte form.** that
byte form is the value's name, its identity, its transport, its
equality test. everything downstream follows:

- identity: two values have the same hash iff they have the
  same canonical bytes.
- equality: `[a equal: b]` is "do the canonical bytes match?"
- hashing: `[v hash]` is "first 48 bits of BLAKE3 over
  canonical bytes."
- serialization: "serialize" is "produce canonical bytes."
- cross-vat copy: send bytes, receiver deserializes.
- content-addressing: the hash IS the URL.
- federation: my hash matches yours → you already have it.

one format solves six problems.

**why this matters**

git did this for blob content. IPLD extended it. moof applies
it to arbitrary typed values. the payoff: dedup across the
entire ecosystem, for free. if a million moof users each produce
`(list 1 2 3)`, the list is stored once per machine, globally.

**the corollary**

when adding a new foreign type, the canonical form isn't a
nice-to-have — it's the point. the canonical form is part of
the type's design, not an afterthought. "how do i canonicalize
this?" is the first question; "does it work?" is the second.

---

## 6. time as an axis

**the surface concepts**

- an **image** has a "current" state.
- a **history** is a record of state changes.
- a **snapshot** is a named point on the history axis.
- a **stream** yields values over time.
- an **Act** is a value pending in time.
- time-travel (future): "show me yesterday's state."

**the unifying pattern**

**time is a first-class axis moof navigates explicitly.** not
implicitly, not "it just happens" — you can point at a moment
and ask for the state at that moment. you can subscribe to "all
states from T onward." you can take a snapshot and compare it
to now.

once wave 10+ persistence makes running-state durable, this
extends: vats themselves have timelines. you scrub a vat
backward and see it mid-computation. you fork a vat at T and
let its new branch diverge.

**why this matters**

three consequences:

- **time is a view axis.** the user can filter by time as
  naturally as by type. "show me my workspace as it was on
  march 5."
- **debugging is archaeology.** bugs aren't "find the current
  state that produces the wrong result" but "find when the
  state went wrong and what produced it."
- **collaboration is merge.** alice and bob both fork your
  image at T, diverge, reconcile. git for objects.

the reason engelbart's bootstrap worked at NLS was that they
could review yesterday's work. modern software rarely lets you.
moof does.

**the corollary**

when a feature treats time as implicit ("just the current
state"), check: does it break if the user wants to scrub? if
yes, the feature needs time-parametric treatment. most things
do.

---

## the unifications, together

| throughline | one word | five examples |
|-------------|----------|---------------|
| contexts | "wrapped" | Option, Result, Cons, Stream, Act, Update |
| constraints | "claim" | Protocol, Schema, Type, Capability |
| walks | "path" | URL, proto chain, env, federation, content DAG |
| additive authoring | "add" | def, defmethod, conform, with:, Update |
| canonical form | "bytes" | hash, equal, save, copy, federate |
| time | "axis" | image, history, snapshot, stream, Act |

---

## how the throughlines compose

these aren't independent; they interlock.

- **contexts × walks**: `(do ...)` over Acts is "walk through
  time across vat boundaries, accumulating values."
- **constraints × canonical form**: a schema is a constraint
  that the canonical bytes conform to a named shape.
- **additive × time**: every addition moves time forward; the
  history is the record of additions.
- **walks × canonical form**: `moof:<hash>` is a path to a
  content-addressed value; `moof:/vats/X` is a path to a live
  one. same URL scheme, same vocabulary.
- **additive × constraints × time**: you ADD a new conformance
  at time T; values created after T respond to the new
  handlers; old values might need a migrator. this is moof's
  version control.

once you see the throughlines, morphic and federation aren't
new layers — they're new applications of the same patterns.
the canvas renders the object graph via walks + aspects
(contexts). federation extends walks across peers. reactive
signals are streams (contexts) with constraints (shape
contracts) on their elements.

---

## the "no new abstractions" rule

every throughline above includes a **corollary** that boils down
to: **before adding a new abstraction, check if an existing
throughline covers it.**

- new composition syntax? first try Monadic.
- new contract kind? first try an existing constraint layer.
- new addressing scheme? first try an existing graph.
- new mutation? you meant addition.
- new identity? first try canonical form.
- new "doing X at some time"? first try an axis you already
  have.

this is how moof stays coherent while growing. new features
reuse deep patterns. the language doesn't sprawl.

if the user insists on a new abstraction — fine, that can be
right sometimes. but: it should be justified by saying
"throughline N doesn't cover this because X." not "i couldn't
be bothered to check."

---

## reading order after this

once these five patterns are internalized, every concept doc
is a specialization:

- [concepts/objects.md](concepts/objects.md) — the material all
  throughlines operate on.
- [concepts/messages.md](concepts/messages.md) — walks + sends.
- [concepts/protocols.md](concepts/protocols.md) — constraints
  on handlers.
- [concepts/schemas.md](concepts/schemas.md) — constraints on
  slots.
- [concepts/vats.md](concepts/vats.md) — the boundaries between
  walks.
- [concepts/effects.md](concepts/effects.md) — contexts for
  cross-vat computation.
- [concepts/streams.md](concepts/streams.md) — contexts over
  time.
- [concepts/persistence.md](concepts/persistence.md) — canonical
  form + history.
- [concepts/addressing.md](concepts/addressing.md) — walks, in
  detail.
- [concepts/capabilities.md](concepts/capabilities.md) —
  constraints on reachability.
- [concepts/authoring.md](concepts/authoring.md) — additive
  gestures, in the UI.

each specializes one or two throughlines. you'll see them
recur.

---

## one sentence

**moof is five patterns — contexts, constraints, walks,
addition, canonical form — played against one axis (time), over
one material (objects).** everything surface in moof is an
instance.
