# vats and the references protocol — design

> **status: brainstormed 2026-05-04. ready for writing-plans.**
> **scope: load-bearing semantics for multi-vat moof. integrates the v2 doc corpus
> (`concepts/vats.md`, `concepts/references.md`, `concepts/persistence.md`,
> `concepts/forms.md`, `concepts/effect-intents.md`, `concepts/replication.md`,
> `concepts/capabilities.md`) with new decisions on freezing, the FormId scope-tag
> scheme, the per-turn nursery, and env-chain unification with the path-table.**

## 1. scope and the picture of a vat's life

This spec defines: per-vat heap isolation, the freezing model, cross-vat sharing
via shared immutable segments, the FormId scope-tag scheme, per-turn nursery +
diff state semantics, the four reference kinds as Forms, environment-chain
unification with the path-table, and the supporting picture (vat-as-Form, message
envelope, promise protocol, mailbox-as-DataSource, capabilities, supervision,
replication tie-in, effect-intents, boot/quit cycle, GC).

Non-goals (real deferrals, see §23): host-language migration, image binary
format byte-level encoding, GUI integration, federation transport wire formats,
MCP integration, scheduler tuning beyond round-robin.

A vat's life from the inside, in one paragraph:

> it boots from its on-disk directory (mmap'd lmdb store + journal-tail-replay),
> reconnects far-refs lazily, signals ready to its supervisor, then enters the
> receive-loop. each turn: dequeue one envelope; dispatch into moof code;
> allocations and mutations buffer in a per-turn **nursery**; frozen forms can
> promote to the process-wide **shared segment** on first cross-vat send;
> cross-vat sends queue cap-token-bearing far-ref envelopes; the turn commits —
> nursery merges into vat-local heap, per-slot **diff** serializes to the
> journal, outbox effect-intents fire under the cap authority, GC may run on
> the just-vacated nursery — and yields. on crash, the supervisor decides; on
> shutdown, the supervisor broadcasts `:prepare-shutdown`. the vat's heap,
> mailbox, cap-bag, supervisor pointer, and `$here` (the persistent root env /
> path-table segment) are all Forms; reflection works on them; nothing is
> hidden.

## 2. architecture overview

Three things compose:

**Vats.** Each vat owns a private heap (`Vec<Form>` indexed by vat-local
FormIds), a mailbox (a DataSource), an env tree with `$here` at the top, a
cap-bag, a supervisor pointer, and a behavior closure. Nothing crosses a vat's
membrane except through the references protocol.

**The shared segment.** A process-wide arena of frozen forms that have been
promoted via cross-vat send. Content-addressed by canonical-bytes hash.
Refcounted at process scope. Vats hold references to shared-segment forms;
reads are direct (no membrane translation, since the form is immutable and
addressable from any vat in this process).

**The path-table-vat.** The world's named-address registry. Each vat's `$here`
is a Form serving as a per-vat segment of the path-table; the path-table-vat
(a system vat at `/system/paths`) federates segments into a global namespace.
Within a vat, env-walk reaches `$here` synchronously. Across vats, path
resolution is an explicit async send (`[#Path "/users/alice/foo" resolve]`
returns a promise of an id-ref or far-ref).

The world is the running collection: a root supervisor vat, the path-table-vat,
and any number of user vats. Boot reconstitutes the world from `.moof/`; quit
broadcasts `:prepare-shutdown` and commits each vat's state.

## 3. the four faces, revisited per-vat

`docs/concepts/forms.md` defines four faces: structure, identity, liveness,
history. This spec layers vat-isolation on top:

- **structure** (`head`/`args`, exposed via slots) — vat-local always.
  structure-faced forms are the most common candidates for freezing and for
  promotion to the shared segment (parsed code-trees especially).
- **identity** (`proto`, `slots`, `handlers`) — vat-local for instances. proto
  chains stay vat-local; protos rarely cross vat boundaries (when they do, see
  §7's deep-copy + receiver-side rebinding).
- **liveness** (`mailbox`, `behavior`, `supervisor`) — present *only* on
  vat-Forms. a vat-Form is the only Form that is alive in this sense;
  everything else is data, even if mutable.
- **history** (`meta`) — vat-local. provenance, source-loc, journal-id;
  participates in freezing; not auto-shared.

The substrate-laws hold per-vat: L1 (everything is a Form), L2 (proto chains
acyclic), L11 (FormIds stable for vat lifetime). New: **L1′** — every vat's
heap is logically distinct, and FormId scope-tags make this enforceable, not
aspirational.

## 4. the freezing model

Forms are born mutable in mutable-by-default vats (stateful actors, UI,
workspaces) and born frozen in frozen-by-default vats (parsers, compilers, AST
passes, computation kernels). Vat mode is a spawn-time parameter:

```moof
[$vat spawn: |...| ... mode: #mutable-by-default]
[$vat spawn: |...| ... mode: #frozen-by-default]
```

The substrate primitive is **shallow `freeze`**, applied to one form at a
time, locking that form's slots/handlers/meta tables atomically. There is no
`:thaw`. The transition itself is a turn-mutation: it journals like any other
mutation.

Mutation attempts on frozen forms raise immediately at the call site (not
deferred to turn-end), with kind `'frozen-form` and the offending form-id in
`data`.

Deep-freezing (transitively walking a form and freezing its reachable
subgraph) is a moof-side helper, not a substrate primitive. Policy decisions —
what counts as a live boundary (vats, mailboxes, mutable-by-design forms),
what to do on cycles, whether to barrier-stop or raise — live in moof code
where they're inspectable and modifiable.

Freezing is the gate to cross-vat sharing: only frozen forms are eligible for
promotion to the shared segment (§5). Mutable forms cross vat boundaries only
as far-refs.

**Dispatch on a frozen form continues to walk its (live, mutable) proto
chain.** Adding a handler to `Point` still affects every frozen Point instance
— moldability survives. Freezing locks state, not behavior.

The substrate refuses to freeze certain forms: vat-Forms, mailbox-Forms,
DataSource handles, cap-tokens (their authority is mutable-by-design). Attempts
raise `'cannot-freeze-live` rather than silently succeeding.

## 5. the FormId scheme

A FormId is 32 bits, top-tagged for scope:

| top bits | scope | semantics |
|---|---|---|
| `00…` | **vat-local** | index into this vat's `Vec<Form>` (~1B max) |
| `01…` | **shared-segment** | index into the process-wide shared arena (~1B max) |
| `10…` | **far-ref entry** | index into this vat's far-ref table; entry holds (vat-id, target-form-id, cap-token) |
| `11…` | reserved | future: NaN-boxed immediates, bigint pool, segmented heaps |

Every `heap.get(id)` becomes a 1-instruction tag-dispatch + table lookup. The
hot path stays O(1) for all three live scopes.

This is the smalltalk-80 OT-tagging move adapted: smalltalk used the bottom
bit for SmallInteger vs OT-pointer. Moof has Value-level tagging for
immediates already (`Value::Int`, `Value::Bool`, etc), so the FormId tag-bits
are free for *scope* instead of immediate-vs-heap. The "reserved" range is the
room for future immediate kinds inside FormId space if NaN-boxing happens
later.

**Promotion to shared segment** (lazy, b+d):

- a frozen form lives vat-local at first.
- on first cross-vat send, the substrate hashes its canonical bytes (blake3 —
  already in the substrate via the embedded Hash mco).
- the per-process intern table is consulted by hash:
  - hit → reuse existing shared-segment FormId.
  - miss → allocate in shared segment, install in intern table.
- the source vat's local id gets a forwarding pointer; subsequent local reads
  transparently resolve to the shared id; the local form becomes
  GC-collectable.
- the shared form caches its hash in meta, so subsequent cross-vat sends are
  O(1) — no re-hash.

**Membrane invariant.** A vat-local FormId never crosses a vat boundary. The
substrate enforces this at message-serialization time: any vat-local id
encountered is auto-promoted (frozen → shared-segment id; mutable →
newly-minted far-ref entry on the receiver side). The user verb is "send a
value"; the substrate handles the rewrite.

## 6. the state model

Within-vat mutation is buffered in a **per-turn nursery**. The nursery is:

- a separate small heap allocated at turn-start.
- the destination for all this-turn allocations (with a temporary
  nursery-scoped FormId; not user-visible).
- the destination for all this-turn mutations to forms in vat-local heap
  (writes go to a nursery shadow entry keyed by `(form-id, slot/handler/meta-key)`).

Reads check nursery first, fall through to vat-local heap. Read-your-writes is
preserved within a turn.

At turn-end:

1. Compute the **per-slot diff** — for every `(form-id, slot/handler/meta-key)`
   touched, emit `(form-id, key-kind, key, prior-value, new-value)`.
2. Append the diff + the input envelope that drove this turn to `inputs.log`.
   fsync.
3. Apply the nursery into vat-local heap: new forms get permanent vat-local
   FormIds, mutations land on the canonical forms.
4. Send queued cross-vat envelopes (their target far-refs were captured during
   the turn).
5. Effect-authority reads outbox effect-intents and fires them at-most-once
   (§20).
6. Drop the nursery. Substrate may opportunistically GC at this point (§18).

On crash mid-turn, the nursery is dropped; canonical heap is unchanged. On
reboot, replay reads `inputs.log` and re-derives the heap from the snapshot +
replayed diffs.

The diff is *also* the replication primitive (§19) and the CRDT op stream. A
slot tagged with a `Mergeable` proto gets its diff fed through the proto's
merge function on incoming envelopes. **The substrate is CRDT-shaped without
knowing what a CRDT is.**

## 7. the reference protocol

Four reference kinds, each a Form whose proto is `Reference`:

| kind | Form slots | scope | semantics |
|---|---|---|---|
| **slot-ref** | `:form` (id-ref), `:slot` (sym) | one vat | sync read/write; mutable cell with `:observe:` |
| **id-ref** | `:id` (FormId) | one vat | sync identity, normal sends |
| **far-ref** | `:vat-id`, `:form-id`, `:cap-token` | crosses vats | async send only → promise |
| **path-ref** | `:path` (string) | the world | name lookup → resolves to id-ref or far-ref |

A FormId itself is the in-memory representation of an id-ref. Wrapping it in a
Reference Form is for explicit reflection and for slot-refs. Ordinary message
dispatch uses raw FormIds; `Reference` Forms exist so users can pass references
as first-class values.

**Membrane translation rules** (enforced by the substrate at envelope
serialization):

- a vat-local id-ref pointing to a *frozen* form → promote target to shared
  segment, rewrite as shared-segment id-ref.
- a vat-local id-ref pointing to a *mutable* form → rewrite as a far-ref to
  `(sender's vat-id, sender's form-id, freshly-minted cap-token)`.
- a shared-segment id-ref → unchanged.
- a far-ref → unchanged (already cross-vat-shaped).
- a slot-ref → only meaningful within one vat. crossing a membrane raises
  `'slot-ref-cannot-cross` unless the form is frozen, in which case the
  slot-ref serializes as a `(shared-id, slot-name)` read-only pair.
- a path-ref → unchanged (string is value-data).

**Capability tokens.** Every far-ref carries a cap-token. The substrate
verifies on every send. Cap-tokens are unforgeable Forms whose only
constructors are (a) the root supervisor at boot and (b) attenuation of an
existing cap. There is no rust escape-hatch.

**Persistence of references.** Within-vat: form-ids are stable across
snapshot/replay (L11). Cross-vat: far-refs persist as triples; resolution
happens at message time, not at load time. Path-refs persist as strings;
resolved by querying the path-table-vat.

The shared segment itself is **not directly persistent** — it's a runtime
in-memory dedup cache. Each vat that needs a frozen form holds it in vat-local
heap; promotion to the shared segment is a per-process optimization that
happens lazily on first cross-vat send. On vat-save, vat-local forms (whether
frozen or not, whether forwarded-to-segment or not) journal as canonical
bytes; on reload they reconstitute in vat-local heap and re-promote on
demand. Different processes loading the same data converge on equivalent
in-memory shared forms because the intern key is the canonical-bytes hash.

## 8. the environment model

An env is a Form (proto: `Env`). Slots are bindings. `meta.parent` is the next
env up. The walker is unchanged from today — it just walks Forms-as-envs.

A vat's env chain bottoms out at **`$here`**: a Form that IS this vat's segment
of the world path-table. So a typical chain:

```
[innermost lexical frame]   ← let, fn args
   ↓ parent
[outer lexical frame]
   ↓ parent
[method-defining frame]     ← captured at closure creation
   ↓ parent
[$here]                     ← persistent, journaled, top of chain
```

Resolution walks until hit. Lexical hits resolve fast. Falling through to
`$here` is effectively a path-bound lookup — but the call site doesn't know
or care. **One resolver, two behaviors.**

`def` always binds at `$here` regardless of lexical position (matches today's
intent: "def is for top-level"). `let`/`do` bind in lexical frames. `def`
becomes a moof macro that expands to `[$here bind: name to: value]` —
substrate doesn't special-case it.

**Closure travel.** A closure-Form is typically frozen-after-creation: its
`:body`, `:params`, `:env` slots are set once. Travelling to another vat:

- the closure-Form itself, being frozen, deep-copies (or content-address-
  promotes to shared segment) cleanly.
- the lexical env-chain it captures: each frame, if frozen, ditto; if mutable,
  becomes a far-ref on the receiving side.
- the topmost `$here` is **rebound to the receiving vat's `$here`** at
  unmarshalling time. so global names resolve against the receiving vat's
  bindings; lexical captures still work.

This is exactly Erlang's "globals are node-local" — moof gets it for free out
of the env-chain unification.

`$here` is a Form; reflection works on it. `[$here slots]` lists path-bound
names. `[$here meta]` carries journal-ids and provenance for each binding.

## 9. the vat-as-Form structure

A vat is a Form. Its slots:

| slot | type | meaning |
|---|---|---|
| `:id` | string (UUIDv7) | vat identity |
| `:mode` | symbol | `#mutable-by-default` or `#frozen-by-default` |
| `:here` | Form | `$here`, the path-table segment / persistent root env |
| `:mailbox` | DataSource | inbox |
| `:outbox` | DataSource | outbound messages + effect intents |
| `:behavior` | closure | message handler |
| `:supervisor` | far-ref | parent vat in the supervision tree (`nil` for root) |
| `:caps` | Form | cap-bag (slot-bound caps: `$clock`, `$out`, …) |
| `:journal` | DataSource | per-vat WAL, exposed as DS for observation |
| `:replication-mode` | symbol | `#solo`, `#replicated-leader`, `#replicated-follower` |
| `:session` | Form? | replicated-only: session-id, epoch, role |

Vat-Forms are **never frozen** (their live face is mutable by definition;
`freeze` raises `'cannot-freeze-live`). They participate in dispatch like any
other Form: `[vat send: msg]`, `[vat mailbox]`, `[vat supervisor]`. Reflection
reveals the slots above; nothing is hidden.

## 10. the message envelope

A Message is a Form (proto: `Message`):

- `:target` — id-ref or far-ref to the receiver
- `:selector` — symbol
- `:args` — list of values
- `:cap-token` — token authorizing the send (verified before delivery)
- `:reply-to` — promise to resolve with the result (`nil` for tell-style sends)
- `:turn-seq` — sender's turn number at send-time
- `:ordinal` — ordinal within sender's turn

Membrane-translation rules (§7) apply to `:target` and any references in
`:args`. Args that are themselves frozen-and-promotable get auto-promoted to
shared segment on first send.

The envelope is allocated in the sender's nursery, journaled at commit, and
dispatched (locally enqueued or wire-sent) post-commit. On crash mid-turn, the
envelope vanishes with the nursery — exactly-once for messaging within the
vat's turn boundary.

`:turn-seq` and `:ordinal` are populated for every send (cheap), but only
load-bear for replicated vats. Solo vats use them for debugging and replay.

## 11. the promise protocol

A Promise is a Form (proto: `Promise`):

- `:state` — `#pending`, `#ready`, or `#broken`
- `:value` — the resolved value (when `#ready`)
- `:reason` — the broken reason (when `#broken`)
- `:waiters` — list of `(callback-fn, vat-id)` pairs

Operations:

```moof
[promise when-resolved: |v| ...]   ; subscribe; runs in subscriber's vat
[promise when-broken: |err| ...]
[promise then: f]                  ; map: value → new-promise (pipelining)
[promise sync-await: 5s]           ; emergency-only; blocks the turn up to timeout
```

Resolution happens between turns, by the scheduler: when the target vat's
reply arrives, the promise transitions to `#ready` (or `#broken`), and waiters
are scheduled — each fires as a `:call` message in its subscriber's vat. The
slot-mutation that resolves a promise journals like any other, so resolution
is replayable.

**Pipelining** (E/Miller 2006): sending to a `#pending` promise doesn't block.
The substrate transforms `[promise foo: x]` on a pending promise into a freshly
allocated promise + a queued message that fires on resolution.

## 12. the mailbox-as-DataSource

A vat's mailbox is an instance of the universal DataSource primitive
(`docs/concepts/data-sources.md`). Unification, not metaphor:

- `[vat mailbox]` returns the inbox DataSource.
- `[mailbox tee: logger]` logs every incoming message without modifying the vat.
- `[mailbox filter: pred]` (test-time) drops messages matching pred.
- a debugger wraps the inbox in a single-step DS for stop-and-resume.
- replay is `[journal for-each: |env| [vat receive: env]]` — replay is "feed
  the inputs.log back through the inbox."

The substrate primitive is "give me a DS that reads from this vat's incoming
queue." Everything observability/debug/replay falls out of DS composition.

## 13. the capability protocol

Caps are Forms (proto: `Capability`). A cap carries:

- `:authority` — the action it grants (an opaque symbol or structured Form)
- `:target` — the resource (a far-ref or path)
- `:attenuation` — constraints (read-only, time-bounded, count-bounded, …)
- `:signature` — the unforgeable token (an opaque substrate-issued Form)

Constructors (these are the *only* ones):

1. **The root supervisor at boot** mints primordial caps from
   `manifest.toml`'s `caps` list (`$clock`, `$random`, `$out`, `$err`, `$fs`,
   `$keyboard`, `$screen`, `$net`).
2. **Attenuation** of an existing cap: `[cap readonly]`, `[cap timeBounded:
   60s]`, `[cap countBounded: 100]`. Attenuated caps are derived; the
   substrate re-signs them with the appropriate restriction encoded.

There is no rust escape-hatch. The substrate's cap module is the only place
cap signatures get minted; the verification path is the dispatch-on-far-ref
code.

Persistence: caps serialize naturally as Forms, including their unforgeable
signature. On boot, primordial caps are re-bound from the substrate (not
loaded from disk — they're regenerated, with the same signature, by the
substrate). Attenuated caps re-derive from their parent on first use.

## 14. supervision and let-it-crash

Every vat has a `:supervisor` far-ref. The supervisor decides:

- **spawning** — only the supervisor mints child vats. `[$vat spawn: ...]` is
  sugar for `[supervisor request-spawn: ...]`.
- **crash recovery** — when a vat's turn raises out, the substrate halts the
  vat (rollback the turn), notifies supervisor, and waits.
- **shutdown** — supervisor broadcasts `:prepare-shutdown` to children; each
  completes its current turn, commits, signals ready.

Restart strategies (per-vat, configured at spawn):

- `#restart-from-snapshot` — load last lmdb snapshot, replay journal up to
  (but not including) the failing turn. recover gracefully.
- `#restart-fresh` — discard state, start from genesis behavior.
- `#never-restart` — death is permanent; supervisor decides what to do with
  the message that killed the child (drop, deadletter, escalate).

The root supervisor's supervisor field is `nil`; it manages itself, escalates
fatal errors to the world's quit cycle.

**Let-it-crash discipline** (Armstrong 2003): use try/catch only for
*anticipated* errors (parse failures, missing files). For *unanticipated*
errors, let the turn rollback, let the supervisor decide. Most code should not
have try/catch at all.

## 15. the life of a turn

What happens between two yields:

1. **dequeue.** Scheduler hands the vat one envelope from `mailbox`. The
   envelope is now in flight.
2. **dispatch.** Substrate looks up vat's behavior; calls it with the message.
   Method dispatch begins; bytecode executes.
3. **env-walk.** Names resolve via the env chain; lexical hits return values;
   falling through to `$here` returns path-bound references.
4. **nursery alloc/mutation.** Every `Form::new` lands in the nursery. Every
   slot/handler/meta mutation writes a nursery shadow entry. Reads check
   nursery first.
5. **freeze events.** `[form freeze]` calls flip the freeze bit on the
   nursery's shadow (or, if the form is older than the nursery, journal a
   freeze-transition).
