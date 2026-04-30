# next session: the deterministic-replicas leap

> **mission: end the session with `cargo test` running a two-replica
> in-process convergence test that hashes-equal at every turn for
> 10000 random envelopes. this is the phase-D gate — the load-bearing
> substrate honesty check. once it passes, phases E and F become
> "wire up renderer + websocket" rather than "build foundations."**

---

## the destination

shreyan's vision (`docs/vision/manifesto.md`, `docs/vision/one-page.md`):

```
$ moof world ./worlds/test-world/      # alice; hosts.
$ moof world join wss://localhost:7878   # bob; joins.
# both inhabit the same 3D zoomable space.
# pixmaps + counters are inhabitants. both fly + click + edit.
# strokes propagate < 50 ms. live-edits to Pencil propagate
# in the same tick. close + reopen: world wakes, state intact.
```

that demo is phase F (`docs/roadmap.md`). between us and it:

- **phase A** — substrate seed (done; `compiler.rs` is 1009 LoC, 245 tests green).
- **phase A-self-host** — parser.moof + compiler.moof. ~1500 LoC of moof, 0 LoC of rust delta.
- **phase B** — single-vat persistence. mco loader, lmdb mco, intent/receipt, journal, snapshot.
- **phase C** — moldability foundations. multi-clause patterns, `become:`, IC invalidation tests.
- **phase D** — replicated vats in-process. determinism laws enforced, canonical hash, 2-replica test.
- **phase E** — single-user 3D world. frame/placement/pose protos, terminal renderer mco.
- **phase F** — multi-user 3D world. websocket transport, presence, leader failover.

**this session aims to land all of phase B, phase C, and the
load-bearing core of phase D — leaving phases E + F as wiring jobs
for the next two sessions.**

ambitious? yes. infeasible? no. the substrate seed is small and
already mostly honest. the work below is mostly *making things
converge that already individually exist*: the heap, the proto
machinery, the macro infrastructure. the new pieces are mco-loading,
canonical encoding, determinism enforcement, and the reflector
loop.

---

## current state (commit `a64b5be`)

what's working today:

- **Form heap, proto chain, send dispatch with inline caches.** L1, L2, L3, L6 of `docs/laws/substrate-laws.md` are honored.
- **bootstrap reader + compiler in rust.** ≤1.5k LoC each.
- **bootstrap.moof** has 950 LoC of moof: every protocol-derived method, plus all source-to-source special forms (`when`, `unless`, `let*`, `let-rec`, `defmethod`, `defproto`, `quasiquote`, `__cascade__`, `__table__`, `__obj__`) as user-modifiable macros.
- **reflection** matches `docs/laws/reflection-contract.md` R1, R2, R7 (in-snapshot Tables for `:slots`/`:handlers`/`:meta`; full `:protos` chain; `:arity`/`:purity`/`:caps-required` placeholders).
- **245 tests passing**, including `doc_alignment.rs` regressions for the doc/impl unifications.

what's *not* working today:

- no real mco loader. `ForeignHandle` exists; `dlopen` does not.
- no persistence. no journal. no snapshot. world dies at process exit.
- no vats. `World` is the lone "vat-shaped" thing; no mailbox, no scheduler, no supervisor.
- no determinism enforcement. wall-clock-via-rust is reachable inside any closure.
- no canonical encoding, no canonical hash.
- no reflector. no replication. no `:replicated-leader` / `:replicated-follower` modes.
- no intent/receipt. caps fire synchronously in-process.
- no pattern matching beyond zero-clause defs. multi-clause is `docs/concepts/blocks-and-patterns.md`'s biggest unbuilt thing.
- no `become:`.
- self-hosted parser.moof and compiler.moof do not exist.

the test gate at session-end (`docs/process/impl-plan-v4.md` D-acceptance):

```
two in-process replica vats of one logical replicated vat,
fed the same totally-ordered input log of 10 000 random
envelopes, produce IDENTICAL canonical-hashes at every
turn boundary. fault injection: drop one replica mid-run,
catch up via input log, re-converge.
```

that is the load-bearing test. nothing else this session matters
if this test does not pass at the end.

---

## the plan, by track

