# vat phase V2 — freezing — design

> **status:** brainstormed 2026-05-07. ready for plan.
>
> **prior art:** V0 (FormId scope-tagging, shipped) + V1 (per-turn nursery + diff, shipped). V2 builds on V1's turn machinery — every freeze is a turn-mutation that journals through the nursery, commits via `TurnDiff`, rolls back via `abort_turn`.
>
> **spec reference:** `2026-05-04-vats-and-references-protocol-design.md` §4 (the freezing model) is the user-facing spec. this document is the substrate-side implementation design.

## 1. scope and motivation

V2 adds shallow freezing as a substrate primitive. once a form is frozen, mutation attempts on its `slots` / `handlers` / `meta` raise `'frozen-form` immediately at the call site (not deferred to turn-end). there is no `:thaw`. freezing is one-way and the transition itself is a turn-mutation: it journals like any other mutation and rolls back cleanly on abort.

V2 also threads a **vat-mode** parameter through `World` so a single substrate can host either a *mutable-by-default* vat (stateful actors / UI / workspaces — `:new` returns mutable instances) or a *frozen-by-default* vat (parsers / compilers / computation kernels — `:new` returns frozen instances).

deep-freezing (transitively walking a form and freezing its reachable subgraph) is **moof-side stdlib**, not a substrate primitive. policy decisions — what counts as a live boundary, what to do on cycles, whether to walk handlers / meta — live in moof code where they're inspectable and modifiable.

V2 does **not** include cross-vat sharing (V5–V6), spawn (V8), or persistence (V9). it is a self-contained substrate refactor whose exit criterion is "freezing works correctly inside one vat, including rollback and journaling."

## 2. storage: a `frozen` bit on `Form`

a single `frozen: bool` field is added to the `Form` struct in `crates/substrate/src/form.rs`:

```rust
pub struct Form {
    pub proto: Value,
    pub slots: IndexMap<SymId, Value>,
    pub handlers: IndexMap<SymId, Value>,
    pub meta: IndexMap<SymId, Value>,
    pub frozen: bool,            // V2
}
```

the size cost is negligible — the three `IndexMap`s already dominate the form's footprint. the hot-path cost of the mutation guard is a single field read.

`Form::default()` and `Form::with_proto(...)` initialize `frozen: false` (born-mutable in substrate-internal allocations). vat-mode-driven born-frozen behavior is applied at the `:new` user-facing layer, never inside `world.alloc()`.

## 3. mutation guard

after V1, all heap mutation flows through `World::form_slot_set` / `form_handler_set` / `form_meta_set`. V2 changes those signatures to return `Result<(), RaiseError>`:

