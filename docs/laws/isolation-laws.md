# isolation laws

> **the rules governing what crosses a vat boundary. these are the
> federation foundation. break them and distribution is impossible;
> hold them and distribution is automatic.**

## I1 — no shared mutable state across vats

two vats cannot share a mutable Form. period. every mutation
happens within the vat that owns the state. cross-vat communication
is exclusively by message-passing.

this is the erlang/E discipline, taken absolutely. it makes
supervision sane, replication tractable, time-travel implementable,
and federation a small extension.

## I2 — only Forms with serializable shapes cross boundaries

values that cross vat boundaries must be one of:
- **value Forms** — numbers, symbols, strings, immutable Tables/Lists,
  Char, etc. (anything whose proto declares `Value-Form` and which
  has no mutable slots transitively).
- **far-refs** — `(vat-id, form-id, cap-token)` triples.
- **closures**, only if pure (no captured mutable state).

mutable Forms (Atoms, mutable Tables holding cycles, mutable objects)
are auto-promoted to a far-ref *to themselves* when they would cross
a boundary. the receiving vat gets a far-ref it can message; it
cannot reach into the original.

## I3 — raw form-ids never escape

a Form's `:identity` is a vat-local heap-id. the substrate's
serialization layer enforces: any in-vat reference embedded in a
value being sent across a boundary is rewritten to a far-ref. user
code cannot bypass this — there is no FFI / native escape that
transports raw ids.

violation = substrate bug. test cases must verify.

## I4 — far-refs are the only cross-vat reference

within a vat: id-refs, slot-refs are normal.

across vats: only far-refs. attempts to construct a "raw cross-vat
reference" via reflection or rust-bridge are refused; the substrate
returns a far-ref instead.

## I5 — capabilities ride on far-refs

a far-ref carries a cap-token. sends to the far-ref are validated
against this token before delivery. attenuation produces a new
far-ref with a smaller token; the original token is unchanged.

caps in their resting state (held in a vat's `caps` table) are *also*
far-refs (from the holder's perspective). the substrate doesn't
distinguish — caps are just refs with cap-bearing semantics.

## I6 — sends to far-refs are async

sending a message to a far-ref:
1. constructs an envelope: `(target-id, selector, args, cap-token,
   reply-to)`.
2. the args are serialized at the boundary (per I2).
3. the envelope is enqueued on the target's inbox.
4. immediately returns a Promise.

the calling vat's turn does not block. the promise resolves when the
target processes the message and sends a reply (or breaks if the
target rejects / dies / never reaches).

within a single turn, a vat *can* call `[promise sync-await: timeout]`
in emergencies. this is rare and visible.

## I7 — promise-pipelining is built-in

```moof
(let outer [remote-counter incr])
(let inner [outer next-step])         ; sends to outer's eventual value
```

sending to a promise pipelines: the message is queued and sent to
the resolved value when ready. the substrate handles this. no
explicit "wait then send" pattern required.

(this is e/joule promise pipelining; miller, hardy 1988.)

## I8 — failures are localized to a vat

if a vat crashes, *only that vat's state is at risk*. its supervisor
is notified. the supervisor decides recovery policy.

cross-vat references to the crashed vat:
- in-flight messages: their promises break with `:vat-crashed`.
- new messages: queued; delivery resumes when the vat is restarted.
- (pessimist mode: configure to break instead of queue.)

## I9 — the routing table is substrate-level

the substrate maintains, per running process, a routing table
mapping `vat-id → location`. locations are one of:
- `local: <handle-to-in-process-vat>`
- `unix-socket: <path>`
- `tcp: <host, port>`
- `websocket: <url>`

routing is invisible to user code. the *same send verb* applies in
all cases. distribution is what's in the routing table.

## I10 — vat birth and death are supervised

a vat can only be born by:
- the root supervisor at world boot.
- another vat using its `$spawn` cap (handed in by its supervisor).

a vat can die by:
- explicit shutdown by self or supervisor.
- supervised crash (failure during a turn).
- forced kill by ancestor in the supervision tree.

orphan vats (no supervisor) are not allowed. on supervisor death,
children are reparented (typically to grand-supervisor) or also
terminated, per the dying supervisor's policy.

## I11 — the world has one root

per running process: one root supervisor. it has no supervisor; it
manages itself; its death is process termination.

federations of worlds across processes are *peers* — root supervisors
can far-ref each other but neither is the boss. cross-world topology
is application-level, not substrate-level.

## I12 — capabilities cannot escape their granting scope

an attenuated cap is no broader than its grantor's cap. the
substrate's attenuation primitive enforces this. there is no
"upgrade" verb.

a cap held by vat A can be passed to vat B. B can attenuate it
further or store it. B cannot upgrade it. if A's cap is revoked, B's
attenuated cap is also revoked transitively (caps are tree-structured;
revoking the root revokes the descendants).

## inspirations

- e's vats and capability discipline: miller (PhD thesis 2006).
- erlang/OTP supervision and process isolation: armstrong et al.
- pony's reference capabilities (statically enforced): clebsch et al.
- ambienttalk's far-ref semantics: van cutsem.
- croquet's deterministic-distributed-actors: kay et al.

## see also

- `laws/substrate-laws.md` — broader guarantees.
- `concepts/references.md` — far-ref taxonomy.
- `concepts/vats.md` — what vats are.
- `concepts/capabilities.md` — cap mechanics.