eight tracks, ordered roughly by dependency. each track has a
forcing function ("we know we are done with track T when X"), a
deliverable list, and an estimated weight.

each track ends with a `cargo test --workspace` green run before the
next track begins. nothing compounds.

---

### track 1 — pattern matching for multi-clause defs (phase C)

**why first.** every other track that follows is going to want
multi-clause defs to write idiomatic moof. doing it first amortizes
the ergonomic gain across the rest of the session.

**doc gates:** `docs/concepts/blocks-and-patterns.md`,
`docs/syntax/binding-and-defs.md`.

**deliverables.**

- **a `Pattern` proto in moof** with `:match?: value bindings: env`.
  patterns are Forms. user code can register new pattern protos by
  extending `Pattern`.
- **the eight basic pattern Forms**, defined as ordinary protos in
  `lib/patterns.moof`:
  - literal (`|0|`, `|'foo|`, `|"hi"|`).
  - variable (`|n|`).
  - wildcard (`|_|`).
  - list-cons (`|'(h …t)|`).
  - table positional (`|#[a b c]|`).
  - table keyed (`|#[name: n]|`).
  - type guard (`|n :: Nat|`).
  - predicate guard (`|n where [n > 0]|`).
- **a moof macro `match`**: `(match expr |pat| body |pat| body)`.
  inside a fresh closure for each clause, the body sees the
  bindings the pattern produced. fall-through chains via nested `if`.
- **multi-clause `def`** (and methods): the macro `def` takes
  multiple `|pat| body` after the name; expands to a single closure
  whose body is a `(match args …)` over the formal parameters list.
- **reader extension**: `|0|`, `|'foo|`, `|n :: Nat|`, `|n where pred|`
  are parsed as block headers. the reader tags each pattern with a
  `Pattern` subclass at parse time. the macro expander then walks
  the pattern tree.

**rust delta.** ~+0 LoC. the rust reader gets new pattern-detection
branches (~+50 LoC), but pattern *semantics* live entirely in
`lib/patterns.moof` (~+200 LoC).

**forcing function.**

```moof
(def fact
  |0|     1
  |n|     [n * (fact [n - 1])])

(fact 5) ; → 120
```

plus the seven other pattern shapes in tests.

**tests.** add `tests/patterns.rs`: one test per pattern kind + one
combined test (`(match xs |'(h _)| 'pair |'() | 'empty |_| 'other)`).

---

### track 2 — `become:`

**why now.** `become:` is small (~80 LoC of rust; ~30 LoC of moof
tests). it's the last "primitive moldability move" the docs
promise. without it, instances cannot be migrated when their proto
changes shape — and we will *want* this for phase B's snapshot
loading.

**doc gates:** `docs/laws/substrate-laws.md` L12,
`docs/concepts/objects-and-protos.md` (the `become:` section).

**deliverables.**

- **id indirection.** `World::heap` already stores Forms by FormId.
  introduce a forwarding table so that `heap.get(id)` may transparently
  redirect to the swapped Form. cheap (one indirection per access)
  in the cold path; inline-cached sends bypass.
- **the `become:` primitive.** `[a become: b]` swaps the heap-slots
  of the two Forms such that every existing reference behaves as if
  a/b had been the other all along.
- **bumps proto generation** on swap (so existing inline caches
  invalidate correctly).
- **forbidden across vat boundaries** (when vats land in track 5).

**rust delta.** ~+80 LoC in `heap.rs` + `world.rs`.

**forcing function.**

```rust
let a = w.alloc(Form::with_proto(...));   // proto = Counter
let b = w.alloc(Form::with_proto(...));   // proto = BoundedCounter
// existing closure captures `a` by id.
let saved = a;
moof::eval(&mut w, "[a become: b]");
// `[saved incr]` now dispatches via BoundedCounter, even though
// the original `a` was a plain Counter.
```

---

### track 3 — canonical encoding + canonical hash

**why now.** the phase-D test demands `canonical_hash(replica_a) ==
canonical_hash(replica_b)`. encoding the heap deterministically is
the substrate primitive that makes this hash meaningful.

