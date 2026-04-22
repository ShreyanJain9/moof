# system

> the unified process-data model. in moof, everything is in
> the image — including the system itself. vat 0 is init.
> services are objects. running state is a value. booting is
> just "let the image continue what it was doing."

---

## the insight

plan 9 said everything is a file. smalltalk said everything is
an object. unix init says something has to bootstrap everything
else. moof takes each of these further:

- **smalltalk** had an image, but it still had separate
  "smalltalk processes" running inside that image —
  conceptually objects, but in practice a privileged layer the
  user didn't get to design. moof has vats that are just
  objects; the runtime is the image's objects talking to each
  other.
- **plan 9** had namespaces and everything-is-a-file, but
  processes and files were distinct categories at the kernel
  boundary. moof has no such boundary — processes (vats, acts,
  frames) and data (values) are the same material, addressed
  the same way.
- **unix init** is a privileged pid 1 because the kernel
  grants it that role. moof vat 0 is just the first object;
  its init-ness is a protocol-level convention, not a runtime
  privilege. anyone could write their own init and point moof
  at it.

the result is "smalltalk + plan 9 + init, taken seriously":

> **the image is the system. the system is the image.**

no separate startup program. no separate "installed software."
no registry, no .config, no dotfiles. the image is the full
computational environment. you snapshot it, ship it,
migrate it, fork it, resume it. your entire moof life is a
single accretive value.

---

## vat 0 is init

vat 0 today is a bare vat that does nothing. that's wrong. it
should be the init system.

### responsibilities

- **boot sequence.** load the image, register services, start
  capability vats, start user vats, grant capabilities, start
  the repl. all of this lives in rust today
  (`crates/moof-cli/src/shell/repl.rs`, ~100 lines of logic).
  it should move into moof code running in vat 0.

- **service registry.** hold the list of declared services,
  their manifest state (running, crashed, disabled), their
  restart policies. inspectable from any repl as
  `[System services]`.

- **supervision tree.** watch child vats. when a supervised
  vat crashes, decide per-policy whether to restart, escalate,
  or give up. when a parent vat dies, re-parent its children
  to vat 0 (adoption of orphans, erlang-style).

- **capability authority.** granting a capability to a vat
  requires asking vat 0. this is the single chokepoint for
  policy, audit, and revocation. today it's ambient via
  `create_farref` + `env_def`; that's a hole.

- **shutdown orchestration.** on ctrl-c / SIGTERM / explicit
  shutdown: stop services in reverse dependency order,
  snapshot, flush, exit cleanly. no "sudden death" of in-flight
  work.

- **snapshots.** on demand or policy-driven, write the full
  image (object state + running state) to persistent storage.

- **signal routing.** host OS signals come in through a tiny
  rust hook and are delivered as moof messages to vat 0's
  handler. vat 0 decides what to do.

### vat 0 is moof code

the rust scheduler just runs it. the scheduler knows nothing
about services, restart, or supervision — that's user-space in
vat 0. this keeps faith with the no-privileged-layer
commitment in `authoring-vision.md`. if you don't like how
moof boots, you edit the init script. it's a workspace in your
image.

---

## services as objects

a service is a named, long-running vat with a lifecycle. we
declare services as values in the image:

```moof
(def ClockService
  (Service {
    name:        'clock
    description: "system clock capability"
    spawner:     || [Scheduler spawnCapability: 'clock]
    restart:     'always
    depends:     nil
    health:      |vat-ref| [vat-ref isAlive]
  }))
```

vat 0 holds a `Registry` (a defserver holding a Table of
services by name). at boot it walks the registry in dependency
order, spawns each, records the live vat-ref. it watches each
service; on crash, it re-spawns per policy; on shutdown, it
stops in reverse dependency order.

this replaces the current rust-side capability-spawning loop
with a moof-side service-manager. same information, now a
live value you can inspect, modify, and extend at runtime. add
a new service? `[System registerService: MyService]`. remove
one? `[System stopService: 'clock]`.

restart policies worth supporting:
- `'always` — restart on any exit
- `'on-failure` — restart only on abnormal exit
- `'never` — log and leave dead
- `'escalate` — crash propagates to vat 0's supervisor

