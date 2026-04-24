# roadmap

**type:** roadmap

> where moof is, where moof is going, and in what order. revised
> as the project learns. written in the spirit of "we commit to
> the trajectory, not to dates."

---

## now — wave 9

### shipped (wave 9.0–9.4)

- **9.0 — namespaces as trees.** URL value type. Table.walk:.
  moof:/caps/ and moof:/vats/ paths addressable. FarRefs carry
  URLs; re-resolved on image load.
- **9.1 — unified Native.** native handlers become Block-proto
  heap objects with a `native_idx` slot. identical dispatch
  semantics as moof closures. `handlerAt:` stops leaking raw
  symbols.
- **9.2 — native arity declarations.** selector-based arity
  inference plus explicit `register_native_arity`. classic lisp
  quine works.
- **9.3 — bignum-Integer unification.** i48 + BigInt under one
  `Integer` type. typeName, hash, arithmetic, equality cross the
  backing. overflow promotes, small results demote.
- **9.4 — service registry skeleton.** System + Registry +
  six built-in Service declarations. `[System services]`,
  `registerService:`, `serviceStatus:`. metadata only — spawners
  are thunks pending wave 9.5.

### in progress

- **9.5 — Scheduler as a capability.** exposes
  `[Scheduler spawnCapability: 'clock]` as a moof-level send.
  service spawner thunks become callable. 1–2 sessions.

### next

- **9.6 — boot inversion.** vat 0's moof-side System owns boot.
  rust main hands System a manifest, calls `[System boot:]`.
  `shell/repl.rs` becomes a moof Interface call.

---

## the jubilee (between 9 and 10)

dedicated cleanup wave. NO new features. rule-based deletion.

1. **doc exodus.** every "wave X.Y" comment in source → either
   `docs/exemptions.md` or deleted. source describes what IS,
   not what SHOULD BE.
2. **broken-or-kill.** fix or delete:
   - `pattern.moof:97` match-constructor with undefined `Env`
   - `query.moof:52` [any] admitted-broken (already fixed)
   - Thenable.map: for Cons (broken path)
   - services.moof dead spawner thunks (wait for 9.5 before
     deciding)
3. **protocol audit.** every defprotocol must have ≥3 conformers.
   Reference (deleted), Interface, Buildable → go. Thenable →
   split into Monadic + Fallible + Awaitable.
4. **pick one, delete the other.**
   - Transducer over Query (convert call sites or delete Query)
   - defmethod over [Proto handle: with:] (convert or document
     why handle: survives)
5. **directory sanity.** `data/act.moof` → `kernel/act.moof`.
   `kernel/namespace.moof` → `system/namespace.moof`.
6. **laws as commit.** `docs/laws/` is the constitution; every
   remaining violation gets a named exemption.

target: ~1 week concentrated work. ~1000 lines smaller. coherent.

---

## wave 10 — the image-first boot

**the big one.** moof stops replaying source at startup and starts
hydrating a content-addressed seed image.

### 10.0 — refactor Heap

extract `SymbolTable`, `ProtoRegistry`, `Arena` as separate types
composed inside Heap. no behavior change. 1–2 sessions.

### 10.1 — `moof build`

new command. walks the manifest's bootstrap list, evals each into
a fresh heap, writes the resulting image as
`.moof/seed.bin` (content-addressed). 2 sessions.

### 10.2 — dual-mode `moof run`

if `.moof/seed.bin` exists, hydrate from it. else fall back to
source replay. verify every current test passes in seed mode.
1–2 sessions.

### 10.3 — kill the fallback

delete `scheduler::bootstrap_sources`. `spawn_vat` becomes a fork
from a seed-image marker. defservers at bootstrap now work
without recursion. 1 session.

### 10.4 — registry as defserver

now that law 1 and law 5 are satisfied, rewrite
`lib/system/registry.moof` as a proper defserver. remove the
wave-9.4 exemption. 1 session.

---

## wave 11 — running-state persistence

**the hard technical lift.** extend the image to include in-flight
computation.

- VM frames become HeapObject-backed.
- mailboxes + outboxes serialize.
- Act chains serialize.
- on load, scheduler reconstructs runnable set from persistent
  mailboxes.
- closures with captured env already serialize; verify round-trip
  on real workloads.

**the payoff: reboot is continuity.** close your laptop, open it,
your vats resume mid-computation. pending Acts are still pending.
a vat waiting for a send sees the send arrive whenever the sender
runs again.

1–2 months. this is the deep thing.

---

## wave 12 — supervision + capability hardening

- OTP-vocabulary supervisors: `supervisor`, `application`,
  gen_server-as-defserver, gen_event-as-reactive-signal,
  gen_statem-as-finite-state-defserver