**doc gates:** `docs/concepts/persistence.md`,
`docs/laws/determinism-laws.md` D9, "the canonical hash".
*write* `docs/reference/canonical-encoding.md` first (it's referenced
across the codebase but doesn't exist yet — the docs are the bug,
fix the docs, then implement).

**deliverables.**

- `docs/reference/canonical-encoding.md` — the binary format spec.
  fixed prefix per kind (Nil/Bool/Int/Float/Sym/Char/Form/Foreign).
  recursive: a Form encodes as `kind + proto-ref + slots-table +
  handlers-table + meta-table`. tables encode in **insertion order**
  (`docs/laws/determinism-laws.md` D5).
- a rust module `crates/substrate/src/canon.rs` (~+400 LoC):
  `canonical_bytes(world, value) -> Vec<u8>`. round-trippable.
- a `:canonical-bytes` reflection method on every Form.
- a `:canonical-hash` method using a stub blake3 first
  (rust `blake3` crate dep, or hand-rolled 256-bit toy hash; the real
  blake3 lands as `core/blake3.mco` once track 4 is in).

**rust delta.** ~+450 LoC.

**forcing function.** `forms_equal(a, b) ⇒ canonical_bytes(a) ==
canonical_bytes(b)` for the substrate's law-tester. plus:
`canonical_hash` agrees across two `World::new()` instances that have
been driven by the same set of evaluations.

---

### track 4 — mco loader

**why now.** mcos are how every substrate concern that *isn't* the
seed comes in: lmdb, blake3, ed25519, terminal renderer, websocket.
no mco loader, no persistence, no replication, no demo. this is the
single biggest unblock for phase B and beyond.

**doc gates:** `docs/concepts/compiled-objects.md`. write the C ABI
spec in `docs/reference/native-abi.md` (does not exist; create it).

**deliverables.**

- **`crates/abi-rust`** (already exists as a stub) gets fleshed out
  to define the actual `MoofContext` C struct, the `MoofResult`
  enum, the function-pointer signatures, the `moof_object!` macro.
- **a `crates/substrate/src/mco.rs`** (~+400 LoC) implementing:
  - the file format reader (header + variants + binding metadata).
  - `dlopen` via the `libloading` crate.
  - native-method-form construction (each native is a Method-Form
    whose `:invoke` is a rust trampoline that calls the dlopened
    symbol).
  - `ForeignHandle` lifecycle (already exists; wire to gc).
  - signature verification (skip for now — `--allow-unsigned` flag).
- **two test mcos built at compile time** via `build.rs`:
  - `core/blake3.mco` — wraps the `blake3` crate. methods:
    `:hash:`, `:incremental`, `:update:`, `:finalize`.
  - `core/ed25519.mco` — wraps the `ed25519-dalek` crate. methods:
    `:sign:`, `:verify:with:`.
- **a `(loadMco path)` global** that returns the loaded proto.
  becomes an `EffectIntent` once track 6 lands; for now it's a
  synchronous direct call.

**rust delta.** ~+500 LoC.
**moof delta.** ~+0 (mcos *are* moof from the moof side).

**forcing function.**

```moof
(def Blake3 (loadMco "core/blake3"))
[Blake3 hash: "hello world"]   ; → 256-bit hash bytes
```

at this point the substrate is mco-capable. *every track after
this can ship rust as an mco rather than as a substrate change.*

---

### track 5 — vats with mailboxes (phase B foundation)

**why now.** persistence, replication, supervisor — all require
that "the world" be split into isolated vats with mailboxes. the
current `World` is a vat-shaped `World`; we name it that.

**doc gates:** `docs/concepts/vats.md`,
`docs/laws/isolation-laws.md`, `docs/concepts/references.md`.

**deliverables.**

- rename `World` → `Vat` internally; the public `moof::new_world()`
  becomes `new_world() → World` where `World` is a thin wrapper
  with a single root `Vat` (phase A semantics preserved).
- introduce `crates/substrate/src/scheduler.rs` (~+200 LoC):
  in-process scheduler that round-robins over a `Vec<Vat>`. each
  vat has an `inbox: VecDeque<Envelope>`. `[vat send: msg]` enqueues;
  the scheduler dispatches one message per turn per vat.