dependencies form a DAG. startup walks leaves-first; shutdown
walks roots-first. if a dependency restarts, dependents get
notified (they can decide whether to also restart or
reconnect).

this is otp-supervisor-pattern, simplified to what moof needs.

---

## running state is in the image

this is the "more extreme" part.

today: moof's store persists object state — prototypes, slots,
handlers, named values. what it does *not* persist:

- a vat's mailbox (pending incoming messages)
- a vat's outbox (pending outgoing messages)
- in-flight Acts (pending continuations, resolution state)
- VM execution frames (what code is currently running, at what
  program counter, with what locals)
- the scheduler's runnable set

on shutdown, all of that is lost. boot starts from zero
computationally, even if object state is preserved.

the target: **the image includes running state.** when you
shut moof down and boot back up, vats resume mid-computation.
a pending Act is still pending. a vat that was waiting for a
send sees the send arrive whenever the sender runs again.

### what this requires

- **vm frames become heap objects.** already mostly true —
  they're structs owned by the VM's frame stack. need to make
  them `HeapObject::General`-backed so they serialize via the
  normal path.
- **mailbox / outbox serialize.** they're already `Vec<Message>`;
  Message is slot-addressable, should serialize.
- **Act chains serialize.** already slot-based; verify that
  the `__chain`, `__forward_to`, etc. handler-slots round-trip.
- **scheduler state comes from the image.** at boot, the
  scheduler reconstructs its runnable set by scanning vats'
  mailboxes/outboxes/ready-acts. no separate "what was I
  doing?" record needed.
- **closures with captured env serialize.** already do via
  `closure_captures`; verify that a closure saved today
  resumes correctly tomorrow.

### the payoff

**reboot is continuity.** you close your laptop. you open it
the next morning. moof resumes exactly where it was — the same
workspace open, the same in-flight computation halfway through,
the same mental context.

this is smalltalk's image persistence taken one step further.
smalltalk persisted object state but its interpreter frames
were transient. moof persists *everything*. the only non-persistent
thing is in-memory caches that can be rebuilt (the JIT
inline-cache is a conceivable future addition that'd need to be
regenerated on load; most of what's in the image doesn't).

### what this is not

we are not promising byzantine-fault-tolerant resumable
computation. if you crash mid-write to the log, you lose the
tail; the last checkpoint is intact. standard journaling-fs
semantics. we are also not promising that a save-and-resume
works across moof versions without migrators. that's the
`schema_version` story in `foundations.md`.

---

## namespaces, per vat, plan-9-style

each vat has a root env today, inherited from its parent at
spawn. we formalize this into a **namespace**: a first-class
moof value you can inspect, share, and manipulate.

```moof
(defprotocol Namespace
  (require (lookup: sym))
  (require (bind: sym to: val))
  (require (names))
  (require (mount: other at: prefix))    ; plan 9 union
  (require (unmount: prefix)))
```

per-vat namespaces get us:

- **capability granularity.** if you don't have `console`
  bound, you can't reach it. no ambient authority, ever.
- **sandboxing by construction.** spawn a vat with an empty
  namespace, mount only `clock`: that vat literally cannot
  touch the filesystem.
- **union mounts (plan 9 directly).** a vat can mount another
  namespace beneath a prefix. `[ns mount: peer-ns at: 'remote]`
  makes all of peer-ns reachable as `remote.<whatever>`. the
  repl's namespace is the user's personal namespace union-mounted
  with the shared system namespace.
- **the namespace is a value.** save it, fork it, diff two,
  send one to a friend. "here's the setup i use" becomes a URI
  exchange.

this is the plan 9 move applied to objects instead of files.
same primitives: mount, bind, unmount; same power.

---

## addressable object tree

every object lives somewhere navigable. by convention, vat 0
exposes well-known roots:

```
/vats/<id>                     — a vat
/vats/<id>/mailbox             — its mailbox
/vats/<id>/namespace           — its namespace
/services/<name>               — service declaration
/services/<name>/status        — running/crashed/disabled
/capabilities/<name>           — capability FarRef
/image/snapshots/<hash>        — historical snapshots
/image/uncommitted             — current tail
/protos/<name>                 — registered prototype
/env/<sym>                     — currently bound value
/peers/<peer-id>/*             — federated remote tree
```