6. **cross-vat sends.** `[far-ref selector: arg]` allocates an envelope in the
   nursery and queues it on the outbox. Membrane translation runs at
   queue-time: vat-local frozen ids → shared-segment promotion + intern;
   mutable ids → new far-refs.
7. **effect-intents.** Cap calls (`[$clock now]`, `[$out write: text]`)
   accumulate as EffectIntent entries on the outbox slot. They do not fire
   mid-turn.
8. **commit.** Behavior returns. Substrate computes the per-slot diff,
   appends to inputs.log + journal, applies nursery into canonical heap,
   fsyncs.
9. **outbox flush.** Queued cross-vat envelopes are dispatched.
   Effect-intents fire under the cap authority at-most-once; each result
   becomes an EffectReceipt envelope on the originating vat's inbox.
10. **GC.** The nursery is dropped; substrate may opportunistically run a
    per-vat scan if pressure warrants.
11. **yield.** Scheduler picks the next runnable vat.

Crashes anywhere in 1–7 roll back the turn (drop nursery, do not journal, do
not flush outbox). Crashes in 8 are the substrate's job to recover via fsync
ordering. Crashes in 9 are observed as "intent fired, receipt missing"; the
cap authority dedupes on retry.

## 16. the life of a form

1. **alloc.** `Form::new` in nursery. Nursery-scoped FormId.
2. **graduation.** At turn-commit, nursery merges into vat-local heap; form
   gets a permanent vat-local FormId.
