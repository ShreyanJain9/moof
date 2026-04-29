# vats

> **a vat is the unit of concurrency, isolation, persistence, and
> distribution. one heap. one mailbox. one journal. one supervisor.
> messages within are sync; messages across are async with promises.**

we use *vat* (e tradition: miller 2006) rather than *actor* or
*process* because the term picks up the right intuitions: a contained
volume of life, with a membrane (the message boundary) and a tap (the
mailbox).

## the model

a vat consists of:

| field | what |
|---|---|
| **id** | unique identifier (UUIDv7-ish, see below) |
| **heap** | a private form-graph |
| **inbox** | a data source — incoming messages |
| **outbox** | a data source — outgoing messages (optional, for publish patterns) |
| **behavior** | a closure invoked on each received message |
| **supervisor** | a far-ref to the supervising vat |
| **caps** | the capabilities this vat holds |
| **journal** | a per-vat WAL (`concepts/persistence.md`) |
| **store** | the canonical on-disk form (`concepts/persistence.md`) |

within a vat: synchronous message sends. shared heap. cheap
allocation. one execution thread.

across vats: asynchronous message sends. no shared heap. messages
queue on target's inbox. replies via promises.

## within-vat semantics

inside a vat, sends are smalltalk-flavored:

```moof
(let c [Counter new])            ; alloc in this vat
[c count: 0]                     ; sync
[c incr]                         ; sync
.count                           ; in a method body, .count reads
```

method dispatch follows proto-chain (`concepts/objects-and-protos.md`).
mutations to slots are sync. allocation does not block.

## across-vat semantics

```moof
(let alice [Workspace at: 'alice])      ; far-ref
(let promise [alice greet: 'world])     ; async send → Promise
[promise when-resolved: |v| (println v)]
```

sending to a far-ref:
1. envelopes the message: `(target-form-id, selector, args, cap-token, reply-to)`.
2. routes via the scheduler to the target's inbox.
3. returns a Promise immediately.
4. when the target processes the message and replies, the promise
   resolves and observers fire.

cross-vat sends are the *only* way to interact across vats. there is
no synchronous read of a remote slot. there is no shared mutable
state. ever.

## message-turns

a vat's main loop is conceptually:

```
loop:
  msg ← [inbox next]              ; block until message
  begin-turn:
    dispatch msg.target msg.selector msg.args
  end-turn:
    commit mutations to journal
    send replies (any promises resolved this turn)
    yield
```

a *message-turn* is the unit of atomicity. mutations within a turn
either all commit or none do (crash-safety). replies are batched at
turn-end. (this is the e/croquet pattern.)

## supervision

every vat has a supervisor (a far-ref to another vat). the supervisor
is responsible for the vat's lifecycle:

- spawning (the supervisor decides who is created).
- crash recovery (when a vat dies, the supervisor decides whether to
  restart, with what state, and how often).
- shutdown (the supervisor commands graceful or hard termination).

the *root supervisor* is the world's top vat. it has no supervisor
above it (its `supervisor` field is nil); it manages itself. this is
the erlang/OTP supervision-tree pattern (armstrong, PhD thesis 2003).

restart strategies (configurable per-vat):

- `:restart-from-snapshot` — load last snapshot, replay journal.
- `:restart-fresh` — discard state, start from genesis.
- `:never-restart` — death is permanent (e.g., for transient ad-hoc vats).

## crashes are normal

let-it-crash (armstrong et al.). a vat's misbehavior triggers:

1. the vat halts (its turn is rolled back).
2. its supervisor is notified.
3. the supervisor decides what to do next.

within-vat code can use `try`/`catch` for *anticipated* errors. for
*unanticipated* errors, the let-it-crash discipline takes over.

## the inbox is a data source

a vat's inbox is just a data source — an instance of the universal
streams primitive (`concepts/data-sources.md`). this means:

- you can `:tee` the inbox to log all incoming messages without
  modifying the vat.
- you can wrap an inbox in a `:filter` for testing (drop messages
  matching a predicate).
- a debugger can intercept the inbox to single-step messages.
- replaying a vat's history is `[journal for-each: |msg| [vat receive: msg]]`.

this single substrate move makes a tremendous amount of debugging /
observability / fault-injection trivial.

## granularity — when is a vat?

guidelines for how to scope vats:

- **a workspace** (one user's working area) is one vat.
- **a window** is often its own vat (so a misbehaving window doesn't
  hang the workspace).
- **a long-running computation** (game-of-life, training run) is
  its own vat.
- **a service / daemon** (a clipboard, a notification center) is
  its own vat.
- **routine objects** (counters, tables, blocks) are *not* vats;
  they live inside vats.

rule of thumb: if it has a non-trivial persistent identity, an
on-disk presence, or a need for fault isolation — it's probably a
vat. otherwise it's a Form inside one.

## vat ids

we use UUIDv7 (timestamp-prefixed) for the substrate id. each vat
has a friendly path alias (`/users/shreyan/workspace`,
`/services/clipboard`) maintained in the world's path-table. ids are
forever; aliases can be reassigned.

## per-vat persistence

each vat has its own directory on disk (`concepts/persistence.md`):

```
.moof/vats/<vat-id>/
  meta.toml        (id, name, supervisor, caps)
  store.lmdb       (canonical form-graph)
  journal.log      (WAL of mutations)
  refs/            (named root pointers, e.g. inbox cursor)
```

vats save independently. you can copy a vat's directory to move it.
you can mount a vat from disk into a running world. you can serialize
a vat for distribution and unpack it elsewhere. (we lean on this for
collaboration.)

## inspirations

- vats: e (mark miller, *robust composition* 2006).
- supervision trees: erlang/OTP (armstrong, *making reliable
  distributed systems* PhD thesis 2003).
- per-process isolation as the unit of fault tolerance: erlang.
- mailbox-as-stream: erlang's processes; modernized by the data
  source primitive.
- distributed determinism: croquet/teaTime (kay, reed, smith ~2003–2007).
- the smalltalk-flavored within-vat experience: smalltalk-80
  (goldberg & robson 1983) — except now there are many smalltalk
  images on the same machine.

## see also

- `concepts/references.md` — far-refs that bridge vats.
- `concepts/data-sources.md` — what mailboxes really are.
- `concepts/persistence.md` — per-vat on-disk shape.
- `concepts/capabilities.md` — what's in a cap-token.
- `laws/isolation-laws.md` — formal isolation rules.
