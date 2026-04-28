# merkle persistence + immutable heap — design + status

snapshot of where the persistence + immutability work landed and where
to pick up next session.

## the four-stage plan (approved)

```
stage A — cached merkle hashes        [DONE]
stage B — immutable heap              [B-1 done, B-2.1 done, rest queued]
stage C — mmap-backed content store
stage D — gc compaction + multi-root coordination
```

stage A makes saves cheap. stage B makes the heap itself immutable
where it can be (so vats can share content). stage C moves the
content into a memory-mapped file (so multiple processes share too).
stage D handles the long-tail garbage collection of stale content.

## the architectural target

```
┌─────────────────────────────────────────────────────────┐
│  IMMUTABLE CONTENT POOL  (shared across all vats)       │
│  • content-addressed by blake3                          │
│  • lives in mmap (stage C)                              │
│  • lists, tables, strings, bytes, closures, numbers     │
└─────────────────────────────────────────────────────────┘
        ▲              ▲                ▲
┌───────────────┐  ┌───────────────┐  ┌───────────────┐
│   vat 0       │  │   vat 1       │  │   vat 2       │
│ heads (mut):  │  │ heads (mut):  │  │ heads (mut):  │
│   root_env →  │  │   root_env →  │  │   root_env →  │
│   frame envs  │  │   frame envs  │  │   frame envs  │
│   inbox/out   │  │   inbox/out   │  │   inbox/out   │
└───────────────┘  └───────────────┘  └───────────────┘
```

**heads** are tiny per-vat mutable identity-typed objects. they carry
no canonical hash; they ARE their identity. mutations land here.

**content** is everything else — immutable, hash-keyed, structurally
shared. once written to the content pool, never mutated. references
to content are by hash.

mutation pattern — what `(def x 5)` does today (after stage B-2.1):
1. read the current bindings table (immutable content, hash #h1).
2. compute a NEW immutable bindings table with x=5 added (hash #h2).
3. mutate the env head's `bindings` slot to point at #h2.

old #h1 is still in the store. closures captured against the env see
the new bindings via the head's now-current `bindings` slot. forward
refs preserved.

## stage A — cached merkle hashes  [LANDED]

every `HeapObject` carries:
- `cached_hash: Cell<Option<[u8; 32]>>` — memoized canonical content hash.
- `child_fingerprint: Cell<Option<[u8; 32]>>` — blake3 over children's hashes
  at the time `cached_hash` was computed. used to detect "did any descendant
  change?" without explicit upward propagation.

mutation invalidates both via `Arena::get_mut`. save's `compute_hash_table`
checks them before recomputing — clean subtrees get reused.

image-load populates `cached_hash` from the blob's stored hash and
`child_fingerprint` from already-decoded children. so the very first
save after a fresh load is fast, not just the second.

## stage B-1 — auto-commit + reachability merge  [LANDED]

**continuous persistence on by default.** every top-level form's eval
is followed by `sys.save_image(vat_id)`. set `MOOF_NO_AUTO_COMMIT=1`
to opt out (e.g. for benchmarks).

**`heap.known_stored: RefCell<HashSet<[u8; 32]>>`** — set of blob
hashes confirmed in lmdb (from this process's earlier saves OR from
image-load). save loop skips lmdb-get + canonical_blob_bytes for any
hash in the set.

**`Heap::reachable_objects_into`** — one merged BFS over many root
values at once. save_snapshot used to call `reachable_objects` once
per root × per closure-desc × per constant — 800+ separate walks.
now one walk seeded with all roots: **400× speedup**.

per-form save cost on conway_live (warm): ~65ms total.

## stage B-2.1 — head/content split + COW env_def  [LANDED]

`HeapObject.is_head: bool` distinguishes mutable identity-typed heads
from immutable content. constructors default to content; promote via
`alloc_head` / `promote_to_head` / `make_head_object`.

`Arena::get_mut` invalidates the merkle cache AND warns on content
mutation when `MOOF_DETECT_HEAD_VIOLATIONS=1` is set. `get_mut_raw`
skips both — for the hashing pass and gc tombstoning.

**heads currently promoted:**
- root_env at heap construction.
- every value in `type_protos` after plugin registration completes,
  via `Heap::promote_known_heads` (called from `Scheduler::spawn_vat`
  between plugin registration and bootstrap).

**`env_def` is now COW.** new helper `Heap::cow_bindings_replace`:
1. read current bindings table via `foreign_ref::<Table>` (read-only).
2. clone its seq + map.
3. apply the caller's mutation to the clone.
4. allocate new Table — fresh content.
5. mutate env head's `bindings` slot to the new Table id.

`bind_in_env` and `env_remove` go through the same path.
inline-backed envs (no bindings slot) fall through to a head-slot
mutation, which is legitimate since the env IS a head.

## audit status

`MOOF_DETECT_HEAD_VIOLATIONS=1 MOOF_DETECT_HEAD_VIOLATIONS_ALL=1
./target/release/moof examples/conway_live.moof 2>/tmp/v.txt`
emits ~138k violations. interpretation:

- the bulk are during plugin registration: each plugin allocates a
  proto via `heap.make_object(...)` then loops `handler_set` on it.
  `promote_known_heads` runs AFTER plugin registration, so those
  intermediate handler_set's hit content. 25 call sites need
  conversion to `make_head_object` — see task #23.
- post-bootstrap violations (mutations during actual user code) are
  the real signal. those need conversion to head-promote or COW.

## what's queued for next session

| task | summary |
|---|---|
| #23 | convert plugin proto allocation to `make_head_object` (mechanical, kills bootstrap-time violations) |
| #24 | HAMT for bindings tables — replace full-clone with structural sharing |
| #25 | closure `:__scope` rewrites to (role_marker, frozen_chain) — closures become pure content |
| #26 | cross-vat sends use hash refs — the shared-content sharing win |
| #27 | unified content store across all vats — system-level, foundation for stage C |

queued but separate:
| task | summary |
|---|---|
| #7 | reactive-demo "0 does not understand call:" (pre-existing) |
| #11 | Phase 4 wrap stacking (pre-existing) |
| #18 | closure env semantics under immutability — design discussion (resolved by B-2.3 plan) |

## perf snapshot (conway_live, 35 generations)

| run | hash | reach | lmdb | total |
|---|---|---|---|---|
| stage A only, warm save | 30ms | 800ms | 50ms | ~880ms (1 save) |
| + reach merge + known_stored, warm | 50ms | 2ms | 12ms | ~65ms (per save) |

| variant | wall time |
|---|---|
| no auto-commit, warm | 11.6s |
| auto-commit, warm | 12.1s |
| auto-commit + COW env_def, warm | 14.3s |

(5.25s of every run is `(sleep 150)` × 35 frames.)

## key files touched

- `crates/moof-core/src/object.rs` — HeapObject struct: cache fields + is_head.
- `crates/moof-core/src/arena.rs` — get_mut invalidation, alloc_head, promote_to_head, violation detection.
- `crates/moof-core/src/heap/mod.rs` — known_stored, cow_bindings_replace, env_def COW, promote_known_heads, make_head_object.
- `crates/moof-core/src/canonical.rs` — reachable_objects_into.
- `crates/moof-runtime/src/blobstore.rs` — cache-aware compute_hash_table, known_stored skip, image-load cache populate, timing.
- `crates/moof-cli/src/shell/script.rs` — default-on auto-commit.
- `crates/moof-cli/src/system.rs` — save_image timing.
- `crates/moof-runtime/src/scheduler.rs` — promote_known_heads call after plugin registration.