3. **freeze (optional).** `[form freeze]` flips the freeze bit. After this,
   mutations raise `'frozen-form`.
4. **promotion to shared segment (optional, only for frozen forms).** On
   first cross-vat send: hash canonical bytes, intern-table lookup, allocate
   in segment or reuse. Vat-local form gets a forwarding pointer.
5. **content-addressed dedup.** Multiple vats sending the same parse tree
   converge on one shared form; intern table guarantees identity-by-hash.
6. **GC.** Vat-local forms collected by per-vat GC at turn boundaries.
   Shared-segment forms refcounted at process scope; collected when refcount
   hits zero.

A form's life is monotonic: mutable → (optionally frozen) → (optionally
promoted) → collected. No regressions, no thaw, no demotion.

## 17. the life of a name

1. **bind.** `(def foo 42)` expands to `[$here bind: 'foo to: 42]`. Substrate
   journals a slot-mutation on `$here`.
2. **resolve.** `foo` walks env chain, hits `$here.foo`, returns 42.
3. **persist.** Slot-mutation journals; on next snapshot, `$here`'s state is
   in lmdb. Across reboots, `foo` resolves the same way.
4. **federate.** Other vats reach this binding via `[#Path
   "/<this-vat-path>/foo" resolve]`, returning an id-ref or far-ref.
