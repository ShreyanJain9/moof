# persistence

**type:** concept
**specializes:** throughline 5 (canonical form), throughline 6 (time),
                 throughline 4 (additive)

> the image is moof's defining feature. everything persists by
> default. close your laptop, reopen, exactly where you were.
> persistence is canonical form (throughline 5) applied to the
> whole reachable graph, indexed across time (throughline 6),
> accreting additively (throughline 4).

---

## the throughlines applied

- **canonical form (throughline 5).** every immutable value has
  ONE canonical byte form. same content → same hash → same
  identity, anywhere. persistence IS: walking the reachable
  graph, canonicalizing each value, storing by hash. load is
  the reverse: hash → bytes → rebuild.
- **time (throughline 6).** the blob store accumulates; old
  hashes stay reachable; snapshots are named points on the
  history axis. time-travel is cheap because past states
  aren't overwritten — they're just not current.
- **additive (throughline 4).** every write is a new blob,
  a new ref, a new snapshot. mutation is a new value
  pointing at old ones. the history is a DAG of values added
  over time; GC (future) collects only what's no longer
  reachable from ANY retained snapshot.

understanding these three makes persistence obvious. the blob
store is a content-addressed DAG of canonical values that grows
monotonically; "loading the image" is walking roots down through
the DAG.

---

## the image is the artifact

in most languages, your program is a file on disk and your data is
somewhere else (sqlite, postgres, json files, memory-that-vanishes).
running the program is transient; persistence is a separate thing
you arrange.

in moof, there's one artifact: **the image.** it contains:
- every object you've created
- every prototype, handler, protocol definition
- every vat's heap state (soon: every vat's running state too)
- every named binding in every env
- the blob-store: content-addressed immutable values

when you close moof, the image is saved. when you open it, the
image is loaded. there's no "start my program" vs "load my data."
the image IS the program AND the data.

this is smalltalk's commitment carried into the present with
better plumbing.

---

## content-addressing

every **immutable value** has a content hash — a 256-bit identifier
derived from canonical serialization of the value's shape and
contents.

rules:
- same content → same hash. always. across machines, across time.
- different content → different hash (with astronomically high
  probability).
- canonical serialization handles the details: slots sorted by
  symbol name, fixed endianness, stable foreign-type encodings.
- cycles are handled with a fixpoint placeholder during hashing.

content-addressing is what makes:

- **sharing cheap.** send a friend a hash; if they already have the
  value (maybe from a different source), no transfer needed.
- **dedup automatic.** two vats that independently produce
  `(list 1 2 3)` store it once.
- **caching safe.** hashes are cache keys with no invalidation
  problem — content can't change under you.
- **federation possible.** machines exchange hashes and request
  only what's missing. see [addressing.md](addressing.md).

---

## the blob-store

content-addressed values live in an LMDB-backed blob store at
`.moof/store`. three tables:

- **blobs**: `hash → canonical-bytes`. the actual value data.
- **refs**: `name → hash`. mutable pointers (like git refs).
  `roots.env`, `roots.closure-descs`, `roots.type-protos` point at
  the current heap's root values.
- **meta**: version info, schema registry.

writes are transactional: the entire image snapshot (type protos,
closure descriptors, environment, every reachable object) goes in
one LMDB transaction.

on load, we pull the three root hashes, walk the value graph,
rebuild the heap. values that reference each other (cycles,
shared subvalues) are deduplicated via a memo: hash → heap-id.

---

## what persists, what doesn't

**persists** (in the blob store):
- every named env binding
- every type prototype and its handlers
- every closure's compiled bytecode (via closure_descs)
- every user-defined Service, Registry, Workspace
- every content-addressed value the roots reach

**does not persist today** (wave 10+):
- in-flight Acts (pending cross-vat sends)
- vat mailboxes (pending incoming messages)
- vat outboxes (pending outgoing messages)
- VM frames (what code is currently executing)
- the scheduler's ready queue

the consequence: today, reboot starts computationally fresh. the
objects are there; the in-flight work isn't. if you were
mid-computation when you quit, it's lost (the result might be in
an Act somewhere, but no vat is running toward it).

**wave 10 makes running state part of the image.** at that point,
reboot really is continuity — vats resume mid-computation, pending
Acts are still pending, in-flight messages arrive when the sender's
vat runs again.

see [../roadmap.md](../roadmap.md) for timing.

---

## GC and persistence together

persistence constrains garbage collection:

**rule**: once a value is persisted, it can never be GC'd out from
under a future session until it's explicitly forgotten.

