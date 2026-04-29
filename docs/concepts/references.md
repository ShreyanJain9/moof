# references

> **four reference kinds, each a Form. cross-vat references are
> first-class from day one. raw heap-pointers never escape a vat
> boundary. federation falls out of getting this right.**

this is the load-bearing isolation discipline. if we get it right,
distribution is a small extension. if we get it wrong, every
problem becomes a special case.

## the taxonomy

| kind | within | shape | semantics |
|---|---|---|---|
| **slot-ref** | one vat | `(form-id, slot-name)` | sync read/write; mutable |
| **id-ref** | one vat | `(form-id)` | sync identity, normal sends |
| **far-ref** | crosses vats | `(vat-id, form-id, cap-token)` | async send only → promise |
| **path-ref** | the world | `("/users/shreyan/notes/today")` | named lookup → resolves to id-ref or far-ref |

every reference is itself a Form with proto `Reference` and a
`:resolve` handler.

## the substrate invariant

> **raw form-ids never cross a vat boundary.**

if a value leaves its origin vat — passed in a message, serialized
to disk, sent across the network — the substrate auto-promotes any
in-vat references it contains to far-refs. this is enforced at the
serialization boundary; user code cannot bypass it.

## slot-ref

a reference to a specific slot of a specific Form within one vat.

```moof
(let r (slot c 'count))          ; slot-ref to c.count
[r read]                         ; → c's current count value
[r write: 5]                     ; → mutates c.count
[r observe: |new old| …]         ; subscribe to changes
```

slot-refs are how reactive bindings, atoms, and observation hook in.
they are mutable cells with optional change-observation. (they are
*not* the same as `Atom` — an Atom is a Form whose proto is `Atom`
and whose slot is the cell-value; a slot-ref points to *any* slot.)

## id-ref

just a Form's id. passing a Form passes its id. within a vat, this
is the normal case — you send messages, the substrate dispatches.

## far-ref

a reference that crosses vat boundaries. always async. always
returns a promise on send.

```moof
(let counter [(workspace 'shreyan) ask: 'counter])  ; → far-ref
[counter incr]                   ; returns a Promise
                                 ; (the message is queued on
                                 ;  shreyan's vat's mailbox)

(let answer [counter read])      ; → Promise
[answer when-resolved: |v|
  (println "counter is now $v")]
```

a far-ref's three fields:

- `vat-id` — the target vat's identity (UUIDv7-ish; see
  `concepts/vats.md`).
- `form-id` — the target Form's id within that vat.
- `cap-token` — an unforgeable token authorizing send rights.

routing is the scheduler's job:

- target vat in same process → enqueue on its inbox.
- same machine, different process → unix-socket envelope.
- different machine → network envelope.

the user verb is the same in all three cases. *this is the federation
primitive.*

## path-ref

a named address in the world's namespace.

```moof
#Path "/users/shreyan/notes/today"
```

resolving a path:

```moof
(let notes [#Path "/users/shreyan/notes/today" resolve])
```

paths resolve to either an id-ref (if the target lives in the current
vat) or a far-ref (if the target lives in another vat). the user
sees a uniform `Reference`; the substrate routes accordingly.

paths are persistent: writing a path is "name this Form here," which
adds an entry to the world's path-table (itself a Form). lookups
walk the path-table.

## promises

a promise is a Form whose state transitions:

- `#pending` — no value yet.
- `#ready: value` — resolved successfully.
- `#broken: reason` — resolved with error.

promises are first-class. you can pass them, store them, send to
them. sending to a pending promise *pipelines* the message — it's
queued and sent to the eventual value when ready (e tradition,
miller 2006).

```moof
[promise when-resolved: |v| ...]
[promise when-broken: |err| ...]
[promise then: f]                ; map: value → new-promise
[promise sync-await: 5s]         ; block this turn up to 5s; emergency only
```

within a vat's message-turn, you cannot block on a promise — promises
resolve between turns. for sync access, the user explicitly opts into
`:sync-await:` which is rare and visible.

## capability-bearing

every far-ref carries a cap-token. the substrate verifies the token
on every send. attenuation is "ask the cap for a smaller version":

```moof
(let read-only-counter [counter readonly-cap])
;; read-only-counter is still a far-ref, but with a smaller cap.
```

capabilities are forge-proof: the substrate's only constructors are
"the root supervisor at boot" and "attenuating an existing cap."
there is no rust escape hatch.

## serialization

far-refs serialize naturally as their three small fields. when a vat
saves its state, far-refs in its heap are written as those triples.
on load, the far-refs are still valid; the substrate doesn't try to
resolve them at load — they sit dormant until messaged.

this means: vat A can be saved and the sent-out far-refs to B remain
valid even if B is offline at A's load time. the first send waits
until B is reachable.

## inspirations

- e's vats and far-refs and capability-passing: mark miller,
  *robust composition* (2006).
- promise pipelining and "when": e and joule (miller, hardy 1988).
- erlang PIDs as location-transparent process references: erlang/OTP.
- ambienttalk's first-class futures: van cutsem, dedecker.
- pony's reference capabilities (statically typed at handoff): clebsch et al.
- croquet's deterministic distributed actors: kay, reed, smith ~2003.

## see also

- `concepts/vats.md` — what a vat is.
- `concepts/capabilities.md` — what cap-tokens carry.
- `concepts/persistence.md` — how far-refs persist.
- `laws/isolation-laws.md` — formal cross-vat rules.
