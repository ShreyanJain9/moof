# Vat phase V1 — per-turn nursery + diff (design)

> **status: brainstormed 2026-05-06. ready for writing-plans.**
> **scope: implementation contract for the per-turn-nursery + diff
> mechanism specified in
> `2026-05-04-vats-and-references-protocol-design.md` §6 and §22 V1
> row. refines the master spec with concrete data structures, API
> surface, read/write paths, and rollback mechanics.**

## 1. scope

V1 introduces three load-bearing concepts to the substrate:

- **the turn** — the unit of atomicity. mutations within a turn either
  all commit or all roll back.
- **the nursery** — a per-turn buffer that holds (a) freshly allocated
  forms and (b) per-form deltas tracking mutations to pre-turn forms.
- **the diff** — the canonical record of what changed during a turn,
  produced at commit. flat keyed map of `(form, face, key) → (prior, new)`.

V1 ships before the scheduler (V4) and message-passing (V7), so there
is no natural "message-turn." V1 introduces an explicit turn API
(`start_turn` / `commit_turn` / `abort_turn`) and wraps `eval_program`
in an implicit turn so existing tests behave unchanged.

**non-goals** (deferred to later phases):
- on-disk `inputs.log` (V9 persistence)
- replication shipping of diffs (V11)
- CRDT merge consumption (V11)
- multi-vat carving — `World` remains the single host, with the turn
  API on `World` directly. V4 will lift the API to `Vat`.
- shared-segment promotion of frozen forms (V6)

## 2. concepts

### 2.1 the turn

A turn is bracketed by `start_turn` and `commit_turn` (or
`abort_turn`). exactly one turn at a time. mutations during a turn
buffer in the nursery; reads see the merged view (nursery deltas
override canonical for the same key, fall-through otherwise).

`eval_program` / `eval` wrap their body in an implicit turn:
- `start_turn` on entry
- `commit_turn` on successful return
- `abort_turn` if a `RaiseError` propagates out

So all existing test behavior is preserved: a `(def x 5)` in a test
mutates the world's heap by the time the next assertion runs, because
the implicit turn has committed.

### 2.2 the nursery

Two storage components, with different physical mechanisms for new
allocs vs. mutations of pre-existing forms:

**new allocs go to the canonical `Vec<Form>` directly, but above a
watermark.** at `start_turn`, record `turn_watermark = heap.forms.len()`
as a `u32`. allocations during the turn just push onto `Vec<Form>` as
before. forms with FormId payload `>= turn_watermark` are this-turn
allocations; forms with payload `< turn_watermark` are committed-in-prior-
turn (or at boot).

**mutations to pre-existing forms go to a delta map.** `nursery_deltas`
is `IndexMap<FormId, Delta>` keyed by the canonical FormId. each `Delta`
holds `IndexMap<SymId, Value>` for each face (`slots`, `handlers`, `meta`).
only touched keys are stored. on first mutation of a form's key, an
entry is created; subsequent writes to the same key overwrite the entry
(last-write-wins within the turn).

### 2.3 the diff

Computed at `commit_turn`. shape:

```rust
pub struct TurnDiff {
    /// per-(form, face, key) → (prior, new). last-write-wins per key
    /// within a turn; intermediate writes don't appear.
    pub mutations: IndexMap<(FormId, FaceKind, SymId), (Value, Value)>,
    /// FormIds allocated this turn (payload range
    /// `old_watermark..new_watermark`).
    pub new_allocs: Vec<FormId>,
}
```

Computed at commit by walking `nursery_deltas`: for each
`(form_id, delta)`, for each `(key, new_value)` in each face's map,
read prior from canonical (`heap.forms[form_id].slots[key]` etc.),
and emit a `(form_id, face, key, prior, new)` entry.

The diff is consumed (in V1) only by tests that verify it; in later
phases it feeds `inputs.log` (V9), replication (V11), and CRDT merge
(V11).

## 3. data structures

Additions to `World` (in `crates/substrate/src/world.rs`):

