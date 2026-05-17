# phase 3 — the cohesive vision: image-as-canon, polyglot players, native
# laziness, compaction, easy mcos, ergonomic vats, 1,000,000× faster

> **status:** brainstormed 2026-05-16. ready for sub-area implementation
> plans. **load-bearing roadmap document for the next several months.**
>
> **scope:** ties phase 1 (gc + dispatch + compression — shipped),
> phase 2 (perf tier 1/2/3 — in progress), and the vats spec V4-V11
> (BEAM-style features — spec'd, mostly not implemented) into a single
> cohesive picture. defines the *experiential* end-state and the
> sequencing-with-forcing-functions that gets us there. NO CODE
> CHANGES — spec only.
>
> **prior reading (in order of relevance):**
> - `docs/roadmap.md` — overall phase plan (phase A → phase G)
> - `docs/vision/manifesto.md` — the why, four faces of Form
> - `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md` — the self-host arc (W1-W5)
> - `docs/superpowers/specs/2026-05-11-phase1-gc-dispatch-compression-design.md` — phase 1 (shipped)
> - `docs/superpowers/specs/2026-05-16-phase2-moof-performance-design.md` — phase 2 (in progress)
> - `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` §22 V1-V11
> - `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4 image format §10
> - `docs/concepts/compiled-objects.md` — the mco model
> - `docs/concepts/vats.md` + `docs/concepts/references.md` + `docs/concepts/replication.md`
> - `docs/laws/substrate-laws.md` L1-L16; `docs/laws/determinism-laws.md` D1-D12
> - `NEXT_SESSION.md` — V4 polyglot state at HEAD `0319c10` (post phase 1; mid phase 2)

---

## table of contents

1.  the cohesive vision — image-as-canon, polyglot reframe
2.  native laziness in the VM
3.  VM compaction (generational + forwarding)
4.  easy mco hooking
5.  Erlang-style vat concurrency (user-facing)
6.  the path to 1,000,000× faster (tier 3 perf)
7.  the user-facing layer (REPL, inspector, debugger, distribution)
8.  sequencing — the version ladder
9.  risks + open questions
10. what makes moof AMAZING

---

## §1 — the cohesive vision: image-as-canon, polyglot reframe

### 1.1 reframe: "polyglot" is a testable property, not a buzzword

"polyglot" sounds like a smug architectural label. it isn't. it's an
**empirically falsifiable claim** about the artifact we ship.

> **the claim:** any host (a "player") that conforms to the V4 image
> format specification can boot any moof `.vat` image and serve as a
> moof world. the `.vat` IS the moof artifact; the substrate is just
> a player.

this reframes the entire stack:

- the **substrate** is a player. it does not own truth. it loads truth.
- the **`.vat` image** is the truth — a content-addressable, byte-
  deterministic encoding of a complete World (forms, syms, chunks,
  bytecode, ICs, side-tables, here_form, macros_form, native bindings
  by-name). per `2026-05-10-vm-V4-opcodes-design.md` §10.
- a **moof player** is anything that can (a) read the image, (b)
  reconstruct the World in its host memory, (c) execute the V4 bytecode
  ISA against it, (d) honor the laws (L1-L16, D1-D12).

today we have one player: `crates/zig-substrate/`. tomorrow we
ship a wasm player (browser), a c player (embedded), and possibly
others (rust-runtime as nostalgia; ocaml as a curiosity). every one
of them must pass the same conformance test suite, byte-for-byte, on
the same image corpus.

this is the **JVM-classfile move**, applied to a live image rather
than a class. it's also the **CLR-IL move**, applied to a richer
runtime model (vats, mailboxes, persistence). but the fact that the
artifact is *a fully-bootstrapped live world*, not a compiled
executable, is what makes it moof's own.

### 1.2 multi-host strategy

we proceed in four host-platform waves, each forced by a concrete
demand:

| wave | player | host stack | forcing function |
|---|---|---|---|
| **H1 (today)** | `crates/zig-substrate/` | native zig 0.16, darwin/linux/wasm | self-host completes; rust deletes; `[1 is nil]` works through stdlib |
| **H2** | wasm-browser | zig → wasm32 + js host shim | someone loads moof in a browser tab; canvas inhabitant renders; no rust on the wire |
| **H3** | c-portable | hand-written c11 with portable allocator | embedded / constrained / non-zig-platform deployment (toolchain stories beyond zig) |
| **H4** | exotic | maybe ocaml-as-player; maybe rust-comeback; maybe a hand-rolled webassembly interpreter player | a host with a constraint zig can't satisfy (RTOS? bizarre arch?) |

H2 is the load-bearing one for the demo: the phase E shared-world
demo (`roadmap.md` phase E + F) wants both desktop and browser users
on the same session. a browser player is the cleanest path.

**we do not need to ship H3/H4 in 2026.** but the **conformance test
suite** must be designed such that they *could* be shipped — i.e. no
hidden zig-specific behavior creeps into the image format.

### 1.3 the conformance test suite

we define a corpus `tests/conformance/` of `.vat` images plus a set
of `(image, message, expected-result)` triples. a player passes the
suite iff, for every triple, loading the image, sending the message,
and reading the result produces the byte-identical expected result.

the corpus must cover:

- **dispatch correctness** — mono-IC, polymorphic IC, PIC eviction,
  super-send, eval, become.
- **arithmetic + comparison** — Int+Int fast-paths, BigInt overflow,
  Char comparisons, Float (when added).
- **strings + collections** — String:at, String:length, Cons:car/cdr,
  list-map, table-at-put.
- **laziness** (when shipped — see §2) — thunk force, delay/force
  identity, lazy-cons enumeration, force-of-already-forced.
- **vats** (when shipped — see §5) — local message, far-ref message,
  promise resolution, supervisor restart, replication hash-equal.
- **mcos** (see §4) — load mco, call native, foreign handle lifecycle,
  cross-vat foreign-handle refusal.
- **gc** — mark+sweep reaches all roots; tombstones reclaim; L11
  preserved; far-ref table integrity.
- **persistence** (when shipped) — save vat, load vat, byte-identical
  heap; replay journal from snapshot.
- **determinism** — same image + same input log + same player ⇒
  same canonical-state-hash, every turn.

each test is a triple emitted as JSON in `tests/conformance/manifest.json`:

```json
{
  "image": "test-corpus/integer-arith.vat",
  "send": ["here", "eval", ["[1 + 2]"]],
  "expect-canonical": "abc123...",
  "expect-stdout": ""
}
```

a player implements a `moof conform <manifest.json>` command. CI runs
it on every player on every commit. **drift between players is a
shipping-blocker bug**, not a "compatibility note."

practical sizing: aim for ~200 conformance triples by phase E. ~50
covering the substrate-laws + dispatch corner cases. ~100 covering
stdlib protocols. ~50 covering vats + replication + persistence.

### 1.4 image format versioning

per `2026-05-10-vm-V4-opcodes-design.md` §10, the image format is
the contract. and per phase 1 §5.3, we already had to bump version
0x0004 → 0x0005 for the compression byte. that hurt because no one
had a v0x0004 file in the wild — but a `.vat` *can* outlive the
substrate that wrote it, and as soon as anyone saves a vat to disk,
**version stability becomes legally binding**.

design rules:

1. **semver-style versioning.** the image header carries
   `major.minor.patch`. format changes:
   - **patch bump** — additive, backward-compatible (new optional
     section; new opcode in a reserved range; new compression
     algorithm).
   - **minor bump** — backward-compatible but new features require
     readers to be at least this version (e.g. new mandatory metadata
     field).
   - **major bump** — wire-incompatible. requires migration tool.
2. **freeze v1.0 at v1.0 binary release.** any change after that
   ships with a migration story. major bumps **must** be accompanied
   by a `moof migrate <old.vat> --to v2 --out <new.vat>` tool.
3. **migration tools are themselves moof code.** the v1→v2 migrator
   is a moof program that loads the v1 image (via a v1-compatible
   reader, kept in the player for one major-version of history), walks
   the World, and emits a v2 image. **the migrator must be byte-
   deterministic.**
4. **reserved-bits / reserved-opcodes.** every byte in the format
   has reserved ranges. v4 ISA already reserves 5 of 24 opcode bytes
   per the V4 spec. compression byte reserves bytes 2/3 for lz4/brotli.
   FormId reserves the `11…` top-bit scope.

what we do *not* support:

- forward compatibility (an old player loading a new image). new
  players load old images; old players reject new images cleanly with
  "version too new, please upgrade."
- arbitrary-order section parsing. sections are emitted in a fixed
  order; readers consume them in that order. otherwise determinism
  is impossible.

### 1.5 the five underlying goals this serves

the polyglot reframe serves five compounding goals:

1. **self-host purity.** when the substrate is "just a player," there
   is no temptation to bury behavior in zig that should live in moof.
   the player's job is to load+execute; everything else is moof code
   stored in the image. (this is the *moldability claim*: if behavior
   isn't in the image, users can't mold it.)

2. **host portability.** moof runs wherever a player runs. browser,
   embedded device, raspberry pi, server. the image-is-canonical
   discipline makes "port moof to platform X" a small project: write a
   player, pass the conformance suite, ship.

3. **image-as-medium.** people can share a `.vat` the way they share
   a `.pdf`. it boots into a complete world with the sender's
   inspector layout, scratchpad notes, half-typed expressions, open
   debugger frames. **this is the HyperCard "stack" idea, modernized**
   — the medium of communication is a live world, not a static
   document.

4. **the demo (phase E shared 3D world).** the demo needs to be
   shareable. a `.vat` you can email someone. or a `.vat` hosted at a
   URL that a browser player loads and joins. polyglot makes this
   trivial — no need for "the moof installer" before bob can see
   alice's world.

5. **hacker reach.** a 3MB self-contained wasm-player + 5MB compressed
   image = 8MB drop-in moof world that runs in any browser tab. that's
   the recruitment story. people don't install; they click.

each of these is a *forcing function* on later phase work. if a
candidate design choice doesn't serve at least one of these, we should
question it.

### 1.6 the artifact's identity

a `.vat` has three identifying properties:

- **content-hash** (blake3 of canonical bytes). per V4 §10.9.
  determinism-law D9. this is the *value identity*.
- **vat-id** (UUIDv7 from the vat header). per `concepts/vats.md`.
  this is the *historical identity* — the same vat across edits.
- **path-name** (in the world's path-table). per
  `concepts/references.md` path-ref. this is the *named identity*.

an image can be referenced by any of these. content-hash is the
hardest-coded; path-name is the most ephemeral. vat-id sits between.
sharing-by-link uses path-name; sharing-by-cache uses content-hash;
in-process referencing uses vat-id.

### 1.7 the polyglot test, in one sentence

> **if `moof-zig conform manifest.json` passes and `moof-wasm conform
> manifest.json` passes against the same `.vat`s, we have polyglot.**

everything in this spec exists to make that sentence true and keep it
true.

---

## §2 — native laziness in the VM

### 2.1 motivation: thunks unify with promises

moof currently has neither thunks nor promises as first-class VM
concepts. closures-as-thunks works (`(fn () expr)` + `[t value]`),
but it's:

- expensive (full closure-Form + chunk + IC dispatch),
- not memoized (every `value` re-evaluates),
- unaware of forcing context (no `force` opcode that auto-memoizes
  + auto-unwraps).

once we ship V7 (eventual sends + promises), the same shape recurs:
a Form that holds a future value, transitions through states, and
caches the result on resolution. **thunk and promise are the same
abstract pattern.** unifying them in the VM gives us:

- one face (`proto: Thunk` ⊆ `proto: Lazy` ⊆ `proto: Promise`),
- one force/await primitive,
- one resolution state machine,
- coherent interop between sync (thunk) and async (promise) laziness.

### 2.2 the Thunk Form face

a Thunk is a Form with:

| slot/meta | value | semantics |
|---|---|---|
| `proto` | `Thunk` (subtypes `Promise` for cross-vat parity) | identity |
| `:body` | chunk-FormId, or closure-FormId, or far-ref | what to compute |
| `:env` | env-FormId | the captured environment (nil if body is a far-ref) |
| `:state` | `#pending` \| `#forcing` \| `#ready` \| `#broken` | the resolution state |
| `:value` | the cached value (after `#ready`) | memoized result |
| `:reason` | the error (after `#broken`) | for `when-broken` |
| `:observers` | optional list of callbacks for cross-vat / async | |

state transitions:

```
#pending  ──[force]──>  #forcing  ──[done]───>  #ready
                                  ──[raise]──>  #broken
#ready    ──[force]──>  return cached :value
#broken   ──[force]──>  re-raise :reason
#forcing  ──[force]──>  error 'recursive-force (cycle detected)
```

"liveness face": a Thunk is *almost* alive — it has a behavior (its
body) and a kind of mailbox (it accepts only one message, `force`,
once). the Promise extension to Thunk adds explicit mailbox-shape for
cross-vat — `when-resolved:`, `when-broken:`, `then:` queue handlers
when state is `#pending`.

### 2.3 explicit `force` vs implicit auto-force

two philosophies:

- **explicit** (recommended): `[t force]` returns the value. user code
  is in control of *when* evaluation happens. semantic clarity wins;
  no debugging surprises like "this 'value' is actually a thunk and
  evaluated during print."
- **implicit** (haskell-style): any dispatch on a thunk first forces
  it. surface code never sees thunks. ergonomic; but every send
  carries a thunk-check overhead, and laziness becomes invisible —
  debugging "why is this slow?" requires substrate-level reasoning.

**decision:** explicit force is the primitive. ergonomic sugar lives
above:

- `(force x)` macro: dispatches to `[x force]` if `[x is-a Thunk]`
  else returns `x` unchanged. **this is the ergonomic primitive.**
- `(delay expr)` macro: expands to `[Thunk body: |_| expr env: $here]`.
- `(lazy expr)` synonym for `(delay expr)`.
- `(force* x)` macro: deeply forces — walks the value, forces any
  thunks reached. analogous to `deepFreeze`.

we do **not** auto-force on send. when a thunk's `:body` is invoked
explicitly by `[t force]`, the body's return value becomes `:value`.
the **sender** decides when forcing happens.

**rationale**: lazy values are first-class. you can pass them around
unforced (printf-debug a value without computing it). you can put
them in tables, in slots, in collections. you can map a function
across a list of thunks producing a list of new thunks (zero
computation). only `force` causes computation.

### 2.4 lazy cons lists and streams

once thunks exist, lazy lists drop out:

```moof
(defmacro lazy-cons (head tail-expr)
  `(cons ,head (delay ,tail-expr)))