5. **observe / unbind / replace.** `[(slot $here 'foo) observe: callback]`,
   `[$here unbind: 'foo]`, `[$here bind: 'foo to: newValue]` — all journaled
   slot-mutations.

**Names are paths; paths resolve to references; references are forms.** There
is no naming primitive that isn't a path-bind under the hood.

## 18. GC

**Per-vat GC**, at turn boundaries. The vat is quiescent at end-of-turn (stack
unwound, only persistent roots reachable: `$here`, mailbox, behavior closure,
supervisor, caps).

**Roots:**

- the vat-Form's slots (each entry a root).
- `$here` and its transitive slot-graph.
- pending Promises whose waiters have been scheduled.
- in-flight message envelopes on outbox.

**Tri-color mark-sweep within the vat-local heap.** Shared-segment FormIds are
leaves (don't traverse into the segment from per-vat GC). Far-ref entries are
scanned (the cap-token / vat-id / form-id triple is value-data).

**Nursery as young generation.** The nursery itself is a generational young
space; turn-commit promotes-or-collects. A form allocated and made
unreachable within one turn is collected at turn-end without ever entering
vat-local heap.

**Shared segment GC.** Process-scope refcounting. Each cross-vat reference is
+1; each forwarding pointer drop or vat-GC pass that sweeps a holder is -1.
Refcount-0 forms are reclaimed. Cycles within the segment are impossible
(frozen forms can only point to other frozen forms or to immediates;
references-out-of-segment must be far-refs which are handle-table entries,
not direct).

**Write barriers.** Only on mutable forms. A slot-mutation in the nursery
records the new value; no extra barrier needed — per-vat GC scans the nursery
during commit-time live-set computation.

**Pause time.** Bounded by per-vat heap size, which is itself bounded by user
vat-sizing discipline (vats scoped narrowly per `concepts/vats.md` granularity
guidance). Different vats GC independently; **no stop-the-world**.
Erlang-flavored.

## 19. replication and CRDT integration

**Replicated vats** (`#replicated-leader` / `#replicated-follower`) consume a
shared input log (`inputs.log` is replicated via the reflector / consensus
layer; details in `concepts/replication.md`). Determinism over the input log
is the convergence guarantee.

The **per-slot diff** (§6) IS the replication primitive:

- on a leader, the diff serializes to `inputs.log` and ships to followers.
- on a follower, the diff arrives, applied at the corresponding turn boundary.
- if a slot is annotated `meta.mergeable: <CRDTProto>`, the diff feeds the
  proto's `:merge:` handler instead of a direct apply. CRDT merge produces a
  converged value; that value lands in the canonical heap.

**Per-slot CRDTs** (the v2 vision's mergeable-protocol):

- `meta.mergeable: GCounter` → `:merge:` is GCounter's increment-merge.
- `meta.mergeable: LWW` → `:merge:` is last-write-wins by timestamp.
- `meta.mergeable: RGA` → `:merge:` is the RGA text-CRDT merge.
- user-defined mergeable protos are first-class.

The substrate is **CRDT-shape-neutral**. It produces and consumes per-slot
diffs. CRDT logic is moof code (or substrate-bundled mergeable protos). Same
"substrate does mechanism, moof does policy" rule as freezing.

**Deterministic allocation** for replicated vats: FormId allocation is keyed
by `(turn-seq, per-turn-ordinal)`, not by Vec push-index. Followers see the
same ids. Solo vats use Vec push-index (faster). Discipline is per-vat-mode.

## 20. effect-intents and the cap authority

Cap calls during a turn don't fire — they accumulate as `EffectIntent` entries
on the vat's outbox. Each intent records `(cap-token, action, args, turn-seq,
ordinal)`.

At turn-commit, the **cap authority** (a non-replicated worker; for solo vats,
typically the same vat's runtime; for replicated vats, a singleton authority
outside the replicated quorum) reads the new intent stream and fires each
at-most-once:

1. lookup intent by `(turn-seq, ordinal)` in the effects-log dedup table; if
   present, this intent has already fired — re-emit cached receipt, don't
   refire.
2. otherwise, fire the side effect (read clock, write byte, etc.), get result.
3. journal `(intent, result)` to `effects.log` with the same key.
4. emit an `EffectReceipt` envelope on the originating vat's inbox.

The originating vat receives the receipt as an ordinary message on the next
turn.

Replay observes receipts in `effects.log`; never re-fires. **Exactly-once for
journaled state, at-most-once for side effects, observed-once for receipts.**
This is the croquet / akka-persistence / datomic synthesis applied to caps.

## 21. boot, quit, supervision-tree assembly

**Boot:**

1. read `.moof/manifest.toml`; instantiate root supervisor vat from declared
   proto.
2. supervisor reads its own state (its vat directory).
3. supervisor reads `[auto-start]` and brings up declared vats in dependency
   order.
4. each vat boots independently: open meta.toml, mmap store.lmdb, replay
   inputs.log tail, re-fire un-receipted intents.
5. far-refs reconnect lazily — first message to a given vat establishes the
   route.
6. once root supervisor signals ready, user-facing canvas / UI appears.

**Quit / sleep:**

1. user signals quit.
2. root supervisor broadcasts `:prepare-shutdown` to children.
3. each vat finishes its current turn, commits journal, signals ready.
4. root supervisor commits its own state.
5. process exits.

Forced quit (kill -9) loses uncommitted turn-state; already-committed turns
persist; replay tail recovers.

**First launch.** No `.moof/` directory: substrate generates default manifest,
mints root supervisor's primordial caps, spawns a default Workspace vat,
shows canvas. World saves itself per-turn from then on.

## 22. implementation phasing

The substrate refactors land in dependency order. Each phase compiles, tests
pass at the boundary. This list is the input to writing-plans, where each
phase becomes a milestone with sub-tasks.

**V0 — FormId scope-tagging.** No behavior change yet. Introduce 2-bit scope
tag on FormId; existing single-heap `World` keeps everything in `00…`
(vat-local) scope. All `heap.get(id)` paths gain a tag-dispatch. Other scopes
stubbed (panic on access). Exit: tests still pass; FormId is now scope-aware.

**V1 — per-turn nursery + diff.** Nursery as separate small heap allocated on
turn-entry. Redirect `Form::alloc` and slot/handler/meta mutations to nursery
during a turn. Read-through (nursery first, fall through to canonical).
Compute per-slot diff at turn-end. Journal diffs to in-memory `inputs.log`
(not yet on disk). Exit: every successful turn produces a diff; rollback drops
nursery cleanly.

**V2 — freezing.** Add `frozen` bit to Form. Shallow `freeze` primitive.
Mutation guard raises `'frozen-form`. Vat-mode parameter on World (substrate
hosts one vat for now; mode held by World). Moof-side `freezeRecursive`
helper in stdlib. Exit: frozen forms reject mutation; vat-mode toggles `[Type
new]` behavior.

