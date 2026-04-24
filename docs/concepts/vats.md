# vats

**type:** concept
**specializes:** throughline 3 (walks), throughline 4 (additive),
                 throughline 5 (canonical form)

> moof's concurrency unit. a vat is a single-threaded actor with
> its own heap, its own message queue, and no shared memory.
> everything concurrent is a vat. messages between vats are the
> only way state crosses a boundary.

---

## how vats touch the throughlines

- **walks (throughline 3).** a FarRef is a named pointer across
  a vat boundary; sending through it is a walk from sender to
  target. the URL on the FarRef (`moof:/caps/console`,
  `moof:/vats/7/objs/42`) names the walk so it survives restart.
- **additive authoring (throughline 4).** vat state changes
  only via Updates — atomic, between-message deltas. each
  state is a snapshot; nothing mutates in place. the vat's
  history is a sequence of states.
- **canonical form (throughline 5).** cross-vat message copy is
  canonical serialization of the args. receiver deserializes
  into its heap. immutable values dedupe via content hash.
  FarRefs carry a URL so they round-trip.
- **streams (context + time).** a vat's mailbox is a Stream of
  incoming messages; the scheduler is a transducer over it.

these aren't extra abstractions bolted onto vats. they ARE what
vats are.

---

## the model

- **a vat is an actor.** one thread of control, one event loop,
  one message queue, one heap.
- **messages are one-way.** a vat sends a message to another vat's
  object; the message is enqueued; it'll be processed later.
- **no shared state.** vats don't share memory. what they share are
  Values (copied or content-addressed) and FarRefs (proxies).
- **isolation is security.** a vat can't reach another vat's heap
  unless it's been given a reference.

this is erlang's process model, with smalltalk's object model, with
E's capability model. each vat is a little image of its own, with
its own time and its own state.

---

## why vats

three reasons:

1. **concurrency without locks.** within a vat, code is single-
   threaded — no race conditions, no mutex discipline. between
   vats, communication is async messaging — no shared state to
   protect.
2. **failure isolation.** a vat that crashes doesn't take down the
   rest of the image. supervisor objects watch vats and restart them
   per policy.
3. **capability boundaries.** a vat only has the capabilities
   handed to it at spawn. holding no reference to Console = can't
   print. this is security by construction (see
   [capabilities.md](capabilities.md)).

---

## anatomy of a vat

```
vat {
  id                  ; unique integer
  heap                ; object arena, symbol table, protos
  vm                  ; execution state, frames, registers
  mailbox             ; incoming messages awaiting processing
  outbox              ; outgoing messages awaiting delivery
  ready_acts          ; Acts whose continuations are ready to run
  status              ; running / idle / dead
}
```

the heap is private. nothing outside this vat reads or writes to
it. when a value needs to cross a boundary (another vat receives a
message with args), the value is copied into the target vat's
heap by the scheduler — a deep traversal that translates immutable
values (often dedupe-able via content hash) and preserves FarRefs
as references.

---

## send semantics

### same-vat sends are synchronous

if A and B are in the same vat, `[B foo: arg]` calls B's `foo:`
handler immediately and returns the result. no queuing. no async.
this is how 99% of sends work in practice.

### cross-vat sends are eventual

if A is in vat 7 and B is a FarRef to something in vat 12,
`[B foo: arg]` enqueues a message on vat 7's outbox. the scheduler
delivers it to vat 12's mailbox. vat 12 eventually processes it.
A gets an **Act** — a first-class effect descriptor — back
immediately, representing the eventual result.

the syntax `[B <- foo: arg]` forces eventual semantics even in-vat
(rare but legal, e.g. to defer to the next tick).

### FarRefs

a FarRef is a cross-vat proxy. it's a regular moof object with a
`doesNotUnderstand:` handler that queues the send. any message
sent to it becomes an outbox entry.

FarRefs carry:
- target vat id + object id (session-local)
- a URL (`moof:/vats/12/objs/42` or `moof:/caps/console`) that
  survives restart

after an image reload, FarRefs resolve their URL to fresh (vat id,
obj id) pairs. you keep your references; the wiring re-establishes.

---

## the scheduler

one rust-side scheduler runs the vats. simplified loop:

```
forever:
  for each vat with pending work:
    process one batch of messages / ready acts
    fuel-limit: stop after N reductions
  deliver outgoing messages to target vats
  resolve pending Acts
  if no vat has work, sleep on an event
```