(def ints-from (fn (n) (lazy-cons n (ints-from (+ n 1)))))

(defmethod LazyList :take: (n) ...)   ; walks, forcing as it goes

[(ints-from 1) take: 10]            ; → (1 2 3 4 5 6 7 8 9 10)
```

key design choices:

- a lazy-cons is **just a cons cell** whose `:cdr` happens to be a
  thunk. nothing structurally special at the Cons proto.
- the iteration combinators (`:take:`, `:map:`, `:filter:`, etc.)
  use `(force ...)` to advance — they handle thunks-or-values
  uniformly.
- finite lists work in lazy combinators naturally — non-thunk `:cdr`
  short-circuits.

bonus: this is **streams** for free. a tcp socket as a lazy-cons-of-
bytes, a file as a lazy-cons-of-lines, a keyboard as a lazy-cons-of-
events. composes with the data-sources protocol
(`concepts/data-sources.md`) — a data-source can present itself
either pull-style (`[ds next]`) or lazy-stream-style (`[ds as-lazy-
cons]`).

### 2.5 (delay expr) / (force expr) primitives

surface:

```moof
(def x (delay (expensive-compute)))   ; no compute happens
[x state]                             ; → #pending
(force x)                             ; computes; returns result
[x state]                             ; → #ready
(force x)                             ; returns cached; no recompute
```

surface for explicit Thunk creation (less common — most users use
(delay):

```moof
(def x [Thunk body: |_| (expensive) env: $here])
[x force]
```

substrate intrinsic: `Thunk:force` is a native that:

1. reads `:state`; short-circuits if `#ready` / `#broken`.
2. sets `:state := #forcing` (catches recursive force).
3. invokes `:body` with `:env` captured. the result becomes `:value`.
4. transitions `#forcing → #ready`; notifies any observers (for
   the Promise subclass).
5. on body error: `#forcing → #broken`; sets `:reason`; notifies.

### 2.6 compiler: explicit wrap; eager by default

the compiler does NOT auto-wrap expressions as thunks. eager
evaluation is the default. `(delay expr)` and `(lazy-cons ...)` are
explicit user wrappers.

exception: an experimental `(lazy ...)` form lets you mark a let-
binding lazy:

```moof
(let ([x (lazy (expensive))])   ; x bound to a Thunk
  ... (force x) ...)
```

this is sugar over `(let ((x (delay (expensive)))) ...)`. clarifies
intent at the binding site.

### 2.7 interaction with the four faces

laziness is an *overlay* on the existing structure + identity faces.
specifically:

- **structure face**: a thunk *is a Form*. it has a head/args
  structure (its `:body` is the abstract source-form of the
  computation; `(thunk-source t)` returns the form). lazy-cons is
  structurally an ordinary cons whose cdr is a thunk.
- **identity face**: `proto: Thunk` participates in the proto chain
  like everything else. users can subclass Thunk to add domain
  semantics (`MemoCache`, `IOAction`).
- **liveness face**: thunks are *latent* life — they become alive on
  force. Promises (cross-vat thunks) have full liveness from spawn.
- **history face**: forcing is recorded in `:meta` (when journaling
  is on). `:meta.first-forced-at` is a timestamp; `:meta.forced-by`
  is the source position. observability for "what evaluated when."

### 2.8 interaction with vats: promises are cross-vat thunks

within a vat, `Thunk`. across vats, `Promise` (which has Thunk as a
proto, adding observer machinery + the `when-resolved:` /
`when-broken:` / `then:` API per `concepts/references.md`).

unified picture:

```
                Thunk
                  ↑
                Promise
                  ↑
        ┌─────────┴─────────┐
        │                   │
  LocalPromise         RemotePromise
  (within-vat,         (cross-vat,
   sync force)          async observe)
```

a `[far-ref message: arg]` returns a Promise. you can `(force ...)`
that promise, but force-on-a-Promise within a vat means "block this
turn until resolution" — i.e. equivalent to the existing
`:sync-await:` per references.md. discouraged but possible (and per
V7 spec, *visible*: `sync-await:` is the user-syntax verb).

simpler model: **promises and thunks are dispatch-compatible**. any
code expecting a `(force ...)` works on either. that's the
moldability win — one set of stream-combinators, one
when-resolved-style API, one shape for "delayed value."

### 2.9 implementation pieces

| piece | scope | depends on |
|---|---|---|
| `Thunk` proto | stdlib `lib/early/thunk.moof` | nothing |
| `Thunk:force` native | zig substrate intrinsic | nothing |
| `(delay)` / `(lazy)` / `(lazy-cons)` macros | `lib/stdlib/lazy.moof` | macro system (works) |
| `LazyList` combinators in moof | `lib/stdlib/lazy.moof` | thunks |
| Promise proto + observers | `lib/early/promise.moof` | thunks + V7 vat-V7 |
| docs: `concepts/laziness.md` | new doc | nothing |
| conformance tests | `tests/conformance/laziness/` | thunks |

estimated effort: ~1-2 weeks for thunks alone; ~3-4 weeks once
LazyList + Promise unification + V7 land.

### 2.10 risks + open questions

- **debug-print surprises.** `print` on a Thunk prints the thunk, not
  its value. users may want auto-force in print. **decision: no.**
  print prints; `[t force]` then print if you want the value.
  inspector shows the Thunk's state-machine prominently.
