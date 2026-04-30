# replicated vats

> **a replicated vat is the same substrate-level vat, born in a
> mode where it is bit-identical across machines. all replicas
> process the same totally-ordered input log; the canonical state
> is a pure fold over that log. this is the croquet pattern,
> adapted to moof.**

replication is moof's answer to "alice and bob are working in the
same world" (`concepts/world-and-space.md`). it is **not** a
separate kind of object; it is a *mode* a vat can be born in
(`concepts/vats.md`). regular point-to-point messaging via far-refs
continues unchanged. typically: the world-vat is replicated; per-
user wrapper vats are solo.

## the model

```
                     reflector
                  (orders inputs)
                     │  │  │
            ┌────────┘  │  └────────┐
            ▼           ▼           ▼
       replica-A    replica-B    replica-C
       (alice)       (bob)        (cyrus)
```

each replica:
- has its own heap (vat-local FormIds — but allocated
  deterministically per `laws/determinism-laws.md` D4, so all replicas
  share the same id assignments).
- has its own bytecode cache (re-derived from source per replica).
- has its own ambient capabilities (each replica's local supervisor
  hands them in; caps are *not* replicated).
- consumes the same totally-ordered input log from the reflector.

at every turn boundary, every replica's `[vat canonical-hash]` is
identical. if any replica's hash drifts, that replica is broken.

## the reflector

the reflector is a *small, opinionated authority* whose only job is:

1. accept user-input events from authenticated replicas.
2. order them into a single sequence.
3. broadcast each ordered event back to every replica as a turn
   envelope.

the reflector does *not*:

- run user code.
- understand capabilities.
- see cross-vat far-ref traffic (that uses regular transport, not
  the reflector's path).
- inspect input-event payloads beyond the envelope header.

the reflector is roughly 200 lines of rust. it can be a separate
process per session, or a thread inside a participating replica
(self-hosted small sessions), or a server-side daemon (large
shared sessions).

## the turn envelope

every input the reflector broadcasts is a *turn envelope*
(`laws/determinism-laws.md` D2):

```
{ session-id   : 'mooofpaint-alice-bob-2026-04-29
  epoch        : 1
  turn-seq     : 5234
  author       : alice    ; the replica that supplied the input
  logical-now  : 4789     ; ticks since session start
  input-event  : (Stroke #[points: …])
  seed         : 0xab12cd34efghijkl }
```

the envelope is signed by the reflector. replicas verify the signature
before processing. mid-turn, the vat reads `[turn now]`, `[turn seed]`,
etc., from the envelope; *not* from any cap.

## ticks

the reflector emits a tick every N ms (default: 50ms; configurable per
session). a tick is a turn envelope with `input-event = #Tick`. it
advances `logical-now`. ticks let the replicated vat react to the
passage of time without consulting wall-clock.

inputs received within a tick window are batched; multiple input
events between two ticks are issued as separate envelopes (each with
a stable turn-seq) but with the same `logical-now`.

## what's replicated, what isn't

| replicated | not replicated |
|---|---|
| forms, slots, handlers, meta | bytecode (per-replica derived) |
| proto edits (as ProtoEdit envelopes) | inline caches |
| stroke logs, undo stacks, etc. | render output, pixel buffers |
| the input log itself (canonical truth) | per-replica $caps |
| effect intents and receipts (data) | running effects (per-replica) |

ambient capabilities are *per-replica bindings*. when alice's replica
runs `[$canvas paint]`, the cap points to alice's local screen. when
bob's replica runs the same code (because both replicas are processing
the same envelope), bob's cap points to bob's local screen. each
replica draws to its own surface; the *abstract canvas state* in the
heap is identical.

## effects in replicated vats

a replicated vat is a *pure deterministic state machine* over the
input log. when its code calls `[$out say: "hi"]`, the substrate does
not invoke `$out` directly. instead:

1. the vat appends an `EffectIntent(turn-seq, ordinal, payload)` to
   its outbox slot.
