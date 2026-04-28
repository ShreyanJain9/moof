# next session — pick up here

read [docs/concepts/persistence-merkle-immutability.md](docs/concepts/persistence-merkle-immutability.md)
first for the full architectural picture. this file is just the
"what's the next move" cheat sheet.

## where we are

three stages landed last session:
- **A** — cached merkle hashes (~5–8× faster saves).
- **B-1** — auto-commit by default; per-form save ~65ms.
- **B-2.1** — `HeapObject.is_head` flag, `env_def` is COW for bindings.

## what to do next, in order

### 1. task #23 — convert plugin protos to heads (mechanical, ~30 min)

audit shows ~138k content-mutation violations during bootstrap. they
all come from plugin code that does:

```rust
let proto = heap.make_object(parent_proto);  // allocates as content
heap.type_protos[PROTO_X] = proto;
heap.get_mut(proto_id).handler_set(...);     // ← violation: mutating content
heap.get_mut(proto_id).handler_set(...);
// ... many more handler_sets
```

`promote_known_heads` runs AFTER plugin registration finishes, so
those interim handler_sets hit content. fix: replace
`heap.make_object(...)` with `heap.make_head_object(...)` for every
proto allocation.

**call sites to convert** (~25 total):

```bash
grep -rn "heap.make_object" crates/moof-plugin-* crates/moof-cap-*
```

ones that are protos (followed by `handler_set`) → swap to
`make_head_object`. ones that allocate transient user-facing objects
(rare in plugin code) → leave as is.

**verify**: rerun
`MOOF_DETECT_HEAD_VIOLATIONS=1 ./target/release/moof examples/conway_live.moof`
and watch the violation count drop from ~138k to a much smaller
number. the residual ones are real signal — mutations during user
code that need conversion to head-promote or COW.

### 2. task #24 — HAMT for bindings tables

`env_def`'s COW path full-clones the bindings table on every `(def)`.
during bootstrap we do hundreds of defs → hundreds of full clones of
a ~50-entry table. fine perf-wise today but doesn't scale to large
namespaces, and stage C mmap NEEDS structural sharing (otherwise mmap
pages get rewritten constantly).

**direction**:
- introduce a HAMT crate dependency (rpds, im, or hand-rolled — pick
  one that's stable and has serde support so it slots into
  `canonical_blob_bytes`).
- specialize `Table` so when its content is the bindings of an env,
  it's HAMT-backed. user-facing tables can stay flat or also become
  HAMT (for consistency).
- `cow_bindings_replace`'s "clone seq + map" step becomes "structural
  insert" — O(log n) instead of O(n).

**watch out**: `canonical_blob_bytes_using_table` for `Table` needs to
serialize HAMT entries in a deterministic order (sorted by canonical
key bytes — same as today). don't break dedup.

### 3. task #25 — closure `:__scope` becomes `(role_marker, frozen_chain)`

today closures capture `:__scope = env_id` (a head id). that means
closure content depends on a head id — closure hash changes every
time the env head's bindings slot changes — and closures aren't
shareable across vats (head ids are vat-local).

**direction**: closures capture two things:
- a **role marker** — a stable string like `"vat-root"`. used at
  call time to dereference the vat's current head for that role.
  enters the closure's content hash.
- a **frozen lexical chain** — the let-bindings + frame locals at
  capture time, immutable content. enters the hash.

variable lookup at call time: walk frozen chain first, then
dereference role marker → vat's current head → its current bindings.

**why this matters**: closures become pure content. they share
across vats by hash. they're stable across env mutations (their
content hash doesn't change when `(def x 5)` happens — they just
see the new x at call time via the role marker).

**migration**: the compiler emits `:__scope` today. it'll need to
emit `(role_marker, frozen_chain)` instead. a transitional period
where both shapes are accepted may help.

### 4. task #26 — cross-vat sends use hash refs

THIS IS THE MOMENT MOOF BECOMES FEDERATED. `scheduler.copy_value_across`
deep-copies values today. with everything-is-content (after #25):

```
fn copy_value_across(val, from_vat, to_vat) -> Value:
  if val is content:
    # send the hash; receiver looks up in the shared content store.
    # zero-copy. structurally identical to sender's value.
    receiver.alloc_or_lookup_by_hash(content_hash(val))
  if val is head:
    # heads are vat-local. either reify (deep-copy) or refuse.
    # in practice: copy is rare and explicit.
    ...
```

before #25, closures-capture-head-ids prevents this. after #25,
closures are content and the path opens.

### 5. task #27 — unified content store across vats

today each vat has its own arena. the `is_head` split tells us most
allocations are content — and content is value-equal across vats.
move the content arena out of `Heap` and into `System` (or a new
`ContentStore` shared via `Arc`). per-vat `Heap` keeps just heads +
a reference to the shared content.

**API shape** to aim for:
```rust
fn alloc_content(&self, obj: HeapObject) -> ContentHash;  // dedups
fn get_content(&self, hash: ContentHash) -> &HeapObject;
```

allocations are content-addressed: same content → same hash → same
slot, deterministically. this is the foundation for stage C mmap —
the content store IS the mmap region.

## the payoff

after #23–#27, two vats with identical bootstrap state share ~6MB
of content in memory instead of having two ~6MB copies. cross-vat
sends become hash refs (no copying). save is essentially free for
unchanged content (already in the shared store). stage C (mmap)
just makes the shared store live in a memory-mapped file so multiple
PROCESSES share too.

## environment knobs available now

- `MOOF_NO_AUTO_COMMIT=1` — disable per-form save.
- `MOOF_TIME_SAVE=1` — print save timing breakdown.
- `MOOF_DETECT_HEAD_VIOLATIONS=1` — log content-mutation violations
  (capped at 50). add `MOOF_DETECT_HEAD_VIOLATIONS_ALL=1` for all.

## quick sanity checks before starting

```bash
# build clean (note: example plugins are outside the workspace,
# rebuild them too if their dylibs are stale)
cargo build --release
(cd examples/type-plugin && cargo build --release)
(cd examples/rust-plugin  && cargo build --release)

# conway works
rm -rf .moof/store && ./target/release/moof examples/conway_live.moof

# tests pass
cargo test --workspace 2>&1 | grep "FAILED\|test result"
```

if anything is broken, plugin ABI drift is the most likely
culprit — example plugins out-of-sync with workspace moof-core.
just rebuild them.

---

stage A laid the merkle foundation. stage B is making the heap
structurally honest about what's mutable. stages C and D get us
mmap + multi-process. by the end of this thread, moof has a real
content-addressed federated heap. let's go (˶ᵔ ᵕ ᵔ˶)