- supervision policies (always, on-failure, never, escalate)
- restart on crash, adoption of orphans
- capability revocation
- membranes (attenuated FarRefs)
- grant events recorded in an append-only log visible via System

we take OTP's vocabulary wholesale (see
[vision/lineage.md](vision/lineage.md)) — decades of production
refinement we'd be foolish to reinvent.

2–3 weeks.

---

## wave 13 — the canvas

the first visual interface.

- Renderable protocol (one render: method per medium)
- vello-backed canvas for vector rendering
- halo primitive: click-and-hold → ring of verbs
- aspect stacking (one object, many views)
- inspector as a moof-level widget (subclassable)
- direct-manipulation handler editing (edit source, live effect)
- skeuomorphic AND typographic visual idioms supported per view
  — no house aesthetic, authored per type. see
  [vision/horizons.md](vision/horizons.md).

3–4 weeks. the gate between "moof works" and "moof is a medium."

---

## wave 14 — the agent

LLM-in-a-vat as a first-class participant.

- LLM capability (call model APIs from a vat)
- agent vat with bounded capabilities + membrane
- tool registration: moof handlers as agent-callable
- conversation history as persistent moof objects
- moof introspection tools (inspector-as-tool, etc.) given to agent

2–3 weeks (the moof side). the LLM side depends on external
APIs.

---

## wave 15 — federation

FarRefs across machines.

- protocol-over-socket (WebSocket or similar)
- peer identity + keys
- content-addressed cache for federated immutable values
- subscription (you follow someone's workspace; their changes
  arrive as Updates)
- conflict resolution as conversation

1–2 months.

---

## beyond

the roadmap past wave 15 is direction, not plan:

- **authoring-for-all**: conform-button, halo-edit,
  view-protocol-picker as canvas gestures
- **`defshape`**: explicit slot-type contracts for data shapes
  (see [concepts/schemas.md](concepts/schemas.md)). optional
  rigor; the emergent-default stays.
- **optional static type layer**: haskell/typescript-style
  annotation-and-check sitting above the dynamic substrate. the
  protocol machinery is already shaped to accept it. sketched in
  the archived [type-system.md](archive/type-system.md).
- **first-class streams with backpressure**: push-based producer
  coordination layered over the pull-based streams that ship
  today (see [concepts/streams.md](concepts/streams.md)).
- **full-text search** as an index-server
- **reactive views**: dashboards that update live
- **time-travel as navigation**: scrubbable past states
- **migration tooling**: user-authored migrators for schema
  evolution
- **headless/server mode**: moof as a backend service, not just
  a REPL
- **recovery mode**: boot into bare repl when image init fails
- **multi-image**: one moof binary hosts multiple images
- **log compaction policy**: when to snapshot and truncate
- **hot init reload**: edit init script, running system adapts

each of these is a future wave. each will have a spec before it
starts.

---

## what's NOT on the roadmap

- **static type system as a replacement for the dynamic core.**
  we do not plan to rebuild moof around haskell-style type
  checking. but an OPTIONAL typed layer that you annotate when
  you want checking is a legitimate future wave — see
  `beyond` above.
- **tell-layer DSL.** grammar-based natural-language command
  input was an exploration; no current intent to ship.
- **morphic-on-vello as specified.** the specific renderer
  experiment; canvas will use vello but the structural approach
  is different now.
- **AI as compiler.** we use LLMs via capability, not as part of
  the compilation pipeline.

these live in the archive as historical exploration.

---

## what to do if the roadmap is wrong

it will be. the roadmap is revised as we learn.

- a wave takes longer than estimated — update the estimate, not
  the plan.
- a wave reveals a prereq we missed — insert a wave, push later
  ones back.
- a wave solves a problem faster than expected — bring the next
  wave forward.
- we realize a wave was the wrong approach — archive it, design
  the replacement.

what we don't do: ship a wave partially and leave "TBD wave X"
comments in source. a wave either completes with its commitments
or gets replanned before completion.

---

## status signals

when looking at moof, the signals that tell you where it's at:

- **can you REPL in it?** → 9.4 state (yes)
- **can you save + restart and see your work?** → 9.4 (yes, minus
  running state)
- **does `[System services]` work?** → 9.4 (yes)
- **does `moof build` exist?** → 10.1+ (no)
- **does reboot resume in-flight Acts?** → 11+ (no)
- **is there a canvas?** → 13+ (no)
- **does the agent collaborate on the workspace?** → 14+ (no)
- **can you federate?** → 15+ (no)

today we are at 9.4. about 7-9 months of concentrated work to
wave 15. more for the beyond items.

---

## in one sentence

> **jubilee → image-first boot → running-state persistence →
> supervision → canvas → agent → federation.** each unblocks the
> next. the trajectory is committed; the timing is what it is.
