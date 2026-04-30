# effect intents and receipts

> **a replicated vat does not directly invoke capabilities. it
> appends an `EffectIntent` to its outbox; an external authority
> executes the cap and writes the result back as an `EffectReceipt`
> input envelope. effects become data; data becomes deterministic.**

this is the model that lets `concepts/replication.md` and
`concepts/persistence.md` and `concepts/capabilities.md` all
cohere without lying to each other. it's the synthesis codex and
gemini independently arrived at during the audit
(`docs/process/audit-2026-04-29.md` post-brainstorm synthesis S-2).

## the problem the model solves

before this model:

- `concepts/persistence.md` said "exactly-once for state, at-most-once
  for side-effects." but if state branches on the result of a cap
  call (`(if [$fs write: …] (cleanup) (retry))`), at-most-once for
  the call means the *result* is uncached, so replay forks.
- `concepts/replication.md` requires every replica to make the same
  decisions. but if one replica calls `[$out say: "hi"]` and another
  doesn't (because only one is the "primary"), they diverge.
- `concepts/capabilities.md` says caps are unforgeable far-refs. but
  if the reflector serializes cap-bearing messages, it sees inside
  the cap, which violates unforgeability.

the resolution is to separate **intent** from **execution**.

## the model

inside a replicated vat:

```moof
[$out say: "hi"]
```

does *not* invoke `$out`. it does:

1. allocates an `EffectIntent` Form:
   ```
   {EffectIntent
     turn-seq: <current>
     ordinal: <next outbox ordinal>
     cap: $out
     selector: :say:
     args: ["hi"]}
   ```
2. appends it to the vat's `outbox` slot (a List).
3. allocates a Promise with id `(turn-seq, ordinal)`.
4. returns the Promise.

the calling code can pass the promise around, await it, or ignore
it. the vat continues its turn deterministically — nothing was
side-effected.

## the effect authority

one replica (typically the leader) plays the role of *effect
authority*. its job:

1. read the outbox slot at end-of-turn (this is read-only;
   determinism preserved).
2. for each unprocessed intent:
   - dedup by `(turn-seq, ordinal)` against an authority-side log.
   - invoke the actual cap with the args.
   - wrap result as `EffectReceipt`.
   - submit the receipt to the reflector as a new input event.

receipts arrive back at *every* replica as ordinary turn envelopes:

```
{EffectReceipt
  turn-seq: <of original intent>
  ordinal: <of original intent>
  status: :ok | :error
  value: <return-value-of-cap-or-error>}
```

each replica processes the receipt by:

- looking up the promise with id `(turn-seq, ordinal)`.
- resolving (or breaking) the promise.
- continuing any computation that was awaiting it.

at this point, every replica's heap is identical *including the
result of the effect*. the cap was called once (by the authority),
but the *value* is replicated.

## crash-recovery

the authority maintains a small log: `(turn-seq, ordinal) → status`.
on restart, before processing any new intents, it reads the log:

- intents already executed: skip.
- intents queued but not executed: re-execute.
- intents in-flight at crash: maybe re-execute.

side-effects in the third case may execute zero, one, or two times
(the at-most-once is on the *receipt commit*, not the cap call).
caps that care about this idempotency provide it themselves
(`$fs write:` is naturally idempotent if the bytes are identical;
`$net send-money:` better be idempotent or a wrapper makes it so).

if the authority crashes and a follower is promoted:

- new authority replays the outbox from its own copy of the heap.
- it sees the same `(turn-seq, ordinal)` keys.
- it submits any *not-yet-receipted* intents (the reflector and
  replicas all see the same receipt log, so the new authority can
  tell which are pending).

## what counts as an effect

every cap call is potentially an intent:

| cap | intent shape |
|---|---|
| `$out say:` | `{cap: $out selector: :say: args: [<text>]}` |
| `$fs read-file:` | `{cap: $fs selector: :read-file: args: [<path>]}` |
| `$random bytes:` | (special — see below) |
| `$canvas paint-pixmap:` | per-replica edge adapter; not an intent |