```rust
pub fn form_slot_set(&mut self, id: FormId, key: SymId, value: Value)
    -> Result<(), RaiseError>
{
    assert!(self.in_turn, "form_slot_set called outside a turn");
    if self.is_frozen(id) {
        let kind = self.intern("frozen-form");
        // per spec §4: kind is `'frozen-form`, the offending form-id
        // travels in `data` (carried as Value::Form, not stringified).
        let mut err = RaiseError::new(kind, "mutation on frozen form");
        err.data = Value::Form(id);
        return Err(err);
    }
    // ... existing V1 fast-path / delta-write logic ...
    Ok(())
}
```

the `?` propagates through the 4–5 substrate higher-level mutators (`env_bind`, `env_set`, `install_native`, `bump_proto_generation`, `macro_register`), which themselves become `Result`-returning. callers either propagate further (most VM op-handlers and intrinsics already live in `Result<Value, RaiseError>` contexts) or `.expect(...)` at boot sites that know the targets aren't frozen.

**why one guard at `form_*_set` rather than at higher layers:** future mutation paths added to the substrate are automatically guarded. one source of truth.

**why the panic guard for `!in_turn` stays an `assert!` rather than a raise:** mutation outside a turn is a substrate-side bug, not a user-recoverable error. V1 already established this invariant.

## 4. nursery integration

freezing is a turn-mutation. `Delta` and `TurnDiff` grow the smallest possible additions to track it.

### 4.1. `Delta.frozen: bool`

```rust
pub struct Delta {
    pub slots: IndexMap<SymId, Value>,
    pub handlers: IndexMap<SymId, Value>,
    pub meta: IndexMap<SymId, Value>,
    pub frozen: bool,            // V2 — one-way: false→true within a turn
}
```

`frozen` defaults to `false`. it is set to `true` exactly once, by `world.freeze(id)`, when the user freezes a pre-existing (canonical) form during a turn. for new-alloc forms (above watermark), freezing writes to `Form.frozen` directly on the canonical heap — the same fast-path principle as the existing `form_*_set` new-alloc branch.

### 4.2. `is_frozen(id)`: nursery-aware lookup

```rust
pub fn is_frozen(&self, id: FormId) -> bool {
    if self.heap.get(id).frozen {
        return true;
    }
    if self.in_turn && id.payload() < self.turn_watermark {
        if let Some(delta) = self.nursery_deltas.get(&id) {
            if delta.frozen {
                return true;
            }
        }
    }
    false
}
```

for new-alloc forms (above watermark), the canonical `Form.frozen` is authoritative — they don't have deltas. for pre-existing forms in an active turn, the delta is checked first.

### 4.3. `TurnDiff.freezings: Vec<FormId>`

```rust
pub struct TurnDiff {
    pub mutations: IndexMap<(FormId, FaceKind, SymId), (Value, Value)>,
    pub new_allocs: Vec<FormId>,
    pub freezings: Vec<FormId>,        // V2 — pre-existing forms that were frozen this turn
}
```

`freezings` lists *pre-existing* forms whose `frozen` bit transitioned `false→true` during the turn. forms allocated *and* frozen in the same turn (i.e. born-frozen via `:new` in frozen-by-default mode) appear in `new_allocs` but **not** in `freezings` — the new-alloc list already implies their final state, and consumers (replication, audit, V11 CRDT merge) need a separate signal only for transitions on already-canonical forms.

### 4.4. commit / abort semantics

- **`commit_turn`** copies each `Delta.frozen=true` for a pre-existing form into the canonical `Form.frozen`, and pushes the FormId onto `TurnDiff.freezings`.
- **`abort_turn`** drops `nursery_deltas` as it already does in V1; the `frozen` bit on the corresponding canonical `Form` is unchanged. this is how rollback unfreezes.

### 4.5. same-turn freeze blocks further mutation

once `world.freeze(id)` is called within turn T, subsequent `form_*_set(id, ...)` calls within turn T raise `'frozen-form`. there is no thaw — not even by abort-and-retry within the same turn. to "undo" a freeze you must abort the entire turn (the standard V1 abort path drops the delta and the freeze with it).

## 5. live-form refusal: `World.live_protos`

per spec §4, the substrate refuses to freeze certain forms — vat-Forms, mailbox-Forms, DataSource handles, cap-tokens — because their authority is mutable-by-design.

V2 implements this as a per-`World` set of "live protos":

```rust
pub struct World {
    // ...
    pub live_protos: HashSet<FormId>,    // V2
}
```

`world.freeze(id)` walks `id`'s proto chain and refuses if any ancestor is in `live_protos`, raising `'cannot-freeze-live` (FormId in `data`).

**why proto-chain rather than a per-form bit:** liveness is a property of *what kind of form this is*, which is exactly what the proto encodes. one source of truth. when V4 introduces `Vat` and `Mailbox` protos, V4's plan registers them in `world.live_protos` at boot — V2's freeze machinery picks up the new live kinds for free, no per-form bookkeeping to remember.

**V2's actual contents for `live_protos`:** the cap-bearing proto(s) currently registered in the substrate — at minimum the Console-cap proto used by `$out` / `$err`, plus whichever proto the moof-side `defcap` macro stamps onto its outputs. small enough to enumerate explicitly in V2 boot. V4+ phases add to it.

## 6. vat-mode and `:new`

### 6.1. the parameter

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VatMode {
    MutableByDefault,
    FrozenByDefault,
}

pub struct World {
    // ...
    pub vat_mode: VatMode,        // V2 — default MutableByDefault
}
```

new constructors at the crate root:

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModeScope {
    /// mode applies to user code only; bootstrap (intrinsics::install +
    /// lib/main.moof load) runs in MutableByDefault regardless of `mode`.
    /// safe default — lib code may mutate post-:initialize and would
    /// break under FrozenByDefault during bootstrap.
    PostBootstrap,
    /// mode applies from boot; lib bootstrap itself runs in `mode`.
    /// only safe for fully pure-ML-y worlds where lib has been audited
    /// or written for it. may break standard lib in V2; intended for
    /// future use cases (post-V4 spawn'd parser-vats / compiler-vats
    /// that boot a slimmer lib).
    FromBoot,
}

pub fn new_world_with_mode(mode: VatMode) -> World;
//   ↑ shortcut for new_world_with_mode_scoped(mode, ModeScope::PostBootstrap)
pub fn new_world_with_mode_scoped(mode: VatMode, scope: ModeScope) -> World;

