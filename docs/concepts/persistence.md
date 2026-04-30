# persistence

> **per-vat database storage. each vat has its own directory: a
> mmap'd transactional store + a write-ahead-log journal. saves
> happen continuously, per-message-turn, atomically. boot is mmap +
> journal-tail-replay.**

we draw from smalltalk images, erlang's ETS/DETS/Mnesia, and
modern transactional kv-stores (LMDB). we explicitly do *not* use
`.moof` source-text for runtime state (clunky), and we explicitly do
*not* use a global merkle blobstore (v3 mistake). per-vat is the
correct unit because that's also the isolation unit, the federation
unit, and the supervision unit.

## per-vat on-disk layout

```
.moof/vats/<vat-id>/
  meta.toml              small: id, name, mode, supervisor, caps, version
  store.lmdb             B-tree, mmap'd: heap-id → canonical-bytes (snapshot)
  journal.log            WAL: append-only, slot-mutations since last snapshot
  inputs.log             append-only input envelope log (replicated vats)
                         OR ordinary inbox messages (solo vats).
  effects.log            append-only effect-intent + receipt log
  refs/                  named root pointers (root form, inbox cursor, etc.)
```

three logs, three jobs:

- **`journal.log`** is *derived data*: the slot-mutations that
  applying a turn produced. used by snapshot compaction; not
  authoritative for replay.
- **`inputs.log`** is *the truth*: the totally-ordered envelope
  stream. for replicated vats, this is the canonical state. for
  solo vats, ordinary inbox messages serialize here.
- **`effects.log`** records intents (with results, when receipted)
  to make the effect-authority idempotent across restart.

on reboot, the loader reads the latest snapshot from `store.lmdb`,
then replays `inputs.log` from snapshot's turn-seq onward. effects
that were already receipted are observed as receipt envelopes;
intents that were not yet receipted are re-fired by the authority.

(see `concepts/effect-intents.md` for the full intent/receipt
mechanics.)

- **store.lmdb**: keys are heap-ids (within this vat). values are
  canonical-bytes encoding the Form at that id. mmap'd, so boot is
  ~"open file, read root entries" — sub-100ms even for huge vats.
  thanks to LMDB's B-tree semantics, we get ACID per write txn for
  free.
- **journal.log**: an append-only file recording mutations between
  checkpoints. *exposed to user code as a data source.*
