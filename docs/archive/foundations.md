# foundations

> opinions on what has to be unbreakable before we add any more
> surface. persistence, purity, effects, addressability, and
> time — the five pillars everything else rests on.

everything moof eventually becomes — the canvas, the inspector,
federation, authoring-for-all — assumes that the substrate
underneath holds. if the image isn't trustworthy, a beautiful notebook UI is just a mirage. if the effect system is leaky, capability security is theater. if values don't have stable identities, transclusion and sharing can't exist.

so before we pour more surface on top, the foundations have to
be boring-reliable. this doc is an opinionated walk through the
five things i think need to be rock-solid and what "rock-solid"
means concretely.

---

## 1. persistence

the image is moof's defining feature — a single, accretive
objectspace that outlives processes. it has to be:

### content-addressed

every immutable value has a stable hash derived from its content.
`(list 1 2 3)` in my image has the same hash as `(list 1 2 3)`
in yours. this is what makes:

- sharing cheap (just send the hash, receiver already has it)
- deduplication automatic (same value, stored once)
- caching safe (hash is a cache key with no invalidation problem)
- federation possible (machines talk in hashes, resolve lazily)

we already hash for `Hashable`, but hashing and content-addressing
aren't the same thing — we need canonical serialization so
`{a:1 b:2}` and `{b:2 a:1}` produce identical bytes. that means
sorting slots by symbol id, fixing endianness, and stabilizing
our foreign-type serde formats.

**decision**: symbols sort by name (not by intern-id, which is
session-local). canonical form is sorted. `schema_version` gets
encoded into every content-addressed blob so old hashes never
alias new ones.

### append-only log + snapshot

writes are an append-only log of events (message deliveries,
server-state deltas, spawns). snapshots are periodic compactions
of the log. this gives us:

- crash safety for free — log tail is the single source of truth
- scrubbable history — any point in the log is a valid state
- cheap replication — ship the log, replay deterministically
- branching — a snapshot + alternate log tail = a parallel
  timeline

today we use heed (lmdb) for key-value. that's fine for the
snapshot half. the log half is missing. i'd add a simple append
log with a fsync discipline (batched, not per-event) and a
background compaction that rolls old events into a new snapshot.

### structural sharing for free

cons cells already share structurally. tables should too — a
persistent hashmap (HAMT-like) instead of copy-on-write dicts.
this means "changing one slot in a 10k-entry object" copies
log(n) nodes, not all n. the 10k-entry object can be versioned
cheaply. time-travel stops being a theoretical feature and
becomes a daily gesture.

slint/im-rs has good HAMT tech we can crib. the user-visible
contract stays the same — `[table at: k put: v]` returns a new
table — but internally we pay log(n) instead of n.

### gc that never lies to persistence

current gc marks from the VM's roots. that's safe within a
process. it's not safe across processes unless snapshots pin
their closures. the contract has to be: **once a value is
persisted, it can never be gc'd out from under a future session**
until it's explicitly forgotten.

this means:
- gc walks the store's root set in addition to the VM's
- every persisted Act, defserver, or named binding is a root
- garbage collection is really a negotiation: we delete only
  what no persisted thing transitively mentions

or we go further: **immutable values aren't gc'd, they're
reference-counted in the store, and when their refcount hits
zero *and* they're not in any snapshot's retention window,
they're evicted.** this matches git's gc model.

### stable across versions

when moof 2.1 loads a moof 2.0 image, every value must round-trip
or produce a clear "i can't read this" error. never silent
corruption. never best-effort. `schema_version` on every foreign
type is the hook; we enforce it on load.

migration happens through **migrator objects** — moof code a user
can write to transform old values to new shapes. the runtime
refuses to eval any schema it doesn't have a migrator for.

---

## 2. purity

moof has already made the right call: userland is pure, mutation
lives in servers behind acts. the question is how tight we want
the enforcement.

### no ambient mutation ever