this is *notational*, not a real filesystem. `[System at:
"/vats/42"]` is the actual primitive. but an inspector renders
it as a tree, and you navigate it the same way you'd navigate
plan 9.

for federation, mounting a remote peer's root at
`/peers/<id>/` means their objects are reachable by
path-resolution, with network round-trips on demand. 9p applied
to an objectspace.

combined with the URI story from `foundations.md`, this gives
us:

```
moof:<hash>                → immutable content-addressed value
moof:/vats/42              → live vat reference (local)
moof:peer/alice/vats/13    → remote vat reference (federated)
moof:/services/clock       → local clock capability
```

everything has a URI. everything is reachable by name. nothing
is ambient.

---

## capabilities, reconciled

today, capabilities are bound by the REPL's boot code. they're
FarRefs that live in the repl vat's env. there's no registry;
the capability's identity is `(vat_id, obj_id)` which is
session-local and not persistable.

in the new design:

- **capabilities are registered with vat 0 at creation.** they
  get a stable **symbol-name** that's independent of their
  current vat/obj ids.
- **capability grants are events vat 0 records.** "vat X was
  granted capability 'clock at time T." audit trail.
- **FarRef-by-name.** a FarRef stores the capability *name*,
  not the raw `(vat_id, obj_id)`. at send-time, the scheduler
  resolves the name through the registry. this means after
  restart, capabilities reconnect even though the ids change.
- **revocation is a thing.** vat 0 can revoke a capability at
  any time. all outstanding FarRefs for that name fail until
  re-granted.

this is a meaningful hardening of the capability story from
"we do it by convention" to "the runtime enforces it through
the registry."

---

## the boot story

what happens when you type `moof`:

1. **rust main**: parse args, open the store at
   `.moof/store`.
2. **rust rehydrate**: if the store has a saved image, load it
   into a heap. otherwise, fresh heap. either way: produce
   vat 0 with its state (including service registry) intact.
3. **rust handoff**: eval the init expression, which is either:
   - `(System resume)` — continue what we were doing before
     shutdown (default for a persisted image)
   - `(System fresh-boot)` — run first-time boot (for a new
     image or with `--fresh` flag)
4. **vat 0 (moof)**: `resume` walks the service registry; for
   each service marked "running", it verifies / restarts as
   needed. `fresh-boot` reads the manifest and registers
   services from scratch.
5. **vat 0 (moof)**: starts the repl service, which grants the
   repl vat its capabilities, connects stdin/stdout.
6. **user lands in repl.**

shutdown is the reverse:

1. user types ctrl-d or `(quit)`.
2. that sends a message to vat 0's `shutdown` handler.
3. vat 0 walks services in reverse dependency order, asking
   each to stop gracefully (with timeout).
4. vat 0 triggers a final snapshot.
5. vat 0 returns control to rust; rust closes the store and
   exits.

no magic. no hidden state. every step is moof code in vat 0's
workspace. every step is editable.

---

## comparisons

**vs. smalltalk.** smalltalk has an image; moof has an image
plus an append-only log plus content-addressing plus supervision.
smalltalk processes are first-class but part of the runtime;
moof vats are first-class and *are* the runtime. smalltalk
doesn't have capability security; moof does. smalltalk images
work great on one machine; moof images federate.

**vs. plan 9.** plan 9 has `/proc/<id>/mem` because processes
*are* the files. moof has `/vats/<id>/mailbox` because vats
*are* the objects. same idea, one level up — files are a
1970s abstraction, objects are the 1970s future-abstraction
that plan 9 didn't yet adopt.

**vs. systemd / launchd.** they supervise processes with
dependency ordering, restart policies, socket activation, etc.
moof's vat 0 does all of this but for in-image objects rather
than OS processes. simpler because we control the entire
environment. more powerful because services can hold complex
moof values, not just fds.

**vs. erlang/otp.** otp pioneered supervised process trees and
"let it crash." moof adopts this wholesale. the difference:
erlang processes are in-runtime but not in a persistent image;
moof vats are both. your supervision tree survives reboot.

**vs. docker / containers.** an image is a runnable
environment you can ship. docker images are os file trees;
moof images are object trees. a moof image is smaller
(~megabytes, not gigabytes) because it's content-addressed and
doesn't bundle an OS.

---

## open questions