- `:solo` vat-mode at birth (the only mode supported in this track;
  replicated modes land in track 7).
- the `$spawn` cap: `[$spawn vat-with: behavior]` creates a child
  vat under the current vat as supervisor.
- **far-refs**. when a value would cross a vat boundary
  (`[remote-vat ask: 'thing]`), the substrate auto-promotes any
  in-vat references to far-ref tuples. add a `FarRef` Form-kind in
  moof; the rust scheduler handles routing.
- promises. `[promise when-resolved: blk]`. resolved at receiver's
  reply.

**rust delta.** ~+400 LoC.
**moof delta.** ~+150 LoC (FarRef proto, Promise proto, the
async/await sugar).

**forcing function.**

```moof
(def alice [$spawn vat-with: |msg| ...])  ; far-ref to a child vat
(let p [alice greet: 'world])              ; returns Promise
[p when-resolved: |v| (println v)]
```

multiple vats run; messages flow; promises resolve. inbox is a
`DataSource` (`docs/concepts/data-sources.md`).

---

### track 6 — per-vat persistence + intent/receipt

**why now.** with vats and an mco loader in place, persistence is
"each vat saves its directory through the lmdb mco; intents and
receipts become the way caps cross the vat boundary into the
authority."

**doc gates:** `docs/concepts/persistence.md`,
`docs/concepts/effect-intents.md`,
`docs/laws/substrate-laws.md` L8.

**deliverables.**

- **`store/lmdb.mco`** built via `build.rs` from the `lmdb-rkv`
  crate. methods: `:open:`, `:close`, `:read-txn:blk`,
  `:write-txn:blk`, plus a `Txn` proto with `:get:`, `:put:value:`,
  `:contains?:`.
- **per-vat directory layout** as documented in
  `docs/concepts/persistence.md`:
  - `meta.toml` — version, supervisor far-ref, cap declarations.
  - `store.lmdb` — heap-id → canonical-bytes (the snapshot).
  - `journal.log` — append-only WAL of slot-mutations.
  - `inputs.log` — append-only input-envelope log.
  - `effects.log` — append-only intent + receipt log.
  - `refs/root` — pointer to the root form-id.
- **commit per turn**: at end-of-turn, journal mutations + input
  envelope, fsync, advance inbox cursor.
- **boot from store**: `Vat::open(path)` mmaps the lmdb, replays
  `inputs.log` from snapshot's turn-seq.