caps that produce *deterministic* results from *replicated input*
need not become intents:

- `$random` in a replicated vat *should not exist*. use the turn
  envelope's `seed` field instead.
- `$logical-clock` doesn't exist as a cap; use turn envelope's
  `logical-now`.

caps that produce per-replica *local* effects (rendering, input
capture) are *edge adapters*. they live in non-replicated wrapper
vats; the replicated vat sends messages to the wrapper, not the
edge directly.

## ordering guarantees

within a turn, intents are emitted in the order the code executes:

```moof
[$out say: "first"]
[$out say: "second"]
[$out say: "third"]
```

emits three intents with increasing ordinals. the authority executes
them in ordinal order. receipts arrive in the same order at every
replica.

across turns, intents from earlier turn-seqs are processed before
intents from later turn-seqs (the reflector orders receipts the same
way it orders other inputs).

## awaiting receipts within the turn that produced them

```moof
(let result [$fs read-file: "config.toml"])
;; result is a Promise here. cannot block-await within the turn.
[result when-resolved: |bytes| (process bytes)]
```

inside the same turn, the receipt has not yet been issued. so the
promise is *pending*; the `when-resolved:` callback fires at a future
turn (when the receipt envelope arrives). this is fine — it's the
async-by-default discipline.

`[promise sync-await: timeout]` is *forbidden* in replicated mode
(would block the turn waiting for an authority that won't run until
turn-end).

## persisting the input log

the input log already includes intents (as part of the turn that
produced them, indirectly via the outbox slot's mutation) and
receipts (as their own envelopes). so the canonical persistence
unit is the input log; effects fall out for free.

on reboot:
1. load snapshot.
2. replay envelopes from snapshot's turn-seq.
3. each receipt envelope resolves a promise; each new turn may
   produce intents.

intents that hadn't been executed at crash time appear in the outbox
again on reboot. the authority re-fires them; receipts arrive; state
converges.

## intents that need not become receipts

some "effects" are pure-from-the-vat's-perspective:

- `[$logger info: "hi"]` if the logger is a per-replica vat that
  doesn't feed back. fire-and-forget.
- `[$telemetry metric: 'fps value: 60.0]` likewise.

these can be marked `:fire-and-forget` in the intent. the authority
executes them but does not produce a receipt. the originating promise
resolves with `nil` immediately at intent-emission.

(this is an optimization. for correctness, fire-and-forget is
equivalent to the receipt being unused.)

## what about reading caps inside a turn (e.g. iterating)?

```moof
(pipe [$fs read-lines: "data.txt"]
  [for-each: |line| (process line)])
```

`[$fs read-lines:]` returns a Promise of a DataSource. the data
source is itself an intent-machine: each `:next` send produces an
intent, gets a receipt, advances. for streaming reads, the
turn-by-turn rate matches the consumer's pull rate.

the user-facing experience: pipe reads come back asynchronously,
one chunk per turn. that's not unlike erlang receive-loops; tools
that want batched reads use `:read-all` (one intent, one big
receipt).

## inspirations

- **e's promise pipelining**: miller, hardy 1988. lets us await
  receipts without blocking.
- **datomic's transactor model**: hickey ~2012. one authority
  serializes writes; reads are everywhere.
- **redux's "actions are data"**: dan abramov ~2015. effects-as-
  data is a frontend pattern that maps perfectly.
- **akka persistence's command/event split**: lightbend. commands
  produce events; events update state; an event journal is the
  source of truth.
- **hot code reload semantics in elixir/erlang**: any in-flight
  cap call can outlive the code that started it; the receipt
  arrives in the new code.
- **the recovery oriented computing project**: berkeley early-2000s.
  re-execution as a first-class strategy.

## see also

- `concepts/replication.md` — replicated vats, the input log.
- `concepts/persistence.md` — how all of this serializes.
- `concepts/capabilities.md` — what caps are; per-replica binding.
- `concepts/references.md` — promises and far-refs.
- `laws/determinism-laws.md` — why effects must be data, not actions.