scheduling is **fuel-based**: each vat gets a budget of reductions
per turn and yields when it runs out, ensuring no one vat starves
others. this is BEAM's strategy, borrowed directly.

the scheduler is not a vat itself (yet). it's a rust struct that
owns the vats and runs them. wave 9.5+ turns it into a moof-level
Scheduler capability — an object with `spawn:` and `spawnCapability:`
handlers — so moof code can spawn vats without rust intermediation.

---

## servers: long-lived vats with state

a **server** is a vat whose only job is to hold mutable state and
respond to messages about it. written with `defserver`:

```moof
(defserver Counter (initial)
  "a cell of integer state."
  { value: initial
    [get]        @value
    [incr]       (update { value: [@value + 1] } @value)
    [reset]      (update { value: 0 } 0)
  })

(def my-counter (Counter 0))    ; spawns a vat, returns a FarRef
[my-counter incr]                ; sends 'incr' to that vat
                                 ; returns Act<0> (old value)
```

the server pattern:
- state lives in slots on the server object.
- handlers return either a pure value, or an `Update` form.
- an Update tells the scheduler: "change these slots, then reply
  with this value." updates happen atomically, between messages.
- from outside, `[my-counter incr]` is an eventual send. it returns
  an Act that resolves when the scheduler processes the update.

see [effects.md](effects.md) for Update semantics.

---

## capability vats

some vats exist to wrap native functionality: Console (stdout),
Clock (time), File (filesystem), Random (PRNG). these are loaded
from dylibs, spawned once at startup, and granted to user vats.

from moof's point of view they're just vats — you send them
messages, you get back Acts. the rust-ness is invisible beyond
"crossing the vat boundary means crossing into native code."

see [capabilities.md](capabilities.md) for the security model.

---

## cross-vat messages: what actually crosses

when you send `[farref foo: val]`:

1. the send is enqueued on the sender's outbox with:
   - target vat id, target obj id
   - selector (interned in sender's heap)
   - args (still references in sender's heap)
   - act id (local, to be resolved with the reply)
2. the scheduler picks up the outbox message.
3. it translates the selector: looks up the symbol name in sender,
   interns it in target. (symbols are per-heap.)
4. it **copies** the args across: for each value, deep-traverse and
   rebuild in the target heap. immutable values (ints, strings,
   cons, tables with no FarRefs) are translated; FarRefs stay as
   references; content-hashed values are dedupe-able.
5. a Message struct is created in the target vat's mailbox.
6. the target vat, when the scheduler gives it time, dispatches the
   message to the target object.

the copy step is the cost of isolation. it's also what makes moof
trivially thread-safe: no shared references, no locks.

---

## why not threads?

threads share memory. that's the problem threads solve and the
problem threads create. moof's argument is that when you take the
memory-sharing away, you get a model that's:

- easier to reason about (no races)
- easier to distribute (vats don't need to be on the same machine)
- easier to persist (vat state is a valid image point, whereas
  thread state is transient)
- easier to debug (messages are observable; shared-memory writes
  are not)

the cost is more copying and more indirection. for personal-
computing workloads this is a very favorable trade. for
high-throughput scientific computing it isn't — but that's not
what moof is for.

---

## let it crash

a vat can die. messaging it then resolves with an error Act. a
supervisor object (another vat) watches its children; on death, it
decides per policy:

- **always** restart
- **on-failure** restart unless exit was clean
- **never** leave dead and log

this is erlang / OTP verbatim. the supervision tree is how you
build reliable systems out of unreliable components.

wave 10+ will have the supervision primitives as moof-level
defservers. they exist today only in rough shape.

---

## what you need to know

- a vat is an isolated actor with its own heap and message queue.
- within a vat: synchronous sends, normal object dispatch.
- across vats: eventual sends, Acts, no shared memory.
- FarRefs are proxies that let you message cross-vat without
  caring where.
- servers are vats holding state; updates change state between
  messages.
- capability vats wrap native functionality (console, clock, etc).
- the scheduler runs vats fairly with fuel-based preemption.
- let-it-crash: failures are isolated, supervised.

---

## next

- [../throughlines.md](../throughlines.md) — walks, additive,
  canonical form — the patterns vats embody
- [effects.md](effects.md) — Acts, Updates, do-notation — how
  cross-vat work composes
- [streams.md](streams.md) — mailboxes as stream sources
- [capabilities.md](capabilities.md) — the security layer on
  top of vats (constraints on reachability)
- [addressing.md](addressing.md) — how vats, services, and
  capabilities have URLs
