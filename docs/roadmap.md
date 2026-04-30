# roadmap

> **what we build, in what order. each phase has a single forcing
> function. earlier phases are not allowed to assume later phases.
> we resist building anything before its phase. v4-take-2 sequencing,
> revised after the 2026-04-29 audit.**

the headline changes from the previous roadmap:

- **the demo is "shared 3D zoomable world"**
  (`concepts/world-and-space.md`), not "moofpaint app." pixmaps
  (`concepts/pixmap.md`) are *one inhabitant proto* among many.
- **the rust line is small.** the substrate seed is ≤3k LoC of rust;
  everything performance-sensitive (lmdb, blake3, ed25519, wgpu,
  websocket, terminal i/o) ships as mcos
  (`concepts/compiled-objects.md`).
- **parser + compiler self-host immediately after phase A.** the
  bootstrap rust parser/compiler are throwaway; parser.moof and
  compiler.moof take over at phase A-self-host.

every phase before the demo earns its keep by removing a substrate
gap that the demo exposes. every phase after polishes the platform.

before reading: skim
[`docs/process/audit-2026-04-29.md`](process/audit-2026-04-29.md)
for the rationale, and
[`docs/process/state-of-the-implementation.md`](process/state-of-the-implementation.md)
for what the previous attempt missed.

## phase 0 — vision and docs

**status:** complete (this folder).

