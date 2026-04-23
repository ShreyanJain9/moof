# persistence

> content-addressed blobs + named refs + a root set. the image as
> a mutable pointer into an immutable dag of values. closes the
> round-trip gap from `docs/round-trip-gap.md` with the right
> shape instead of a memory dump.

---

## the model

three spaces, one lmdb env:

- **blobs** — key: 32-byte blake3 hash; value: canonical bytes.
  holds every immutable moof value that's ever been persisted.
  same content → same hash → same key, always. this is where
  cons cells, strings, objects, closures, closure-descs, foreign
  payloads all live.
- **refs** — key: symbolic name (`system.caps`, `vats.7.mailbox`,
  `snapshots.2026-04-22.root`); value: 32-byte hash. these are
  the mutable heads. writing a new value is "put a blob, update
  a ref." atomic inside one lmdb txn.
- **roots** — a small fixed set of refs the loader starts from.
  `roots.heap` (the whole vat heap as a value), `roots.closure-descs`
  (the VM's desc vec), `roots.symbols` (the symbol table, for
  native-handler id stability). boot reads these, walks outward.

## canonical encoding

the hash is only useful if the same moof value always produces
the same bytes. canonical means deterministic.

### values

every `Value` encodes as one of these tagged forms:

```
0x01 int     — tag + i64 big-endian             (9 bytes)
0x02 float   — tag + f64 bits big-endian        (9 bytes)
0x03 nil     — tag                              (1 byte)
0x04 true    — tag                              (1 byte)
0x05 false   — tag                              (1 byte)
0x06 symbol  — tag + u32 name-length + utf8     (5 + len)
0x07 blob    — tag + 32-byte hash               (33 bytes)
```

primitives are always inlined. heap-allocated values are always
referenced by their blob hash. no exceptions.

why: with a fixed rule, no "sometimes inline, sometimes ref"
ambiguity, canonicalization is a one-pass recursive traversal,
no ordering dependencies.

### blobs

a blob's bytes start with a type-tag and contain nested value
encodings.

```
0x01 cons         — car-value + cdr-value
0x02 text         — u32 length + utf8 bytes
0x03 bytes        — u32 length + raw bytes
0x04 table        — u32 n-seq + n-seq value-encs
                  + u32 n-map + n-map (key-value, value-value) pairs
                  (map entries sorted by key-value's canonical bytes)
0x05 general-obj  — proto-value + u32 n-slots
                  + n-slots (symbol-name-utf8, value-enc)
                  (slots sorted by name utf8)
                  + u32 n-handlers
                  + n-handlers (selector-name-utf8, handler-value-enc)
                  (handlers sorted by selector name)
                  + u32 foreign-len + foreign-bytes-if-any
0x06 foreign-payload — foreign-type-name-utf8 + payload bytes
```

the key canonicality gotchas:

- **slots sorted by name, not by symbol-id.** symbol ids are
  session-local; sorting by id would give different bytes per boot.
  sort by the UTF-8 name bytes of the slot symbol.
- **handlers similarly sorted by selector name.**
- **table map entries sorted by key's canonical bytes.** keys can
  be any value; pairs-of-(key-enc, val-enc) get sorted by the
  key's encoding bytes.
- **symbols serialize by name content, not intern id.**
- **float encoding preserves bits.** even NaN bits. no
  normalization. deterministic.

### hash

`hash(v)` = blake3 of the canonical bytes of `v`.

- primitives (inline) have a well-defined hash too (hash of their
  9 or 1 or 5+len bytes).
- heap values have their blob's hash as their identity.
- identical cons lists, identical strings → identical hash always.

## saving

one function: `Store::save(heap, closure_descs) -> Result<(), Error>`.

1. enter a write txn.
2. walk every value reachable from the heap's root set. for each
   heap object v: canonical encode → hash → if not already in
   `blobs`, insert. recursive — inserts the blob for every
   sub-value that's heap-allocated.
3. put the env's hash into `refs.roots.heap`.
4. put the closure-descs-vec's hash into `refs.roots.closure-descs`.
5. put the symbol table's hash into `refs.roots.symbols`.
6. commit.

dedup is automatic: if a blob is already there, we don't write it.
a 10mb image sharing a 1mb sublist writes the sublist once.

## loading

reverse flow:

1. read `refs.roots.symbols` → fetch blob → deserialize the symbol
   table → populate `heap.symbols` and `heap.sym_reverse` in the
   stored order. symbol ids match what the image was saved with.
2. register type plugins. plugins call `intern(name)` for natives
   like `+#0`; since the symbol table is already populated, they
   reuse existing ids. native registrations land in the same slots.
3. read `refs.roots.heap` → fetch blob → deserialize the env value,
   recursively fetching referenced blobs as needed. objects get
   fresh local ids; hash-refs inside objects are remapped via a
   `hash → new_id` cache built during deserialization.
4. read `refs.roots.closure-descs` → fetch blob → deserialize into
   `VM.closure_descs`. closure heap objects reference descs by
   code_idx (a flat index); since descs are restored in order, ids
   match.
5. skip bootstrap eval — the image already has it all.

## plugin reconciliation

plugins are the tricky part. they create both prototypes
(allocates heap objects) and natives (registers rust fns).

without the image, plugins create prototypes and `type_protos`
points at them. with the image, `type_protos` is already
populated from the loaded heap.

decision: **plugin register runs unconditionally. the part that
installs natives is what matters; the prototype heap-object
creation just produces orphans that get collected on next gc.**

concretely: every `plugin.register(heap)` call does:
1. `register_foreign_proto::<T>` — creates a prototype heap
   object and stashes it in `type_protos[PROTO_X]`. if the image
   already loaded a prototype at that index, we overwrite —
   that's the bug. need to detect and skip.
2. `native(heap, proto_id, selector, f)` — this is the part we
   always need, to populate `heap.natives` so handler symbols
   resolve to rust fns.

solution: split `register_foreign_proto` into "create proto (or
reuse)" and "install natives." the proto_id returned by the
heap is either fresh or pre-existing.

the `type_protos[PROTO_X]` stays sticky: if loaded, don't
overwrite. the natives get registered against whichever
proto_id is current.

symbols interning is deterministic — if plugins run in the same
order, they produce the same `foo#N` symbols at the same ids.
pre-loading symbols (step 1 of load) means those ids exist before
plugins run and they're reused.

## what's still out of scope for this wave

- **closure source records** already serialize into blobs as
  part of the desc blob.
- **mutable-entity refs.** defservers and capabilities are
  currently FarRefs (session-local `(vat_id, obj_id)`). for this
  wave, cross-vat state (the cap vats) still re-spawns in
  deterministic order so FarRefs line up by coincidence. proper
  stable cross-vat refs come in the url/resolver wave
  (docs/addressing.md).
- **time-travel snapshots.** the model supports them (each
  save = a root, keep old roots), but the ui (pick a snapshot,
  open read-only) is a later wave.
- **concurrent writers.** single-writer only. not a problem for
  a personal image. would need a lock file if we ever want multi.
- **gc over blobs.** blobs accumulate; a saved value that nobody
  references is dead weight. gc = "reachable from any current
  root, keep; else evict after retention window." defer —
  today's image is small enough to not care.

## sequencing (implementation order)

1. **canonical serialization.** pure library additions:
   `Heap::canonical_encode_value(v) -> Vec<u8>`,
   `Heap::canonical_encode_blob(obj_id) -> Vec<u8>`, with tests
   for every value shape. no store yet.
2. **hash function.** `Heap::hash_value(v) -> [u8; 32]` via
   blake3. native handler `[value hash]` returns the hex string.
3. **blob + ref store.** new module: `crates/moof-runtime/src/blobstore.rs`.
   lmdb with `blobs`, `refs` dbs. api: `put_blob`, `get_blob`,
   `put_ref`, `get_ref`.
4. **save: walk heap, stuff blobs, write root refs.** replaces
   `Store::save_all`.
5. **load: read roots, rebuild heap.** wire into `System::boot`.
6. **plugin reconciliation.** detect pre-loaded prototypes in
   `register_foreign_proto`, skip creation.
7. **round-trip integration test.** a moof script + a test
   harness that asserts state persists.

roughly 1 week of focused work. each step is commit-worthy.

---

*keeps foundations.md's vision (persistence pillar) honest by
actually implementing it. once this lands, the url/resolver work
from addressing.md slots in cleanly — urls become the "what
mutable refs get called" layer on top of content-hashed blobs.*