2. *one replica* (typically the leader, or a designated "effect
   authority") reads outbox entries and actually invokes the local
   cap.
3. the result is wrapped as `EffectReceipt(turn-seq, ordinal, value)`
   and submitted to the reflector as a new input event.
4. all replicas receive the receipt envelope and process it as
   ordinary deterministic data — the receipt resolves the in-flight
   promise that the original `[$out say:]` produced.

this gives:
- **convergence**: state is determined entirely by the input log,
  including effect-receipts.
- **at-most-once effects**: the effect authority dedups by `(turn-seq,
  ordinal)`. crash-recovered authorities never re-execute.
- **purity-by-construction**: the replicated vat itself is pure; all
  observable side-effects happen *outside* the replicated state machine.

(see `concepts/effect-intents.md` for full mechanics.)

## proto edits as input

live-edits to protos are *also* input envelopes:

```
input-event := {ProtoEdit
  target: <proto-id>
  selector: ':incr
  source: <new-source-form>}
```

every replica receives the edit at the same logical-now. each
recompiles the new method locally; bytecode is derived.

this is how distributed live editing works: alice changes `[Pencil
draw:]` mid-session, bob's replica picks up the change at the same
turn-seq, the *very next* `[pencil draw: ...]` send on every replica
uses the new code.

## what crosses replicas vs what stays local

cross-vat *messaging* (one moof user's vat far-refs into another's)
continues to flow over regular transport, not the reflector. the
reflector is exclusively for the *one* replicated vat's input log.

a single moof world can host many replicated sessions concurrently,
each with its own reflector and replica set. cross-session traffic
is regular far-ref messaging.

## persistence of a replicated vat

each replica persists *the input log* to its own per-vat directory.
the snapshot of state (the heap canonical encoding) is also persisted
locally, but the snapshot is *derived*; the input log is the truth.

on reboot:
1. load latest snapshot.
2. replay input log from snapshot's turn-seq onward.
3. resume.

if the local snapshot is corrupted: replay from genesis (slow but
correct).

if the local input log is behind another replica's: catch up by
asking the reflector for missing envelopes.

if the local heap drifts from the input log's implied state: the
input log wins (`laws/determinism-laws.md` D11).

## late-joining replicas

a new replica joins:
1. asks the reflector for the current epoch's session metadata.
2. fetches a recent snapshot from a participating replica (or genesis
   if available).
3. replays input log from snapshot's turn-seq to current.
4. signals "ready"; reflector starts including it in broadcasts.

snapshot transfer is out-of-band (not via the reflector). the
reflector only handles current-and-future input flow.

## leader, followers, failover

every replicated session has a *leader* — the replica with effect-
authority. effect-receipts come from the leader. if the leader fails:

1. one of the followers is promoted (election protocol; raft-style
   for small replica counts).
2. the new leader announces "epoch+1, i am leader" to the reflector.
3. inflight effect-intents from the old leader are *retried* by the
   new leader, with the same `(turn-seq, ordinal)` ids — so they
   remain idempotent at the cap level.

(this is the raft / zab pattern. moof's reflector is the trust
anchor; the leader is the active replica.)

## what about the world demo specifically?

for a 2–4 user shared-world session
(`concepts/world-and-space.md`, `concepts/pixmap.md`):
- the world-vat is replicated; each user has a local wrapper vat
  (solo, holding the local viewport + caps).
- the reflector is a thread inside one of the participating
  processes (self-hosted).
- alice's world-replica is the leader; bob's is a follower.
- if alice closes her laptop, bob's replica gets promoted to
  leader; the reflector reuses bob's machine.
- the input log is mirrored to both machines.
- if both leave and rejoin later, whoever wakes first is leader.

larger sessions (~20 users) want a dedicated reflector process;
that's deployment, not substrate.

## what's *not* in this model

we are explicitly *not* doing:
- **byzantine fault tolerance.** replicas trust the reflector and
  trust each other. malicious replicas are out-of-scope. (the docs
  promise capabilities are unforgeable; they don't promise replicas
  are honest. that's a future-research territory.)
- **strong consistency across sessions.** different replicated
  sessions are independent. world-wide consistency would require
  multi-master coordination, which we don't attempt.
- **CRDT-style concurrent merging.** every replicated session has a
  single canonical input log. concurrent edits to commutative ops
  could *opt out* of total ordering, but the substrate doesn't
  expose that primitive yet.

## inspirations

- **croquet / teaTime**: kay, reed, smith ~2003. the original
  "deterministic actors over a totally-ordered message stream"
  formulation.
- **raft**: ongaro & ousterhout 2014. log-as-truth, leader-follower,
  epoch-on-failover. moof's reflector borrows this discipline.
- **virtual synchrony / spread / zab**: birman & joseph 1987;
  amir & stanton 2003; junqueira et al. 2011. the lineage of
  totally-ordered group communication.
- **e / dyc**: miller, hardy 1988. promise pipelining, on which
  effect-receipts depend.
- **datomic**: hickey ~2012. the database is a value; history is
  free. moof's input log is the croquet/datomic synthesis.

## see also

- `concepts/vats.md` — what a vat is, replication-mode.
- `concepts/world-and-space.md` — the canonical replicated thing.
- `concepts/effect-intents.md` — intent + receipt mechanics.
- `concepts/transport.md` — wire format and reflector lifecycle.
- `concepts/persistence.md` — input-log + snapshot storage.
- `laws/determinism-laws.md` — what determinism actually requires.
- `laws/isolation-laws.md` — vat boundary rules (replicas are still
  separate vats).
- `concepts/pixmap.md` — one inhabitant proto.