```rust
pub struct World {
    // ... existing fields ...

    /// the current turn's mutation deltas, keyed by FormId of
    /// pre-existing forms (payload < `turn_watermark`). forms
    /// allocated this turn are NOT in this map — they're at
    /// `heap.forms[i]` for `i >= turn_watermark`.
    pub nursery_deltas: IndexMap<FormId, Delta>,

    /// the FormId payload below which forms are canonical
    /// (committed in a prior turn or at boot). forms at payloads
    /// `>= turn_watermark` are this-turn allocations.
    pub turn_watermark: u32,

    /// `true` iff a turn is currently active. `start_turn` flips
    /// on; `commit_turn` / `abort_turn` flip off. nested
    /// `start_turn` calls panic — V1 supports exactly one active
    /// turn at a time.
    pub in_turn: bool,
}
```

New types in a new module `crates/substrate/src/nursery.rs`:

```rust
use indexmap::IndexMap;
use crate::sym::SymId;
use crate::value::Value;
use crate::form::FormId;

/// the three faces of a Form that participate in mutation buffering.
/// matches `Form`'s structural shape.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FaceKind {
    Slots,
    Handlers,
    Meta,
}

/// per-form delta accumulated during a turn for forms allocated
/// before the turn started. only touched keys are stored.
#[derive(Default, Debug)]
pub struct Delta {
    pub slots: IndexMap<SymId, Value>,
    pub handlers: IndexMap<SymId, Value>,
    pub meta: IndexMap<SymId, Value>,
}

impl Delta {
    /// access the IndexMap for a given face.
    pub fn face_mut(&mut self, face: FaceKind) -> &mut IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &mut self.slots,
            FaceKind::Handlers => &mut self.handlers,
            FaceKind::Meta => &mut self.meta,
        }
    }

    pub fn face(&self, face: FaceKind) -> &IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &self.slots,
            FaceKind::Handlers => &self.handlers,
            FaceKind::Meta => &self.meta,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.handlers.is_empty() && self.meta.is_empty()
    }
}

/// the diff produced at `commit_turn`. consumed by V1 tests; will
/// feed `inputs.log` (V9), replication (V11), CRDT merge (V11).
#[derive(Default, Debug)]
pub struct TurnDiff {
    /// per-(form, face, key) → (prior, new). dedup-keyed: last
    /// write within a turn wins. forms allocated this turn DO NOT
    /// appear here — they're listed in `new_allocs` instead.
    pub mutations: IndexMap<(FormId, FaceKind, SymId), (Value, Value)>,

    /// FormIds allocated this turn, in allocation order. payload
    /// range is `old_watermark..new_watermark`.
    pub new_allocs: Vec<FormId>,
}
```

## 4. API surface

Public methods on `World`, in `crates/substrate/src/world.rs`:

```rust
impl World {
    /// begin a turn. panics if a turn is already active.
    pub fn start_turn(&mut self);

    /// commit the active turn. computes and returns the
    /// `TurnDiff`. applies nursery deltas to canonical heap.
    /// advances `turn_watermark` to current heap length.
    /// clears `nursery_deltas`. flips `in_turn` off.
    /// panics if no turn is active.
    pub fn commit_turn(&mut self) -> TurnDiff;

    /// abort the active turn. truncates `heap.forms` to
    /// `turn_watermark` (drops this-turn allocations). clears
    /// `nursery_deltas` (drops buffered mutations). flips
    /// `in_turn` off. watermark unchanged. panics if no turn
    /// is active.
    pub fn abort_turn(&mut self);

    /// `true` iff a turn is currently active.
    pub fn in_turn(&self) -> bool;
}
```

`eval` and `eval_program` (in `lib.rs`) wrap their body:

```rust
pub fn eval_program(w: &mut World, source: &str) -> Result<Value, RaiseError> {
    let was_in_turn = w.in_turn();
    if !was_in_turn {
        w.start_turn();
    }
    let result = eval_program_inner(w, source);
    if !was_in_turn {
        match &result {
            Ok(_) => { w.commit_turn(); }
            Err(_) => { w.abort_turn(); }
        }
    }
    result
}
```

