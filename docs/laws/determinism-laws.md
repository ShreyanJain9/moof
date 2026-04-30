# determinism laws

> **the substrate's promises about *what is observably the same*
> across two replicas of the same vat fed the same input log. these
> are stricter than purity: a `#pure` function is allowed to
> consult `$clock`, but a determinism-mode vat is not.**

determinism is what makes croquet-style replication possible
(`concepts/replication.md`). without it, two replicas processing the
same totally-ordered input stream will drift. with it, they are
bit-identical at every turn boundary, forever.

## D1 — determinism is a per-vat property, set at birth

a vat's *replication mode* is one of:

| mode | determinism scope |
|---|---|
| `:solo` | no determinism guarantee; can hold OS caps. |
| `:replicated-leader` | deterministic; emits the canonical input log. |
| `:replicated-follower` | deterministic; consumes the input log. |

modes are fixed at vat birth (`concepts/vats.md`). a vat cannot be
promoted from solo to replicated; it has to be born replicated. a
replicated-follower can be promoted to leader during failover.

## D2 — within a replicated turn, every observable depends only on the turn-envelope and the heap

a replicated turn's input is exactly the *turn envelope*:

```
{ session-id   : symbol         ; which replication session
  epoch        : integer         ; reflector epoch (bumped on rejoin)
  turn-seq     : integer         ; monotonic per epoch
  author       : far-ref         ; who supplied this input
  logical-now  : integer         ; ticks since session start
  input-event  : Form            ; the user input or effect-receipt
  seed         : 64-bit          ; per-turn entropy from the reflector
}
```

inside the turn, every observable behavior is a pure function of the
envelope and the current heap. the substrate refuses any operation
that would observe something else.

## D3 — forbidden operations in a replicated turn

a replicated-mode vat cannot, mid-turn:

- read wall-clock or monotonic time (substrate refuses; `$clock` is
  not in scope).
- read OS entropy (substrate refuses; `$random` is not in scope —
  use the envelope's `seed` instead).
- read process-id, thread-id, host-id, or any os-environmental value.
- read network state, file-system state, or any other OS resource.
- iterate a hashmap in hash-bucket order (must be insertion-order
  or sorted-key order; see D5).
- compare pointer addresses or anything that would expose memory
  layout.
- depend on GC timing (D6).
- depend on bytecode-cache hit/miss (`laws/substrate-laws.md` L5;
  bytecode is derived).
- depend on inline-cache hit/miss.

violation is a substrate error, raised at the offending operation.
the offending input is rejected; the turn is *not committed*; the
reflector is notified.

## D4 — deterministic allocation order

within a replicated turn, FormIds are allocated in a deterministic
sequence:

```
form-id := (turn-seq << N) | local-counter
```

where `local-counter` is a per-turn counter starting at zero and
incremented on each allocation, and `N` is large enough to not
overflow per-turn (default: 32 bits, allowing 2³² allocs per turn).

this means: replica A and replica B, processing the same envelope,
allocate forms with the *same form-ids*. cross-replica references
remain meaningful for snapshot/replay.

trade-off: heap-ids grow over session lifetime. compaction at
checkpoints (`concepts/persistence.md`) reclaims unused ids by
rewriting heap pages; the (turn-seq, local-counter) decomposition
is stable across compaction.

## D5 — deterministic iteration order

operations that iterate a Table or Set:

- if the user requested `:sorted-by:` or `:sorted`, the ordering is
  the explicit comparator.
- otherwise, the substrate iterates in **insertion order**
  (preserves the order keys were added; like javascript Maps and
  python dicts since 3.7).

never hash-bucket order. never address order.

## D6 — gc runs at turn boundaries only

garbage collection in a replicated vat is *deferred until turn-end*.
mid-turn allocations may push the heap past soft limits without
triggering collection. the collection pass runs as part of the
commit phase, after journaling.

this means: GC pauses are predictable (one per turn), and GC
behavior cannot affect observable computation within a turn.

## D7 — deterministic promise ids

promises (`concepts/references.md`) get ids `(turn-seq, ordinal)`,
the same shape as effect-intents. two replicas processing the same
envelope create the same promise ids in the same order.

awaiting a promise within a replicated turn is permitted only for
*intra-vat* promises (which resolve before turn-end) and for
*receipt-shaped* promises (which resolve in a future turn from a
receipt envelope). awaiting a far-ref promise mid-turn is forbidden
in replicated mode.

## D8 — proto edits are turn-envelopes

mutating a proto's handler table (live editing a method) is itself
a turn-input. it appears in the input log just like a stroke, a
mouse click, or a flood-fill. all replicas see the edit at the same
logical-now and re-derive bytecode locally.

```
input-event := {ProtoEdit
  target: <proto-id>
  selector: 'incr
  source: <new-source-form>}
```

bytecode is per-replica derived state, never replicated.

## D9 — the canonical hash

every replicated vat has a `:canonical-hash` method that returns a
deterministic blake3 hash of the heap's canonical encoding
(`concepts/persistence.md`). the hash:

- is invariant under the iteration-order rule (D5).
- ignores derived-only state: bytecode caches, inline caches.
- includes: form-ids, slots, handlers (as source-forms), meta,
  and the input log up to this turn.

two replicas at the same turn-seq must produce the same hash. this
is the test gate for phase D
(`docs/process/impl-plan-v4.md`).

## D10 — leader-follower promotion preserves determinism

if the leader fails and a follower is promoted, the new leader:

1. reads its committed turn-seq.
2. announces "i am leader at epoch+1" to the reflector.
3. the reflector closes the old epoch's input log; opens epoch+1.
4. all subsequent envelopes carry epoch+1.

inputs after the failover that hadn't reached the old leader are
*lost*. the input log is the truth; nothing the old leader didn't
durably commit existed.

(this is the raft/zab discipline. moof's reflector is the
log-authority; the leader is the canonical replica that controls
which inputs make it in.)

## D11 — the input log is the truth

if a replica's local state disagrees with what the input log
implies, the input log wins. the replica must:

- if it has uncommitted state ahead of the log: discard.
- if it has committed state behind the log: catch up.

snapshots are derived; the log is canonical. snapshot+log-replay
must reconstruct exact state, byte-for-byte.

## D12 — snapshot equivalence

a snapshot taken at turn-seq N, plus the input log from N+1 onward,
must produce a replica bit-identical to one that processed every
turn from genesis. this is the property checkpoints depend on.

violation = substrate bug. test cases must verify on every snapshot
implementation change.

## inspirations

- croquet's "teaTime" deterministic-actor model: kay, reed, smith
  ~2003.
- raft's leader/follower/log-as-truth discipline: ongaro & ousterhout
  2014.
- erlang's "process determinism by default" rhetoric — though
  erlang doesn't actually enforce determinism, the moof discipline
  here is what erlang-shaped code naturally writes.
- chrome's deterministic-replay debugger (RR-style record/replay).
- python 3.7+ dict insertion-order: van rossum / de boer.
- web platform's "structured clone" determinism rules.

## see also

- `concepts/replication.md` — what replicated vats are.
- `concepts/persistence.md` — how the input log is stored.
- `concepts/effect-intents.md` — how effects fit into the model.
- `laws/substrate-laws.md` — the broader substrate promises.
- `laws/isolation-laws.md` — vat-boundary rules.