- **recursive thunks.** `(define x (delay (force x)))` cycles.
  `'recursive-force` error catches it. tested in conformance.
- **memoization + mutation.** `:value` is set once; `:state` flips
  to `#ready`. is the thunk frozen after that? **decision: yes.**
  thunk transitions to frozen at `#ready`. ensures reproducibility.
- **laziness + replication.** in a replicated vat, force-order must
  be deterministic. since force is invoked explicitly by user code
  and user code runs the same on every replica, this is L13/D-laws-
  preserving. caveat: if a thunk's body uses ambient caps, the cap
  is the determinism risk — same as any other code.

---

## §3 — VM compaction (generational + forwarding)

### 3.1 current state recap

phase 1 (`2026-05-11-phase1-gc-dispatch-compression-design.md`)
shipped mark-sweep with no compaction. tombstones accumulate. recent
work (commit `9570852`) added free-list reuse for tombstoned FormIds,
which mitigates fragmentation at the cost of careful L11 semantics
(reused FormIds are *new* identities, not the same forms).

we also now have:

- per-vat nursery types (`b1494b1` — V1.0 Delta / TurnDiff / FaceKind)
- world turn lifecycle + nursery-aware r/w (`2b7eb80` — V1.1-V1.3)
- adaptive GC trigger on heap-growth threshold (`c08c865`)
- runTop wraps start/commit/abort turn (`81f89f0`)

**phase 3 finishes the generational story** by:

1. promoting the V1 nursery from "turn-local diff buffer" to "young
   generation."
2. adding write barriers so mature→young pointers are tracked.
3. adding compacting collection for the mature generation.
4. preserving L11 (FormId stability) via a forwarding table.

### 3.2 generational layout

per-vat heap splits into two generations:

```
Vat
├── nursery (young gen)
│   ├── small fixed-size arena (~64K Forms; reset every turn)
│   ├── FormIds carved from the 11… scope (reserved per V0 §5)
│   └── lifetime: one turn
└── mature gen
    ├── ArrayList<Form> as today (vat-local 00… scope)
    ├── tombstones from mark-sweep + free-list slots
    └── lifetime: vat-lifetime
```

allocation rules:

- new Forms born in nursery. cheap (bump-pointer).
- end-of-turn promotion: live nursery Forms copy into mature; their
  FormIds rewrite (nursery `11…` → mature `00…`); a forwarding map
  resolves any straggling references.
- nursery wholesale-reset after promotion. *no per-form sweep of
  nursery* — that's the generational win.
- mature gen GC'd less often, via mark-sweep (phase 1 algorithm)
  plus periodic full-compaction (this section).

(this exactly matches `vats-spec §22 V1` design, now operationalized.)

### 3.3 mature-gen compaction algorithm

semispace-style with **forwarding table**:

```
1. mark phase: walk roots, mark reachable Forms (phase 1 algorithm).
2. compute compacted layout: live Forms get new positions in a fresh
   ArrayList<Form>. ordering preserved (D5 wins; insertion order is
   determinism's friend).
3. build forwarding table: orig_form_id → new_form_id. *kept
   indefinitely* — L11 says original FormIds stay valid.
4. copy live Forms to new array; rewrite all Form-typed Value
   internal references using the forwarding table.
5. rewrite side-tables (chunk_bytecode, chunk_consts, chunk_ics,
   chunk_params, native_fns, proto_generation, far_ref_table) to use
   new FormIds.
6. install new array as mature gen; drop old.
7. forwarding table stays in `world.forwarding` as a chain
   (per heap.zig redirects model — capped at MAX_BECOME_HOPS=32).
```

**L11 preserved.** old FormIds resolve through the forwarding table
on every `heap.get`. the forwarding chain is short by construction:
post-compaction, every old id has at most 1 hop. consecutive
compactions could extend chains, but in practice we compact rarely
(say, every 10 mark-sweeps) and each compact resets prior chains
(rewriting forwarding entries to point at the freshest new id).

### 3.4 write barrier (mature → young)

generational GC's central trick: young-only collections are cheap
*because* we don't have to scan the mature gen. but if a mature form
holds a reference to a young form, that reference would be missed
(young form looks unreachable; collected; mature form now dangles).

solution: a **write barrier** on every slot/handler/meta mutation
that *writes a Form-typed Value*. if the receiver is mature and the
value is young, record the receiver's FormId in a **remembered set**
(a small per-vat hash set).

on young-only GC, the roots are:
- the usual roots (here_form, frames, etc.)
- **plus the remembered set** — every mature Form in the remembered
  set is treated as if it were a root for the young-only collection.

minor cost per mutation. zig generates this as a 4-line check after
the slot set:

```
if (target_id.scope == .vat_local and value.form_id.scope == .nursery) {
    world.remembered_set.put(target_id, {});
}
```

amortized ~5 ns/mutation. negligible.

### 3.5 compaction policy / triggering

three triggers:

- **young-only GC**: every turn-boundary (already wired). cheap;
  promotes live, discards dead.
- **mature mark-sweep**: every N young-only GCs (e.g. N=20). same as
  phase 1's existing collector.
- **mature compaction**: every M mark-sweeps (e.g. M=10), or when
  fragmentation ratio exceeds threshold (>50% tombstone).

each is replication-deterministic (only fires at turn boundaries; D6).

cost characterization:

| collection | freq | cost | scope |
|---|---|---|---|
| young-only | per turn | <1ms | nursery |
| mature mark-sweep | per ~20 turns | ~30ms @ 700K forms | mature gen |
| mature compaction | per ~200 turns | ~100ms @ 700K forms | mature gen |

target heap size at which these timings hold: ~1M forms. beyond
that, mark-sweep + compaction scale linearly with live-set, so a 10M
form heap takes ~300ms+1s. **acceptable for long-running federation
vats.** for interactive vats (the demo), per-turn timing budget is
~16ms (60fps); young-only fits; mature events should be infrequent
(monthly?) and might be moved to background.

future: **incremental mark** to amortize mature mark-sweep cost.
phase 4 territory.

### 3.6 L11 + the forwarding table

L11 (FormId stability) is non-negotiable. compaction would normally
violate it. forwarding tables save it:

- *active* FormIds (live forms) get rewritten everywhere they're
  internally referenced.
- *external* FormIds (held by user code, by other vats via far-refs,
  by serialized images) resolve through forwarding indirection.

forwarding is a O(1) hashmap lookup. once a held FormId is
dereferenced, it's "updated" — the user can opt-in to caching the
new id back at the call site.

post-compaction, two FormIds for the same form coexist temporarily
(old + new). they're equal via `Value.identity?` because both
forward to the same live form.

**determinism:** forwarding map iteration is insertion-ordered (D5);
compaction order is insertion-order on the original heap; thus same-
input vats produce same-layout vats post-compaction.

### 3.7 cross-vat interaction

a far-ref points at `(vat-id, form-id, cap-token)`. when the target
vat compacts, its form-ids rewrite. the far-ref's form-id becomes
stale-but-resolvable via the receiving vat's forwarding table. **the
sending vat's far-ref does not need updating.**

key invariant: far-ref form-ids are interpreted by the *receiving*
vat. as long as the receiving vat's forwarding chain resolves the
old id, the far-ref keeps working. forwarding is private to a vat.

### 3.8 serialization (image-write) compaction

opportunity: serializing an image compacts implicitly. only live
forms are written; their FormIds renumber starting from 1; no
tombstones survive the round-trip. **a vat-save is a free compaction
+ defragmentation event.**

care: post-save, the in-memory heap is still as it was
(if the vat continues running). only the on-disk image is compact. on
reload, the new id space is what gets reconstructed — which becomes
the next-session's canonical id space.

this is **L11 across reload**: stable within session, but a session
boundary is a legitimate rewrite point. users observing form-id
stability across sessions are doing it wrong (they should use
content-hashes or path-names).

### 3.9 interaction with capabilities

cap-tokens are unforgeable Forms (L9). compacting a cap-token's
underlying form just rewrites its id — the cap stays valid.

far-ref cap-tokens are valuable; lose one and the user loses access
to a remote vat. forwarding tables preserve them across compaction.
on reload, cap-tokens reconstitute with new FormIds.

vat-local Reference Forms (slot-ref, id-ref) survive compaction
trivially — they're just Forms with FormId-bearing slots; the slots
get rewritten like any other Form-typed slot.

### 3.10 risks + open questions

- **forwarding chains explode if compaction interleaves with
  `become:`.** `become:` already uses heap.redirects (capped at 32
  hops). compaction would add to that chain. **mitigation:** compaction
  *consolidates* redirects — after compaction, redirects table is
  empty (all references in the new layout point directly to new ids).
  `become:` after compaction starts fresh.
- **write-barrier cost on hot mutation paths.** measured ~5 ns; in
  e.g. parser AST construction this is ~100K mutations = 0.5 ms
  total. negligible. but if it isn't, we can lazily-batch the
  remembered-set into a thread-local buffer.
- **mature compaction during a long native call?** natives mustn't
  call `world.send` mid-iteration of a Form's slots if compaction
  could fire. **policy:** compaction only at turn-boundaries (D6).
  inside a turn, no compaction. natives are safe.
- **moof-visible compaction events?** users may want to observe
  "compaction just happened" for tooling. **proposal:** emit a
  `:compaction-event` notification through the world's
  observability stream. opt-in via `[$gc observe: my-handler]`.

---

## §4 — easy mco hooking

### 4.1 the goal

mcos (`docs/concepts/compiled-objects.md`) are the substrate's pressure
valve for native performance. they exist today but are clunky to
author. **make user-level fast paths trivial.**

target experience:

```moof
(defmco MyHashFast
  impl: rust
  sig: ((bytes :: Bytes) -> Int)
  body:
    "pub fn my_hash_fast(bytes: &[u8]) -> i64 {
         use std::hash::*;
         let mut h = std::collections::hash_map::DefaultHasher::new();
         bytes.hash(&mut h);
         h.finish() as i64
     }")

(def big-bytes [Bytes of-string: "..."])
[MyHashFast call: big-bytes]   ; runs the rust code, microsecond-fast
```

one moof file. one `defmco` form. the substrate handles compilation,
caching, loading, invocation, ABI.

### 4.2 `defmco` macro

```
(defmco <name>
  impl:   #rust | #zig | #c | #c++ | #wasm
  sig:    (<params> -> <return-type>)
  body:   "<source code as a string>"
  deps:   [list of (cargo-dep, version) tuples]    ; optional
  purity: #pure | #unsafe | #cap-using             ; default #unsafe
  caps:   [list of required capability protos]     ; optional
  inline: #true | #false                           ; default #false
  )
```

expansion-time behavior:

1. emit a placeholder Form: a Method-Form whose `:body` is a fresh
   chunk that contains one `CallMco` opcode.
2. attach a `:mco-source` meta slot containing the source-code
   string, impl-language tag, signature, deps, purity, caps.
3. install the method on the receiver proto specified at definition
   site (or as a singleton Form if no receiver context).
4. emit a side-channel build manifest entry: `(name, content-hash,
   impl, sig, body, deps)`. accumulated across the build into
   `lib/mcos/index.moof`.

build-time behavior (via a moof-side script, run by `moof build`):

1. read `lib/mcos/index.moof`. for each mco:
   - compute the content-hash of (body + deps + sig).
   - check cache: is there a built `mcos/<hash>.wasm`?
   - if hit: re-link to existing wasm.
   - if miss: invoke the language's toolchain (`rustc --target
     wasm32-wasi -O3 -o mcos/<hash>.wasm`), cache the result.
2. emit `mcos/manifest.json` mapping mco-name → wasm path + entry
   point + sig.

load-time behavior (in zig substrate):

1. on `defmco`, the substrate's mco-loader reads the manifest, finds
   the wasm path, mmap's the wasm binary, instantiates via the
   wasm-runtime mco (already in zig — see V4 polyglot story).
2. attaches the wasm-export as a native fn keyed by the method
   FormId (registered in `world.native_fns`).
3. on `CallMco` dispatch: marshal Forms across the wasm ABI (§4.5),
   invoke, unmarshal result, push to operand stack.

### 4.3 build pipeline + caching

key invariant: **mco builds are content-addressable.** the cache
key is the content-hash of (source, sig, deps, impl-language). a
moof world ships its own mco cache in `<world>/.moof/mcos/<hash>.wasm`.

build invocation:

```
$ moof build .
  scanning lib/mcos/index.moof ... 17 mcos found
  cache hit: 14
  cache miss: 3 (compiling)
    MyHashFast (rust)         done   in 0.8s   (build/<hash>.wasm)
    SlowAlgo (rust, deps: ndarray, blas) ... done  in 12.3s
    LowLevelFFT (c, fftw)     done   in 0.5s
  mcos/manifest.json written.
```

build infrastructure:

- per-language adapters in `tools/build-mco/`:
  - `rust-adapter`: writes a cargo project to a temp dir, runs
    `cargo build --target wasm32-wasi --release`, extracts the wasm.
  - `c-adapter`: invokes `clang --target=wasm32-wasi -O3 -c source
    -o output.wasm`.
  - `zig-adapter`: invokes `zig build-obj -target wasm32-wasi -O
    ReleaseFast`.
- adapter selection driven by `impl:` on the `defmco`.
- toolchain detection: the build script reports missing toolchains
  clearly. mco's that don't have a corresponding toolchain are
  marked degraded — the moof world still loads, but the affected
  mco's methods raise `'no-mco-toolchain` on call.

### 4.4 hot-reload (moldability!)

edit an mco source string in the REPL; the substrate detects, rebuilds,
swaps in the new wasm.

mechanism:

1. user evaluates `(set-mco-source! 'MyHashFast "new rust source...")`.
2. substrate updates `:mco-source` meta on the Method-Form.
3. substrate triggers an async build (in a separate thread/subprocess
   to avoid blocking the vat).
4. on build-completion, atomic swap: new wasm replaces old wasm
   binding; IC invalidates for the method (L10); future calls use
   new code.

old binding stays valid for in-flight calls; the wasm-runtime mco
holds a reference until the call returns.

key UX: editing rust code and seeing the change reflect on the next
send. **the demo of this**: open the inspector, find a method, see
the rust source, edit it, save, send a message — new behavior.

### 4.5 ABI: tight Form-passing

current mco ABI (`concepts/compiled-objects.md` "the dylib's ABI")
already passes `MoofValue` (a Value enum + Form-handle). polish it:

```c
// the shared MoofValue tagged union (~16 bytes).
struct MoofValue {
    uint8_t  tag;
    union {
        bool    b;
        int64_t i;
        uint32_t form_id;     // for tag = .form
        uint32_t sym_id;      // for tag = .sym
        char32_t ch;
        double  f;
    } payload;
};

// callable signature.
typedef int32_t MoofResult;

MoofResult my_mco_method(
    MoofCtx*       ctx,
    MoofValue      self,
    const MoofValue* args,
    size_t         argc,
    MoofValue*     out
);
```

through `MoofCtx`, the mco can:

- `moof_form_slot(ctx, form, sym_id) → MoofValue` — slot read.
- `moof_form_slot_set(ctx, form, sym_id, value) → MoofResult` — slot
  write.
- `moof_form_proto(ctx, form) → MoofValue` — proto read.
- `moof_form_alloc(ctx, proto) → MoofValue` — allocate.
- `moof_intern_str(ctx, bytes, len) → sym_id` — intern.
- `moof_call(ctx, receiver, sel, args, argc, out) → MoofResult` —
  invoke a moof method (re-enter the VM).
- `moof_foreign_handle(ctx, ptr, destructor, content_tag) → MoofValue`
  — wrap an opaque pointer.
- `moof_raise(ctx, kind_sym, data) → MoofResult` — raise an error.

this is the ABI the conformance suite tests. it stays stable across
host-platform changes.

design constraints from the laws:

- L9 (cap unforgeability) — mco code receives caps as ordinary
  values; it can hold them; it cannot construct them.
- L7 (vat boundary) — `moof_foreign_handle` Forms cannot cross vat
  boundaries (substrate enforces at serialization).
- L11 (FormId stability) — FormIds in `MoofValue.payload.form_id` are
  vat-scoped and stable. mco code can hold them, store them, return
  them. they remain valid as long as the source vat lives.

### 4.6 cross-language: same ABI for everything

the wasm ABI is language-agnostic. any toolchain that emits wasm32-
wasi with the right export signatures is a valid impl-language.

today targets: rust, zig, c, c++. tomorrow: maybe go (via tinygo),
maybe ocaml-of-wasm, maybe nim. **but the ABI is fixed.** new
languages bring new toolchains, not new ABIs.

each language gets a small "moof glue" library that hides the raw
MoofValue marshaling:

```rust
// rust glue (lib mco-rs)
#[moof::method]
pub fn my_hash_fast(bytes: Bytes) -> i64 {
    use std::hash::*;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish() as i64
}
```

the `#[moof::method]` proc-macro wraps the C ABI. users write rust;
glue handles ABI.

similar adapters for c/zig/c++.

### 4.7 security: capability-tied mco loading

an mco can do arbitrary native work in its wasm sandbox. wasm
itself bounds memory access. but moof's cap-discipline (L9) says
authority is granted, not assumed.

design:

- loading an mco *requires the* `$mco-loader` *cap*. root supervisor
  holds it; can attenuate.
- a vat without the cap cannot install new mco bindings (existing
  bindings keep working — capability is for installation only).
- the `:mco-source` meta is reflectable; users can audit what code
  ran.
- mco's running inside an isolated wasm-runtime per-vat (wasm
  sandbox = vat fault isolation). a misbehaving mco can't corrupt
  vat heap.
- `moof_raise` is the only way an mco signals failure; substrate
  bridges to moof's error-handling.

### 4.8 the `defmco` ergonomic story

users encounter mcos progressively:

1. **never**, if they don't want to. stdlib mcos (hash, lmdb,
   websocket, wgpu) "just work" — they're installed by the system.
2. **as a consumer**: they reference an mco by name, invoke its
   methods, get fast paths. zero ceremony.
3. **as an author**: they write a `defmco` block in moof, write rust
   in a multiline string. the toolchain handles the rest. *no
   separate cargo project, no separate build pipeline.*
4. **as a library author**: they ship a `lib/mcos/*.moof` with many
   `defmco` blocks; users `[$import: my-lib]` and the mcos load
   alongside the moof code.

the "rust source as a multiline string" feels weird. it's deliberate:
the source-code IS the canonical thing (L5 again); the wasm is
derived. you can read it. you can audit it. you can rewrite it.

### 4.9 risks + open questions

- **wasm encoder absence.** zig 0.16 has a wasm decoder; encoding
  remains a build-step concern via host toolchains. fine for now;
  revisit when zig ships encoders or we vendor wasm-tools.
- **rust toolchain bloat.** a `defmco` block requires a working rust
  toolchain on the build host. **mitigation:** ship pre-built wasm
  for stdlib mcos in the distribution; only user-authored mcos
  trigger build-time toolchain. degraded mode (no toolchain) still
  loads stdlib mcos.
- **ABI evolution.** as we learn what mcos need, the C ABI will
  grow. how do we add functions without breaking existing mcos? use
  the same versioning discipline as the image format — additive
  patches, major bumps for incompatible changes. mco's declare their
  ABI version in metadata.
- **wasm sandbox escape via opaque pointer abuse?** if an mco
  obtains a `MoofValue` with `.form` payload, it can construct
  arbitrary FormId values and feed them back. **mitigation:** the
  substrate's `world.heap.get(id)` validates id range; out-of-range
  raises `'invalid-formid`. forwarding-table check catches stale
  ids. the wasm sandbox can't *escape*; it can only return garbage
  or raise.