The `was_in_turn` check makes the wrapping idempotent: if the caller
already started a turn (e.g., a test, or future scheduler code), the
inner `eval_program` doesn't open a nested turn.

## 5. read path

Every existing `heap.get(id).slot(k)` / `handler(k)` / `meta_at(k)`
call site must route through a nursery-aware path during a turn.

**new methods on `World`** (the canonical read path):

```rust
impl World {
    /// read a form's slot value, nursery-aware. checks nursery
    /// delta first when the form is pre-existing and a turn is
    /// active; falls through to canonical heap otherwise.
    pub fn form_slot(&self, id: FormId, key: SymId) -> Value;

    /// read a form's handler entry. returns `None` if absent.
    pub fn form_handler(&self, id: FormId, key: SymId) -> Option<Value>;

    /// read a form's meta entry. returns `Value::Nil` if absent.
    pub fn form_meta(&self, id: FormId, key: SymId) -> Value;
}
```

Internal logic (slots example; handlers and meta are analogous):

```rust
pub fn form_slot(&self, id: FormId, key: SymId) -> Value {
    if self.in_turn && id.payload() < self.turn_watermark {
        if let Some(delta) = self.nursery_deltas.get(&id) {
            if let Some(v) = delta.slots.get(&key) {
                return *v;
            }
        }
    }
    // fall through to canonical (also covers new-alloc forms,
    // which are in heap.forms[id.payload()] directly).
    self.heap.get(id).slot(key)
}
```

For new-alloc forms (id.payload() >= turn_watermark), reads go
straight through to `heap.get(id).slot(key)` — they're physically
in the canonical Vec already, just above the watermark.

## 6. write path

Every existing `heap.get_mut(id).slots.insert(k, v)` /
`handlers.insert` / `meta.insert` site must route through nursery-
aware writers.

**new methods on `World`** (the canonical write path):

```rust
impl World {
    /// set a slot, nursery-aware. for pre-existing forms during
    /// an active turn, writes to nursery delta. for new-alloc
    /// forms (id.payload() >= turn_watermark), writes directly
    /// to canonical heap (they're already nursery-semantic).
    /// panics if `!in_turn` — substrate disallows mutation
    /// outside a turn.
    pub fn form_slot_set(&mut self, id: FormId, key: SymId, value: Value);

    pub fn form_handler_set(&mut self, id: FormId, key: SymId, value: Value);

    pub fn form_meta_set(&mut self, id: FormId, key: SymId, value: Value);
}
```

Internal logic (slots example):

```rust
pub fn form_slot_set(&mut self, id: FormId, key: SymId, value: Value) {
    assert!(self.in_turn, "form_slot_set outside a turn");
    if id.payload() >= self.turn_watermark {
        // new alloc — write directly to canonical heap
        self.heap.get_mut(id).slots.insert(key, value);
    } else {
        // pre-existing form — buffer in nursery
        let delta = self.nursery_deltas.entry(id).or_default();
        delta.slots.insert(key, value);
    }
}
```

The `panic!("...outside a turn")` is load-bearing: V1 makes it a
substrate-laws invariant that **all mutation happens within a turn**.
This is what makes the turn the unit of atomicity.

## 7. mutation site audit

Every site in the substrate that currently calls
`heap.get_mut(id).slots.insert(...)` / `handlers.insert(...)` /
`meta.insert(...)` must migrate to the new write API. preliminary
audit (final list during implementation):

**in `world.rs`:**
- `env_bind` — uses `heap.get_mut(env).slots.insert(name, value)`
- `env_set` — same shape, walking proto chain
- `install_native` — writes to `meta` and `handlers` of method/proto forms
- `bump_proto_generation` — writes to proto's `meta`
- `macro_register` — writes to `Macros` form's slots
- `frame_snapshot` — allocates a new form; populates its slots (new-
  alloc path; writes through nursery aware would go to canonical
  directly since the new form is above watermark)

**in `intrinsics.rs`:**
- the `slotSet!` native
- the `setHandler!` native
- the `getOrCreateProto` native (writes to a fresh form's meta;
  new-alloc path)
- proto-bootstrap calls during `intrinsics::install` (BEFORE first
  turn — see §11 Edge Cases)