- **refs/**: small files holding canonical-bytes for the world-root
  references — the topmost vat-form, the inbox, the supervisor pointer.
- **meta.toml**: human-readable metadata. keep it small and
  human-edit-friendly for diagnostic recovery.

## the canonical encoding

a canonical encoding turns a Form into bytes deterministically. same
form ⇒ same bytes. structurally-equal forms produce identical bytes.
this enables:

- content-addressing within a vat (if we want to dedup proto forms).
- stable hashing of Forms (used for Hashable-as-key).
- safe round-trip: parse back into an isomorphic Form.

(format details in `reference/canonical-encoding.md` when written.)

## commit cadence — per message-turn

every message-turn in a vat is one transaction:

1. dequeue one message from `inbox`.
2. dispatch the message. mutations buffer in memory. cap calls
   accumulate as `EffectIntent` entries in the outbox slot
   (`concepts/effect-intents.md`); they do *not* fire mid-turn.
3. at turn-end:
   - serialize buffered mutations into a journal entry.
   - serialize the input envelope (or message) that drove this
     turn into the *input log*.
   - fsync.
   - mark the message as "processed" (advance `inbox` cursor).
4. the **effect authority** (a non-replicated worker; for solo vats,
   typically the same vat's runtime) reads the new outbox entries
   and executes them at-most-once. each result becomes an
   `EffectReceipt` envelope on the inbox of the originating vat.
5. yield to the scheduler.

state is **exactly-once** — replay reconstructs the same heap because
the input log + intents + receipts are all data. cap effects are
**at-most-once** — the authority dedups by `(turn-seq, ordinal)`. a
replay never re-fires effects; it observes their journaled receipts.

(this is the croquet/datomic/akka-persistence synthesis. the input
log is the canonical truth; the heap snapshot is derived.)

## checkpointing / compaction

journals grow. periodically, the substrate (or a user invocation):

1. snapshots current state from the heap into the store.lmdb.
2. truncates the journal to entries newer than the snapshot.
3. updates `refs/` to point at the new root.

checkpoints can run concurrently with normal vat operation (using
LMDB's MVCC: a long-running read transaction sees consistent state
while writes proceed). vats don't pause.

frequency is policy: every N turns, every X seconds, on idle. each
vat can configure its own.

## boot

booting a vat:

1. open meta.toml; verify version, recover supervisor pointer +
   replication-mode.
2. mmap store.lmdb. resolve the root form-id from `refs/root`.
3. tail-replay any input envelopes newer than the snapshot
   (`inputs.log`).
4. for replicated vats: re-handshake with the reflector; catch up
   any envelopes received after the local replica went offline.
5. effect-authority: re-fire intents not yet receipted.
6. signal "ready" to the supervisor.

elapsed: typically tens of milliseconds for a vat with thousands of
forms; LMDB's mmap + lazy-page-fault model means we don't actually
load everything upfront — pages are pulled in on access. cold-start
time is dominated by os file-open and mmap setup.

## persistence of references

within-vat: form-ids are local to the vat. on save, all in-vat refs
serialize as just their form-ids. on load, ids retain their
identity.

across-vat: far-refs are `(vat-id, form-id, cap-token)` triples.
these serialize as their three fields and remain valid across
saves/loads. the target vat doesn't need to be alive at our load
time; far-refs are dormant until messaged.

paths: path-refs are strings; serializing them is trivial.
resolution happens at message time, by querying the world's
path-table (itself a vat).

## persistence of caps

a capability held by a vat is itself a Form (a far-ref + maybe
attenuation metadata). persists like any other Form. on boot, caps
re-resolve when first used.

primordial caps (`$clock`, `$random`, `$out`, etc.) are *not*
persisted as values — they're re-bound at boot from the substrate.
the vat's meta.toml declares which caps it expects; the supervisor
hands them in.

## what we deliberately do not do

- **no `.moof` source-text snapshot.** clunky, brittle for state
  that's hard to express as code.
- **no global merkle blobstore.** v3 mistake — invariants leaked
  everywhere.
- **no head/content distinction at the substrate level.** mutability
  is just mutability inside a vat. the journal captures changes; the
  store reflects committed state.
- **no shared persistence across vats by default.** each vat saves
  separately. cross-vat consistency is an application-level concern,
  built atop messages.

## what makes this database-flavored

| database concept | moof equivalent |
|---|---|
| transaction | one message-turn |
| WAL | journal.log |
| ACID | per-turn ACID, single-writer, single-vat |
| MVCC | journals + LMDB read-snapshots |
| schema | proto definitions |
| index | derived Tables (datalog rule outputs) |
| query | datalog rules + queries |
| backup | `cp -r .moof/vats/<id>` (the directory IS the backup) |
| replication | (later) cross-vat far-refs to mirror vats |

## inspirations

- per-vat directory granularity: e (mark miller) and erlang (per-process state).
- mmap'd transactional store: LMDB (howard chu).
- WAL + checkpoint + compact: postgres / sqlite.
- snapshot-as-image: smalltalk-80 (kay et al.).
- ETS/DETS/Mnesia table model: erlang/OTP.
- "per-message-turn ACID": e and croquet.
- time-as-axis (history is free): datomic (rich hickey).

## see also

- `concepts/vats.md` — what a vat is.
- `concepts/data-sources.md` — journal as DS.
- `concepts/time-and-journal.md` — time-travel, undo, replay.
- `concepts/references.md` — how cross-vat refs persist.
- `concepts/effect-intents.md` — intent/receipt model.
- `concepts/replication.md` — replicated-vat persistence (input log).
- `laws/determinism-laws.md` — determinism is what makes replay sound.
- `reference/canonical-encoding.md` — binary encoding (when written).