the compiler already rejects `slotAt:put:`. good. extend this:
- any rust-side native handler that touches Heap mutably must
  be either a capability (effectful, clearly marked) or a pure
  constructor (allocates a new value, doesn't mutate existing ones)
- we should audit foreign-type handlers to ensure none of them
  mutate in place. `Atom` is the only exception and it does it
  via a capability boundary.

this is mostly done. the remaining hole: **the repl doesn't
distinguish pure from effectful evaluation.** when you type
`[x + 1]` vs `[console println: x]` the runtime treats them the
same. we should mark the second as yielding an Act that must be
drained.

right now: `println:` returns nil immediately and *also* prints
as a side effect of running. that's the hole.

fix: `[console println: x]` returns an `Act<nil>`. printing only
happens when the scheduler drains. this means printing is no
longer "instant" — but it becomes reproducible, replayable, and
cancellable. all wins.

### the Act type is the only effect marker

any function whose return type is `T` is pure. any function
whose return type is `Act<T>` is effectful. a pure function
calling an effectful function forces you to either:
- receive an `Act<T>` and propagate it up (`do`-notation)
- or explicitly `[act now]` to block (which is forbidden in
  pure context)

we already have the first. the second — forbidding `now`-in-pure
— is a future static check. for now the discipline is:
**if the function is called from within a pure context, it must
return a value, not an Act.** we enforce by convention and by
`do`-notation lifting everywhere.

### capability security is not ambient

every capability — console, clock, file, random — is a FarRef
held by whoever was granted it. no global access. no
`env.get("console")` backdoor. no magic.

we have this in principle; we should harden it:
- no capability bound in any vat's default environment besides
  the one that was explicitly granted
- when code is loaded from source, it doesn't inherit the
  granter's capabilities — the loader passes them explicitly
- a loaded closure that wasn't passed a capability can't
  synthesize one

the test: **can i write a moof program that calls the filesystem
without having been given the file capability?** if the answer
is ever "yes," something's leaking.

---

## 3. effects

acts are the right shape. they compose, they cancel, they can
fail. the remaining work:

### cancellation from day one

every Act should support cancellation. the caller can say "i
don't need this anymore" and the scheduler stops processing it.
this is particularly important for:

- timeouts: `[act withTimeout: 5000]`
- competing effects: `[race a b c]` — first to resolve wins,
  others cancel
- user-driven interrupts: closing the inspector kills its Acts

without cancellation, an effect system leaks work. we pay for
this now only because the toy programs we write are short.

### structured concurrency

when a vat spawns children, those children's lifetimes should be
scoped to the parent unless explicitly detached. no orphaned
vats consuming resources forever. this is the erlang-lineage
answer: supervision trees.

moof's vat model already supports this — we just haven't built
the discipline. i'd add:

- `[vat spawn: body]` returns the child vat-ref, bound to the
  parent's scope. when the parent exits, the child is killed.
- `[vat spawn: body detach]` opts out, for long-lived servers.

this is a runtime change, not a language change.

### back-pressure is a scheduler concern

a vat whose mailbox grows unboundedly is a leak. the scheduler
should:

- track mailbox depth per vat
- slow senders when a receiver is overloaded (CSP-style
  back-pressure)
- expose metrics so the inspector can see which vat is falling
  behind

today the scheduler has a fuel budget per turn, which is good.
but there's no feedback loop between receiver load and sender
rate. this is a wave-on-its-own when we get to real workloads.

### effects compose through protocols, not magic

`Thenable` is already the right abstraction. the only special
case is "class-side `pure:`" — the dual. we formalize:

```
Thenable
  instance: then:    — flatMap
  class:    pure:    — lift

Mappable (lifts on top of Thenable)
  instance: map:     — fmap = flatMap + pure
```

`Act` conforms to both. `Option`, `Result`, `Cons` conform to
both. `do`-notation works over anything `Thenable`. the user
writes:

```
(do (x <- effect1)
    (y <- effect2)
    [x + y])
```

and doesn't have to know whether `effect1` is an Act or an
Option. it works on whichever monad the first arrow lifted into,
and the rest follow.

this is mostly there. the remaining work is **enforcing the
laws** — right now we could write a `then:` that violates
associativity and nothing would catch it. a property-test suite
that exercises the monad laws on every conformance would be
cheap insurance.

---

## 4. addressability

this is the one that's genuinely missing and i think it's the
most important for the long-term authoring story.

### every value deserves a URI

a URI in moof is probably shaped like:

```
moof:<content-hash>              — content-addressed value
moof:vat/<vat-id>/obj/<obj-id>   — live object in a running vat
moof:peer/<peer-id>/...          — federated reference
```

content-hashes are for immutable values. `(list 1 2 3)` has the
same hash on every machine. sending the hash is equivalent to
sending the list — the receiver looks it up in their store, or
asks the network for it.

vat/obj references are for mutable state — defservers, live
objects. they're only valid within a running session.

peer references are for federation — "this thing exists on my
friend's machine, and here's how to reach it."

once every value has a URI:
- **transclusion becomes trivial.** a workspace block can say
  "render whatever is at moof:abc123" and the renderer resolves.
- **sharing becomes a gesture.** copy a URI, paste it. if the
  hash is known locally, done. if not, the network fetches.
- **links in any medium.** a paper scroll of your grimoire can
  have `moof:...` references printed on it. scan, resolve, get
  the live thing back.
- **the inspector has a real address bar.** navigate your
  objectspace like a filesystem or the web.

### no object without a name

every object created in a running vat gets an id. today that id
is a `u32` local to the vat. that's fine internally but it
doesn't cross machines. we need a globally-unique form that
encodes `(vat-id, obj-id)` plus the vat's cluster / peer context.

for immutable values, the hash replaces the id entirely. two
objects with the same content are the same object, identity-wise,
everywhere.

### equality aligns with addressing

`identical:` means same address. `equal:` means same content.
for immutable values these collapse. for mutable values they
don't — two Atom(0) have the same *content* but different
identities.

this is the cleanest statement of why mutability is the only
reason we need reference equality. and it's why the authoring
story (everything is a value in the grimoire) wants as little
mutability as possible — so `identical:` and `equal:` can
collapse for most of what users touch.

---

## 5. time

> "it's not what you're looking at, it's what you're looking at
> *a view of*." — engelbart

the image is append-only. every change adds events. which means
**every past moment of the image is recoverable** — not as a
feature we build later, but as the obvious consequence of how we
wrote the log.

### scrub points

the store records snapshot points (explicit or periodic). any
snapshot is a complete rehydrated past-moment. opening a snapshot
gives you:

- read-only view of the image as it was at that moment
- ability to fork from that point (new timeline)
- ability to compute diffs against the current state

this is git for your objectspace. the cli primitive is:

```
moof history          # list snapshots
moof checkout <hash>  # read-only open
moof fork <hash>      # writable copy in a new branch
moof diff a b         # what changed between snapshots
```

the repl primitive is:

```
[Image snapshotAt: <timestamp>]  — returns an Image value
[img inspectAt: <selector>]      — navigate
```

### undo is a universal affordance

since every mutation goes through an Act, and every Act can be
recorded, **undo is a built-in protocol**. any Act has an
inverse Act (conceptually: "apply the delta backwards"). the
inspector has an undo button; the user doesn't have to build it
per-app.

this is maybe the biggest engelbartian move we can make. it
requires:
- every defserver's Update to record enough to be invertible
  (store the previous delta alongside the new one)
- a global undo stack per workspace / per vat

the cost is storage — undo records grow. the mitigation is
snapshots + retention policy — old undo events roll up into
snapshots that lose individual-event granularity but preserve
the end state.

### tracing is built in

every message send is a potential observation point. the
scheduler should support a tracing mode where every delivery is
logged, with arguments and return values. this is what makes:

- time-travel debugging possible
- profiling possible
- the inspector's "why did this happen?" possible

we should not bolt this on later. the hooks go in now; the UI
comes later.

---

## what this means for our ordering

wave ordering, if i got to write it:

1. **content-addressing** — canonical serde, stable hashes,
   hash-as-identity for immutable values. touches every foreign
   type's serialize method.
2. **append-only log + snapshot discipline** — formalize the
   store as log + periodic snapshot, not a bag of keys.
3. **act cancellation + structured concurrency** — no more
   orphaned work.
4. **URIs and resolution protocol** — `moof:<hash>` resolution,
   first step toward transclusion and federation.
5. **history as a first-class protocol** — `[Image snapshotAt:]`,
   undo, diff, fork.
6. **static purity checks** — compile-time enforcement that pure
   code doesn't call effectful code without lifting.

only after the above do we build canvas, inspector, fancy
notebook UI. those are the *consequences* of a rock-solid
substrate; they're not what makes moof trustworthy.

---

## what we're not trying to be

- **a database.** moof's persistence isn't for transactional
  workloads with thousands of writers. it's for a personal
  image, maybe federated with a few peers. optimize for
  single-writer, snapshot-heavy, read-mostly patterns.
- **a smart contract platform.** the determinism story is for
  replay and debugging, not adversarial consensus. no byzantine
  tolerance expected.
- **erlang.** we're not trying to handle millions of messages
  per second. thousands is fine. the vat model is for isolation,
  not scale.
- **a browser.** even when we get a canvas, we're not reimplementing
  the web. we're implementing a *medium* in the kay / atkinson
  sense. different shape, different audience.

keeping these in mind keeps the foundations focused. we're
building a personal dynamic medium. everything rock-solid has to
serve *that*, not some other system's goals.
