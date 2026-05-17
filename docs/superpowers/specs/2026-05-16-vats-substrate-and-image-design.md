# vats, substrate carve, and the image — design

> **status:** brainstormed 2026-05-16. ready for `superpowers:writing-plans`.
> **load-bearing for the next several months.** integrates the vats
> spec (`2026-05-04-vats-and-references-protocol-design.md` V4-V11),
> the phase 3 cohesive vision (`2026-05-16-phase3-cohesive-vision-
> design.md`), the self-host arc (`2026-05-10-self-host-and-rust-
> deletion-design.md`), and the transporter/mco/datasource stack
> (`2026-05-02`, `2026-05-03`) into one cohesive substrate + persistence
> picture. NO CODE CHANGES — spec only.
>
> **prior reading (in order of relevance):**
> - `2026-05-04-vats-and-references-protocol-design.md` — vat semantics, V1-V11
> - `2026-05-16-phase3-cohesive-vision-design.md` — image-as-canon framing
> - `2026-05-10-self-host-and-rust-deletion-design.md` — W1-W5 self-host
> - `2026-05-03-track-1-mcos-and-datasource-design.md` — mco ABI
> - `docs/concepts/forms.md` — the four faces
> - `docs/laws/substrate-laws.md` — L1-L16
> - `docs/laws/determinism-laws.md` — D1-D12
> - `NEXT_SESSION.md` — current state at HEAD `041f8fd`

---

## table of contents

§0 — the picture in one shot
§1 — directory rename + substrate scope
§2 — vat = struct + proto-Form skin
§3 — the turn as universal commit
§4 — freezing + mutation contract
§5 — shared segment + content-addressed promotion
§6 — the identity triple
§7 — scheduler model + federation transport-neutrality
§8 — boot lifecycle + the merkle object store
§9 — autosave + explicit checkpoints + multi-world
§10 — compaction: reflog, gc, pack, journal prune
§11 — mandatory mco serialize/restore
§12 — transporter round-trip + intrinsic shrink + vat ergonomics
§13 — performance + compactness as design goals
§14 — implementation phasing
§15 — risks + open questions
§16 — what's NOT in this spec
§17 — see also

---

## §0 — the picture in one shot

a moof world is a federation of vats. each vat advances in **turns** —
the atomic unit of mutation (nursery → diff → journal → commit). each
vat carries three identities: **content-hash** (value, for dedup +
sharing + federation discovery), **vat-id** (history, UUIDv7, same
vat across edits), **path** (name, human address through `$here`).

within a process, vats share frozen forms via the **shared segment**
(content-addressed, lock-free read) and pass mutable values via
**mailbox messages** (MPMC, async). across processes, the same
primitives work over a transport — far-refs become network-resolved
instead of in-memory; nothing semantically changes.

**rewindability falls out naturally**: every turn produces a
journaled diff; replaying journal from snapshot reconstructs any past
state; receipts in the effect-cap journal prevent side-effects re-
firing during replay. **the image is the snapshot at a turn
boundary**; saving = pause all schedulers at their next turn
boundary, walk the merkle store, atomically advance refs, resume.

**autosaves are the canonical model.** the per-turn journal fsyncs;
the merkle store flushes in batches; opening moof tomorrow lands you
exactly where you left off. explicit `[$image save: ...]`,
`[$image fork: as: ...]`, `[$image load: ...]` exist for naming
history and multi-world experiments. **the image is the artifact,
turns are the time unit, the journal is the bridge, the merkle store
is the spine.**

---

## §1 — directory rename + substrate scope

### 1.1 rename

`crates/` is cargo legacy. only `mco-pack` remains a true cargo crate
post-W5. proposed layout:

```
players/
  zig/         — the canonical reference player (was crates/zig-substrate)
  wasm/        — future, post v2.0; browser deployment
seed/          — ocaml build tool, minimal seed.vat compiler
                 (was crates/ocaml-seed)
tools/
  mco-pack/    — rust; packs wasm + manifest into .mco
  abi/         — language-neutral ABI specs + per-language glue libs
                 (was crates/abi, crates/abi-rust merged)
lib/           — moof source tree (parser, compiler, stdlib, mcos)
docs/
tests/
  conformance/ — image+message+result triples, run by every player
```

rename is mechanical; ships during the substrate carve work (V4),
not as a separate session. cargo workspace becomes `tools/` only;
`players/zig/` is a standalone `zig build` project; `seed/ocaml/`
is a standalone `dune build` project.

### 1.2 substrate scope

target: **~7K LoC zig** (from 10.7K today). everything derivable
moves out. the substrate is:

- **VM** — bytecode interpreter, opcode handlers, dispatch
- **heap + nursery** — per-vat 00… scope, gc, freeze guard
- **shared segment** — 01… scope, intern table, lock-free read
- **scheduler** — pinned threads, turn loop, mpmc mailbox plumbing
- **image i/o** — merkle store reader/writer, journal append, ref atomicity
- **mco runtime** — wasm host, ABI, serialize/restore plumbing
- **the minimum intrinsic ABI** to make moof code run (~50-80 natives)

everything else — parser, compiler, stdlib derivations, vat
ergonomics macros, supervisor patterns, replication policy —
lives above the substrate in moof code. **the substrate is a
VM + image i/o; the moof image is everything else.**

per-player conformance test corpus at `tests/conformance/` lives
independent of players. all players run the same triples; drift is
a shipping-blocker bug.

---

## §2 — vat = struct + proto-Form skin

### 2.1 internal representation (zig)

```zig
const Vat = struct {
    id: VatId,                          // UUIDv7
    mode: VatMode,                      // .frozen_default | .mutable_default
    heap: Heap,                         // 00… scope, vat-local Forms
    nursery: Nursery,                   // turn-local mutations buffer
    mailbox: MpmcQueue(Envelope),       // inbox; cross-scheduler senders
    outbox: Outbox,                     // queued cross-vat sends + intents
    here: FormId,                       // $here — root env + path-table seg
    behavior: FormId,                   // method-form for receive loop
    supervisor: ?FarRef,                // nil for root supervisor only
    caps: FormId,                       // cap-bag (slot-bound caps)
    journal: ?JournalHandle,            // persistence; lazy on first mutation
    far_ref_table: ArrayList(FarRefEntry),  // 10… scope local entries
    forwarding: HashMap(FormId, FormId),    // become: + compaction redirects
    scheduler_id: u8,                   // which thread owns this vat
    turn_counter: u64,                  // monotonic per-vat
};

const VatMode = enum { frozen_default, mutable_default };
```

### 2.2 moof reflection face

every vat is also a `Vat`-proto Form. its slots are **read-only
mirrors** of the struct fields:

| slot | type | meaning |
|---|---|---|
| `:id` | String | UUIDv7 |
| `:mode` | Symbol | `'frozen-by-default` or `'mutable-by-default` |
| `:here` | Form | `$here` segment |
| `:supervisor` | FarRef? | parent in supervision tree |
| `:caps` | Form | cap-bag |
| `:mailbox` | DataSource | inbox, exposed as DS for tee/filter/observe |
| `:journal` | DataSource? | per-vat WAL, observable |
| `:turn-counter` | Integer | monotonic turn count |
| `:scheduler` | Integer | scheduler-id (informational) |

**writes go through supervisor sends only.** moof code in vat A
cannot reach across to flip vat B's mode or rewrite B's mailbox.
the only legitimate operations on another vat are: send a message,
ask the supervisor to restart it, ask the supervisor to spawn its
sibling.

vat-Forms can never be frozen (their liveness face is mutable-by-
definition; `[vat-form freeze]` raises `'cannot-freeze-live`).

### 2.3 vat lifecycle

monotonic: **spawn → running → (optionally crashed-and-restarted)*
→ shutdown.** see §7 for spawn mechanics; §12.3 + §14 phase 8
for supervisor logic.

---

## §3 — the turn as universal commit

### 3.1 what a turn is

every change to vat state lives at the granularity of one turn. a
turn processes **one envelope** from the mailbox. the substrate's
discipline:

```
turn(vat):
  1. dequeue one envelope from mailbox (or yield if empty)
  2. invoke vat.behavior with the envelope
  3. all allocations → nursery
  4. all mutations → nursery shadow entries
  5. cross-vat sends → outbox queue (not yet flushed)
  6. effect-intents (cap calls) → outbox queue
  → on raise propagating to top of behavior:
       drop nursery, no commit, notify supervisor
  → on normal return:
       7. compute per-slot diff
            (form-id, key-kind, key, prior-value, new-value)
            (this also identifies which form-ids' canonical bytes changed)
       8. append (envelope, diff) to inputs.log + fsync
       9. apply nursery into vat-local heap (new form-ids permanent)
      10. for each changed form: recompute its canonical-bytes hash;
          recompute vat-state hash from form hashes;
          advance in-memory world-root hash
      11. emit outbox:
            cross-vat sends → receiver mailboxes via MPMC enqueue
            effect-intents → cap authority queue
      12. drop nursery; opportunistic per-vat gc if pressure warrants
      13. accumulate (hash → canonical-bytes) pairs for changed forms
          into pending-flush buffer
      14. (every N turns or every M ms or on idle) flush pending
          objects to .moof/store/objects/; atomic ref update on
          refs/world/current
  15. yield to scheduler
```

### 3.2 why this matters

**this is the universal sync point.** every cross-cutting concern
hooks here:

- image save happens between turns
- replication ships diffs between turns
- gc fires between turns
- compaction fires between turns
- the journal advances between turns
- rewindability operates on turn N → N+1 transitions
- mco hot-reload swaps at turn boundary
- compactor coalesces packfiles between turns

### 3.3 within-turn semantics

- **read-your-writes**: reads consult nursery first, fall through
  to heap. you see your own mutations inside a turn.
- **no observable mutation until commit**: another vat dequeuing a
  message from this vat cannot see the in-progress nursery.
- **all-or-nothing**: crash mid-turn rolls back to the previous
  turn's heap. journal isn't appended; outbox isn't flushed.

### 3.4 the per-slot diff is everything

the diff record drives:

- **journal entries** (inputs.log) — replay primitive
- **replication** — leader ships diffs to followers
- **CRDT merge** — slots annotated `meta.mergeable: <Proto>` route
  the diff through `:merge:` instead of direct apply
- **hash-cache invalidation** — every form-id in the diff has its
  cached canonical-bytes-hash invalidated; on next save, that
  form's hash is recomputed
- **autosave object accumulation** — the substrate knows exactly
  which forms changed, so it knows which canonical-bytes blobs
  need writing on the next merkle flush

a single per-slot-diff data structure does all five jobs.

---

## §4 — freezing + mutation contract

### 4.1 vat-mode is a spawn-time decision

immutable for the vat's life. two modes:

- **`'frozen-by-default`** — new forms auto-freeze at the end of
  their allocation expression. internal building during the alloc
  is mutable (you `set-slot!`, `set-handler!`, attach meta), then
  the form locks. methods that need mutation use scoped
  `(let-mutable form-binding body)` which thaws inside the form
  for the dynamic extent of `body` and freezes back at exit. the
  let-mutable form is itself macro sugar; substrate primitive is
  a per-call-frame "mutable-allow" stack.

  use case: parsers, compilers, computation kernels, pure
  data structures. all consumers downstream get cheap shared-segment
  promotion (§5).

- **`'mutable-by-default`** — new forms born mutable. `[form
  freeze]` is explicit when desired. workspaces, ui, stateful
  actors, anything that evolves over many turns.

### 4.2 substrate primitive: shallow freeze

```
[form freeze]:
  if form is vat-Form, mailbox, cap-token, foreign-handle:
    raise 'cannot-freeze-live
  set form.frozen = true
  invalidate any cached canonical-bytes hash (will recompute on next read)
```

flips a bit on Form. slot/handler/meta tables become read-only.
mutation guard raises `'frozen-form` at the call site, not at turn-
end:

```
[frozen-form slot: 'foo := 42]    ; raises 'frozen-form immediately
```

### 4.3 no thaw

monotonic lifecycle: **mutable → frozen → (optionally promoted to
shared segment) → collected.** thaw is intentionally absent — it
breaks content-addressing semantics (the form's canonical-bytes-
hash would change after promotion, but other vats may have already
cached the hash as their reference).

`let-mutable` (§4.1) is the one exception, and is scoped: it only
operates on forms allocated within the dynamic extent of the
let-mutable block; it cannot thaw a form that has already escaped
to another reference. semantically: the form was never truly frozen
until let-mutable exits.

### 4.4 deep-freeze is moof code

policy decisions — cycles, live boundaries, frozen-but-shared —
live in `stdlib/freezing.moof`:

```moof
(defmethod Form (freezeRecursive)
  ;; user-policy: walk reachable, freeze each, handle cycles,
  ;; respect 'cannot-freeze-live boundaries.
  ...)
```

substrate handles single-form freezing only.

### 4.5 cross-vat rules

| sender's form is | becomes on cross-vat send |
|---|---|
| frozen, in vat-local heap | promoted to shared segment (01… id); both vats reference same arena |
| frozen, already in shared segment | unchanged; receiver references same 01… id |
| mutable, in vat-local heap | minted as far-ref (10… id); receiver gets async-only access |
| already a far-ref | unchanged |
| path-ref | unchanged; resolved at receiver if used |
| vat-Form, mailbox, cap-token | raise `'cannot-cross-membrane` |

### 4.6 freezing locks state, not behavior

dispatch on a frozen form still walks the (live, mutable) proto
chain. adding a handler to `Point` proto still affects every frozen
`Point` instance. **moldability survives freezing.**

L11 (FormId stability) holds across freezing.
L9 (cap unforgeability) interacts: a cap-token form is born
frozen-and-unfreezable (cannot be thawed; cannot mutate authority).
L7 (vat boundary): freeze cannot cross a membrane to freeze a form
in another vat; you must send a message to the owning vat.

---

## §5 — shared segment + content-addressed promotion

### 5.1 what it is

one process-wide arena of frozen forms, indexed by `01…`-scope
FormId. each scheduler dereferences shared-segment ids by direct
arena read — **lock-free, no membrane translation.** this is the
mechanism that makes cross-vat sharing of immutable values cheap.

### 5.2 the intern table

**the one concurrent surface in the substrate.** keyed by
`blake3(canonical-bytes(form))`; valued by `(SharedFormId,
refcount)`.

- **lookup**: atomic acquire-load on each probed slot; lock-free by
  construction (open addressing; slots are monotonic — once installed,
  never overwritten until rebuild). blake3's collision resistance
  separately ensures we never retrieve a wrong form for a given hash.
- **install**: CAS — compute hash, locate slot, see if occupied,
  install with atomic exchange, retry on collision. all schedulers
  can install concurrently into different slots; concurrent install
  at the same hash means one wins, others observe and reuse.

implementation: a fixed-size hash table with open addressing;
slot count grows by rebuild when occupancy crosses threshold (rare).
the rebuild itself is a synchronization point — substrate signals
"intern table grow"; all schedulers reach turn boundary; rebuild
runs; resume. acceptable because rebuilds are infrequent (~one per
few thousand promotions).

### 5.3 the canonical-bytes encoding

reuse the V4 image format's per-form encoding for both content-
addressing AND storage. **one encoder, one decoder, one canonical
bytes representation.** changes to the encoding require a major
image-format version bump per `2026-05-10-vm-V4-opcodes-design.md`
§10.

per-form encoding includes:
- proto FormId (canonicalized as content-hash if shared-segment)
- slot table (sorted by sym-id for D5 insertion-order determinism)
- handler table (sorted by sym-id)
- meta table (sorted by sym-id)
- frozen bit

it does NOT include:
- form-id (varies per vat)
- gc bookkeeping
- nursery shadow state

### 5.4 promotion algorithm

lazy — only on first cross-vat send:

```
serializing envelope for cross-vat delivery:
  for each Form-valued reference in args (recursive walk):
    case vat-local frozen:
      hash = blake3(canonical-bytes(form))
      lookup hash in intern table:
        hit (SharedFormId existing, refcount):
          atomic increment refcount
          rewrite reference: form-id → existing SharedFormId
        miss:
          alloc in shared arena at fresh SharedFormId
          install (hash → (SharedFormId, 1)) into intern via CAS
            on CAS failure (lost race): retry lookup; one promotor wins
          rewrite reference: form-id → new SharedFormId
      install forwarding pointer in source vat:
        source_vat.forwarding[old_id] = new SharedFormId
        future reads through old_id transparently resolve to shared
    case vat-local mutable:
      mint far-ref entry in sender's far_ref_table:
        new FarRefEntry { vat_id: sender_id, form_id: original, cap }
      rewrite reference: form-id → far-ref id (10… scope)
    case already-shared (01…) or far-ref (10…) or path-ref:
      no change
```

forms that never cross stay vat-local and are gc'd by per-vat gc.
**you don't pay for sharing you don't do.**

### 5.5 shared-segment gc

process-scope refcount. each promotion increments; each forwarding-
drop and each holder-vat gc-pass decrements. zero → reclaim.

cycles within the segment are **impossible by construction**:
frozen forms can only reference other frozen forms or immediates.
"outside the segment" references must be far-refs which are vat-
local handle-table entries, not direct heap edges.

shared-segment gc runs at process idle moments or at scheduled
intervals; doesn't block schedulers (refcount work is atomic).

### 5.6 invariants

- **L11 across promotion**: holders of the old vat-local FormId
  resolve through forwarding indefinitely. external observers (other
  vats, far-refs, serialized images) see a stable id either way.
- **L7 across the segment**: a vat reading a 01… id is not reading
  the other vat's heap — it's reading the shared arena, which has
  no owner. no membrane violation.
- **D9 across promotion**: same canonical bytes → same hash → same
  SharedFormId across all hosts. federation gets dedup for free.

---

## §6 — the identity triple

### 6.1 three identities, three roles

every form and every vat carries:

| identity | hash/value | scope | role | persistence |
|---|---|---|---|---|
| **content-hash** | blake3 of canonical bytes | process-wide (frozen forms + images) | value identity; dedup; federation discovery; cache keys; `.vat` filenames | stable across reboots |
| **vat-id** | UUIDv7 at spawn | process-wide (vats) | historical identity; same vat across edits; supervisor refs; far-ref targeting | stable across reboots |
| **path** | string in `$here` segment | world-wide | human-readable address; user binding; federation routing | as durable as the segment binding it |

### 6.2 composition