- **the intent/receipt model** (`docs/concepts/effect-intents.md`):
  - inside a vat, `[$out say: "hi"]` does **not** invoke `$out`.
    it appends `EffectIntent(turn-seq, ordinal, cap, sel, args)`
    to the vat's outbox slot.
  - allocates a Promise with id `(turn-seq, ordinal)`.
  - returns the Promise.
  - the **effect authority** (a non-replicated worker; for now the
    same vat's runtime) reads outbox at end-of-turn, executes,
    emits `EffectReceipt(turn-seq, ordinal, status, value)` as a
    new input event.
  - replicas (when track 7 lands) receive the receipt as a new turn
    envelope and resolve the promise.
- the `$out` / `$err` caps move from rust intrinsics into a tiny
  `os/console.mco`.

**rust delta.** ~+300 LoC (vat boot, journal serialization, intent
plumbing).
**moof delta.** ~+200 LoC (Promise, EffectIntent, EffectReceipt
protos; the `(intent-emit cap sel args)` helper for caps).
**mco delta.** `store/lmdb.mco`, `os/console.mco`,
`os/clock.mco`, `os/random.mco`.

**forcing function.**

```bash
$ moof world ./worlds/test-world/
> creates the directory; binds $out, $err, etc.
> evaluates a script that defproto's a Counter, increments it,
>   prints "5" via $out (which goes through intent → receipt
>   → Promise resolution → console emit).
> exits gracefully.

$ moof world ./worlds/test-world/
> reopens the same directory.
> reads the journal; the Counter is at 5.
> evaluates a new line; it now reads 6.
> exits.
```

state survives reboot; intents are journaled; replay is
reconstructive.

---

### track 7 — determinism enforcement + replicated mode

**why now.** all the prior tracks build the foundation. this track
flips the bit: `:replicated-leader` and `:replicated-follower` are
born modes that the substrate enforces.

**doc gates:** `docs/laws/determinism-laws.md` (every D-rule),
`docs/concepts/replication.md`.

**deliverables.**

- **`Vat::mode`** field. set at birth. enums to `:solo`,
  `:replicated-leader`, `:replicated-follower`.
- **deterministic alloc order (D4)**: replicated vats use FormIds
  of shape `(turn-seq << 32) | local-counter`, not a free-running
  counter.
- **insertion-order iteration (D5)**: already true (we use
  `IndexMap`); add a regression test.
- **forbidden mid-turn operations (D3)**: the substrate refuses
  `$clock`, `$random`, `$fs`, etc., to a replicated vat. `logical-
  now` and `seed` are read from the turn envelope, never from a cap.
- **gc at turn boundaries (D6)**: the existing implicit "no gc"
  is honored; once gc lands, gate it on turn boundary.
- **deterministic promise ids (D7)**: `(turn-seq, ordinal)`,
  matching effect-intents.
- **proto-edit-as-input (D8)**: `(set-handler! Foo 'bar fn)`
  inside a replicated vat is reified as a `ProtoEdit` envelope;
  the reflector orders it; every replica recompiles bytecode locally.

**rust delta.** ~+200 LoC.

**forcing function.** the substrate refuses to grant `$clock` to a
replicated vat. trying raises `replicated-cap-violation`. logical-
now from the turn envelope works.

---

### track 8 — in-process reflector + the 2-replica convergence test

**why last.** every prior track is foundational. *this is the
session's success criterion.*

**doc gates:** `docs/concepts/replication.md`,
`docs/concepts/transport.md`.

**deliverables.**

- **`crates/substrate/src/reflector.rs`** (~+200 LoC). takes user-
  input events, totally orders them, broadcasts as turn envelopes.
  signs envelopes (using `core/ed25519.mco`).
- **the turn envelope** as a Form, matching D2:
  `{TurnEnvelope session-id epoch turn-seq author logical-now seed
  input-event signature}`.
- **two replicas in one process.** a test harness: build two
  `Vat`s in `:replicated-follower` mode, both subscribing to the
  same in-process reflector. the test feeds the reflector 10 000
  random `(input-event, author)` pairs. after each, both replicas'
  `:canonical-hash` must agree.
- **fault injection**: drop one replica midway, snapshot the other,
  bring the dropped replica back, catch it up via input log replay,
  verify hash convergence.
- **the proto-edit-during-replication** test: emit a `ProtoEdit`
  envelope; both replicas recompile bytecode locally; both observe
  the new behavior on the very next send.

**rust delta.** ~+250 LoC (reflector loop + the convergence test).

**forcing function.**

```rust
#[test]
fn two_replicas_converge_over_10k_envelopes() {
    let mut harness = ReplicatedHarness::new();
    let alice = harness.spawn_replica(Mode::Leader);
    let bob   = harness.spawn_replica(Mode::Follower);
    let rng   = SeedableRng::seed_from_u64(42);
    for _ in 0..10_000 {
        let env = harness.random_input_envelope(&rng);
        harness.broadcast(env);
        assert_eq!(alice.canonical_hash(), bob.canonical_hash());
    }
}

#[test]
fn rejoin_after_drop_converges() {
    let mut harness = ReplicatedHarness::new();
    let alice = harness.spawn_replica(Mode::Leader);
    let bob   = harness.spawn_replica(Mode::Follower);
    for _ in 0..1000 {
        harness.broadcast(harness.random_input());
    }
    harness.disconnect(bob);
    for _ in 0..1000 {
        harness.broadcast(harness.random_input());
    }
    let bob = harness.reconnect_with_snapshot_from(alice);
    for _ in 0..1000 {
        harness.broadcast(harness.random_input());
    }
    assert_eq!(alice.canonical_hash(), bob.canonical_hash());
}

#[test]
fn proto_edit_propagates_across_replicas() {
    let mut harness = ReplicatedHarness::new();
    let alice = harness.spawn_replica(Mode::Leader);
    let bob   = harness.spawn_replica(Mode::Follower);
    harness.eval_on_leader("(defproto Counter (handlers (incr) [.count + 1]))");
    harness.eval_on_leader("(def c [Counter new])");
    harness.eval_on_leader("[c incr]");
    let alice_count = harness.eval_on_replica(alice, "[c read]");
    let bob_count   = harness.eval_on_replica(bob, "[c read]");
    assert_eq!(alice_count, bob_count);
    // now live-edit
    harness.eval_on_leader("(setHandler! Counter 'incr (fn () [self count: [.count + 100]]))");
    harness.eval_on_leader("[c incr]");
    let alice_count = harness.eval_on_replica(alice, "[c read]");
    let bob_count   = harness.eval_on_replica(bob, "[c read]");
    assert_eq!(alice_count, bob_count);  // both jumped by 100
}
```

**when these three tests pass, phase D is honored.** the
substrate is a real moldable replicated environment. the rest of
the vision (renderer, websocket, world-and-space) is wiring on top
of an honest foundation.

---

## what is *not* in scope this session

even at the ambitious end of the plan, several tracks intentionally
slip to the next session:

| deferred | why |
|---|---|
| parser.moof + compiler.moof (phase A-self-host) | beneficial but not load-bearing for replication. land it in session 3. |
| 3D world primitives — Frame, Pose, Camera, Viewport | phase E. depends on terminal renderer mco. |
| terminal renderer mco (`render/terminal`) | phase E. ~700 LoC of rust + ~500 LoC of moof; substantive on its own. |
| websocket transport mco | phase F. depends on phase E. |
| webrtc / browser renderer | phase G. |
| real type system / refinement / dependent types | phase H+. |
| datalog queries | phase H+. |
| become:-related serialization across vats | phase B+ stretch. |
| GC | minimal mark-sweep at turn boundaries; deferred unless heap pressure shows up in 2-replica tests. |
| operator precedence | `docs/process/open-questions.md` Q1 stands; explicit nesting only. |

---

## the read-the-docs-first discipline

before each track:

1. **re-read its doc gate.** every track above has cited doc files;
   the contract is in those files, not in this plan.
2. **ask: does the doc cover what you're about to do?** if no — that
   is the bug. fix the doc *first*, then implement.
3. **forcing function before writing code.** the test exists before
   the impl exists.
4. **245 → growing.** each track ends with a green
   `cargo test --workspace`. tracks compose; nothing piles up debt.

this matches `docs/process/docs-driven.md`. drift between docs and
code is the v3 mistake. we do not repeat it.

---

## risk register

ranked by likelihood × impact:

1. **mco loader's `dlopen` cross-platform variance.** macos /linux/
   windows dylib semantics differ enough that the one-mco-per-platform
   variant story may need refinement. mitigation: ship single-platform
   mcos initially (host's platform only); merge multi-platform later.
   *probability: medium. impact: high.*
2. **canonical encoding's IndexMap iteration drift.** if any leaf
   type uses a `HashMap` instead of `IndexMap`, determinism breaks.
   mitigation: a `cargo audit` pass over the substrate to ensure
   no `HashMap` is in the path of a heap value before track 3.
   *probability: medium. impact: high.*
3. **borrow-checker pain in vat scheduler + intent/receipt.** the
   re-entrant nature of "vat A sends to vat B which sends to vat A"
   stresses Rust's borrow rules. mitigation: vat state owned by the
   scheduler, not held across calls; pass `VatId` indices, look up
   on demand.
   *probability: high. impact: medium.*
4. **proto-edit-as-input in the replicated case.** D8's claim is
   that bytecode is per-replica-derived; the source is replicated.
   testing this end-to-end requires the recompile to be synchronous
   on the receiving replica, which depends on `compile.moof`
   working — and we are still using the rust compiler this session.
   mitigation: the rust compiler is fine for D's test. self-hosting
   is session 3's track.
   *probability: low. impact: medium.*
5. **time.** all eight tracks in one session is a lot. session-level
   strategy: tracks 1, 2, 3 are confident; track 4 (mco loader) is
   the keystone; tracks 5–8 may need a second session if 4 takes
   longer than budgeted. acceptable degradation: end of session at
   end of track 6, with tracks 7+8 deferred — *with persistence
   working*, that is still a substantial leap.
   *probability: high. impact: low (graceful degradation).*

---

## the ladder of acceptable session-end states

if the session goes ideally, all eight tracks land. if it doesn't,
here is the descending ladder of what counts as a successful
session-end:

1. **tracks 1–8 done.** phase D's gate passes. **target: this.**
2. **tracks 1–7 done; track 8 partially done.** reflector and
   envelope work; convergence test runs; small known-divergence
   sites identified. session 3 closes them.
3. **tracks 1–6 done.** persistence works end-to-end; `moof world`
   subcommand exists; intent/receipt model runs in the solo case.
   replication is foundational but not exercised. **fallback: this.**
4. **tracks 1–5 done.** vats and mailboxes work; persistence
   stubbed. graceful degradation; session 3 finishes B and lands D.
5. **tracks 1–4 done.** mco loader works; pattern matching ships;
   `become:` ships; the substrate is meaningfully more honest. all
   later tracks deferred.
6. **tracks 1–3 done.** smaller but real wins. session 3 has more
   to do; not a failure.

below ladder rung 6 the session is judged as "we learned but did
not ship." that is also fine. we adjust scope and try again.

---

## the inputs to the session

before this session starts, the following are pre-conditions:

- `git pull` to current state. confirm `cargo test --workspace`
  shows 245 / 245 passing. (it does as of `a64b5be`.)
- `cargo add libloading` for the mco loader.
- `cargo add lmdb-rkv` (or `rkv`, or `heed`; pick one and stick with it
  before track 6).
- `cargo add blake3` and `ed25519-dalek` (substrate-side stub
  implementations behind feature flags so the test mcos can be built).
- a coffee. this is dense work.

---

## the session's success rhetoric

the v4-take-2 manifesto (`docs/vision/manifesto.md`) ends with the
test:

> can a person, working only from inside moof, redefine a special
> form, rewrite the inspector for a domain object, query the world's
> history relationally, spawn a collaborator on another machine, and
> watch them edit a method live, and find everything as they left it
> tomorrow plus the collaborator's changes?

this session's deliverable is the **half** of that promise that's
load-bearing: *spawn a collaborator on another machine and watch
them edit a method live.* by session-end, the substrate must be
honest about determinism + replication + persistence. the rest
(renderer, websocket transport, presence) is wiring jobs in
sessions 3 and 4.

the substrate gets us to the threshold of the vision. after this
session, the world is one renderer mco + one transport mco + one
viewport-vat protocol away from `moof world join wss://…` working
end-to-end.

---

## post-session: the next sessions, briefly

the trajectory after this session, sketched:

| session | scope | end-state |
|---|---|---|
| **session N (this one)** | tracks 1–8 — pattern matching, become:, canonical encoding, mco loader, vats, persistence, determinism, 2-replica convergence test | phase D gate passes; substrate is honest |
| **session N+1** | parser.moof + compiler.moof (phase A-self-host); tighten any phase-D leftover; start phase E foundations (Frame, Placement, Pose) | rust line stops growing; world primitives exist |
| **session N+2** | terminal renderer mco; xterm-mouse mco; pixmap proto + tools; single-user 3D world (phase E) | `moof world ./worlds/...` shows a navigable world |
| **session N+3** | websocket transport mco; presence (cursor); leader failover; phase F gate | `moof world join wss://…` works |
| **session N+4** | gpu renderer mco; package format; type system foundations; long-tail polish (phase G) | the demo from `vision/one-page.md` is real |

four sessions to the demo. this one is the foundational keystone.

---

## final note

the docs are the source of truth. when the implementation diverges
from a doc, the doc is the bug to fix first — *unless* the doc is
the bug, in which case the doc is the bug to fix first. either way
the doc moves before the code.

the session begins with re-reading `docs/laws/substrate-laws.md`
and `docs/laws/determinism-laws.md` end-to-end. the laws are what
the code is *promising*; track 7's enforcement gate verifies the
promise; track 8's convergence test verifies the substrate has
been honest about it.

`>.<` softly. let's get to the vision. ૮ ◞ ﻌ ◟ ა