this means:
- GC walks the blob-store's root set in addition to the VM's.
- every persisted Act, defserver, named binding is a root.
- collection is really a negotiation: we delete only what no
  persisted thing transitively mentions.

the blob store's refcounting (like git's) handles this: values the
current roots no longer reach become eligible for eviction,
subject to snapshot retention policy.

---

## schemas and migration

every foreign type (Rust-backed value types like BigInt, Vec3,
plugin-defined types) carries a `schema_version`. when moof loads
an image, it checks each foreign payload's schema version against
what's registered.

- match → load normally.
- mismatch + no migrator → load fails loudly. we refuse silent
  corruption.
- mismatch + migrator present → run migrator, transform old values
  to new shape.

migrators are moof code. a user can write one: "here's how to turn
a v1 Recipe into a v2 Recipe." the runtime uses it. no
behind-the-scenes format changes.

this is how moof images survive across runtime versions without
forcing you to export/import.

---

## structural sharing

immutable values share structure automatically.

- cons cells: `(cons x rest)` doesn't copy rest; both old and new
  lists share it.
- tables: today they're copy-on-write (a change copies the whole
  thing). planned: HAMT-backed (changing one slot in a 10k-entry
  table copies log(n) nodes, not n).
- general objects: `[obj with: { x: 99 }]` shares every unchanged
  slot with `obj`.

this makes versioning cheap. "what was this table yesterday?" is a
pointer lookup, not a deep clone.

---

## snapshots and time-travel

because every heap state has a root hash, "where were we an hour
ago?" is a question moof can answer. any past root hash identifies
a complete consistent state.

wave 10 adds an **append-only log** of events (message deliveries,
server updates, spawns). snapshots become compactions of the log.
this gives:

- **crash safety.** the log tail is always the truth.
- **scrubbable history.** any point in the log is a valid state.
- **cheap replication.** ship the log, replay deterministically.
- **branching.** snapshot + alternate log tail = parallel timeline.

today we have the snapshot half (LMDB transactional writes). the
append-only log is the remaining piece.

---

## the save cycle, today

```
on startup:
  open blob store at .moof/store
  if roots.env exists:
    load type_protos, closure_descs, env from blob-store
    hydrate into fresh vat 7 heap
    rewire capability FarRefs (URL → live refs)
    "image loaded into vat 7"
  else:
    fresh heap, run bootstrap sources

during runtime:
  every message, every eval, normal operation

on exit:
  save_image(vat 7):
    walk vat heap, write every reachable value to blob-store
    update roots.env, roots.closure-descs, roots.type-protos
    commit LMDB transaction
  exit
```

"save" isn't explicit. you don't call `(save-image)` to persist
your workspace; moof handles it on shutdown. you CAN call it
explicitly if you want a checkpoint, but the default is "close
moof and everything is safe."

---

## wave 10: image as the only artifact

the present: `moof` runs, replays every bootstrap `.moof` file into
a fresh vat, loads the user's image on top. the replay exists
because the bootstrap files define protocols, closures, builtins.

the future (wave 10): `moof build` produces a **seed image** — a
content-addressed blob containing the fully-constructed bootstrap
state. `moof run` hydrates the seed image. no source replay. no
parse time at startup. the image is the program.

this inversion solves several current problems:

- **recursion traps** (a defserver at bootstrap spawns a child vat
  that re-runs bootstrap) — gone. the child vat forks from the
  seed, doesn't replay it.
- **bootstrap ordering** comments in moof.toml — gone. the ordering
  happens at build time and freezes.
- **plugin ABI drift** (stale dylibs segfaulting) — detectable at
  build time, not at runtime.
- **parse cost on startup** — gone.
- **hot reload** — replaces `a.moof` → build a new seed → delta the
  image. tractable.

see [../roadmap.md](../roadmap.md) for the wave 10 plan.

---

## what you need to know

- the image is the single artifact. everything persists by default.
- content-addressing gives immutable values stable, global IDs.
- LMDB-backed blob store holds everything; three roots index it.
- GC walks both VM roots and store roots — persisted values don't
  vanish.
- schemas version; migrators handle upgrades; no silent corruption.
- running state (Acts, mailboxes, VM frames) isn't persistent yet
  (wave 10+).
- wave 10 makes the seed image a build artifact, replacing
  bootstrap source replay.

---

## next

- [../throughlines.md](../throughlines.md) — canonical form +
  time + additive, the patterns persistence embodies
- [addressing.md](addressing.md) — URLs, paths, the walks
  layer above persistence
- [objects.md](objects.md) — the material that persists
- [../roadmap.md](../roadmap.md) — when wave 10 and running-
  state persistence land