**V3 — env-chain / `$here` unification.** Rename `World.global_env` to
`World.here_form`. Expose `$here` to moof as a Form. Redefine `def` as a moof
macro expanding to `[$here bind: ...]`. Env walker unchanged. Exit: REPL
`(def foo 42)` then `foo` resolves through `$here`; reflection on `$here`
lists path-bound names.

**V4 — multi-vat container.** Carve `World` into `World` (process-scope:
shared segment, far-ref directory, path-table-vat) + `Vat` (per-vat: heap,
nursery, mailbox, here, behavior, supervisor, caps). Substrate hosts multiple
Vat instances; scheduler is round-robin. Exit: two solo vats coexist;
within-vat sends still sync.

**V5 — references protocol + membrane translation.** Reference-Form proto
(slot-ref / id-ref / far-ref / path-ref). Far-ref table per vat (the `10…`
scope). Envelope serializer walks message values, applies membrane
translation: mutable forms become far-refs; frozen forms deep-copy into the
receiver's vat-local heap (segment promotion is a V6 optimization, not yet
required). Exit: cross-vat sends transit a far-ref for mutable, deep-copy for
frozen; mutable forms cannot escape; membrane-translation tests pass.

**V6 — shared segment + content-addressed promotion.** Process-scope shared
arena (the `01…` scope). Intern table keyed by blake3 hash of canonical
bytes. Promotion path on first cross-vat send replaces V5's deep-copy for
frozen forms. Forwarding pointers in vat-local heap. Exit: identical frozen
forms sent twice resolve to the same shared id; deep-copy path retained as
fallback for not-yet-promoted forms.