- **`[$path-table resolve: "/users/alice/inbox"]`** returns an id-ref
  (local) or far-ref (remote). resolution proceeds:
  1. path → vat-id (federation routing layer; which vat owns this
     segment of the path-table)
  2. vat-id → form-id (within the target vat; an internal binding)
  3. form-id → form (via the target vat's heap)
  three layers of indirection, each swappable independently.

- **a `.vat` file's canonical name is its content-hash.** share-by-
  link = content-hash. share-by-name = path. share-by-history-thread
  = vat-id.

- **replication convergence proof**: same content-hash for canonical
  state at turn N = same state. D9 makes this checkable on every
  commit. divergence between replicas surfaces as hash mismatch.

### 6.3 content-addressing consequences

a `.vat` is **named by what it contains**. this gives:

- rename = rebind path
- fork = same content-hash at fork point, new vat-id, divergent
  futures
- version pin = path → content-hash (not path → vat-id)
- dedup across the network = trivial — two hosts can prove they
  have the "same" data without comparing bytes
- merkle-store sharing — any subgraph identified by its root hash
  can be transported as a unit

### 6.4 path-table-vat as a federation primitive

the path-table-vat lives at `/system/paths`. it federates per-vat
segments of the path-table into a global namespace. each vat's
`$here` is its own segment; the path-table-vat knows how segments
compose into paths.

cross-vat path resolution is an explicit async send (`[#Path
"/users/alice/foo" resolve]` returns a promise). within-vat path
resolution falls through the env chain to `$here` synchronously.

across processes, the path-table-vat itself is replicated or
sharded (depends on federation topology; see §7).

---

## §7 — scheduler model + federation transport-neutrality

### 7.1 within a process: pinned schedulers

- **N schedulers = N cores** by default (configurable via env
  `MOOF_SCHEDULERS=N`). each scheduler is a zig thread.
- each scheduler **owns a fixed pool of vats**. a vat is born on a
  scheduler and lives there for its lifetime. **no work-stealing in
  v1.**
- **pinned**: vat → scheduler mapping is recorded at spawn. user
  can hint placement: `[$vat spawn: ... pin: cpu-2]`. default:
  round-robin assignment to least-loaded scheduler.
- per-scheduler turn-loop: round-robin its pinned vats, one turn
  each, drop idle vats from the rotation (re-add on mailbox arrival).

### 7.2 cross-scheduler communication

- mailbox is **MPMC** (multi-producer multi-consumer-able-by-design,
  but we use it as multi-producer single-consumer: many sender
  schedulers, one receiver scheduler which is the target vat's
  owner).
- cross-scheduler send: sender computes envelope; runs membrane
  translation (§5.4); enqueues on target vat's mailbox via the MPMC
  primitive. lock-free enqueue.
- target scheduler picks up at next turn boundary in its rotation.
- **shared segment reads are lock-free** across all schedulers.
- **far-ref tables are per-vat** (no cross-scheduler sync).

### 7.3 work-stealing is a v2 optimization

if load-imbalance measurements show hot vats overloading one core
while others idle, v2 adds work-stealing: a scheduler can claim a
runnable vat from another scheduler's queue. requires careful sync
on per-vat heap (only one scheduler at a time runs a vat, but
ownership transfer needs an atomic CAS on `vat.scheduler_id`).

we do NOT do this in v1. measurements first; complexity second.

### 7.4 across processes: same primitives

a far-ref `(vat-id, form-id, cap-token)` is **location-agnostic**.
the transport-layer decides delivery:

- **in-memory transport** (in-process far-refs): MPMC enqueue
  directly on receiver's mailbox.
- **network transport** (websocket, unix-socket, …): serialize
  envelope to canonical bytes, ship over the wire, deserialize
  on receiver, enqueue.

**the same primitives** — `[far-ref <- selector: args]`, `[promise
when-resolved: ...]`, `[$vat spawn: ... mode: 'replicated-leader]`
— **work identically over in-memory and network transports.**
federation is the same vat protocol over a different transport,
not a new language.

### 7.5 transport-layer concerns (not vat-layer)

- **ed25519 auth** — cap-token verification on cross-process
  envelopes
- **reflector** — central message-ordering authority for replicated
  vats (per `concepts/replication.md`)
- **tls** — wire encryption
- **dedup** — by content-hash on the wire; identical canonical
  bytes seen twice ship only the second time's "we already have
  this" ack
- **backpressure** — per-link queue depth; throttle sender when
  receiver queue fills

these are concerns of `concepts/transport.md` (a separate spec,
deferred per §16). the vat-layer's contract is: **give me an
envelope; deliver it.**

### 7.6 the path-table-vat under federation

across processes, the path-table-vat is either:

- **shared** — one replicated path-table-vat per federation, with
  each host as a follower (cheap reads, expensive writes); or
- **sharded** — each host owns its own segment; cross-host paths
  resolve by hopping host-to-host.

default: shared via replication for small federations; sharded for
large. configurable at federation-creation time.

---

## §8 — boot lifecycle + the merkle object store

### 8.1 the object store on disk

`.moof/store/` is laid out git-shaped:

```
.moof/store/
  objects/
    ab/cdef…          ← content-addressed form blob, blake3 keyed,
    01/2345…            canonical bytes
    fe/dcba…
  refs/
    world/current     ← single-line file: world-root hash
    world/turn-000487 ← historical refs (reflog)
    world/turn-000488
    vats/<vat-id>     ← per-vat latest state hash
    scratch/<name>    ← named forks (user-created via [$image fork:])
  journal/
    <vat-id>/inputs.log    ← append-only (envelope + diff per turn)
    <vat-id>/effects.log   ← intent + receipt per side-effect
  packs/
    pack-<sha>.pack    ← consolidated packfiles (post-compaction)
    pack-<sha>.idx
  config.toml          ← retention policy, scheduler count, etc.
```

### 8.2 hashing discipline

- **every form has a canonical-bytes hash** computable from its
  proto, slot table, handler table, meta table, frozen bit (§5.3).
- **hashes are recomputed at turn commit** for forms whose canonical
  bytes changed (the per-slot diff identifies these). recomputation
  is cheap (blake3 over typically <100 bytes) and keeps the in-
  memory world-root hash always-current. subsequent merkle flush
  to disk is pure i/o.
- **every vat has a state-hash** = blake3 of canonical encoding of:
  ```
  (id, mode, heap-root-hash, mailbox-hash, here-hash,
   behavior-hash, supervisor-hash, caps-hash, far-ref-table-hash)
  ```
  where heap-root-hash is itself a merkle root over the vat's
  forms.
- **the world has a root hash** = blake3 of canonical encoding of:
  ```
  (vat-list-with-state-hashes, shared-segment-root-hash,
   path-table-vat-state-hash, intern-table-snapshot-hash)
  ```

an idle turn re-hashes zero forms (nothing was dirty). a hot turn
re-hashes the few dozen forms touched. **save cost is O(modified
subgraph), not O(world).**

### 8.3 first boot (no .moof/store yet)

```
zig substrate ← seed.vat (~91 KB, from seed/ocaml builder)
  seed.vat opens; parser + compiler + transporter alive
  ↓
  [$transporter load: "lib/main.moof"]
  ↓
main.moof:
  1. spawn /system supervisor tree (frozen-by-default)
       /system/parser, /system/compiler, /system/mco-loader,
       /system/clock, /system/transporter-watcher
  2. [$transporter loadAll: "lib/early/*.moof" + "lib/stdlib/*.moof"]
     → all loaded into frozen heap, promoted to shared segment
  3. [$mco-loader loadAll: "lib/mcos/*"]
     → mco protos in shared segment
  4. spawn /users/<id>/workspace ('mutable-by-default)
     install morphic gui
  5. [$image checkpoint]
     → first flush; refs/world/current written
```

after first boot, `.moof/store/` exists and is the canonical truth.

### 8.4 subsequent boots

```
moof run:
  1. read refs/world/current → world hash
  2. lazily mmap .moof/store/objects/ + packs/
     forms materialize on first access
  3. reconstitute vats with pointers; schedulers come up
  4. each vat resumes at its saved turn-boundary state
  → morphic on screen, no full-heap-deserialization wait
  → autosave loop active from turn 1
```

### 8.5 the image as merkle-walkable

an "image" is everything reachable from a root hash. that includes:

- every vat × its state-at-snapshot-turn × the shared segment ×
  the path-table × the intern table snapshot
- mco wasm bytecode is **by-hash-reference** (loaded from
  `.moof/store/objects/` on demand, like any other content-addressed
  blob — keeps image small AND lets users update mco builds without
  re-saving the world)

### 8.6 portable artifacts vs the store

`.moof/store/` is the local working repo. when you want to ship a
world or subtree:

```moof
[$image pack: "alice-counter.vat" from: "/users/alice/counter-demo"]
  → walks merkle subgraph rooted at the path's target
  → packs into a single .vat file:
      header (magic + version)
      packed objects (concatenated canonical bytes + index)
      refs.json (the named refs for this subtree)
      signature (optional, ed25519)

[$image unpack: "alice-counter.vat" into: $store]
  → reads .vat header; verifies signature if present
  → unpacks objects into local .moof/store/objects/
  → binds refs from refs.json
```

**so system.vat, alice.vat, etc. still exist as portable single-
file artifacts — they're git pack-files for moof.** the local store
is the running truth; .vat files are the shareable form.

---

## §9 — autosave + explicit checkpoints + multi-world

### 9.1 autosave is the canonical model

**there is no save button.** opening moof tomorrow lands you exactly
where you left off — scratchpad intact, half-typed expressions
intact, inspector panes still open, message queues at their last-
committed state.

implementation: the turn loop (§3.1) appends to inputs.log + fsyncs
per-turn. that's the durability primitive. the merkle store flushes
in batches:

```
flush trigger (any of):
  - every N turns (default N=10)
  - every M milliseconds (default M=100ms)
  - on idle (no incoming envelopes for ~30s; opportunistic)
  - on clean shutdown (forced flush)
  - on explicit [$image checkpoint]

flush procedure:
  for each pending object in the in-memory buffer:
    write to .moof/store/objects/<ab>/<rest>
    (fsync the directory after batch; not every file)
  atomic ref update:
    write refs/world/current.tmp
    rename refs/world/current.tmp → refs/world/current
  optionally:
    write refs/world/turn-<N> (every K flushes; checkpoint cadence)
```

### 9.2 crash recovery

on crash, the journal saves us:

```
moof run (after crash):
  1. read refs/world/current → world hash at last flush
  2. mmap store; reconstitute vats at that state
  3. for each vat with journal entries past last-flush:
       replay envelope + diff (idempotently — the diff IS the truth)
       effects.log receipts prevent side-effect re-fire
  4. resume schedulers
  → state matches what was in-memory at crash time (modulo unfsynced
     turns; default fsync-per-turn → at most one turn lost; can
     reduce further by aggressive flush cadence)
```

### 9.3 explicit ops for multi-world

```moof
[$image checkpoint]
  ; force flush right now; advance refs/world/turn-<N>

[$image fork: as: "/scratch/before-refactor"]
  ; name the current state as a fork
  ; writes refs/scratch/before-refactor pointing at current world hash
  ; survives gc until released

[$image load: "/scratch/before-refactor"]
  ; switch the active world to this fork
  ; pause all schedulers, save current state under refs/scratch/auto-N,
  ; restore from fork, resume

[$image load: "system.vat"]
  ; unpack a portable artifact + activate
  ; (same flow as load: <ref> but reads from a pack-file first)

[$image pack: "alice-counter.vat"
        from: "/users/alice/counter-demo"]
  ; export a subtree as a portable .vat (git-style packfile)

[$image worlds]
  ; list named refs in refs/scratch/ and refs/world/

[$image release: "/scratch/before-refactor"]
  ; drop the named ref; underlying objects eligible for gc
```

### 9.4 multi-world topology

**multiple worlds coexist as named refs.** alice's prod world,
alice's experiment, the base seed, all in `refs/`. switching =
pause-current / cold-mmap-new / resume. cost: ~100ms for a small
world; ~seconds for a large one (limited by lazy materialization).

**`[$image load: "seed"]`** gives a fresh world from the base seed
shipped with the player binary, without nuking anything else. user
can experiment from a known-clean state.

### 9.5 user-facing flow

```
$ moof run
  ; opens last world from refs/world/current
  ; morphic up; you're back where you left off
  ; (autosave running silently)

> [$image fork: as: "/scratch/risky"]
  ; name a safe point

> ; ...do something experimental that goes badly...

> [$image load: "/scratch/risky"]
  ; back to safe state; experimental state preserved at refs/auto-N

> [$image release: "/scratch/risky"]
  ; done with the safe point; let gc reclaim

$ moof run --world seed
  ; explicit: open from the base seed instead of the saved world
```

---

## §10 — compaction: reflog, gc, pack, journal prune

### 10.1 reflog retention policy

without compaction, the store grows monotonically. retention bounds
growth:

```
default policy (configurable per world via .moof/store/config.toml):
  all checkpoints within last 24h: kept
  outside 24h-7d: every 10th checkpoint
  outside 7d-30d: every 100th checkpoint
  outside 30d: every 1000th checkpoint
  named forks: kept until released
  refs/world/current: kept (obviously)
```

retention runs alongside gc; only refs eligible for pruning are
unnamed historical refs outside the retention window.

### 10.2 garbage collection

```
[$image gc]:
  1. walk all refs in refs/ → collect set of reachable hashes
  2. walk subgraphs from each ref → mark all transitively-reachable
     objects
  3. sweep: for each object in .moof/store/objects/ and in packfiles:
       if hash is unmarked → remove (loose) or mark for repack (packfile)
  4. report: N objects swept, M bytes reclaimed
```

automatic trigger:
- on idle (no turns for ~30s)
- when store size exceeds threshold (e.g., 2× live-set heuristic)
- on explicit `[$image gc]`

gc is per-vat parallel where possible (each scheduler can sweep
objects in its vat's exclusive subgraph). cross-vat shared-segment
forms require a coordinated sweep.

### 10.3 packfiles

loose-object storage wastes filesystem inodes (one per blob).
git solves this with packfiles. moof does the same:

```
[$image pack]:
  1. enumerate loose objects in .moof/store/objects/<ab>/<rest>
  2. group into packfiles by reachability locality:
       forms reachable from the same vat-root cluster together
       forms in shared segment cluster together
       (this is heuristic; tune as we learn what reads correlate)
  3. for each group:
       write .moof/store/packs/pack-<sha>.pack:
         header (magic + version + object count)
         concatenated canonical-bytes blobs
       write .moof/store/packs/pack-<sha>.idx:
         hash → offset-in-packfile map
  4. remove the loose copies (atomic rename trick on the index file
     ensures readers don't see torn state)
```

reads are transparent — store knows pack-vs-loose; mmaps packfiles;
index lookup gives offset; one mmap, no per-object syscall.
**packfile reads are faster than loose-object reads.**

automatic trigger: weekly / on idle / on explicit `[$image pack]`.

### 10.4 journal pruning

once a merkle checkpoint exists for turn N, journal entries before
N are redundant (the snapshot captures their effect):

```
[$image prune-journal-before: turn-300]:
  for each vat:
    rewrite vat's inputs.log: discard entries for turns < 300
    preserve a "snapshot at turn 300" marker at the journal head
  rewrite effects.log similarly
```

automatic when the oldest retained checkpoint advances past a
journal entry. **bounds inputs.log size at "retention window worth
of turns,"** typically megabytes for a busy vat over 24h.

### 10.5 storage targets

| world type | size after settling |
|---|---|
| interactive workspace, months of use | 50-200 MB |
| heavy multi-fork experimental | 500 MB - 2 GB |
| production federation node, years | 5-20 GB (with gc) |
| unlimited-history archival | unbounded; opt-out of retention |

a long-history multi-fork world is gb-scale and gc'd manually when
desired (the user values the history more than the disk).

### 10.6 invariants

- gc never collects objects reachable from any ref. `refs/` is the
  oracle.
- packfile + loose representations are equivalent; reads work
  across both transparently.
- journal pruning never removes entries past the oldest retained
  checkpoint. **rewindability up to the oldest retained checkpoint
  always works.**
- ref updates are atomic (write-temp-then-rename).
- gc + pack + prune are themselves journaled (the maintenance
  operation appears in the world's audit log, observable via the
  inspector).

---

## §11 — mandatory mco serialize/restore

### 11.1 every mco implements the protocol

no exceptions. the ABI grows two required exports per mco:

```c
// returns the bytes needed to restore this mco's state; substrate
// embeds these in the merkle store keyed by content-hash.
MoofResult mco_serialize(MoofCtx* ctx, uint32_t* out_handle);

// reconstitutes the mco's state from previously-serialized bytes;
// rebinds any os handles (reopen files, reconnect sockets,
// recreate gpu surfaces, etc.).
MoofResult mco_restore(MoofCtx* ctx, uint32_t bytes_handle);
```

manifest.moof gains a required field:

```moof
{ serializability:
    'pure              ; pure compute; serialize = nothing; restore = nothing
    'linmem-only       ; serialize copies linmem; restore copies back
    'rebind-handles    ; serialize captures state; restore rebinds external
    'ephemeral-warn    ; serialize emits a warning blob; restore raises
                       ;   recoverable error; consumer decides to retry or
                       ;   skip (e.g., a connection died mid-protocol)
}
```

### 11.2 mco categories

| category | example mcos | serialize | restore |
|---|---|---|---|
| **pure compute** | hash, base64, utf8 | empty bytes | no-op |
| **linmem-only state** | random (prng state in linmem) | copy linmem | copy back |
| **filesystem-bound** | lmdb, sqlite | path + open transaction id | reopen db; reconcile |
| **network-bound** | websocket, http | url + handshake state + pending queue | reconnect; reapply pending |
| **gpu-bound** | wgpu canvas, render targets | canvas dimensions + framebuffer contents | recreate canvas; reblit |
| **os-bound** | clock (stateless) | empty bytes | no-op |

**ephemeral-warn category**: mcos with truly unrecoverable transient
state (e.g., mid-protocol tcp that depends on remote nonce). on
restore, raise a recoverable error; the consumer (typically a
workspace vat) decides whether to reset state or signal disconnect.

### 11.3 image save flow involving mcos

```
during [$image checkpoint]:
  for each mco-bound proto-Form in any vat:
    call mco's mco_serialize → bytes
    treat bytes as a content-addressed blob; store via blake3
    record (proto-FormId, bytes-hash) in the proto-Form's slot table
      (as :mco-state hash-pointer)
  proceed with normal merkle walk; the mco's bytes are now just
  another set of objects in the store
```

### 11.4 image load flow

```
during boot from a stored ref:
  reconstitute vats; for each mco-bound proto-Form encountered:
    read :mco-state hash-pointer
    fetch bytes from store
    instantiate the mco's wasm module
    call mco's mco_restore with the bytes
    on ephemeral-warn raise: log a warning; mark proto as 'needs-reset
    on rebind-handles raise (e.g., file unreachable): same
  schedulers resume; vats can detect 'needs-reset and ask user
  what to do (typically: retry, drop, escalate)
```

### 11.5 the discipline

mandatory serialize/restore **forces clean reasoning about state
ownership**. if a piece of state ought to survive image save, it
lives in linmem or in proto-Form slots. if it can't survive, the
mco declares so explicitly via `'ephemeral-warn`. **no escape
hatch where the substrate silently loses state.**

### 11.6 cost

most mcos are `'pure` or `'linmem-only` and serialize trivially
(microseconds, kilobytes). a wgpu mco serializing a 1080p
framebuffer is the heaviest case — ~8 MB per save during active
rendering. saves while idle re-hash to identical bytes → same hash
→ already in store → zero new disk write. **the cost concentrates
on active animation; flush cadence can adapt** (longer batching
while a render-heavy vat is hot; tighter batching when only ui
state is changing).

---

## §12 — transporter round-trip + intrinsic shrink + vat ergonomics

### 12.1 transporter as bidirectional sync

self-style. the transporter is the bridge between **text on disk**
(human-editable source) and **objects in the image** (canonical
state).

```moof
[$transporter load: "stdlib/cons.moof"]
  ; source → image. exists today.

[$transporter dump: cons-proto to: "stdlib/cons.moof"]
  ; image → source. uses each method's :source slot to emit the
  ; exact original text; for methods born in the image (not loaded
  ; from a file), uses a default decompiler that emits canonical
  ; moof code from the Form structure.

[$transporter watch: "lib/"]
  ; spawn a file-watcher vat in the system-services tree
  ; on filesystem change: reload affected definitions; methods
  ;   invalidate ICs (per L10); running closures keep old code
  ;   until they return.
  ; on form change in image: queue a debounced write to source
  ;   file (every ~1s of inactivity on that form).

[$transporter conflict-policy: 'file-wins | 'image-wins | 'prompt]
  ; default 'file-wins during dev (the human's typing IS the intent);
  ; 'image-wins for headless mode (no typist).
  ; 'prompt opens an inspector to resolve manually.
```

### 12.2 intrinsic shrink list

specific axe-list. target: `intrinsics.zig` **2506 → ~1500 LoC**.

| stays in zig (truly primitive) | moves to moof | moves to mco |
|---|---|---|
| `Form:proto`, `:slots`, `:handlers`, `:meta`, `:freeze`, `:identity`, `:source` | `Object:=`, `:!=`, `:satisfies?`, `:is-fallback`, `:toString-name-fallback`, `:initialize` | — |
| `Cons:car`, `:cdr`, `:cons:` | `Cons:length`, `:reverse`, `:map:`, `:filter:`, `:reduce:`, `:forEach:`, `:take:`, `:drop:`, `:any?:`, `:all?:`, `:contains?:`, `:append:`, `:zip:`, `:scan:`, `:at:`, etc. (all derivable) | — |
| `Integer:+`, `:-`, `:*`, `:/`, `:=`, `:<`, `:>`, `:asFloat` | `Integer:abs`, `:even?`, `:odd?`, `:between?:`, `:max:`, `:min:`, `:<=`, `:>=`, `:!=` | BigInt heavy ops (already partly mco) |
| `Float:+`, `:-`, `:*`, `:/`, `:=`, `:<`, `:>` | `Float:abs`, `:max:`, `:min:`, `:asInteger`, `:round`, `:floor`, `:ceil`, `:<=`, `:>=`, `:!=` | — |
| `String:byteAt:`, `:length`, `:byteEq`, `:concat`, `:at:`, `:slice:length:`, `:as:`, `:toList`, `:contains?:` | `String:trim`, `:indexOf:`, `:replace:with:`, `:split:`, `:lines`, `:toString`, `:inspect`, `:asTable`, `:startsWith?:`, `:endsWith?:`, `:reverse` | full utf8 codepoint walker (utf8 mco) |
| `Char:codepoint`, `:<` | `Char:inspect`, `:toString`, `:digit?`, `:letter?`, `:uppercase`, `:lowercase` | — |
| `Table:new`, `:length`, `:at:`, `:at:put:`, `:push:`, `:pop`, `:keys`, `:values`, `:remove:`, `:containsKey?:` | `Table:size`, `:empty?`, `:nonEmpty?`, `:asString`, `:toString`, `:inspect`, `:=`, `:as:`, `:forEach:` | — |
| `Method:body`, `:source`, `:params`, `:consts`, `:bytecodes`, `:ics`, `:call` | `Method:toString`, `:inspect` (slot-walking) | — |
| `Console:emit:`, `:close`, `:next` | — | (could become an mco if we want; small enough to stay native) |
| substrate-VM primitives: `setHandler!`, `slotSet!`, `metaSet!`, `globalEnv`, `intern`, `raise:`, `__send__`, `__decode-header`, ICs | — | — |
| **vat-related** (new): `__spawn-vat`, `__send-async`, `__mailbox-receive`, `__image-checkpoint`, `__image-load`, `__image-fork`, `__supervisor-link` | — | — |

### 12.3 vat ergonomics in moof

the user-facing layer lives in `lib/early/11-vats.moof` and
`lib/stdlib/vats.moof`. macros and parser-level sugar:

#### eventual send: `<-`

parser-level sugar. reader rewrites `[obj <- selector: arg]` →
`(__send-async__ obj 'selector arg)`. compiler emits
`OP_EVENTUAL_SEND` per V7 of the vats spec. evaluates immediately
to a Promise.

requires a small parser/reader change in `lib/parser/02-parser.moof`
(adding `<-` as a recognized token before keyword sends).

#### `spawn` macro

```moof
(spawn body
        mode: 'mutable-by-default
        at: "/users/me/counter"
        pin: 'cpu-2)

;; expands to:
[$vat spawn: (fn (mb) body)
              mode: 'mutable-by-default
              at: "/users/me/counter"
              pin: 'cpu-2]
```

#### `receive` macro

cleaner than `match-receive`. binds the envelope implicitly so
`reply` knows its target:

```moof
(receive
  ['incr]            (do (set! count (+ count 1)) (reply count))
  ['get]             (reply count)
  ['stop]            (return)
  [other]            (raise 'unknown-message other))

;; expands to:
(let envelope (__mailbox-receive))
(match (envelope :body)
  ['incr]  (do (set! count (+ count 1)) [envelope reply: count])
  ['get]   [envelope reply: count]
  ['stop]  (raise 'vat-stop)
  [other]  (raise 'unknown-message other))
```

`reply` is a special form inside `receive`-body; expands to
`[envelope reply: value]`.

#### `loop` macro

tail-recursive idiom; doesn't require a function name:

```moof
(loop body)

;; expands to:
(let-rec __loop () body (__loop))
```

#### supervisor declaration

```moof
[$supervisor children:
  ['parser    [Parser spawn]    strategy: 'permanent]
  ['compiler  [Compiler spawn]  strategy: 'permanent]
  ['workspace [Workspace spawn] strategy: 'transient]]
```

`[$supervisor children: ...]` is a regular method send on the
supervisor singleton. `strategy` and child-spawn-thunks are
declarative; supervisor walks the list and brings each up under
the chosen strategy (`'permanent` = always restart; `'temporary` =
never; `'transient` = only on abnormal exit).

#### example: a counter vat, end-to-end

```moof
(def counter
  (spawn
    (do
      (def count 0)
      (loop
        (receive
          ['incr]  (do (set! count (+ count 1)) (reply count))
          ['get]   (reply count)
          ['stop]  (return)
          [other]  (raise 'unknown-message other))))
    mode: 'mutable-by-default
    at: "/users/me/counter"))

;; usage:
(let p (counter <- incr))
[p when-resolved: |v| [$out say: "now: " v]]
```

### 12.4 what parses vs what needs new work

| feature | status |
|---|---|
| `<-` token | parser-level addition (new) |
| `(spawn body opts...)` | macro (new) |
| `(receive ...)` + `reply` | macro (new) |
| `(loop body)` | macro (new) |
| `[$supervisor children: ...]` | regular send; supervisor implemented as singleton (new moof file) |
| `[$vat spawn: ...]` | regular send; substrate intrinsic |
| `[$image checkpoint/fork/load/pack]` | regular send; substrate intrinsics |
| `[$transporter load: / dump: / watch:]` | regular send; substrate primitives + system-services-vat |

---

## §13 — performance + compactness as design goals

every section of this spec leaves room for tightness. the
architecture is shaped around these targets; **meeting them is a
first-class deliverable, not a follow-up.** the substrate is
small, the forms are tight, the turns are fast, the persistence is
incremental — by design, not by accident.

### 13.1 substrate compactness

| metric | target | how the design supports it |
|---|---|---|
| substrate zig LoC | **5-7K** (from 10.7K) | §1, §12: aggressive push to moof + mcos |
| player binary size | **3-5 MB** static (release) | one self-contained binary; embedded wasm runtime |
| bundled seed.vat | **~90 KB** | minimal subset; ocaml-seed strips heavily |
| stdlib + mcos in shared segment | **<2 MB** | content-addressed dedup; canonical encoding |

stretch goal: **5K LoC zig substrate**, if we push more to moof
than the conservative shrink list in §12.2 implies (e.g., move
parts of the VM dispatch into moof-side specialization).

### 13.2 form memory layout

| metric | target | how |
|---|---|---|
| minimum Form struct (empty) | **~32 bytes** | lazy slot/handler/meta tables; null = no alloc |
| typical Form (3-5 slots, 1-2 handlers) | **64-128 bytes** | packed alignment; small-table inline storage |
| cached hash (vat-local) | **8 bytes** (truncated blake3-64) | safe to ~10^10 forms; full 256-bit only on shared-segment |
| proto pointer | **4 bytes** (32-bit FormId) | already in design |
| frozen + dirty + scope-tag bits | **1 byte** packed flags | bit-packed |

**invariant: a freshly-allocated Form with no slots costs ~32 bytes.**
table allocations happen only on first slot-set / handler-attach.
this lets a workspace with thousands of small forms (every word in
a text buffer, every pixel in a sprite) fit easily.

### 13.3 vat memory + spawn rate

| metric | target | how |
|---|---|---|
| minimum Vat struct (idle) | **<1 KB** | lazy mailbox, lazy nursery, lazy journal, lazy far-ref-table |
| typical Vat (active) | **4-16 KB** | most state in heap, not in struct |
| vat spawn rate | **100K/sec** on desktop | lightweight struct; no syscalls on spawn |
| concurrent vat count | **100K** | sub-KB overhead × 100K ≈ 100 MB at saturation |

implies: **lazy initialization of every Vat substructure that isn't
needed on day 1 of the vat's life.** a brand-new vat that hasn't
received a message yet has no allocated mailbox; a vat that hasn't
mutated state yet has no allocated nursery; a vat that hasn't
linked to a far-ref hasn't allocated the far-ref-table.

target rationale: BEAM spawns 100K+ processes routinely. matching
this opens the (γ) "everything-is-a-vat" granularity option if a
workload ever needs it.

### 13.4 per-turn cost

| metric | target | how |
|---|---|---|
| empty turn (yield, no message) | **<1 μs** | tight loop in scheduler |
| typical turn (1-3 mutations) | **10-100 μs** | nursery + diff + hash recompute |
| turn rate per scheduler | **100K-1M/sec** | workload-dependent |
| journal fsync per turn | **100 μs - 1 ms** | SSD-bound; batching reduces |

at 1M turns/sec/scheduler × 4 cores = 4M turns/sec total per
process. each turn produces a journal entry → 4M fsyncs/sec
saturates disk. **mitigation**: configurable fsync batching (lose
<1ms of work on crash for a typical batch); per-vat journal
grouped writes.

### 13.5 cross-scheduler messaging

| metric | target | how |
|---|---|---|
| MPMC enqueue | **<100 ns** | lock-free atomic CAS on tail |
| per-message envelope | **<100 bytes** typical | tight encoding; vat-id + form-id + sym-id + args |
| mailbox memory | **64 B header + 8 B/slot** | ring buffer; grows on demand |
| backpressure trigger | **10K messages** (configurable) | per-link flow control |

### 13.6 shared segment + intern table

| metric | target | how |
|---|---|---|
| intern lookup | **<100 ns** | atomic acquire-load on probed slot |
| intern install (CAS) | **<1 μs** | hash + slot probe + atomic exchange |
| per-interned-form overhead | **<50 bytes** | hash entry + table slot + arena slot header |
| shared segment cap | **256 MB** default (configurable) | gc reclaims when refcount drops or cap nears |

### 13.7 persistence

| metric | target | how |
|---|---|---|
| typical merkle flush size | **<100 KB** per flush | only changed-subgraph objects |
| flush latency | **<10 ms** | batched sequential writes; one fsync per batch |
| store size (months interactive) | **50-200 MB** | reflog retention + gc + pack |
| cold boot from system.vat | **<1 sec** | lazy mmap; materialize on access |
| time-travel to past turn | **<100 ms** | direct merkle ref load; no full replay needed |

### 13.8 mcos

| metric | target | how |
|---|---|---|
| typical mco binary | **<100 KB** | -O3 wasm + size-tuned languages |
| mco instantiation | **<1 ms** | wasmtime AOT cache |
| mco call overhead | **<1 μs** | tight trampoline; handle-table reuse |
| mco serialize, pure compute | **<1 μs**, empty bytes | most stdlib mcos |
| mco serialize, linmem-only | **proportional to linmem** | bulk memcpy |

### 13.9 tuning levers

knobs the substrate exposes for measurement-driven tuning:

```
MOOF_SCHEDULERS              N pinned scheduler threads (default: ncpu)
MOOF_FLUSH_TURNS             turns between merkle flushes (default 10)
MOOF_FLUSH_MS                ms between merkle flushes (default 100)
MOOF_FSYNC_BATCH             turns between journal fsync (default 1)
MOOF_RETENTION_HOURS         reflog window (default 24)
MOOF_SHARED_CAP_MB           shared segment cap (default 256)
MOOF_HASH_BITS               64 or 256 (default 64; 256 on shared seg)
MOOF_VAT_SPAWN_PREALLOC      N idle vat structs preallocated (default 0)
MOOF_PACKFILE_THRESHOLD      loose objects before pack triggers (default 1000)
MOOF_NURSERY_INITIAL_KB      initial nursery size (default 64; grows)
```

defaults work for an interactive workspace. tune for headless
servers (more aggressive batching), embedded (smaller caps),
high-throughput (larger nursery, batched fsync).

### 13.10 measurement discipline

every implementation phase ships **microbenchmarks** for its
targets:

- phase 2 (vat carve): vat spawn rate, idle vat memory, turn cost
- phase 3 (shared segment): intern lookup, install, promotion cost
- phase 4 (multi-scheduler): MPMC enqueue, cross-scheduler send latency, contended-shared-segment throughput
- phase 5 (persistence): journal append, merkle flush, cold boot
- phase 6 (mco serialize): per-category serialize/restore cost
- phase 7 (compaction): gc throughput, pack consolidation, size reduction
- phase 8 (supervision): supervisor restart latency, child spawn

**perf regressions block merge.** the conformance suite includes
perf oracles with tolerance bands per metric. measurement is part
of the conformance contract, not separate.

### 13.11 stretch goals (post-v1.0)

if v1.0 ships and we want to push further:

- **1B sends/sec hot-code path** via Self-style shape specialization
  + per-call-site JIT (phase 3 vision §6.3)
- **<100 ns turn loop** via threaded dispatch + flat env + flat
  closure (phase 2 perf spec §5.3-5.5)
- **substrate <5K LoC** via more aggressive moof-side migration of
  things currently in `intrinsics.zig` (e.g., bytecode emit becomes
  a moof-internal concern, substrate just consumes finished chunks)
- **<10 MB image for typical workspaces** via per-vat zstd on
  packfiles + smarter encoding
- **1M concurrent vats** via tighter Vat struct (256-512 bytes
  minimum) and pooled allocator
- **zero-copy cross-process far-ref via shared memory** when both
  parties on same host

---

## §14 — implementation phasing

dependency order. each phase compiles, tests pass at boundary.

**this spec is the umbrella design; each phase below is a session-
sized implementation plan generated by its own `superpowers:writing-
plans` invocation, not all at once.** the writing-plans handoff at
the end of this spec produces the plan for **phase 1 only**;
subsequent phases get their own writing-plans sessions after their
predecessors land.

### phase 1: housekeeping + freezing (substrate, V2)

- **§1**: directory rename (`crates/` → `players/`, `seed/`,
  `tools/`). mechanical; no semantic change. **half a day.**
- **§4**: freezing primitive + mutation guard. vat-mode flag on
  current World (multi-vat not yet). `let-mutable` form. moof-side
  `freezeRecursive` helper. **~2-3 days.**
- **§12.2** intrinsic shrink first pass: move derivable Cons /
  Integer / Float / String methods to moof. measure
  `intrinsics.zig` LoC drop. **~3-4 days, parallelizable.**

exit: freezing is solid; `[1 is nil]` still works; intrinsics.zig
shrunk by ~30%.

### phase 2: vat carve (substrate, V4)

- **§2** + **§3**: `World` carved into `World + Vat`. single
  scheduler initially. `Vat` struct, per-vat heap, mailbox, here,
  behavior, caps. turn loop in scheduler. **~1-2 weeks.**
- intrinsics for vat ops: `__spawn-vat`, `__send-async`,
  `__mailbox-receive`, `__supervisor-link`.

exit: two vats coexist in one process; within-vat sends still sync;
cross-vat send delivers async at next turn-boundary.

### phase 3: references protocol + shared segment (V5 + V6)

- **§5**: shared segment + intern table. promotion on cross-vat
  send. forwarding pointers. **single-threaded data structure in
  this phase** (locking unnecessary because phase 4 hasn't added
  threads yet); lock-freedom comes in phase 4. **~1-2 weeks.**
- **§6**: content-hash recomputed at turn commit for changed forms.
  vat-id + path-table-vat. **~3-5 days.**

exit: cross-vat sends route through membrane translation; frozen
forms dedup automatically; identical canonical bytes converge on
same SharedFormId.

### phase 4: scheduler multi-threading (V7-V8 prereq)

- **§7**: N=cores schedulers; pinned vat-pool per scheduler; MPMC
  mailbox; cross-scheduler send. **~1-2 weeks.**
- thread-safety pass: convert shared segment intern table from
  single-threaded (phase 3) to lock-free reads + CAS writes;
  far-ref tables stay per-vat; mailbox MPMC primitive lands.
- stress tests specifically targeting the intern table under
  concurrent promotion load.

exit: parallel scheduler measurement shows ~Nx throughput on
embarrassingly-parallel vat workloads; single-vat sequential
performance unchanged.

### phase 5: persistence via merkle store (V9)

- **§8**: object store layout; canonical-bytes encoding; merkle
  hash propagation. **~1-2 weeks.**
- **§9**: autosave loop; per-turn journal fsync; batched merkle
  flush; checkpoint refs. **~1 week.**
- crash recovery: replay journal from last flushed ref. **~3-5 days.**

exit: `.moof/store/` is canonical; restart preserves state;
explicit `[$image checkpoint / fork / load]` work.

### phase 6: mco serialize/restore (V9 follow-up)

- **§11**: ABI growth (mco_serialize, mco_restore exports);
  manifest field; per-mco implementation for hash, random, base64,
  utf8, clock. **~1 week per mco for ones with state; days for
  pure ones.**

exit: image save survives across all currently-shipped mcos. new
mcos must include serialize/restore.

### phase 7: compaction (V12)

- **§10**: gc, pack, reflog retention, journal prune. **~1-2 weeks.**

exit: store stays bounded under continuous use; pack reduces inode
pressure; gc reclaims unreachable history.

### phase 8: supervision + promises (V7 + V8)

- supervisors as Forms; spawn-child mechanics; restart strategies
  (`'permanent`, `'transient`, `'temporary`). **~1-2 weeks.**
- promises as Forms; three-state machine; `when-resolved:`,
  `when-broken:`, `then:`; pipelining. **~1-2 weeks.**

exit: crash-then-restart works; `<-` syntax returns Promise;
chained promises pipeline correctly.

### phase 9: caps + effect-intents (V10)

- cap-bag on vat; cap-token unforgeability via substrate-issuance;
  attenuation handlers. **~1 week.**
- effect-intent accumulation in outbox; cap authority worker;
  at-most-once receipt journaling. **~1-2 weeks.**

exit: replicated vats can do side-effects via intents; receipts
prevent re-fire across reboots.

### phase 10: replication (V11)

- replicated-leader / replicated-follower modes; deterministic
  FormId allocation; input-log replication transport. **~2-3 weeks.**
- per-slot mergeable annotation; CRDT merge on arrival. **~1-2 weeks.**

exit: two-replica convergence test passes; one vat can be migrated
across hosts and resume.

### phase 11: transporter round-trip + vat ergonomics (V13)

- **§12.1**: transporter `dump:to:` half; `watch:` system-services
  vat. **~1-2 weeks.**
- **§12.3**: vat ergonomics macros (`spawn`, `receive`, `loop`);
  parser extension for `<-`; supervisor declaration. **~1 week.**

exit: edit a method in a file, see image update. edit a method in
inspector, see file update. counter-vat example runs as written.

### phase 12: ongoing — perf, security, polish

- conformance suite expansion to 200+ triples
- security audit of the mco surface
- tier-2 perf (PICs, threaded dispatch) — overlaps with above

### total wall-clock estimate

phases 1-7 (substrate + persistence + mco serialize + compaction):
**~8-12 weeks** of focused work, mostly serial. phases 8-12 are
largely independent and can parallelize across two or three streams:
**~8-12 additional weeks** to v1.0 from spec acceptance.

**target v1.0 ship: ~5-6 months from spec acceptance** if focused,
~8-10 months realistically.

---

## §15 — risks + open questions

### 15.1 substrate refactor scope

the world → world + vat carve is large. underestimating it would
stall the entire roadmap. **mitigation:** phase 2 is its own
session; do it before scoping later phases. budget bug fix-loop
time generously.

### 15.2 thread-safety of the shared segment

CAS install on the intern table is the most subtle code path.
**risks:** ABA, lost updates under high promotion concurrency,
table-grow synchronization. **mitigation:** start single-threaded
(phase 3); enable multi-thread (phase 4) with stress tests
specifically for the intern table; consider hazard pointers if
needed.

### 15.3 mco linmem serialization edge cases

wgpu framebuffers, mid-protocol tcp, file-handles. **risks:**
restore semantics that look right in isolation but don't actually
recover usable state. **mitigation:** define `'ephemeral-warn`
category explicitly; mcos that can't truly recover declare so;
consumers (workspace vats) handle the warning.

### 15.4 autosave performance under load

per-turn journal fsync at 1000+ turns/sec on a hot vat could
overwhelm disk. **mitigation:** measure first; if it bites,
options include: batched fsync (lose <1ms of work on crash),
async io_uring, dedicated journal thread. acceptable to start
with per-turn fsync and tune later — most vats aren't at 1000+
turns/sec.

### 15.5 vat-mode + parser/compiler interaction

the parser and compiler are frozen-by-default vats. they construct
new Forms during parsing/compiling. **risk:** building a complex
AST involves many mutations to many forms before final
freezing-at-end; `let-mutable` discipline must handle this
ergonomically. **mitigation:** the parser builds via local helpers;
the helper-call expression's return value is the to-be-frozen form;
helper's body is a `let-mutable` block; mutation is local. test
this on real parser code in phase 1.

### 15.6 merkle hash collisions

blake3 has ~2^128 collision resistance. for moof workloads (max
~10^12 forms over a project's lifetime) this is comfortably safe.
**mitigation:** if a collision ever surfaces (it won't), the
substrate raises `'hash-collision` and refuses to install — gives
a debuggable failure mode rather than silent corruption.

### 15.7 path-table-vat as a federation bottleneck

if every cross-host send goes through path resolution, the path-
table-vat is a hot spot. **mitigation:** path resolution caches on
the resolving vat (the resolved id-ref or far-ref is itself stable
for the resolved form's life). cache invalidation only on
path-table-vat mutation.

### 15.8 user adoption of mandatory mco serialize/restore

users writing custom mcos face higher burden. **mitigation:**
ship language glue libraries (in `tools/abi/`) that provide
default impls for `'pure` and `'linmem-only` cases — most user
mcos won't need to write more than a `serializability:` field.

### 15.9 transporter conflict resolution under heavy editing

simultaneous file-edit and inspector-edit of the same definition
is rare but possible. **mitigation:** debounce both directions;
`'prompt` policy opens an inspector view with diff. for now
default to `'file-wins`; revisit if anyone complains.

### 15.10 image format binding at v1.0

once v1.0 ships, the merkle store layout + canonical-bytes
encoding is binding. **mitigation:** write the format spec as
part of v1.0; freeze it; future changes go through `moof migrate
<old-store> --to v2`. format spec lives at `docs/reference/image-
format.md` (new doc, late phase 5).

---

## §16 — what's NOT in this spec

deferred to follow-up specs / sessions:

- **federation transport details** (websocket / unix-socket / tls
  / ed25519 wire formats). the protocol is specified here
  (envelopes, far-refs, cap-tokens, promises); the wire is
  `docs/concepts/transport.md` and `docs/reference/wire-
  format.md` (later).
- **morphic gui design** (canvas, render protocol, input
  dispatch, per-morph state, the inspector-as-morphic-app). its
  own session-sized brainstorm.
- **conformance test corpus** (the 200+ triples). its own session
  to design + author.
- **tier-2 / tier-3 perf** (PICs, JIT, shape specialization). see
  `2026-05-16-phase2-moof-performance-design.md` and phase 3
  cohesive vision §6.
- **mcp integration** (model context protocol over the federation
  layer). rests on federation transport spec.
- **gpu / native dylib mcos (tier 3)** — format reserved per
  mcos-and-datasource spec; loader work is its own session.
- **typed moof** (refinement types, dependent types). research
  arc; later.
- **work-stealing scheduler** — explicitly v2 optimization (§7.3).

---

## §17 — see also

- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md`
- `docs/superpowers/specs/2026-05-16-phase3-cohesive-vision-design.md`
- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md`
- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4 image format §10
- `docs/superpowers/specs/2026-05-16-phase2-moof-performance-design.md`
- `docs/superpowers/specs/2026-05-11-phase1-gc-dispatch-compression-design.md`
- `docs/superpowers/specs/2026-05-02-transporter-and-stdlib-modularization-design.md`
- `docs/superpowers/specs/2026-05-03-track-1-mcos-and-datasource-design.md`
- `docs/concepts/forms.md`, `vats.md`, `references.md`,
  `replication.md`, `compiled-objects.md`, `data-sources.md`,
  `capabilities.md`, `effect-intents.md`
- `docs/laws/substrate-laws.md`, `determinism-laws.md`
- `NEXT_SESSION.md` — substrate state at HEAD `041f8fd`

---

`٩(◕‿◕｡)۶` — image is canon, turns are time, journal is memory.