- **multi-image.** can one moof binary host multiple images
  simultaneously? (like one sqlite serving multiple dbs). would
  each get its own vat 0? probably yes, but not urgent — start
  with one.

- **log compaction.** the event log grows forever. when do we
  snapshot-and-truncate? options: periodic (every n events),
  on-demand (user says `(snapshot)`), or threshold-based (log
  > n mb). i'd start with on-demand only, add periodic later.

- **hot reload of init code.** if vat 0's init script is a
  moof workspace you edit, and you reload `lib/system/init.moof`,
  does the running system pick up the changes? handlers are
  late-bound, so yes for most things. but some handlers might
  close over initial captures. need to be careful — "reload
  init" is a delicate operation.

- **migration across machines.** snapshot a vat on machine A,
  restore on machine B. identities (object ids) are local to
  the vat; capabilities resolve by name; content-addressed
  values dedupe. should work in principle. foreign-payload
  types need `schema_version` compatible between machines'
  plugin sets.

- **headless vs interactive boot.** today moof boots into a
  repl. for servers we want "boot into service mode, no repl."
  vat 0's init script should take a mode parameter: `'repl`
  vs `'service`. service mode just waits on a signal.

- **recovery mode.** if vat 0 itself is broken (init script
  has a syntax error, say), how do you get in? probably a
  `--rescue` flag that bypasses the image's init and drops you
  into a bare repl. then you can fix and `(save-image)`.

- **concurrent writers.** if two processes open the same
  image... multi-writer is hard. probably enforce
  single-writer via file lock, like sqlite does by default.

- **service crash blast radius.** if the `file` capability
  crashes, every vat holding a FarRef to it sees failure. is
  that correct? probably — better than the alternative of
  silent no-ops. callers handle with result / retry.

---

## sequencing

concrete phases, each a well-defined wave:

1. **phase 0: vat-0 skeleton in moof.** create
   `lib/system/system.moof` with a minimal `System` defserver.
   port the capability-spawning loop from `repl.rs` to moof
   code. ~1 week.
2. **phase 1: service registry.** define `Service` prototype
   and `Registry` defserver. declare existing capabilities
   (console, clock, file, random) as services. vat 0 starts
   them from the registry. ~2 weeks.
3. **phase 2: supervision policies.** implement restart,
   adoption, dependency ordering. exercise by deliberately
   crashing services and observing restart. ~2 weeks.
4. **phase 3: capability-by-name + persistence.** FarRefs
   carry capability names, not raw ids. revocation possible.
   grants become recorded events. ~2-3 weeks.
5. **phase 4: running-state persistence (the big one).**
   audit serialization for vm frames, mailboxes, acts. extend
   snapshot to include running state. verify round-trip on
   real workloads. ~1-2 months.
6. **phase 5: namespaces as first-class values.** generalize
   env to a Namespace protocol with mount/unmount. ~2 weeks.
7. **phase 6: addressable tree + uri routing.** implement
   `[System at: "/vats/42"]` resolution. wire to the URI
   scheme from `foundations.md`. ~1 week.

total: ~4-5 months. first three phases are the minimum viable
"vat 0 is init" system. phase 4 is the hard technical lift
that delivers the full "image includes running state" story.

---

## the test we're trying to pass

the system is working when:

1. `moof` boots, lands in a repl with capabilities granted.
2. `(quit)` snapshots everything including what was in flight.
3. `moof` again: you see exactly what you had, pending Acts
   still pending, mailbox messages intact.
4. you inspect `[System services]` and see your running
   services.
5. you kill a service deliberately; it restarts per policy.
6. you add a new service by defining a value and registering
   it — no restart needed; it just starts running.
7. you edit the init script in your workspace, reload it,
   observe behavior change on next boot — or live, for
   late-bound handlers.
8. you send your image to a friend; they resume it; their
   machine is running what yours was.

when we pass that test, moof is a system in the kay-engelbart-
plan9-smalltalk sense. the surface (morphic, canvas, whatever)
goes on top of this substrate. the substrate is the thing that
makes the surface live up to its ambitions.

---

*everything above depends on the foundations being solid:
content-addressing (foundations.md phase 1), append-only log
(foundations.md phase 2), cancellation (foundations.md phase
3). they're prerequisites, not co-timers. get foundations
right first; then build the system on top.*