**V7 — eventual sends + promises.** `<-` syntax in reader; `OP_EVENTUAL_SEND`
in compiler. Promise Form proto with three-state machine. `when-resolved:` /
`when-broken:` / `then:` / `sync-await:` handlers. Pipelining for sends to
pending promises. Exit: cross-vat send returns a promise; `when-resolved:`
fires after target turn-completes.

**V8 — supervision + spawn.** `[$vat spawn: ...]` returns a far-ref to the
new child. Supervisor field on vat-Form; set at spawn. Crash → rollback turn
→ notify supervisor → restart per strategy. Exit: child vat raising mid-turn
is restarted by its supervisor per the strategy.

**V9 — persistence.** Per-vat directory layout `.moof/vats/<id>/`. Mmap'd
lmdb store (cargo: heed or rkv). `inputs.log` and `effects.log` write-ahead.
Snapshot/compaction worker. Boot: read manifest, replay tail, signal ready.
Exit: a vat's state survives reboot; `moof world ./worlds/test/` runs the
manifesto demo's first 60 seconds.

**V10 — capabilities, effect-intents, cap authority.** Capability Form proto,
attenuation handlers. Outbox accumulates EffectIntent entries during a turn.
Cap authority worker fires intents post-commit, journals receipts, emits to
inbox. Replay observes receipts. Exit: a vat that calls `[$out write:]` fires
the side-effect exactly-once across reboots.