---

## §5 — Erlang-style vat concurrency (user-facing)

### 5.1 the gap: substrate is spec'd, ergonomics is fuzzy

`2026-05-04-vats-and-references-protocol-design.md` §22 lays out V4
(multi-vat container) through V11 (replication + CRDT). that spec
defines the substrate semantics rigorously. it leaves the *user-
facing ergonomics* under-specified.

this section fills in: **what does it FEEL like to write moof code
that uses vats?**

target experience: as friendly as `defproto`. as immediate as a
smalltalk send. as concurrent as erlang. as inspectable as anything
else in moof.

### 5.2 spawn syntax

basic spawn:

```moof
(def my-vat
  [$vat spawn: |mailbox|
    (loop forever
      (match-receive mailbox
        ['greet name]  (println "hi $name")
        ['stop]        (return)))])
```

the block-form takes `|mailbox|` as a parameter. the body runs in
the new vat. result is a far-ref to the spawned vat.

named spawn (registers in the path-table):

```moof
(def alice [$vat spawn: |mb| ... at: "/users/alice"])
```

after spawn, `[$path-table resolve: "/users/alice"]` returns the
same far-ref.

mode at spawn:

```moof
(def world-vat [$vat spawn: |mb| ...
                 mode: #replicated-leader
                 session: 'shared-world])

(def server [$vat spawn: |mb| ...
              mode: #solo
              caps: [$out $net]])
```

mode is fixed at birth (per `concepts/vats.md`). caps are an
explicit list of caps granted from the parent.

structured spawn (with supervisor):

```moof
(def supervised
  [$supervisor spawn-child: |mb| ...
                strategy: #one-for-one
                restart: #permanent])
```

shorthand for `[$vat spawn:]` plus child-registration with the
supervisor.

### 5.3 async send: `<-` / `[obj <- ...]`

surface syntax for cross-vat send:

```moof
;; promise sugar
(let p (<- some-far-ref greet: 'world))
;; explicit form
(let p [some-far-ref <- greet: 'world])
```

`<-` is parser-level sugar for `(__send-async__ obj sel args...)`
which compiles to V7's `OP_EVENTUAL_SEND` opcode. evaluates
immediately to a Promise.

within-vat sends use normal syntax `[obj greet: 'world]`. **the
syntax difference IS the semantic difference** — `[obj ...]` is sync;
`[obj <- ...]` is async. users see it. there's no hidden async-ness
in plain `[...]`.

future shorthand (if ergonomics push for it):

```moof
.greet                ; .field
[obj :sel: arg]       ; sync send
[obj <- :sel: arg]    ; async send
[obj => :sel: arg]    ; reserved for spawned-locally
```

### 5.4 await: `when-resolved:` / `sync-await:`

async continuation:

```moof
(let p (<- counter incr))
[p when-resolved: |v|
  (println "now $v")]
```