**done.** vision, manifesto, lineage, concepts/*, syntax/*, laws/*,
process/*. the docs read coherently; the brainstorm round of
2026-04-29 surfaced and patched the load-bearing gaps.

## phase A — substrate seed

**forcing function:** `moof '(+ 1 2)' → 3`, with every law in
`laws/substrate-laws.md` either honored or doc'd as deferred.
≤3k LoC of rust. the seed is the *only* rust binary we expect to
keep growing for a long time.

**deliverables:**
- one Form heap kind. tagged-immediates for nil/bool/int/sym;
  reflection through implicit proto.
- methods are Forms (closures with `proto: Method`). chunks are
  Forms. `Object` proto with the reflection method set installed.
- proto-chain dispatch with inline caches.
- bytecode interpreter (~30 ops).
- bootstrap reader/compiler in rust (carry `src/reader.rs` /
  `src/sym.rs`; rewrite the rest).
- **mco loader** (the substrate's tiny native-loading mechanism).
- bootstrap stdlib in moof.
- a `moof` cli binary that runs a single expression.

**not in scope:** persistence, vats-as-actors, replication, types,
real parser-in-moof, GC, defs in moof.

## phase A-self-host — parser and compiler in moof

**forcing function:** `parser.moof` parses its own source;
`compiler.moof` compiles its own source; both produce identical
trees/chunks across runs. the rust parser/compiler are quarantined
behind a debug flag, used only to bring up new compiler.moof
versions.

**deliverables:**
- `parser.moof` (~800 LoC) — the production parser. supports the
  full surface (`syntax/*`).
- `compiler.moof` (~700 LoC) — the production compiler. supports
  multi-clause patterns, defproto, defop, super sends.
- self-hosting test suite.

after this phase the rust line stops growing; everything new is
moof or mco.

## phase B — single-vat persistence

**forcing function:** a vat saves to disk per turn; quit and reboot
restores its state. `(println "hi")` works through `$out` cap that
survives reboot.

**deliverables (mostly mcos and moof; rust seed grows only ~+500
LoC):**
- per-vat heap (vat-local FormIds).
- vat scheduler (single vat for now).
- intent/receipt model for cap effects
  (`concepts/effect-intents.md`).
- mcos: `core/canonical-encoder`, `store/lmdb`, `os/clock`,
  `os/random`, `os/console`.
- bootloader: mmap, replay journal-tail.
- mark-sweep GC at turn boundaries.

**not in scope:** replication, types, datalog, distribution.

## phase C — moldability foundations

**forcing function:** edit a method via `(set-handler! Counter
:incr ...)`; the next call uses the new code. existing inline caches
invalidate.

**deliverables:**
- proto-handler mutation + inline-cache invalidation.
- `does-not-understand:` extension hook.
- multi-clause pattern-matched defs.
- text-line inspector `[obj inspect]`.
- bytecode-from-source recompile on edit.
- **test matrix**: live-edit a method, see new-method dispatch on
  the next send; multi-clause exhaustiveness; pattern destructuring
  for List/Table.

**ratio:** ~500 lines of rust + ~1500 lines of moof.

**deferred to phase D:** `become:` (until id-indirection lands).

## phase D — replicated vats (in-process)

**forcing function:** a single rust process holds **two replica
vats** of one logical vat. an in-process reflector feeds both the
same totally-ordered input log. after every turn, the substrate
asserts `canonical_hash(replica_a) == canonical_hash(replica_b)`.

**deliverables:**
- vat-mode at birth: `:solo | :replicated-leader |
  :replicated-follower` (`concepts/replication.md`).
- determinism-laws enforced in rust (`laws/determinism-laws.md`):
  deterministic alloc order, ordered hashmap iteration, no wall-
  clock during a replicated turn, GC at turn boundaries, deterministic
  promise ids.
- the turn-envelope shape `(session-id, epoch, turn-seq, author,
  logical-now, input-event, seed)`.
- a tiny in-rust reflector (orders user-inputs and effect-receipts
  only; not in cap traffic path).
- the canonical-hash function over a vat's heap.
- intent-receipt round-trip in the replicated case.
- `become:` (with id-indirection now feasible).
- **test matrix**: 10k random inputs, two replicas, hash-equal at
  every turn. fault injection (drop one replica; rejoin from
  snapshot; catch up via input log). proto-edit-as-input convergence.

**ratio:** ~1500 lines of rust + ~1000 lines of moof.

## phase E — single-user 3D world

**forcing function:** one user, one terminal. boots a world with
several inhabitants — Pixmaps, a Counter, a Cube, a Scratchpad.
flies the camera around in 3D, double-clicks to focus, edits
inhabitants in place. canvas persists across reboot.

**deliverables (almost entirely moof + mcos):**
- Frame, Placement, Pose, Camera, Viewport protos
  (`concepts/world-and-space.md`).
- the universal `:render-with: ctx` protocol on every visible Form.
- inhabitant protos: Pixmap, Counter, Scratchpad, Cube,
  ToolPalette, Inspector.
- per-user undo for pixmaps.
- mcos: `render/terminal` (3D software rasterizer to half-blocks /
  braille), `input/xterm-mouse`, `input/xterm-keys`,
  `pixel-bits`, `math3d`.
- world-vat (replicated mode, but only one replica live in this
  phase).
- wrapper vat (solo) bridging $canvas/$pointer/$keyboard to the
  world-vat via input envelopes; ray-casts in the wrapper.

## phase F — multi-user 3D world (websocket)

**forcing function:** alice runs `moof world ./worlds/test/`. bob
runs `moof world join wss://localhost:7878`. both inhabit the same
3D world — see each other's cursors, edit pixmaps and counters in
place, see live-edits to tool protos propagate within reflector
tick (50ms). bob disconnects mid-stroke, reconnects, converges.

**deliverables (one new mco; everything else moof):**
- mco: `transport/websocket`.
- handshake / authentication via ed25519.
- snapshot transfer.
- reconnect with epoch.
- leader failover.
- Cursor inhabitant proto (presence as a first-class Form).

## phase G — gpu rendering, browser, polish

**forcing function:** stretchy. "the world renders via wgpu at 60fps
on a desktop; alice and bob are on different machines and one of
them joined via a browser tab; the world persists for a week of
intermittent edits."

**deliverables:** mcos `render/wgpu` (gpu renderer), `render/web`
(browser via wasm), `format/png` (pixmap export); session
persistence as on-disk artifact (shareable); long-lived user
identity (ed25519 keys as Forms).

## phase H+ — everything else

real type system (nominal + structural; refinement deferred);
datalog queries; APL-flavored Tables; `defop` / user macros;
hyperCard accessibility studies; profile-and-tune the substrate
based on moofpaint workloads.

## guidelines for living with this roadmap

- **no skipping ahead.** writing code for phase F before phase D
  passes is unsafe.
- **no skipping back.** if phase D forces a redesign of phase A,
  fix phase A first.
- **forcing functions are not optional.** "phase n is done" means
  the forcing function passes and the test matrix is green.
- **the docs lead.** if a phase requires something undocumented,
  the docs go first (`process/docs-driven.md`).

## inspirations

- the rust 2015 / 2018 / 2021 edition rollout: phased substrate
  evolution.
- the smalltalk-80 bootstrap.
- the maru posture (piumarta).
- croquet's tea-time replication discipline (kay, reed, smith).

## see also

- `vision/manifesto.md` — the why.
- `process/docs-driven.md` — the discipline.
- `process/audit-2026-04-29.md` — what changed about this roadmap.
- `process/impl-plan-v4.md` — the concrete day-by-day next steps.