- the cap-binding sites (`$transporter`, `$compiler`, `$mco`, `$hash`,
  etc.) — most happen during boot, before any user turn

**in `compiler.rs` / `vm.rs`:**
- chunk side-table inserts (`chunk_ops`, `chunk_consts`, `chunk_ics`)
  — these are NOT slot/handler/meta mutations. they're substrate-
  internal caches keyed by FormId. they don't go through the nursery.
  (the chunk's `:bytecodes` reflection method reads through them as
  if they were slots, but the writes happen during compilation which
  is itself part of a turn.)

**in `wasm.rs`:**
- mco loader writes proto handlers when installing wasm-backed
  methods — happens at `[$mco load: ...]` time, during a turn (the
  turn that's running the user's `[$mco load:]` call).

Estimated ~25–30 sites across the codebase.

## 8. commit logic

```rust
pub fn commit_turn(&mut self) -> TurnDiff {
    assert!(self.in_turn, "commit_turn outside a turn");

    let mut diff = TurnDiff::default();

    // process mutations to pre-existing forms
    for (form_id, delta) in std::mem::take(&mut self.nursery_deltas) {
        let canonical = self.heap.get_mut(form_id);

        // slots
        for (key, new_value) in delta.slots {
            let prior = canonical.slots.get(&key).copied().unwrap_or(Value::Nil);
            diff.mutations.insert((form_id, FaceKind::Slots, key), (prior, new_value));
            canonical.slots.insert(key, new_value);
        }
        // handlers
        for (key, new_value) in delta.handlers {
            let prior = canonical.handlers.get(&key).copied().unwrap_or(Value::Nil);
            diff.mutations.insert((form_id, FaceKind::Handlers, key), (prior, new_value));
            canonical.handlers.insert(key, new_value);
        }
        // meta
        for (key, new_value) in delta.meta {
            let prior = canonical.meta.get(&key).copied().unwrap_or(Value::Nil);
            diff.mutations.insert((form_id, FaceKind::Meta, key), (prior, new_value));
            canonical.meta.insert(key, new_value);
        }
    }

    // collect new-alloc FormIds
    let new_high = self.heap.forms.len() as u32;
    diff.new_allocs = (self.turn_watermark..new_high)
        .map(FormId::vat_local)
        .collect();

    // advance watermark to include this turn's allocs
    self.turn_watermark = new_high;
    self.in_turn = false;

    diff
}
```

Note: prior value reads use `Value::Nil` as the sentinel for "key was
absent before." This conflates "absent" with "explicitly nil," which
matches `Form::slot`'s existing behavior. Honest about the limitation.

## 9. abort logic

```rust
pub fn abort_turn(&mut self) {
    assert!(self.in_turn, "abort_turn outside a turn");

    // drop new-alloc forms by truncating Vec to watermark
    self.heap.forms.truncate(self.turn_watermark as usize);

    // drop buffered mutations
    self.nursery_deltas.clear();

    self.in_turn = false;
}
```

## 10. diff structure rationale

Why dedup-keyed (last-write-wins per key per turn) rather than
ordered mutation log?

**replication.** what gets shipped to a follower is the *result* of
applying the turn, not the trace. shipping `(form, key, prior, new)`
is enough; followers don't need to see intermediate states.

**CRDT merge.** Mergeable protos receive `(prior, new)` per slot and
compute their merge function. intermediate writes within a turn are
local-only — only the final committed value participates in CRDT
merging across replicas.

**replay.** the input log is the source of truth; replay re-runs the
input, regenerating intermediate states as needed. the diff is for
verification, not replay.

Ordered mutation logs would matter for *time-travel debugging within
a turn*, which is a debugging-tool concern, not a substrate concern.
defer.

## 11. edge cases

**Turn-aware boot.** `World::new()` and `intrinsics::install` run
during construction, before any `eval_program` call. they perform
many mutations (proto setup, native installs, env binding). for V1,
the rule is: **boot runs in an implicit "boot turn" that
auto-commits before `new_world` returns.** specifically:

- `World::new()` calls `start_turn()` immediately after constructing
  the empty world.
- the constructor + `intrinsics::install` mutate freely through the
  nursery.
- before returning, `World::new()` calls `commit_turn()` and
  discards the diff (no observer).

This means the post-boot watermark sits *above* the bootstrap forms,
and post-boot turns see them as canonical. clean.

The `lib/main.moof` load (which happens in `new_world`) runs in its
own implicit turn via `eval_program`'s wrapping.

**Existing `heap.get(id).slot(k)` reads.** there are many. they
work *correctly outside a turn* (read canonical directly). they
read *stale* values *during a turn* if the slot has been mutated
this turn. for V1, **all read sites in the substrate must migrate
to `World::form_slot` etc. when they could possibly run during a
turn**. since boot is wrapped in a turn, this means essentially
everything migrates. the audit will be careful.

(`Form::slot` etc. remain — they're the underlying access used by
`World::form_slot` after the nursery check. tests can call them
directly when they want raw post-commit state.)

**Tagged-immediate singletons.** writing to `(slotSet! 5 'foo 42)`
goes through `World::ensure_writable_form_id` which lazy-allocates
a singleton-Form for the immediate. that allocation happens inside
the turn (since mutation is inside a turn) and lives above the
watermark — naturally treated as a new alloc.

**Reading slots during commit.** `commit_turn` itself does
`heap.get_mut(form_id)` to apply deltas. this is fine: by then
`in_turn = true` still, but the implementation reads canonical
directly (the deltas have been moved out via `std::mem::take`).
no nursery cycle.

**Nested `start_turn` panics.** V1 does not support nested turns.
calling `start_turn()` while `in_turn` is true panics. this
forecloses on a subtle correctness question (do nested turns nest
deltas? is rollback partial?) that doesn't need answering for V1.
when V4 introduces multi-vat, each vat has its own turn state;
nesting within a single vat remains disallowed.

**Crash mid-mutation.** if the rust code itself panics mid-turn
(not a moof RaiseError, but a real rust panic), the turn doesn't
get aborted — the world is in an inconsistent state. V1 doesn't
guard against this; rust panics are substrate bugs, recoverable
only by process restart. moof RaiseErrors propagate to
`eval_program`'s `Err` arm and trigger `abort_turn`.

## 12. testing strategy

New tests in a dedicated module
`crates/substrate/src/nursery.rs` (or inline in `world.rs` near the
turn API):

1. **explicit turn happy path.** start_turn → alloc form → mutate
   slot → commit_turn → assert diff has expected entries → assert
   canonical heap has new state.
2. **explicit turn abort.** start_turn → alloc form → mutate slot →
   abort_turn → assert canonical heap unchanged → assert nursery is
   empty → assert Vec is truncated.