`when-resolved:` registers a callback. when the promise resolves,
the callback fires (in the receiving vat's next turn).

chained:

```moof
(<- counter incr) then: [v|
  (<- counter incr-by: 2)] then: [v|
  (println "ended at $v")]
```

`then:` is monadic chaining — returns a new promise that resolves
when the inner promise resolves.

synchronous await (rare; visible):

```moof
(let v [p sync-await: 5s])
```

blocks the current turn until promise resolves or timeout. used
sparingly; per references.md "rare and visible."

### 5.5 receive: `match-receive` in vat body

inside a vat's behavior:

```moof
[$vat spawn: |mb|
  (loop forever
    (match-receive mb
      ['ping]            [reply: 'pong]
      ['add x y]         [reply: (+ x y)]
      ['stop]            (return)
      [other]            (println "huh? $other")))]
```

`match-receive` pattern-matches on incoming messages (consumes one
from mailbox, dispatches on shape). `[reply: ...]` sends back to
the message's `reply-to` automatically.

`(loop forever ...)` is a top-level tail-recursive `loop` (already
in stdlib).

vat-as-Form: `mb` is the mailbox, a data-source. you can `[mb peek]`,
`[mb tee: ...]`, etc.

### 5.6 supervisors: declarative

```moof
(def root-sup
  [$supervisor new
    strategy: #one-for-one
    children:
      ['my-counter   [Counter spawn]]
      ['my-logger    [Logger spawn]]
      ['my-server    [Server spawn]]])
```

supervisor protos:

- `#one-for-one` — only crashed child restarts.
- `#all-for-one` — any crash restarts all children.
- `#rest-for-one` — crash + everything started after.
- `#simple-one-for-one` — dynamic children of one type.

restart strategies (per child):

- `#permanent` — always restart.
- `#temporary` — never restart.
- `#transient` — restart only on abnormal exit.

supervisor inspector view: a tree. visualize who supervises what,
restart counts, recent crashes.

### 5.7 registry: path-table

```moof
[$path-table register: 'my-actor at: my-far-ref]
[$path-table resolve: 'my-actor]   ; → far-ref
[$path-table unregister: 'my-actor]
```

paths are namespaced by `/`:

```moof
[$path-table register: my-actor at: "/users/shreyan/inbox"]
```

resolves to id-ref if local, far-ref if remote. uniform across.

### 5.8 inspector: live view

(detail in §7.) for vat-concurrency specifically, the inspector
shows:

- **vat tree** — root supervisor at top; children below.
  per-vat:
  - mailbox depth (how many messages queued).
  - last 10 messages received (with timestamps).
  - last 5 errors / crashes.
  - current state of behavior (which `match-receive` arm is active).
  - cpu time consumed.
  - heap size.
- **message graph** — directed graph of recent cross-vat sends.
- **promise tracker** — pending promises, what they're waiting on,
  age.

inspector is itself a vat (separate from the inspected vats).
inspector queries via cap-attenuated read access to other vats'
metadata.

### 5.9 recovery: let-it-crash + try

```moof
;; supervisor handles unanticipated:
[$supervisor spawn-child: |mb|
  (loop forever (do-work mb))
  strategy: #permanent]

;; in-vat handles anticipated:
(try
  [(socket connect: addr)]
  catch: |e|
    (case (e :kind)
      ['network-error]  (retry-with-backoff)
      ['auth-error]     (raise e)         ; bubble to supervisor
      [other]           (raise e)))
```

distinction: anticipated (you know what to do) → `try`. unanticipated
(you don't) → let it crash, supervisor decides.

### 5.10 replication: opt-in at spawn

```moof
(def world-vat
  [$vat spawn: |mb| ...
   mode: #replicated-leader
   session: 'shared-world
   reflector: "wss://localhost:7878"])

(def alice-replica
  [$vat spawn: |mb| ...
   mode: #replicated-follower
   session: 'shared-world
   reflector: "wss://localhost:7878"
   leader: world-vat])
```

substrate enforces the determinism laws for replicated vats: no
direct wall-clock, no direct OS-entropy, no direct cap usage (effect-
intent only).

reflector is a separate concern (per `concepts/replication.md`); the
substrate just connects to it.

### 5.11 federation: `[$net join: ...]`

```moof
;; alice's machine:
(def world [$vat spawn: |mb| ... mode: #replicated-leader])

;; bob's machine:
(def world [$net join: "wss://alice-host:7878/shared-world"])
;; world is now a far-ref to alice's vat (a replica spun up locally)
```

joining a remote vat:

1. authenticate via ed25519 (cap-token presented).
2. fetch latest snapshot.
3. start as a replicated-follower with the leader at the remote host.
4. participate in reflector traffic from then on.

`[$net join: ...]` is the cap that makes this work. transport details
(`concepts/transport.md`) are out-of-scope.

### 5.12 sketch of the in-flight phase: V4 → V11

per `2026-05-04-vats §22`, the sub-phases land in dependency order.
this spec maps them to **user-visible milestones**:

| substrate phase | when user sees what |
|---|---|
| V4 (multi-vat container) | `[$vat spawn: ...]` returns a (still-local) far-ref; two vats coexist |
| V5 (references + membrane) | cross-vat messages work; mutable forms can't escape |
| V6 (shared segment) | invisible perf improvement: frozen forms dedupe across vats |
| V7 (`<-` + promises) | `(<- obj msg: arg)` returns Promise; async pattern usable |
| V8 (supervisor + spawn) | supervisor declarative; crash-then-restart works |
| V9 (persistence) | `[$vat save]` / `[$vat load]`; vat outlives reboot |
| V10 (capabilities + effect-intents) | replicated vats can use `[$out say:]` (via intent/receipt) |
| V11 (replication) | two-replica session converges; `[$net join: ...]` works |

at v11 we are at full BEAM-style concurrency. user-visible: spawn,
send, receive, supervise, persist, replicate, federate.

### 5.13 risks + open questions

- **scheduler tuning.** round-robin gets us to v11 (per spec §23
  deferred). fuel-based, priority, fair-share — all later. **for
  the demo (4 vats), round-robin is fine.**
- **mailbox backpressure.** what happens if a producer outpaces a
  consumer? sender's mailbox grows unboundedly? **mitigation:**
  per-vat soft limit on outbound queue; once exceeded, sends block
  until drain. configurable cap on the sending vat.
- **the syntax `<-`** — does it parse cleanly in the moof reader?
  `[obj <- :sel: arg]` requires `<-` as a token. parser update.
- **when-resolved + GC interaction.** observers are held by the
  promise (their callbacks reference closures). if the promise lives
  forever (never resolves), observers leak. **mitigation:** promise
  with deadline (`with-deadline: 30s`) reaps + breaks observers on
  timeout.

---

## §6 — the path to 1,000,000× faster (tier 3 perf)

### 6.1 current state + target ladder

baseline (post phase 1, mid phase 2): **532K sends/sec** on a
realistic compile workload. microbench ceiling ~8M sends/sec
(smp_allocator, monomorphic native). per the phase 2 spec, tier 1
should land us at ~5-10M sustained on micros; tier 2 brings PIC +
inline arith + threaded dispatch.

target progression:

| stage | target sends/sec | how |
|---|---|---|
| **today** | 0.5M sustained / 8M microbench | smp_allocator + phase 1 |
| **stage A** | 5M sustained | tier 2 PICs + threaded dispatch + flat env |
| **stage B** | 100M | tier 3 copy-and-patch JIT for hot methods |
| **stage C** | 1B | per-call-site specialization (Self-style "maps") |

scale of ambition: stage C is **~2000× over stage A**. it's the
ambitious end of the curve. but the tradition of Self / V8 / Truffle
shows it's reachable for a small team if architecture supports it.

### 6.2 tier 3 design space

three approaches, in increasing risk + reward:

#### copy-and-patch JIT (recommended first)

per phase 2 §6.1. pre-compile "stencils" of native code for each
opcode shape. at chunk-load time, copy stencils end-to-end into an
executable buffer; patch in immediates (literal values, IC pointers).

pros:
- simpler than full code-gen (no LLVM/cranelift dep).
- portable (per-arch stencil libraries).
- near-native dispatch (5-10× over interpreted).
- compatible with our mco approach (stencils are themselves like mcos).
- has good prior art: webkit, juliapy, the original truffle paper.

cons:
- per-architecture stencil generation (darwin-arm64, linux-x86_64,
  linux-arm64, wasm32-…).
- not trivially specializable per-call-site (that's tier 4).
- requires JIT memory permissions (mprotect dance).

estimated effort: **4-6 weeks** for a minimal viable tier covering
the 10 hottest opcodes. fallback to interpreter for the rest.

target speedup: **10-50×** on hot code. ~100M sends/sec.

#### cranelift backend (defer)

emit chunk bytecode as cranelift IR; let cranelift do machine code.

pros: well-supported; near-native speed; aggressive optimizations
for free.

cons: ~10-30MB cranelift bloat (vs ~50KB substrate); compile times
real (seconds for cranelift); not portable to browser-wasm (we'd
have to use lift compiler for that, separate work).

defer indefinitely. only reach for it if copy-and-patch caps out
short of needs.

#### tracing JIT (defer indefinitely)

PyPy-style. profile hot loops; emit specialized native traces;
deopt on guard failure.

pros: handles polymorphism beautifully; self-optimizing.

cons: enormous engineering complexity. PyPy is 20+ years. deopt
machinery is most of the bug surface. **not worth it for moof.**

### 6.3 specialization via auto-shape (stage C)

this is the **1B sends/sec move**. requires architectural foresight.

idea: every Form has a *shape* — the set of slot names it carries.
two Forms with the same shape have the same slot-layout, can be
accessed by direct index. dispatch on shape, not on full proto-chain
walk.

current: per `2026-05-16-phase2-moof-performance-design.md` §5.5
(flat closure), §5.4 (flat env), §5.8 (cons flat repr) — we're
already moving toward layout-based access for specific types.

generalize: **auto-flatten** any user proto whose instances stabilize
on a fixed shape. the compiler analyzes hotspots; for protos with
N instances all sharing the same slots, emit a flat layout. dispatch
on flat-shape becomes a single load + compare.

this is exactly **Self's "maps"** (Chambers & Ungar, 1991) and V8's
"hidden classes." we're not inventing anything; we're applying a
20-year-old technique.

implementation sketch:

1. profile-driven: count `(receiver-proto, slot-pattern)` over many
   calls.
2. for stable patterns, the substrate emits a "layout descriptor"
   on the proto.
3. instances created with that layout are flat (fixed offset array).
4. slot-access compiles to direct array index (~1 cycle).
5. dispatch on shape: per-call-site PIC keys on layout descriptor,
   not full proto walk.
6. shape changes (`set-handler!` adds a slot) → invalidate the
   layout; instances revert to slow path.

per-call-site stubs: the JIT generates specialized machine code per
*shape* per *call site*. as observed shapes accumulate, the call
site polymorphizes. compounding 4-8× over plain PIC.

estimated effort: **3-6 months** beyond stage B. requires:
- shape inference in compiler.
- per-shape layout codegen.
- guard-and-deopt machinery.
- per-call-site stub cache + GC.

target: **500M-1B sends/sec** on monomorphic-after-warmup code.

### 6.4 stage-by-stage roadmap

```
phase 2 (in progress)
  tier 1A (today):     smp_allocator + args scratch  → 5M sustained
  tier 1B (week 2):    chunk caching + closure params → 7M
  tier 2-pre (week 3): cached SymIds + intern cache  → 8M
  tier 2A  (month 1):  hot-natives + PICs            → 15M
  tier 2B  (month 2):  inline arith + cons-pool      → 25M
  tier 2C  (month 3):  threaded dispatch + flat env  → 40M
  ─── BEAM-interpreted parity reached ───

phase 3 (this spec)
  stage B (month 4-5): copy-and-patch JIT — 10 hot ops  → 100M
  stage B+ (month 6):  CAP for all bytecode ops          → 200M
  ─── BEAMJIT parity reached ───

phase 4 (future)
  stage C (month 9+):  shape-based specialization        → 500M-1B
  stage C+:            per-call-site stubs               → 1B+
  ─── Self/V8 parity ───
```

### 6.5 supporting infrastructure for tier 3+

once we go below interpreter:

- **profile-guided optimization (PGO)** — collect (receiver-proto,
  selector) frequencies. drive PIC entry order. drive stencil
  generation priority.
- **inline expansion** — small leaf methods (`Integer:+:`) inline at
  call site after profile reports the call site is monomorphic.
- **branch prediction hints** — emit pgo-aware ordering of `if`
  branches in dispatch.
- **icache layout** — colocate hot opcode handlers + stencils for
  cache locality. consider per-method "code regions" that group
  related stencils.
- **escape analysis** — eliminate closure allocation when the
  closure is known not to escape its caller. small but adds up.
- **type specialization** — combine with auto-shape (§6.3). every
  IC entry caches a type-and-shape pair; dispatch becomes ~1
  comparison.

### 6.6 risk surface for tier 3+

- **JIT memory permissions.** mprotect dance differs per OS. wasm
  has no JIT support. **mitigation:** native players JIT; wasm
  player stays interpreted (still gets tier 1+2 wins).
- **code cache management.** as proto edits invalidate, code cache
  fragments. **mitigation:** generational code cache; minor compactions.
- **deopt machinery** (for specialization). a guard fails; we have
  to back off to interpreted. all in-flight frames must be migratable.
  **mitigation:** keep stack frame layouts compatible interp↔JIT;
  only OSR (on-stack replacement) at safepoints.
- **deteriorating ABI as JIT specializes** — different JIT'd
  versions may have different calling conventions. **mitigation:**
  treat the interpreter as the universal ABI; JIT'd code converts
  to/from it at entry/exit.
- **bug surface explosion.** every architectural ambition multiplies
  bugs. **mitigation:** the conformance suite (§1.3) is the bug-
  catching mechanism. JIT bugs that produce different results are
  caught immediately.

### 6.7 baseline-mandatory wins (no JIT required)

even without JIT, several tier-2 improvements compound:

- **inline arithmetic** (phase 2 §5.2) — Int+Int fast path bypasses
  IC + dispatch entirely. 4× on int-heavy code.
- **PICs** (phase 2 §5.1) — 4-way poly IC. 2-3× on polymorphic.
- **threaded dispatch** (phase 2 §5.3) — `@call(.always_tail)`.
  2-3× on interpreter floor.
- **flat env + flat closure** (phase 2 §5.4, §5.5) — direct array
  access. 5× on var-load-heavy.
- **branch hints + icache layout** — small but free.

stacking realistically: ~10-15× over phase-2-baseline. brings us to
~50M sends/sec sustained. that's already enough for the demo
(60fps × 800k ops/frame = 48M ops/sec budget). JIT is *the next
ambition*, not the *required* one.

---

## §7 — the user-facing layer

once the substrate is fast + concurrent + persistent, what does the
user-visible product look like? this section defines constraints
that downstream substrate decisions must respect.

### 7.1 REPL — moldable, inspector-integrated, live

target experience:

```
$ moof repl
moof v1.0.0  player: zig 0.16  image: ~/.moof/system.vat (3.4 MB)
$here = /users/shreyan
> (def x 42)
42
> x
42
> [Counter new count: 10]
#<Counter@vat:18 :count 10>
> [.0 incr]                ; .0 refers to last result
11
> [inspect Counter]
opens inspector in pane
```

key properties:

- **the REPL is itself a vat.** its `$here` is `/repl/<user-id>`.
  defs go into that env. workspaces survive REPL exit.
- **inspector hooks integrated.** `inspect <form>` opens an inspector
  on the form. inspector is a vat that displays + edits other vats.
- **live image.** REPL is connected to a running image. changes
  persist. tomorrow you reopen and your defs are still there.
- **moldable from inside.** `[$repl prompt-format: "your-prompt> "]`
  changes the prompt. `[$repl rewrite: 'inspect …]` redefines the
  inspect command.

implementation:

- 200-300 LoC of moof for the REPL itself
  (`lib/tools/repl.moof`).
- uses `[$reader read]` for parsing, `[$compiler compile:]` for
  bytecode, `[$vm run:]` for execution. all in-image.
- terminal i/o via the `os/console` mco.

### 7.2 inspector — graph view of vats, slots, history

a graphical (or terminal-rendered) view of the world's state.
inspector itself is a moof program with a domain-specific render
proto.

views:

- **vat tree** — root supervisor → children → grandchildren. shows
  mailbox depths, recent activity.
- **form graph** — pick a form; see its proto, slots, handlers,
  meta. clicking a slot value navigates to that form. forms render
  via their proto's `:render-with: ctx` method
  (`concepts/world-and-space.md`).
- **journal timeline** — for a chosen vat, scrubbable timeline of
  inputs + state changes. select a point; restore vat to that state
  (read-only); inspect "what was this form back then?"
- **proto editor** — edit a method's source; recompile;
  next-call uses new code. L10 + L5 in action.

inspector is one of the demo's keystones. demonstrated
inspector-edit-then-watch-it-take-effect is the WOW moment.

### 7.3 debugger — break, replay, time-travel

```moof
[Counter :incr break-before:]   ; install breakpoint
[counter incr]                  ; pauses; debugger opens
```

at the break point:

- inspect locals, stack, this-frame's source position.
- step one bytecode op; step one source expression; step over send;
  step out.
- inspect any other vat (separate inspector pane).
- modify a local or slot; resume.
- "replay-and-modify": step backward (replay journal), branch
  state, re-step forward with edits.

implementation hinges on:

- the per-turn journal (V9). every input that drove a state change
  recorded. backward step replays from snapshot+log.
- L5 + L10. source is canonical; edits are observable. debugger
  edits source, gets new bytecode, dispatches.
- reflection contract. frames expose their state via standard sends.

### 7.4 installation: drop-in

target:

```
$ curl -sL https://moof.dev/install | bash
$ moof world create demo
$ moof world demo
... boots into the demo world ...
```

`moof` is one binary (~3-5 MB native zig player + bundled system.vat
compressed inside). pulls additional mcos lazily from the
`<world>/.moof/mcos/` cache.

cross-platform binaries: darwin-arm64, darwin-x86_64, linux-x86_64,
linux-arm64, windows-x86_64.

browser deployment: `moof.dev` hosts the wasm player + the system.vat;
visiting a moof URL boots a player in-tab.

### 7.5 distribution — `moof package myapp.vat`

```
$ moof package my-counter-demo
  scanning vat /users/shreyan/projects/counter-demo
  serializing forms: 1,234 (uncompressed 2.1 MB)
  compressing with zstd-19: 320 KB
  bundling mcos: 2 (hash-cache.wasm, lmdb.wasm; 480 KB)
  signing with ed25519: ssh-key abc123
  → counter-demo.vat (810 KB)

$ moof load counter-demo.vat
  loading counter-demo.vat: signature OK, opening...
  vat /users/shreyan/projects/counter-demo ready.
```

a `.vat` is a complete, self-contained artifact. you email it. you
post it on a URL. another user runs `moof load` and gets an
exact-bit-identical world.

distribution affordances:

- content-hash addressed (the vat's hash IS its name).
- signature-bearing (optional; for trust chains in shared
  worlds).
- compressed (zstd by default — phase 1).
- mco-bundled (the recipient doesn't need to re-build mcos for
  the right architecture; the vat ships precompiled per-arch
  variants).

### 7.6 the demo (phase E)

forcing function: **3D shared world, multi-user, browser-runnable,
moldable in place**.

demonstrates:
- vats (each user is a vat; the world is a vat; the canvas is a
  vat).
- references protocol (cross-user cursors are far-refs).
- replication (V11 — both users see consistent world state).
- live moldability (user edits a tool proto, other user sees
  change).
- persistence (close laptop, reopen tomorrow, world intact).
- mcos (wgpu rendering; ed25519 auth; lmdb storage).
- the polyglot story (alice on desktop player, bob in browser
  player).
- inspector (open inspector on a shared inhabitant, edit, watch).

once this works, the moldability claim is no longer aspirational.
it's a thing people can use.

### 7.7 constraints for the substrate

the user-facing layer imposes back-pressure on substrate design:

- **journal-replay needs deterministic re-execution** (D-laws).
  any non-deterministic substrate choice (hashmap iteration order,
  wall-clock, OS-entropy) breaks the debugger. all already addressed.
- **live inspector needs cheap reflection.** if `[form slots]` is
  slow (eg. requires walking the proto chain on every access),
  inspector is slow. **mitigation:** slot iteration is already fast
  (one hashmap walk). proto-chain walks are cached in ICs.
- **REPL needs fast eval.** the user types, hits enter, expects
  response within ~50ms. phase 2 perf work (tier 2 → 5-10M
  sends/sec) clears this.
- **proto edits are common.** L10 + IC invalidation handles correctness;
  performance of "edit then run" should be no slower than first-time
  load. **mitigation:** IC eviction is granular; only the affected
  call sites recompile.
- **time-travel needs efficient snapshots.** for the debugger to
  step backward 1000 turns in <1s, snapshots must be incremental
  (only changed slots written). per-turn diff (V1 nursery) supplies
  this.

---

## §8 — sequencing — the version ladder

the realistic, dependency-respecting roadmap. **each version has a
single forcing function**; until it passes, no scope creep.

### v0.x — substrate finalization (current; ~4-8 weeks remaining)

forcing function: **self-host complete + rust deleted + tier-2 perf
landed**.

deliverables:
- W1-W5 from `2026-05-10-self-host-and-rust-deletion-design.md`.
- tier 1A/1B/2-pre/2A from `2026-05-16-phase2-moof-performance-
  design.md`. → ~10M sends/sec sustained.
- conformance test suite v0.5 (50-100 triples).
- rust runtime deleted. only `crates/zig-substrate/` + `crates/
  ocaml-seed/` (build-time) remain.

at v0.x exit: `moof` is one binary; runs the stdlib; reasonable
performance.

### v1.0 — stable substrate + vats + REPL + small demo (~3 months from v0.x)

forcing function: **two vats coexist with cross-vat messaging;
supervisor restarts a crashed child; REPL is interactive at
sustained throughput**.

deliverables:
- vats V4-V8 (multi-vat container; references protocol; shared
  segment; eventual sends + promises; supervision).
- REPL (`lib/tools/repl.moof`).
- inspector v1 — text-line; per-form slot viewer; per-vat tree view.
- small demo: a chat between two vats. user sees messages flying.
- conformance suite v1.0 (200 triples). **frozen**: v1.0 image format
  is binding.

at v1.0 exit: moof is shippable as a curiosity. people can write
multi-actor moof programs.

### v1.5 — persistence + JIT + native laziness (~3 months from v1.0)

forcing function: **vats persist across reboot; a 100M sends/sec
microbench passes; lazy infinite list works**.

deliverables:
- vats V9 (persistence) + V10 (caps + effect-intents). per-vat
  directory layout; journal; snapshot.
- tier 3 stage B (copy-and-patch JIT). →  ~100M sends/sec micro.
- native laziness (§2): Thunk proto, force, lazy-cons, Promise unification.
- conformance suite v1.5.

at v1.5: moof has BEAMJIT-parity perf, persistent vats, and lazy
evaluation. it's plausibly production-grade for self-hosted scripts.

### v2.0 — wasm host + compaction + easy mcos + inspector (~4 months from v1.5)

forcing function: **moof runs in a browser tab with a wgpu-rendered
canvas; vat heaps stay bounded indefinitely; a user installs a
custom rust mco with one defmco form**.

deliverables:
- **H2 wasm-browser player** (§1.2).
- compaction (§3): generational GC + forwarding tables.
- easy mco hooking (§4): defmco macro; build pipeline; hot reload.
- inspector v2: graphical (per host); vat tree visualization;
  proto editor in-pane.
- conformance suite v2.0 covering wasm parity.

at v2.0: the polyglot claim is empirically demonstrated. browser
moof works. mcos are trivial. inspector is real.

### v2.5 — federation + the demo (~3 months from v2.0)

forcing function: **alice (desktop) + bob (browser) edit the same
3D world; both see each other's cursors live; world persists**.

deliverables:
- vats V11 (replication). reflector. leader/follower failover.
- transport: websocket mco. ed25519 auth.
- phase E demo: 3D world; inhabitant protos (pixmap, counter,
  cube, scratchpad); per-user wrapper vats; world-vat replicated.
- the moof.dev website hosting the demo + binary downloads.

at v2.5: the manifesto's promise is real. people can show this off.

### v3.0 — specialization + production hardening (~6 months from v2.5)

forcing function: **the demo runs at 60fps with 8 concurrent users
on a midrange laptop; security audit complete; cargo of stdlib
mcos covers common needs**.

deliverables:
- tier 3 stage C: shape-based specialization. → 500M+ sends/sec
  on monomorphic-after-warmup code.
- hardened transport (TLS, deduplication, message-ordering).
- standard mco library: hash, sign, encrypt, lmdb, sqlite,
  websocket, http, regex, format, ...
- doc site at full polish.
- conformance suite v3.0 covering specialization.

at v3.0: moof is a place you could plausibly live, full-time, for
work.

### beyond v3.0

- gpu compute mcos (CUDA-via-wasm, metal-via-wasm).
- typed moof (refinement types, dependent types — `concepts/types.md`
  full realization).
- datalog query layer.
- APL Tables performance work.
- mobile players (ios via webview, android via webview).
- collaborative multi-vat dev tools.

### sequencing rules

1. **forcing functions are inviolate.** "v1.0 done" means the
   forcing function works, not that "most of v1.0 is done."
2. **no leaping ahead.** writing v2.0 code while v1.5 doesn't pass
   its function is forbidden. (rust 2015→2018 discipline.)
3. **no leaping back.** if v1.5 reveals v1.0 was wrong, fix v1.0
   first.
4. **conformance tests are the contract.** every release freezes a
   conformance suite. regressions are shipping blockers.

---

## §9 — risks + open questions

### 9.1 BEAM lineage: what to match, what to leapfrog

erlang+BEAM is 35 years of war-tested production code. we are
explicitly drinking from that well (vats=processes; mailbox; let-
it-crash; supervisor trees; mode-at-spawn; deterministic distributed
state).

what we **need to match**:

- **per-process isolation as a hard wall.** BEAM's heap-per-process
  is what makes "let it crash" survivable. moof's per-vat heaps +
  references protocol need the same property. **mitigation:** L7
  (vat boundaries are absolute) is a substrate law; serialization
  enforces.
- **lightweight processes.** BEAM spawns 100K+ processes routinely.
  moof spawn must be cheap (target: 1M vat spawns/sec on a desktop).
  this requires lightweight Vat struct + lazy mailbox + lazy journal.
  defer journals to first-mutation; defer mailboxes to first-receive.
- **fault tolerance from outside.** supervisors handle child crashes;
  child code is naive about errors. moof supervisors per V8 give us
  this.
- **distribution as a small extension.** in BEAM, `Pid` works
  identically across nodes. in moof, `far-ref` should work
  identically across processes / hosts / machines. references.md
  promises this; transport spec makes it concrete.

what we **can leapfrog**:

- **the language itself.** erlang is weird (immutable; pattern-
  syntax-heavy; small stdlib). moof is lisp-flavored, smalltalk-
  shaped, with macros and a moldable inspector. nicer for users.
- **the inspector + IDE.** BEAM's observer is functional but
  utilitarian. our inspector + REPL + moldable everything beats
  it for novice + expert alike.
- **the artifact format.** BEAM beam files are class-files. our
  `.vat` is a live image. shareable; bootable; mutable.
- **dynamic moldability.** in BEAM, hot-code reload is painful
  (two versions in memory; module reload; `:code.purge`).
  moof's L10 + per-method recompile makes it trivial.

what we **leave on the table** (consciously):

- **massive concurrency.** BEAM scales to millions of processes per
  machine. moof aims for thousands. enough for our use case.
- **soft real-time scheduling.** BEAM's preemptive scheduler.
  moof's cooperative + turn-bound. for the demo's interactive
  workload, fine.
- **NIF (native-implemented functions) hot-loading nuance.** BEAM's
  NIFs have tricky reload semantics. moof's mco hot-reload (§4.4)
  benefits from per-mco wasm sandboxing; cleaner than BEAM's
  C dynamic-linking story.

### 9.2 freezing the `.vat` format at v1.0

once v1.0 ships, the `.vat` format is binding. drift is a back-compat
catastrophe.

**how to lock it well:**

- write a **format specification document** at v1.0. byte-by-byte.
  every reserved bit, every section ordering, every endian choice.
  treat it as load-bearing as RFC 793.
- ship a **format validator** that tests any image against the spec.
  used in conformance tests.
- ship a **migration framework** from day 1. v1→v2 (when it happens)
  goes through `moof migrate`. the framework is moof code so users
  can write their own (proprietary subtypes).
- **bug fixes never change the format.** if a v1.0 player has a
  bug that interprets a section incorrectly, fix the player, not
  the format. the format is the contract.

**risks if we don't:**

- a popular moof world ships some custom mco that ends up in a vat.
  user upgrades player; mco crashes; vat unloadable. user's data
  effectively lost.
- forward / backward compat dance — v2 reader reading v1, v1 reader
  reading v2-with-fallback-section — becomes the dominant complexity
  cost.
- different players ship slightly-different implementations of an
  ambiguity in the spec. images written by one don't load in
  another. polyglot claim falsified.

### 9.3 mco wasm: right choice long-term?

we chose wasm as the cross-language compiled-object format. is wasm
the right call?

**arguments for wasm:**
- universal target. every meaningful systems language emits wasm.
- sandboxed. memory isolation comes free.
- portable. one wasm binary runs on every architecture (with a wasm
  runtime).
- mature spec. WebAssembly Component Model brings ABI standardization.
- our zig substrate already has a wasm runtime (for mcos today).

**arguments against:**
- 20-40% perf overhead vs native (last we checked; Wasm 2.0 / GC
  proposal may close this).
- wasm runtimes themselves are large dependencies (wasmtime is
  ~10MB compiled).
- newer wasm features (GC, exception-handling, tail-call) aren't
  universally supported yet.

**alternatives considered:**
- **native dylibs only.** per-arch builds. fastest at runtime;
  worst portability. clunky distribution.
- **two formats: wasm for portable, native for hot.** more
  complexity; ABI proliferation.
- **bytecode (PIC?) for portability + JIT'd for perf.** that's
  what JVM does; great if you have a JIT for it; we're building a
  moof JIT; mco-jit is a separate ambition.

**verdict:** wasm is the right call for v2.0. revisit at v3.0 if
perf bottlenecks emerge. consider native-dylib opt-in for "blessed"
hot mcos (lmdb, blake3) only.

### 9.4 inspector MVP — what's the right scope?

a fully-graphical inspector is months of work. demo wants something
sooner. what's the minimum that demonstrates the moldability claim?

candidates:

- **MVP-A (terminal text):** `[form inspect]` prints a structured
  view; edit via `[form set-handler! …]`; refresh via re-print.
  no live updates. ~200 LoC moof. **available v1.0.**
- **MVP-B (TUI live):** crossterm-style. live updates as state
  changes. tree-navigable. edit-in-place. ~1k LoC moof + 1 mco for
  terminal i/o. **target v1.5.**
- **MVP-C (graphical):** wgpu rendered. mouse + keyboard. multi-pane.
  ~5k LoC moof + several rendering mcos. **target v2.0.**

**recommendation:** ship MVP-A by v1.0 (for the small demo). MVP-B
by v1.5 (real moldability story). MVP-C by v2.0 (the demo).

### 9.5 the 5 most important open questions

1. **how do we handle the wasm player's lack of JIT?** browser wasm
   can't JIT another wasm. browser moof stays interpreted forever.
   that may be fine (tier 2 perf is enough for the demo), but it
   means *no parity* with native at the top tier. **needs decision
   by v2.0.**

2. **when does ocaml-seed disappear?** it's a build tool for self-
   host. after `parser.moof` + `compiler.moof` are byte-identical
   to ocaml output, ocaml's role is "compile system.vat from
   scratch in case of bit-rot." minimal but still maintenance.
   alternatives: keep zig as the compile-system.vat target (use the
   running zig player + parser.moof to rebuild). **needs decision
   by v1.0.**

3. **what happens to in-flight messages on supervisor restart?**
   when a vat crashes, its mailbox has unprocessed messages. options:
   (a) discard (BEAM does this); (b) replay (V9 persistence makes
   possible); (c) move to dead-letter queue. **needs decision by
   v1.0.**

4. **how do we test that mcos don't violate caps?** the wasm sandbox
   prevents memory access, but the mco's API surface (moof_call,
   moof_form_slot_set) is the cap surface. malicious mco can do
   anything its caller can. **mitigation:** every native ABI call
   goes through cap-token check; mco runs with the caller's caps,
   not more. **needs an mco security audit before v2.0.**

5. **what's the right host-language for the next player after zig?**
   c for portability? rust for tooling? a self-hosted moof
   interpreter (yo dawg)? **decision deferred to v2.0+.**

---

## §10 — what makes moof AMAZING

three paragraphs on the dream state.

a v3.0 moof user opens their laptop. their world is exactly as
they left it last night — half-typed expressions in the scratchpad,
two open inspectors, a chat window with collaborators (whose own
worlds are connected, but currently asleep), a counter inhabitant
they're prototyping that broke last night but they remember
exactly what they wanted to fix. they type a fix into the broken
counter's `:incr` method, hit enter; the next message landing on
it uses the new code. they pull up the journal view, scrub back to
yesterday's bug, see the exact input that triggered it, watch state
move forward in slow motion. one of their collaborators (in
shanghai) wakes up, their cursor appears in the shared workspace,
they wave hello via a custom hand-wave inhabitant the user
prototyped last week. their collaborator clicks the counter the
user just fixed, edits the source themselves, suggests a refinement;
the user sees the diff highlighted. it's a quiet morning;
everything is alive; nothing is hidden.

what makes this *amazing*, not just *neat*, is the absence of
friction between idea and action. when the user thinks "this counter
should max out at 100," they edit a method, and the next message
applies the new behavior. no compile cycle. no deploy. no restart.
when they think "i want to see what happened yesterday," they
open the journal. no log file. no debugger setup. no breakpoint.
when they think "my collaborator should see this," it's already
visible to the collaborator — the world is replicated; both see
the same state at every turn. when they think "i want to share this
prototype publicly," they email the `.vat` file; the recipient runs
`moof load`; the world boots in 1.2 seconds, exactly as the sender
saw it, complete with the half-finished thoughts in the scratchpad
and the inspector layout. **the medium of communication is a live
world.** that's the thing nobody else has built.

the engineering that gets us there is meticulous: an image format
locked tight enough that polyglot works, a generational gc cheap
enough that interactive perf survives, a JIT fast enough to make
moof feel native, a vat model rigorous enough that distribution
falls out, an inspector good enough that "ah, this is how it works"
moments happen daily, an mco ABI clean enough that any language
can plug in, a substrate small enough that a determined hacker can
read every line in a weekend. it's a lot of things to get right.
but they all serve one image: a person sitting at their desk
inhabiting a place that responds to thought, persists across
sleep, talks with other people's places, and gets better the more
they reshape it. **that is what we are making.** if we make it,
we win. if we don't, we'll have learned for real what stops people
from building computing this way — and that, too, is fine.

`٩(◕‿◕｡)۶`

---

## see also

- `docs/roadmap.md` — overall phase plan
- `docs/vision/manifesto.md` — the why
- `docs/vision/one-page.md` — the elevator pitch
- `docs/concepts/forms.md` — the four faces
- `docs/concepts/vats.md`, `concepts/references.md`,
  `concepts/replication.md` — the concurrency model
- `docs/concepts/compiled-objects.md` — the mco model
- `docs/concepts/world-and-space.md` — the demo's substrate
- `docs/laws/substrate-laws.md` — L1-L16
- `docs/laws/determinism-laws.md` — D1-D12
- `docs/superpowers/specs/2026-05-10-self-host-and-rust-deletion-design.md`
- `docs/superpowers/specs/2026-05-11-phase1-gc-dispatch-compression-design.md`
- `docs/superpowers/specs/2026-05-16-phase2-moof-performance-design.md`
- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md`
- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`
- `NEXT_SESSION.md` — current state at HEAD `0319c10`