pub fn new_world_bare_with_mode(mode: VatMode) -> World;
pub fn new_world_bare_with_mode_scoped(mode: VatMode, scope: ModeScope) -> World;
```

the existing `new_world()` and `new_world_bare()` default to `MutableByDefault` + `PostBootstrap`. backwards compatible — every existing test, intrinsic, and lib-load runs in mutable mode unchanged.

### 6.2. seal-after-initialize for `:new`

today's `Object:new` substrate native (`crates/substrate/src/intrinsics.rs:1682`):

```
alloc fresh form with proto=self → send :initialize → return instance
```

V2 inserts a freeze step at the tail in frozen-by-default mode:

```
alloc fresh form with proto=self
  → send :initialize  (still mutable; user's override populates slots)
  → if vat_mode == FrozenByDefault: world.freeze(instance)
  → return instance
```

this preserves the smalltalk-y constructor pattern. user-defined `:initialize` runs as before. the vat-mode switch is transparent — existing user protos in `:initialize` continue to work; in frozen-by-default mode they just produce frozen results.

`Table:new` (`intrinsics.rs:117`) gets the same treatment.

### 6.3. exemptions

substrate-internal allocations — `world.alloc()`, `alloc_env()`, `install_native()`'s method-Form alloc, the chunk-Form allocs in `compile`, and the reader's parser-Form allocs — are **not** mode-aware. they always produce mutable forms. these forms aren't user-visible "instances"; they're substrate machinery that needs to be writable for boot and for ongoing dispatch / compilation work.

if `:new` becomes the universal allocator for moof code (post-parser-port), more allocations naturally become mode-aware. that's a future migration, not V2 work.

### 6.4. boot consistency

substrate-internal boot (`intrinsics::install`, `$hash` mco install, the V1 boot turn) is mode-exempt — those allocations go through `world.alloc()` / `install_native()`, never through user-facing `:new`. so boot itself runs identically in either mode.

**but** the lib bootstrap that follows (`lib/main.moof` and the early modules it loads) calls user-facing `:new` and `:initialize` extensively while building up Compiler / Match / Defn / DefProto / etc. some of those instances may need to mutate after construction (e.g. macros that lazily populate a slot table). running that under `FrozenByDefault` would seal lib forms before lib code finishes setup, and break things in non-obvious ways. **but** users wanting a fully pure-ML-y world legitimately want the mode to apply all the way down, including to lib.

V2 exposes this as a knob (the `ModeScope` enum from §6.1) with a sensible default:

| scope | behavior | when to pick |
|---|---|---|
| `PostBootstrap` (default) | bootstrap loads in `MutableByDefault`; mode flips to `mode` immediately after `lib/main.moof` finishes | safe ergonomic default. existing lib code Just Works. user code sees the requested mode from its first instruction. |
| `FromBoot` | mode is `mode` from the very first allocation | only safe with audited / re-authored lib that doesn't post-`:initialize`-mutate. intended for future minimal-lib parser-vats / compiler-vats spawned via V8. **may break standard lib in V2** — opt-in expert path. |

implementation:

```rust
pub fn new_world_with_mode_scoped(mode: VatMode, scope: ModeScope) -> World {
    let initial_mode = match scope {
        ModeScope::PostBootstrap => VatMode::MutableByDefault,
        ModeScope::FromBoot => mode,
    };
    let mut w = build_world_with_initial_mode(initial_mode);  // intrinsics::install + lib load
    w.vat_mode = mode;  // either no-op (FromBoot) or post-bootstrap flip (PostBootstrap)
    w
}

pub fn new_world_with_mode(mode: VatMode) -> World {
    new_world_with_mode_scoped(mode, ModeScope::PostBootstrap)
}
```

semantically: in the default scope, `vat_mode` describes the mode for *user code that the embedder runs against this world*; lib is part of the world's substrate, not user code. in `FromBoot` scope, the user is asserting they have a lib (or no lib — `*_bare_*` variants) that's compatible with running under `mode`.

once V4 introduces multi-vat with spawn-time mode parameters per `Vat`, child vats will inherit / override mode at spawn; lib will already be loaded at the substrate level so the bootstrap concern doesn't recur. `ModeScope` becomes V2-only scaffolding that quietly retires.

## 7. moof-side `freezeRecursive`

deep-freeze policy lives in moof per spec §4. the substrate exposes the primitives; stdlib provides a sensible default and a granular variant.

### 7.1. substrate primitives bound on `Object`

| selector | shape | semantics |
|---|---|---|
| `:freeze` | `(self) -> self` (raises `'cannot-freeze-live`) | calls `world.freeze(id)`. on success, returns the same form (now frozen). on the live list, raises. journals via the nursery; rolls back on turn-abort. |
| `:frozen?` | `(self) -> Bool` | returns `world.is_frozen(id)`. nursery-aware: sees in-turn freezes. |
| `:freezable?` | `(self) -> Bool` | returns `not(is-live or already-frozen)`. lets policy code branch without `try` / `raise` / `catch`. |

### 7.2. stdlib: parameterized core + named variants

three layers in a new file `lib/stdlib/freezing.moof` (kept separate from `object.moof` so it's locatable; `freezeRecursive*` selectors install onto the `Object` proto from there):

**parameterized core** (does the actual walk):

```moof
(defmethod (Object freezeRecursiveWalking: faces)
  ;; faces: a Cons of face symbols, e.g. '(slots), '(slots handlers),
  ;; '(slots handlers meta). user-extensible.
  (cond
    ([self frozen?]              self)        ; cycle / idempotency
    ([not [self freezable?]]     self)        ; live boundary — silent skip
    (else
      [self freeze]
      [faces forEach:
        (fn (face)
          (cond
            ([face = 'slots]
             [[Heap slotKeysOf: self] forEach:
               (fn (k) [[Heap slotOf: self at: k] freezeRecursiveWalking: faces])])
            ([face = 'handlers]
             [[Heap handlerKeysOf: self] forEach:
               (fn (k) [[Heap handlerOf: self at: k] freezeRecursiveWalking: faces])])
            ([face = 'meta]
             [[Heap metaKeysOf: self] forEach:
               (fn (k) [[Heap metaOf: self at: k] freezeRecursiveWalking: faces])])
            (else (raise: 'unknown-face "freezeRecursiveWalking: unknown face symbol"))))]
      self)))
```

(the `Heap`-singleton accessors `slotKeysOf:` / `slotOf:at:` etc. are the V1 task-8 nursery-aware reflection helpers — they see in-turn writes correctly. `forEach:` is the standard Cons iterator.)

**named convenience variants:**

```moof
(defmethod (Object freezeRecursive)
  ;; default: slots only — the "freeze my data tree" intent.
  [self freezeRecursiveWalking: '(slots)])

(defmethod (Object freezeRecursiveSealed)
  ;; slots + handlers — pure-ML-y mode. seals behavior too.
  ;; what you reach for when freezing the output of a parser-vat
  ;; or sealing a computation kernel's protos.
  [self freezeRecursiveWalking: '(slots handlers)])
```

slots+handlers+meta (the exhaustive case) is rare enough that explicit `freezeRecursiveWalking: '(slots handlers meta)` is the supported path — no dedicated variant in V2 stdlib.

### 7.3. policy summary

| condition | behavior |
|---|---|
| form already frozen | stop recursing (cycle-safe; idempotent) |
| live boundary (proto-chain hits `live_protos`) | stop recursing silently (the user's data graph continues, terminates at the boundary) |
| non-Form value (tagged immediate, atomic) | no-op (already immutable) |

## 8. forward-looking: V6 shared-segment eligibility

V6 introduces a process-scope shared segment (`01…` scope per V0 §5) for content-addressed promotion of frozen forms across vats. **V2 does not implement promotion.** V2 only establishes the precondition: forms must be frozen before they can promote.

the V2 spec notes this so that V6's plan can refer back to a stable "what makes a form promotable" checkpoint:

- a form is **promotable** when `frozen == true` AND no proto in its chain is in `live_protos`.
- V2 ensures both conditions are queryable at any time via `is_frozen(id)` and the proto-chain walk inside `world.freeze`.

V6 will add the promotion path itself: blake3 canonical-bytes hashing, intern table, forwarding pointers, etc. V2 just makes sure we know which forms are eligible.

## 9. exit criteria

V2 lands when:

1. `Form.frozen` field exists; `Form::default()` initializes it `false`.
2. `World::form_slot_set` / `form_handler_set` / `form_meta_set` return `Result<(), RaiseError>` and raise `'frozen-form` on frozen targets. all V1 callers propagate `?`.
3. `Delta.frozen: bool` and `TurnDiff.freezings: Vec<FormId>` exist; commit copies delta-frozen into canonical and into `freezings`; abort drops the delta (and hence the freeze).
4. `World::is_frozen(id)` and `World::freeze(id) -> Result<(), RaiseError>` are public. `world.freeze` walks the proto chain and raises `'cannot-freeze-live` against `World.live_protos`.
5. `World.live_protos: HashSet<FormId>` exists, populated at boot with the cap-bearing proto(s).
6. `World.vat_mode: VatMode` field + `ModeScope` enum; `new_world_with_mode` / `new_world_with_mode_scoped` (and `_bare_` variants) constructors with `PostBootstrap` as the default scope; `:new` (Object, Table) seals-after-initialize when mode is `FrozenByDefault`.
7. `:freeze`, `:frozen?`, `:freezable?` methods bound on Object.
8. `lib/stdlib/object.moof` (or `freezing.moof`) exposes `freezeRecursiveWalking:`, `freezeRecursive`, `freezeRecursiveSealed`.
9. all 436 pre-V2 tests still pass; new tests cover: freeze-then-mutate raises; freeze-on-live raises; freeze-then-abort unfreezes; mode-toggle changes `:new` born-state; same-turn freeze blocks subsequent mutation; `freezeRecursive` walks slots only; `freezeRecursiveSealed` walks slots+handlers; cycles (back-edges hitting frozen ancestor) terminate; live boundary stops recursion.
10. zero new warnings; spec invariants documented inline at each public-API site.

## 10. test plan (sketch)

unit tests in `crates/substrate/src/world.rs::tests` cover:
- `freeze_then_form_slot_set_raises_frozen_form`
- `freeze_then_form_handler_set_raises_frozen_form`
- `freeze_then_form_meta_set_raises_frozen_form`
- `freeze_on_live_proto_raises_cannot_freeze_live`
- `is_frozen_reads_canonical_when_not_in_turn`
- `is_frozen_reads_delta_when_seeded`
- `freeze_in_turn_then_abort_canonical_unchanged`
- `freeze_in_turn_then_commit_canonical_frozen_and_in_freezings`
- `same_turn_freeze_then_mutate_raises_immediately`
- `new_alloc_can_be_frozen_via_form_frozen_direct_write`

integration tests in `crates/substrate/tests/freeze_e2e.rs`:
- `mutable_by_default_new_returns_mutable_form`
- `frozen_by_default_new_returns_frozen_form`
- `frozen_by_default_initialize_runs_before_freeze`
- `freeze_recursive_walks_slots_default`
- `freeze_recursive_sealed_walks_slots_and_handlers`
- `freeze_recursive_stops_at_live_boundary`
- `freeze_recursive_handles_cycle_via_already_frozen`
- `eval_program_raise_in_turn_after_freeze_rolls_back_freeze`

## 11. out of scope (deferred to later phases)

- **shared-segment promotion** of frozen forms across vats (V6).
- **multi-vat container** (V4): V2 hosts one vat; vat-mode lives on `World` rather than a `Vat` struct. V4 will move it.
- **`[$vat spawn:]` syntax** (V8): V2's mode is a `new_world` constructor parameter, not a moof-side spawn keyword.
- **CRDT-style per-slot merge hooks** (V11): `TurnDiff.freezings` is a list a future replicator can apply, but V2 doesn't replicate.
- **persistence of the frozen bit / freezings log** (V9): V2 keeps everything in memory.
- **explicit `freezeRecursiveExhaustive`** (slots + handlers + meta): V2 stdlib provides only `freezeRecursive` (slots) and `freezeRecursiveSealed` (slots + handlers). users wanting all-three call `freezeRecursiveWalking: '(slots handlers meta)` explicitly.
- **auditing standard lib for `ModeScope::FromBoot` compatibility**: V2 ships `FromBoot` as an opt-in expert constructor whose contract is "your lib must be compatible with `mode` from the very first allocation." V2 does **not** guarantee that the existing standard lib (`lib/main.moof` and friends) loads cleanly under `FromBoot` + `FrozenByDefault`. tests under that scope are best-effort. a future session would either rewrite lib to be `FromBoot`-clean, or maintain a slimmer "pure-ML-y baseline lib" alongside.

## see also

- `2026-05-04-vats-and-references-protocol-design.md` — overall vat phasing spec; §4 freezing model is the user-facing source of truth this document implements.
- `2026-05-06-vat-V1-nursery-diff-design.md` — V1's per-turn nursery, which V2 extends with the `frozen` bit on `Delta`.
- `2026-05-04-vat-V0-formid-scope-tagging-design.md` (or equivalent) — V0's FormId scope tagging; V6 promotion will use scope `01…` for shared-segment forms.
