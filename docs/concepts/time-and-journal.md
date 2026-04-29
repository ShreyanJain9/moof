# time and journal

> **every vat journals every committed mutation. the journal is a
> data source. time-travel is reading from a position. undo, replay,
> audit, and forensics are all queries over the journal.**

time is a first-class axis in moof. we take this from datomic
(rich hickey ~2012) and apply it per-vat: each vat has its own
timeline.

## the journal

a vat's journal is an append-only log of mutations:

| field | what |
|---|---|
| `seq-id` | monotonic per-vat sequence number |
| `timestamp` | wall-clock, from `$clock` |
| `cause` | message-id that triggered this turn |
| `mutations` | list of `(form-id, slot, old-value, new-value)` |
| `caps-used` | which capabilities the turn invoked |
| `prev-hash` | merkle-chain prev-pointer (for tamper-evidence, optional) |

every committed message-turn appends one entry
(`concepts/persistence.md`). nothing is deleted; entries roll into
periodic checkpoints which compact past entries into a snapshot.

## the journal is a data source

```moof
(let history [(vat 'shreyan) journal])
(pipe history
  [filter: |entry| [entry timestamp > yesterday]]
  [map: 'cause]
  [for-each: print-event])
```

journal entries stream lazily. queries don't materialize history;
they pull as needed.

## time-travel

```moof
(let yesterday-vat [vat as-of: yesterday])
;; → a read-only view of the vat as it was yesterday
[yesterday-vat root-counter count]
;; → the count value at that point in time
```

`as-of` produces a vat-shaped value backed by the snapshot before
`yesterday` plus journal entries replayed up to that point. it's
read-only (reading consistent past state). 

multiple `as-of` views can coexist; they don't disturb the live vat.

## undo / redo

undo is "rewind one journal entry." the substrate exposes this as a
vat-level operation:

```moof
[vat undo]                       ; undo the last committed turn
[vat redo]                       ; reapply if undone
[vat rewind: 30s]                ; undo turns back 30s
[vat rewind-to: marker]          ; back to a named checkpoint
```

undo doesn't delete history; it adds an inverse-application entry to
the journal. redo replays the originally-undone entries. the
timeline is acyclic but allows revisiting past states.

within reason: undo of cap-side-effects (a file write, a network
call) is *not* automatic. cap-effects are at-most-once
(`concepts/persistence.md`); state effects are exactly-once.

## replay

```moof
(let new-vat [vat clone-and-replay-from: snapshot to: marker])
```

replay produces a fresh vat by:
1. starting from a snapshot.
2. replaying journal entries up to the marker.

useful for forensics ("what would have happened if?"), debugging
(replay with logging tee'd in), and creating divergent forks.

## divergent timelines

a vat at point T can be forked into two timelines, both valid:

```moof
(let alt [vat fork-as-of: now])
(pipe (vat → input)
  [for-each: |evt| [vat receive: evt]])      ; original timeline
(pipe (alt → input)
  [for-each: |evt| [alt receive: evt]])      ; alt timeline
```

both vats share history before the fork point. each has independent
history after. journals diverge.

## audit, forensics, observability

every turn has a `cause` linking back to the message that triggered
it. messages have causes (the sender). this builds a *causal graph*
across the world's vats. queries over this graph answer:

- "what caused this state change?"
- "which vat sent this message?"
- "what did this user do in the last hour?"
- "which actor is making this slot mutate so often?"

(the inspector / debugger / profiler are all queries over the
causal graph.)

## privacy / redaction

journal entries can be flagged `:redacted` to prevent serialization
of their contents (still keep the seq-id and timestamp; not the
mutation body). useful for sensitive data. user code controls this
via meta-annotations on the relevant Forms.

## the cost

journals grow. compaction trims them. for high-mutation workloads,
journal volume is real. for typical user-environment workloads
(editing notes, browsing objects, running queries), journal volume
is negligible — most messages are small, most turns mutate few
slots.

we accept the cost because the value is enormous: time-travel is the
single feature that distinguishes a moldable environment from a
disposable one.

## inspirations

- datomic: rich hickey. "the database is a value; history is free."
- WAL + MVCC: postgres / lmdb.
- erlang's process tracing: per-process history with cause chains.
- the smalltalk image's inherent persistence (modulo lack of
  fine-grained history): smalltalk-80.
- git: every commit has a parent; history is a DAG.

## see also

- `concepts/persistence.md` — journal as substrate.
- `concepts/data-sources.md` — journal as DS.
- `concepts/queries.md` — querying the journal.
- `concepts/vats.md` — per-vat journal scope.