3. **read-your-writes within a turn.** start → write → read → assert
   read returns new value (not canonical's old value).
4. **last-write-wins in diff.** start → write k=1 → write k=2 →
   commit → assert diff has `(prior=Nil, new=2)`, not (1, 2) and not
   (Nil, 1).
5. **raise auto-aborts.** eval that raises mid-program → assert
   prior state intact.
6. **eval_program implicit-turn happy path.** existing 388 tests
   continue to pass — this is the regression test.
7. **mutation outside a turn panics.** call `form_slot_set` without
   start_turn → panic with expected message.
8. **nested start_turn panics.** start_turn → start_turn → panic.
9. **boot runs in a turn that commits cleanly.** `new_world()` returns
   with `in_turn() == false` and a populated, queryable heap.
10. **diff captures handlers and meta mutations**, not just slots.

Estimated: ~10 new tests. existing 388 should all pass unchanged.

## 13. implementation phasing — the V1 sub-tasks

Dependency-ordered. each sub-task lands as one or more commits;
tests pass at each boundary. this list is the input to writing-plans.

**V1.0 — Delta + TurnDiff + FaceKind types.** create
`crates/substrate/src/nursery.rs` with the type definitions. add to
`lib.rs` `pub mod nursery`. unit tests for `Delta::face` /
`face_mut` / `is_empty` and `TurnDiff::default`. no behavior change.

**V1.1 — World fields + turn lifecycle API.** add
`nursery_deltas`, `turn_watermark`, `in_turn` fields to `World::new`.
implement `start_turn`, `commit_turn`, `abort_turn`, `in_turn` with
the assertion patterns. tests for the lifecycle (nested-turn panic,
abort-without-start panic, etc.). no other code uses the API yet.

**V1.2 — read path: form_slot / form_handler / form_meta.** add the
nursery-aware read methods. unit-test that they fall through to
canonical when `!in_turn`, and that they check the delta when
`in_turn` is set with seeded delta. internal substrate code does NOT
migrate yet.

**V1.3 — write path: form_slot_set / form_handler_set /
form_meta_set.** add the nursery-aware write methods. unit-test:
mutation outside a turn panics; mutation inside a turn lands in
delta for pre-existing forms; mutation inside a turn lands in
canonical for new-alloc forms; commit applies delta correctly.

**V1.4 — boot wrapping.** wrap `World::new` body in a
start/commit-turn pair (the "boot turn"). discard the diff. confirm
post-boot `in_turn() == false` and heap is queryable. all 388 tests
still pass via direct heap reads (which are still correct outside a
turn).

**V1.5 — eval_program / eval implicit-turn wrap.** wrap
`eval_program` and `eval` in `lib.rs` with the `was_in_turn` idempotent
pattern. tests pass; raise auto-aborts via `Err` arm. one new test
for raise-aborts-implicit-turn.

**V1.6 — migrate substrate mutation sites: env_bind, env_set,
macro_register, install_native (its meta + handler writes).** these
are in `world.rs`. relatively self-contained.

**V1.7 — migrate substrate read sites that read in turn-active
contexts: env_lookup, lookup_handler, etc.** these need to use
`form_slot` etc. instead of `heap.get(id).slot(k)`. careful audit.

**V1.8 — migrate `intrinsics.rs` mutation sites.** `slotSet!`,
`setHandler!`, `getOrCreateProto`, the cap-binding installers. there
are many; this is the bulk of the audit.

**V1.9 — migrate `intrinsics.rs` read sites that match the
mutation-site audit's coverage.** any intrinsic that reads a slot it
might have just written, or reads from a form that was mutated
this turn, must use the nursery-aware path.

**V1.10 — migrate `wasm.rs` mco-load mutations.** the wasm trampoline
installs handlers on a proto when loading an mco; this happens
inside a user turn.

**V1.11 — migrate `compiler.rs` and `vm.rs` mutation sites that fall
within a turn.** chunk side-tables stay direct (substrate-internal
caches, not user-visible Form mutations). but any place that mutates
a Form's slots/handlers/meta to record compilation results goes
through the nursery.

**V1.12 — bump_proto_generation migration.** the generation counter
lives in proto's `meta`. routing it through the nursery is necessary
for IC invalidation correctness across rollback (an aborted turn must
not leave generations bumped).

**V1.13 — comprehensive test sweep.** all 10 new tests + the
existing 388 must pass. add a dedicated `nursery_e2e` test file in
`crates/substrate/tests/` exercising the public API end-to-end.

**V1.14 — verification gate.** run full workspace + count tests +
inspect warnings + `cargo audit`.

Each sub-task is tightly bounded. implementing all of V1 is
substantially larger than V0 — estimated 14 sub-tasks vs. V0's 6.
sub-tasks V1.6–V1.12 are the migration grind; the others are
small.

## 14. references

- master spec: `2026-05-04-vats-and-references-protocol-design.md` —
  §6 (state model), §15 (life of a turn), §16 (life of a form),
  §18 (GC sketch — nursery-as-young-gen), §22 V1 row.
- forms: `docs/concepts/forms.md` — the four faces this design uses.
- determinism: `laws/determinism-laws.md` D5 — IndexMap iteration
  order is the substrate's promise; nursery and diff inherit it.
- reflection: `laws/reflection-contract.md` R6 — anything substrate-
  side needs to be queryable from moof. the diff *itself* is not
  yet exposed to moof in V1; doing so is a V11 concern.