**V11 — replication + per-slot CRDT hooks.** `#replicated-leader` /
`#replicated-follower` modes on vat spawn. Deterministic FormId allocator for
replicated vats. Input-log replication primitive (consensus is its own
concern; minimum: two-replica with designated leader). Per-slot Mergeable
annotation; merge-on-arrival path. Exit: two-replica convergence test passes.

Each phase is a substrate-side refactor + test suite. User-facing moof code
mostly doesn't change between phases — only at V3 (def-as-path-bind), V4
(spawn syntax), V7 (`<-` and promises), V8 (supervisor visible). Earlier
phases are invisible above the substrate.

## 23. deferred to follow-up specs

Real deferrals — each a session-sized brainstorm + spec on its own:

- **host-language migration** (rust → ?). The contract here is host-neutral;
  migration is an orthogonal refactor with the same target shape.
- **image binary format byte-level encoding** — the *shape* of per-vat
  persistence is fixed in this spec (per-vat directory, lmdb store, journal,
  inputs.log, effects.log); the *encoding* — canonical bytes for forms, wire
  format of journal entries, wire format of envelopes — is
  `reference/canonical-encoding.md`.
- **GUI / canvas integration with per-vat liveness** — vats own widgets;
  canvas is its own vat or set of vats; render protocol; input dispatch.
- **federation transport details** — websocket / unix-socket / network wire
  formats. The *protocol* is fixed here (envelopes, far-refs, cap-tokens,
  promises); the *wire* is later.
- **MCP integration** — the v2 vision has MCP-over-HTTPS for federation;
  rests on the federation transport spec.
- **scheduler tuning** — fuel-based vs round-robin vs priority; backpressure;
  starvation avoidance. Round-robin is the default starting point; tuning is
  its own concern.

## see also

- `docs/concepts/vats.md` — the vat model, granularity guidance.
- `docs/concepts/references.md` — the four reference kinds.
- `docs/concepts/persistence.md` — per-vat on-disk shape.
- `docs/concepts/forms.md` — the four faces.
- `docs/concepts/effect-intents.md` — intent/receipt mechanics.
- `docs/concepts/replication.md` — replicated-vat mode details.
- `docs/concepts/capabilities.md` — cap-token semantics.
- `docs/laws/substrate-laws.md` — load-bearing invariants this spec preserves.
- `docs/laws/isolation-laws.md` — formal cross-vat rules.
- `docs/laws/reflection-contract.md` — what reflection promises hold.
