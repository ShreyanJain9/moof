# Vat phase V1 — per-turn nursery + diff implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce per-turn nursery + diff mechanism into the substrate. After this lands, all mutation routes through a buffered nursery during a turn; commit produces a per-(form, face, key) diff; abort rolls back. Existing 388 tests continue passing via implicit turn wrapping in `eval_program` / `eval`. New tests cover explicit turn API, rollback, raise-auto-aborts, and diff structure.

**Architecture:** A "turn" is the unit of atomicity. Mutations during a turn buffer in (a) per-form `Delta` maps for pre-existing forms (keyed by canonical FormId) and (b) the canonical `Vec<Form>` directly for new allocations (above a `turn_watermark`). Reads check delta first, fall through to canonical. Commit applies deltas + emits a `TurnDiff`. Abort truncates the Vec to watermark + clears deltas. `eval_program` wraps its body in an implicit turn (idempotent via `was_in_turn`). Boot also runs in an implicit "boot turn" that auto-commits before `new_world` returns.

**Tech Stack:** Rust 2021, no new dependencies. `IndexMap` from the existing `indexmap` crate for delta storage (deterministic insertion-order iteration per `laws/determinism-laws.md` D5). All work is within `crates/substrate/`.

**Spec reference:** `docs/superpowers/specs/2026-05-06-vat-V1-nursery-diff-design.md`

**Series context:** This is the second of ~12 plans (V0–V11) implementing the vat architecture spec. V0 (FormId scope-tagging) is already merged. V2 (per-form freezing) follows.

---

## File Structure

| file | role | change kind |
|---|---|---|
| `crates/substrate/src/nursery.rs` | new module: `Delta`, `FaceKind`, `TurnDiff` types | **created** |
| `crates/substrate/src/lib.rs` | add `pub mod nursery;` declaration | minor edit |
| `crates/substrate/src/world.rs` | add fields (`nursery_deltas`, `turn_watermark`, `in_turn`); add API (`start_turn`, `commit_turn`, `abort_turn`, `in_turn`, `form_slot`, `form_handler`, `form_meta`, `form_slot_set`, `form_handler_set`, `form_meta_set`); migrate mutation+read sites (env_bind, env_set, install_native, macro_register, bump_proto_generation, frame_snapshot, env_lookup, lookup_handler, lookup_handler_super); wrap `World::new` body in boot turn | **substantial** |
| `crates/substrate/src/lib.rs` | wrap `eval_program` / `eval` in implicit turns | minor edit |
| `crates/substrate/src/intrinsics.rs` | migrate `slotSet!`, `setHandler!`, `getOrCreateProto`, cap-binding sites; migrate corresponding read paths | **substantial** |
| `crates/substrate/src/wasm.rs` | migrate proto-handler installs at `[$mco load:]` time | medium edit |
| `crates/substrate/src/compiler.rs`, `crates/substrate/src/vm.rs` | migrate any Form mutations (chunk side-tables stay direct as substrate caches) | small edits |
| `crates/substrate/tests/nursery_e2e.rs` | new integration test file: explicit turn API, commit/abort, diff capture | **created** |

---

## Task 1: Create `nursery` module with `Delta`, `FaceKind`, `TurnDiff` types

**Files:**
- Create: `crates/substrate/src/nursery.rs`
- Modify: `crates/substrate/src/lib.rs` (add `pub mod nursery;`)

- [ ] **Step 1: Write failing tests for the new types**

In a fresh `crates/substrate/src/nursery.rs`, define the types and inline tests at the bottom:

(We write the file contents directly because the types are small and the tests reference them.)

- [ ] **Step 2: Run tests to verify the module doesn't exist yet**

Run: `cargo test -p substrate nursery::tests`
Expected: compile error — module `nursery` not found.

- [ ] **Step 3: Create `crates/substrate/src/nursery.rs` with the full module**

```rust
//! per-turn nursery + diff types.
//!
//! a turn is the unit of atomicity (`docs/superpowers/specs/
//! 2026-05-06-vat-V1-nursery-diff-design.md`). mutations during
//! a turn either land in the nursery (for pre-existing forms,
//! keyed deltas) or directly in the canonical heap above the
//! `turn_watermark` (for new allocations). commit produces a
//! `TurnDiff` summarizing what changed; abort drops the buffered
//! state and truncates the heap to watermark.

use indexmap::IndexMap;

use crate::form::FormId;
use crate::sym::SymId;
use crate::value::Value;

/// the three faces of a Form that participate in mutation
/// buffering. matches `Form`'s structural shape: slots /
/// handlers / meta. (`docs/concepts/forms.md`.)
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FaceKind {
    Slots,
    Handlers,
    Meta,
}

/// per-form delta accumulated during a turn for forms that
/// existed before the turn started. only touched keys are
/// stored; unchanged keys fall through to canonical at read
/// time.
///
/// note: forms allocated *during* the turn (FormId payload
/// >= `turn_watermark`) do NOT use a Delta — they live in the
/// canonical `Vec<Form>` above the watermark and are mutated
/// directly. the Delta is exclusively for pre-existing forms.
#[derive(Default, Debug)]
pub struct Delta {
    pub slots: IndexMap<SymId, Value>,
    pub handlers: IndexMap<SymId, Value>,
    pub meta: IndexMap<SymId, Value>,
}

impl Delta {
    /// access the `IndexMap` for a given face, mutably.
    pub fn face_mut(&mut self, face: FaceKind) -> &mut IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &mut self.slots,
            FaceKind::Handlers => &mut self.handlers,
            FaceKind::Meta => &mut self.meta,
        }
    }

    /// access the `IndexMap` for a given face, immutably.
    pub fn face(&self, face: FaceKind) -> &IndexMap<SymId, Value> {
        match face {
            FaceKind::Slots => &self.slots,
            FaceKind::Handlers => &self.handlers,
            FaceKind::Meta => &self.meta,
        }
    }

    /// `true` iff no key has been touched in any face.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
            && self.handlers.is_empty()
            && self.meta.is_empty()
    }
}

/// the result of `commit_turn`: a record of what changed
/// during the turn. consumed (in V1) by tests; will feed the
/// `inputs.log` (V9), replication (V11), and CRDT merge
/// pathways (V11).
///
/// the `mutations` map is dedup-keyed by `(form, face, key)` —
/// last-write-wins per key per turn. intermediate writes within
/// a turn don't appear; only the final value at commit-time does.
/// the `prior` value is what was in the canonical heap at
/// turn-start; `new` is the final value the turn settled on.
///
/// `new_allocs` lists FormIds allocated this turn, in
/// allocation order. forms in `new_allocs` do NOT appear in
/// `mutations` (they have no prior state).
#[derive(Default, Debug)]
pub struct TurnDiff {
    pub mutations: IndexMap<(FormId, FaceKind, SymId), (Value, Value)>,
    pub new_allocs: Vec<FormId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_default_is_empty() {
        let d = Delta::default();
        assert!(d.is_empty());
        assert!(d.slots.is_empty());
        assert!(d.handlers.is_empty());
        assert!(d.meta.is_empty());
    }

    #[test]
    fn delta_face_returns_correct_map() {
        let mut d = Delta::default();
        d.slots.insert(SymId(1), Value::Int(10));
        d.handlers.insert(SymId(2), Value::Int(20));
        d.meta.insert(SymId(3), Value::Int(30));

        assert_eq!(d.face(FaceKind::Slots).get(&SymId(1)).copied(), Some(Value::Int(10)));
        assert_eq!(d.face(FaceKind::Handlers).get(&SymId(2)).copied(), Some(Value::Int(20)));
        assert_eq!(d.face(FaceKind::Meta).get(&SymId(3)).copied(), Some(Value::Int(30)));
    }

    #[test]
    fn delta_face_mut_returns_mutable_map() {
        let mut d = Delta::default();
        d.face_mut(FaceKind::Slots).insert(SymId(1), Value::Int(42));
        assert_eq!(d.slots.get(&SymId(1)).copied(), Some(Value::Int(42)));
    }

    #[test]
    fn delta_is_empty_after_only_default_face_mut_lookups() {
        let mut d = Delta::default();
        // touching face_mut without inserting shouldn't make it non-empty.
        let _ = d.face_mut(FaceKind::Slots);
        assert!(d.is_empty());
    }

    #[test]
    fn delta_is_not_empty_after_inserting_into_any_face() {
        let mut d1 = Delta::default();
        d1.slots.insert(SymId(1), Value::Nil);
        assert!(!d1.is_empty());

        let mut d2 = Delta::default();
        d2.handlers.insert(SymId(1), Value::Nil);
        assert!(!d2.is_empty());

        let mut d3 = Delta::default();
        d3.meta.insert(SymId(1), Value::Nil);
        assert!(!d3.is_empty());
    }

    #[test]
    fn turn_diff_default_is_empty() {
        let td = TurnDiff::default();
        assert!(td.mutations.is_empty());
        assert!(td.new_allocs.is_empty());
    }

    #[test]
    fn turn_diff_can_record_mutations_and_allocs() {
        let mut td = TurnDiff::default();
        td.mutations.insert(
            (FormId::vat_local(5), FaceKind::Slots, SymId(7)),
            (Value::Int(1), Value::Int(2)),
        );
        td.new_allocs.push(FormId::vat_local(10));

        assert_eq!(td.mutations.len(), 1);
        assert_eq!(td.new_allocs.len(), 1);
        let entry = td.mutations
            .get(&(FormId::vat_local(5), FaceKind::Slots, SymId(7)))
            .copied();
        assert_eq!(entry, Some((Value::Int(1), Value::Int(2))));
    }

    #[test]
    fn face_kind_variants_distinguish() {
        assert_ne!(FaceKind::Slots, FaceKind::Handlers);
        assert_ne!(FaceKind::Handlers, FaceKind::Meta);
        assert_ne!(FaceKind::Slots, FaceKind::Meta);
    }
}
```

- [ ] **Step 4: Add `pub mod nursery;` to `crates/substrate/src/lib.rs`**

Find the existing `pub mod ...;` block (around lines 22–36) and insert `pub mod nursery;` alphabetically (between `meta` if any and `opcodes`, or where it fits — currently between `intrinsics` and `opcodes`):

```rust
pub mod compiler;
pub mod foreign;
pub mod form;
pub mod heap;
pub mod intrinsics;
pub mod nursery;          // ← add this
pub mod opcodes;
pub mod protos;
pub mod reader;
pub mod sym;
pub mod table;
pub mod transporter;
pub mod value;
pub mod vm;
pub mod wasm;
pub mod world;
```

- [ ] **Step 5: Run tests to verify all 8 new tests pass**

Run: `cargo test -p substrate nursery::tests`
Expected: 8 tests pass.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: `TOTAL pass=396` (388 baseline + 8 new tests).

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/nursery.rs crates/substrate/src/lib.rs
git commit -m "$(cat <<'EOF'
nursery: introduce Delta / FaceKind / TurnDiff types

V1.0 — type foundation for the per-turn nursery + diff mechanism.
no behavior change yet; the types are unused by world.rs / heap.rs
in this commit. subsequent tasks add World fields, lifecycle API,
read/write paths, and migrate mutation sites.

spec ref: docs/superpowers/specs/2026-05-06-vat-V1-nursery-diff-design.md §3

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: World fields + turn lifecycle API

**Files:**
- Modify: `crates/substrate/src/world.rs`

- [ ] **Step 1: Write failing tests for the lifecycle API**

In `crates/substrate/src/world.rs`, inside the existing `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn fresh_world_is_not_in_turn_after_construction() {
        let w = World::new();
        // post-boot, the boot turn has committed; we're outside a turn.
        assert!(!w.in_turn());
    }

    #[test]
    fn start_turn_flips_in_turn_on_and_commit_flips_off() {
        let mut w = World::new();
        assert!(!w.in_turn());
        w.start_turn();
        assert!(w.in_turn());
        let _diff = w.commit_turn();
        assert!(!w.in_turn());
    }

    #[test]
    fn start_turn_then_abort_flips_in_turn_off() {
        let mut w = World::new();
        w.start_turn();
        assert!(w.in_turn());
        w.abort_turn();
        assert!(!w.in_turn());
    }

    #[test]
    #[should_panic(expected = "start_turn called while a turn is already active")]
    fn nested_start_turn_panics() {
        let mut w = World::new();
        w.start_turn();
        w.start_turn();
    }

    #[test]
    #[should_panic(expected = "commit_turn called outside a turn")]
    fn commit_turn_outside_a_turn_panics() {
        let mut w = World::new();
        w.commit_turn();
    }

    #[test]
    #[should_panic(expected = "abort_turn called outside a turn")]
    fn abort_turn_outside_a_turn_panics() {
        let mut w = World::new();
        w.abort_turn();
    }

    #[test]
    fn empty_turn_commit_returns_empty_diff() {
        let mut w = World::new();
        w.start_turn();
        let diff = w.commit_turn();
        assert!(diff.mutations.is_empty());
        assert!(diff.new_allocs.is_empty());
    }

    #[test]
    fn turn_watermark_advances_on_commit_for_new_allocs() {
        use crate::form::Form;
        let mut w = World::new();
        let mark_before = w.turn_watermark;
        w.start_turn();
        w.heap.alloc(Form::default());
        let diff = w.commit_turn();
        assert_eq!(diff.new_allocs.len(), 1);
        // watermark moved up by 1 to include the new alloc.
        assert_eq!(w.turn_watermark, mark_before + 1);
    }

    #[test]
    fn turn_abort_truncates_new_allocs() {
        use crate::form::Form;
        let mut w = World::new();
        let mark_before = w.turn_watermark;
        let len_before = w.heap.len();
        w.start_turn();
        let _ = w.heap.alloc(Form::default());
        let _ = w.heap.alloc(Form::default());
        assert_eq!(w.heap.len(), len_before + 2);
        w.abort_turn();
        // after abort, heap is back to pre-turn state.
        assert_eq!(w.heap.len(), len_before);
        // watermark unchanged.
        assert_eq!(w.turn_watermark, mark_before);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate world::tests`
Expected: compile errors — `World::start_turn`, `World::commit_turn`, `World::abort_turn`, `World::in_turn`, `World::turn_watermark` not defined.

- [ ] **Step 3: Add the fields and lifecycle API to `World`**

In `crates/substrate/src/world.rs`:

**3a. Add the imports near the top of the file (with the other `use crate::...` lines):**

```rust
use crate::nursery::{Delta, FaceKind, TurnDiff};
```

**3b. Add the three new fields to the `World` struct (around line 122-238 is the existing struct body — add after the `vm: Vm,` field, before the cached SymIds):**

```rust
    /// the current turn's mutation deltas, keyed by FormId of
    /// pre-existing forms (payload < `turn_watermark`). forms
    /// allocated this turn are NOT in this map — they're at
    /// `heap.forms[i]` for `i >= turn_watermark`. cleared on
    /// commit and abort.
    pub nursery_deltas: IndexMap<FormId, Delta>,

    /// the FormId payload below which forms are canonical
    /// (committed in a prior turn or at boot). forms with
    /// payload `>= turn_watermark` are this-turn allocations
    /// during an active turn. advanced on commit; unchanged on
    /// abort.
    pub turn_watermark: u32,

    /// `true` iff a turn is currently active. `start_turn`
    /// flips on; `commit_turn` and `abort_turn` flip off.
    /// nested `start_turn` calls panic — V1 supports exactly
    /// one active turn at a time.
    pub in_turn: bool,
```

**3c. Initialize the new fields in `World::new`. Find the `World { ... }` literal at the end of `new()` and add:**

```rust
        World {
            heap,
            syms,
            // ... existing fields ...
            vm: Vm::default(),
            nursery_deltas: IndexMap::new(),
            turn_watermark: 0,  // will be updated after the boot turn commits (Task 5)
            in_turn: false,
            // ... cached SymIds ...
        }
```

(Place these three new field initializers right after `vm: Vm::default(),` and before the cached SymIds.)

**3d. Add the API methods to `impl World`. Place them after the existing `pub fn ensure_writable_form_id` method (around the bottom of the impl block, before `lookup_handler` works fine — find a logical spot in the impl block):**

```rust
    /// `true` iff a turn is currently active.
    pub fn in_turn(&self) -> bool {
        self.in_turn
    }

    /// begin a turn. panics if a turn is already active —
    /// V1 supports exactly one active turn at a time.
    pub fn start_turn(&mut self) {
        assert!(
            !self.in_turn,
            "start_turn called while a turn is already active"
        );
        self.in_turn = true;
        // nursery_deltas should already be empty (clear on commit/abort);
        // assert defensively.
        debug_assert!(self.nursery_deltas.is_empty());
    }

    /// commit the active turn. computes and returns the
    /// `TurnDiff`. applies nursery deltas to canonical heap.
    /// advances `turn_watermark` to current heap length.
    /// clears `nursery_deltas`. flips `in_turn` off.
    /// panics if no turn is active.
    pub fn commit_turn(&mut self) -> TurnDiff {
        assert!(
            self.in_turn,
            "commit_turn called outside a turn"
        );

        let mut diff = TurnDiff::default();

        // process deltas: read canonical prior, emit diff entry,
        // apply mutation. order is `IndexMap` insertion order,
        // which is deterministic per `laws/determinism-laws.md` D5.
        for (form_id, delta) in std::mem::take(&mut self.nursery_deltas) {
            let canonical = self.heap.get_mut(form_id);

            for (key, new_value) in delta.slots {
                let prior = canonical
                    .slots
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Slots, key),
                    (prior, new_value),
                );
                canonical.slots.insert(key, new_value);
            }
            for (key, new_value) in delta.handlers {
                let prior = canonical
                    .handlers
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Handlers, key),
                    (prior, new_value),
                );
                canonical.handlers.insert(key, new_value);
            }
            for (key, new_value) in delta.meta {
                let prior = canonical
                    .meta
                    .get(&key)
                    .copied()
                    .unwrap_or(Value::Nil);
                diff.mutations.insert(
                    (form_id, FaceKind::Meta, key),
                    (prior, new_value),
                );
                canonical.meta.insert(key, new_value);
            }
        }

        // collect new-alloc FormIds (allocations during this turn
        // sit at `heap.forms[turn_watermark..]`).
        let new_high = self.heap.len() as u32;
        diff.new_allocs = (self.turn_watermark..new_high)
            .map(FormId::vat_local)
            .collect();

        // advance watermark to include this turn's allocs.
        self.turn_watermark = new_high;
        self.in_turn = false;

        diff
    }

    /// abort the active turn. truncates `heap.forms` to
    /// `turn_watermark` (drops this-turn allocations). clears
    /// `nursery_deltas` (drops buffered mutations). flips
    /// `in_turn` off. watermark unchanged. panics if no turn
    /// is active.
    pub fn abort_turn(&mut self) {
        assert!(
            self.in_turn,
            "abort_turn called outside a turn"
        );

        // drop new-alloc forms by truncating Vec to watermark.
        // this is the rollback for allocations.
        self.heap.forms.truncate(self.turn_watermark as usize);

        // drop buffered mutations (no canonical writes occurred).
        self.nursery_deltas.clear();

        self.in_turn = false;
    }
```

**3e. Add a public accessor to `Heap` so `World::abort_turn` can truncate it.** In `crates/substrate/src/heap.rs`, the `forms` field is already a `pub` field (or a `pub(crate)` field — check). If the existing `Heap::forms` is private, change it to `pub(crate)`:

Find:
```rust
pub struct Heap {
    forms: Vec<Form>,
}
```

Change to:
```rust
pub struct Heap {
    pub(crate) forms: Vec<Form>,
}
```

This lets `World::abort_turn` (in the same crate) call `self.heap.forms.truncate(...)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate world::tests`
Expected: all world.rs tests pass, including the 9 new ones.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 405 (396 + 9 new tests). Note: the `fresh_world_is_not_in_turn_after_construction` test will fail with `assertion failed: !w.in_turn()` until Task 5 (boot turn wrapping) adds the auto-commit. Marker for now: **it's expected to fail until Task 5**.

If you want the test count to be cleanly green at this commit, mark the test `#[ignore]` for now with a comment "ignored until Task 5 lands boot wrapping," then unignore it in Task 5. Alternative: leave it failing and have Task 5 fix it. **Lean: ignore for now**, to keep `cargo test --workspace` clean across the migration. So:

```rust
    #[test]
    #[ignore = "passes after Task 5 wraps World::new in a boot turn"]
    fn fresh_world_is_not_in_turn_after_construction() {
        let w = World::new();
        assert!(!w.in_turn());
    }
```

After this, `cargo test --workspace` should show 404 passing + 1 ignored (the post-boot test).

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/world.rs crates/substrate/src/heap.rs
git commit -m "$(cat <<'EOF'
world: turn lifecycle API + nursery_deltas / turn_watermark / in_turn

V1.1 — adds the start_turn / commit_turn / abort_turn / in_turn
public API to World. nursery_deltas is an IndexMap<FormId, Delta>
keyed by canonical FormId (deterministic iteration per D5).
turn_watermark separates pre-existing forms (payload < watermark)
from this-turn allocations (payload >= watermark).

commit_turn computes a TurnDiff from the deltas (reading canonical
priors), applies the deltas, advances the watermark, returns the
diff. abort_turn truncates heap.forms to watermark + clears deltas.

no caller of the API yet; existing reads/writes still go through
heap.get / heap.get_mut directly. tasks 3-5 add the read/write
methods + boot wrapping; tasks 6-9 migrate the substrate's
mutation/read sites.

Heap.forms made pub(crate) so abort_turn can truncate it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Read path — `form_slot` / `form_handler` / `form_meta` on `World`

**Files:**
- Modify: `crates/substrate/src/world.rs`

- [ ] **Step 1: Write failing tests**

In `crates/substrate/src/world.rs`, inside the existing test module, add:

```rust
    #[test]
    fn form_slot_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        // not in a turn: form_slot reads canonical directly.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(99));
    }

    #[test]
    fn form_slot_falls_through_to_canonical_when_no_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        // advance watermark so id is "pre-existing" relative to the next turn.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // no delta; falls through to canonical.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(99));
        w.commit_turn();
    }

    #[test]
    fn form_slot_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // seed nursery_deltas manually for the test.
        let mut d = Delta::default();
        d.slots.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        // form_slot should see the delta value, not canonical's 99.
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(77));
        w.abort_turn();
    }

    #[test]
    fn form_slot_falls_through_when_key_not_in_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(99));
        f.slots.insert(SymId(8), Value::Int(88));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // delta touches slot 7 but not 8.
        let mut d = Delta::default();
        d.slots.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_slot(id, SymId(7)), Value::Int(77));
        // slot 8 falls through to canonical.
        assert_eq!(w.form_slot(id, SymId(8)), Value::Int(88));
        w.abort_turn();
    }

    #[test]
    fn form_handler_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        assert_eq!(w.form_handler(id, SymId(7)), Some(Value::Int(99)));
        assert_eq!(w.form_handler(id, SymId(99)), None);
    }

    #[test]
    fn form_handler_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.handlers.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_handler(id, SymId(7)), Some(Value::Int(77)));
        w.abort_turn();
    }

    #[test]
    fn form_meta_reads_canonical_when_not_in_turn() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        assert_eq!(w.form_meta(id, SymId(7)), Value::Int(99));
        // missing key returns nil (matches Form::meta_at behavior).
        assert_eq!(w.form_meta(id, SymId(99)), Value::Nil);
    }

    #[test]
    fn form_meta_reads_delta_when_seeded() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(7), Value::Int(99));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.meta.insert(SymId(7), Value::Int(77));
        w.nursery_deltas.insert(id, d);
        assert_eq!(w.form_meta(id, SymId(7)), Value::Int(77));
        w.abort_turn();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate world::tests::form_slot_reads_canonical_when_not_in_turn world::tests::form_handler_reads_canonical_when_not_in_turn world::tests::form_meta_reads_canonical_when_not_in_turn`
Expected: compile errors — `form_slot`, `form_handler`, `form_meta` not defined on `World`.

- [ ] **Step 3: Add the read methods to `impl World`**

In `crates/substrate/src/world.rs`, add to the `impl World` block (near the lifecycle API added in Task 2):

```rust
    /// read a form's slot value, nursery-aware. checks nursery
    /// delta first when the form is pre-existing and a turn is
    /// active; falls through to canonical heap otherwise.
    /// returns `Value::Nil` if the slot is absent in both
    /// nursery delta (if any) and canonical (matching `Form::slot`'s
    /// behavior).
    pub fn form_slot(&self, id: FormId, key: SymId) -> Value {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.slots.get(&key).copied() {
                    return v;
                }
            }
        }
        self.heap.get(id).slot(key)
    }

    /// read a form's handler entry, nursery-aware. returns
    /// `None` if absent in both nursery delta and canonical
    /// (matching `Form::handler`'s behavior — callers walking
    /// the proto chain rely on `None` to keep walking).
    pub fn form_handler(&self, id: FormId, key: SymId) -> Option<Value> {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.handlers.get(&key).copied() {
                    return Some(v);
                }
            }
        }
        self.heap.get(id).handler(key)
    }

    /// read a form's meta entry, nursery-aware. returns
    /// `Value::Nil` if absent in both nursery delta and
    /// canonical (matching `Form::meta_at`'s behavior).
    pub fn form_meta(&self, id: FormId, key: SymId) -> Value {
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.meta.get(&key).copied() {
                    return v;
                }
            }
        }
        self.heap.get(id).meta_at(key)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate world::tests`
Expected: all the new read-path tests pass.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 412 (404 from prior + 8 new read-path tests).

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
world: nursery-aware read path — form_slot / form_handler / form_meta

V1.2 — adds nursery-aware read methods on World. when in_turn and
the FormId is pre-existing (payload < turn_watermark), check the
nursery delta first and fall through to canonical. for new-alloc
forms (payload >= turn_watermark) and outside-of-turn reads, go
straight to canonical.

API mirrors Form's slot / handler / meta_at semantics: form_slot
and form_meta return Value::Nil for absent keys; form_handler
returns Option<Value> so proto-chain walks can detect "keep walking."

no internal callers migrated yet — Task 6 (world.rs full migration)
+ Task 7 (intrinsics.rs) + Task 8 (wasm.rs) + Task 9 (compiler/vm)
do that. Task 4 next adds the matching write methods.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Write path — `form_slot_set` / `form_handler_set` / `form_meta_set` on `World`

**Files:**
- Modify: `crates/substrate/src/world.rs`

- [ ] **Step 1: Write failing tests**

In `crates/substrate/src/world.rs`, inside the test module:

```rust
    #[test]
    #[should_panic(expected = "form_slot_set called outside a turn")]
    fn form_slot_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.form_slot_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    #[should_panic(expected = "form_handler_set called outside a turn")]
    fn form_handler_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.form_handler_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    #[should_panic(expected = "form_meta_set called outside a turn")]
    fn form_meta_set_outside_turn_panics() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.form_meta_set(id, SymId(1), Value::Int(42));
    }

    #[test]
    fn form_slot_set_buffers_in_delta_for_pre_existing_form() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        // mark id as pre-existing.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(42));
        // canonical heap still has empty slots for this form.
        assert!(w.heap.get(id).slots.is_empty());
        // delta should have the entry.
        let delta = w.nursery_deltas.get(&id).unwrap();
        assert_eq!(delta.slots.get(&SymId(1)).copied(), Some(Value::Int(42)));
        // read-your-writes via form_slot.
        assert_eq!(w.form_slot(id, SymId(1)), Value::Int(42));
        w.abort_turn();
    }

    #[test]
    fn form_slot_set_writes_canonical_directly_for_new_alloc() {
        let mut w = World::new();
        // set a watermark; alloc above it.
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let id = w.heap.alloc(Form::default());
        // id.payload() >= turn_watermark, so the form is new-alloc.
        w.form_slot_set(id, SymId(1), Value::Int(42));
        // canonical heap has the value directly (no delta needed).
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(42));
        // delta is empty (new-alloc forms don't use the delta map).
        assert!(w.nursery_deltas.get(&id).is_none() || w.nursery_deltas.get(&id).unwrap().is_empty());
        w.commit_turn();
    }

    #[test]
    fn commit_applies_delta_and_emits_diff() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(20));
        w.form_slot_set(id, SymId(2), Value::Int(30));
        let diff = w.commit_turn();
        // canonical now has updated values.
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(20));
        assert_eq!(w.heap.get(id).slot(SymId(2)), Value::Int(30));
        // diff has both entries with correct prior/new.
        let e1 = diff.mutations.get(&(id, FaceKind::Slots, SymId(1))).copied();
        assert_eq!(e1, Some((Value::Int(10), Value::Int(20))));
        let e2 = diff.mutations.get(&(id, FaceKind::Slots, SymId(2))).copied();
        // SymId(2) was absent before — prior is Nil.
        assert_eq!(e2, Some((Value::Nil, Value::Int(30))));
    }

    #[test]
    fn last_write_wins_within_a_turn() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(1));
        w.form_slot_set(id, SymId(1), Value::Int(2));
        w.form_slot_set(id, SymId(1), Value::Int(3));
        let diff = w.commit_turn();
        // diff has the final value 3, with prior nil (key was absent).
        let e = diff.mutations.get(&(id, FaceKind::Slots, SymId(1))).copied();
        assert_eq!(e, Some((Value::Nil, Value::Int(3))));
    }

    #[test]
    fn abort_drops_delta_no_canonical_change() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(99));
        // mid-turn: read sees new value via delta.
        assert_eq!(w.form_slot(id, SymId(1)), Value::Int(99));
        w.abort_turn();
        // post-abort: canonical untouched.
        assert_eq!(w.heap.get(id).slot(SymId(1)), Value::Int(10));
    }

    #[test]
    fn diff_handles_all_three_faces() {
        let mut w = World::new();
        let id = w.heap.alloc(Form::default());
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        w.form_slot_set(id, SymId(1), Value::Int(11));
        w.form_handler_set(id, SymId(2), Value::Int(22));
        w.form_meta_set(id, SymId(3), Value::Int(33));
        let diff = w.commit_turn();
        assert_eq!(diff.mutations.len(), 3);
        assert!(diff.mutations.contains_key(&(id, FaceKind::Slots, SymId(1))));
        assert!(diff.mutations.contains_key(&(id, FaceKind::Handlers, SymId(2))));
        assert!(diff.mutations.contains_key(&(id, FaceKind::Meta, SymId(3))));
    }
```

Note: tests 4-9 above use a custom watermark to simulate "pre-existing forms." This is OK because Task 5 hasn't yet wrapped `World::new` in a boot turn — these tests exercise the API in isolation.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate world::tests::form_slot_set_outside_turn_panics`
Expected: compile error — `form_slot_set` not defined.

- [ ] **Step 3: Add the write methods to `impl World`**

In `crates/substrate/src/world.rs`, add to `impl World`:

```rust
    /// set a slot value on a form, nursery-aware. for
    /// pre-existing forms (payload < turn_watermark) during an
    /// active turn, writes to the nursery delta. for new-alloc
    /// forms (payload >= turn_watermark), writes directly to
    /// canonical heap (they're already nursery-semantic).
    /// panics if `!in_turn` — substrate disallows mutation
    /// outside a turn (V1 invariant: turn = unit of atomicity).
    pub fn form_slot_set(&mut self, id: FormId, key: SymId, value: Value) {
        assert!(
            self.in_turn,
            "form_slot_set called outside a turn"
        );
        if id.payload() >= self.turn_watermark {
            // new alloc — write directly to canonical.
            self.heap.get_mut(id).slots.insert(key, value);
        } else {
            // pre-existing — buffer in nursery delta.
            self.nursery_deltas
                .entry(id)
                .or_default()
                .slots
                .insert(key, value);
        }
    }

    /// set a handler entry on a form, nursery-aware. semantics
    /// mirror `form_slot_set`.
    pub fn form_handler_set(&mut self, id: FormId, key: SymId, value: Value) {
        assert!(
            self.in_turn,
            "form_handler_set called outside a turn"
        );
        if id.payload() >= self.turn_watermark {
            self.heap.get_mut(id).handlers.insert(key, value);
        } else {
            self.nursery_deltas
                .entry(id)
                .or_default()
                .handlers
                .insert(key, value);
        }
    }

    /// set a meta entry on a form, nursery-aware. semantics
    /// mirror `form_slot_set`.
    pub fn form_meta_set(&mut self, id: FormId, key: SymId, value: Value) {
        assert!(
            self.in_turn,
            "form_meta_set called outside a turn"
        );
        if id.payload() >= self.turn_watermark {
            self.heap.get_mut(id).meta.insert(key, value);
        } else {
            self.nursery_deltas
                .entry(id)
                .or_default()
                .meta
                .insert(key, value);
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate world::tests`
Expected: all the new write-path tests pass; should_panic tests panic with the expected messages.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 421 (412 + 9 new write-path tests).

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
world: nursery-aware write path — form_slot_set / form_handler_set / form_meta_set

V1.3 — adds nursery-aware write methods on World. routing rules:
- !in_turn → panic ("substrate invariant: mutation only in a turn")
- in_turn AND payload >= watermark → direct canonical write
  (new-alloc forms are already nursery-semantic)
- in_turn AND payload < watermark → buffer in nursery delta

read-your-writes preserved: subsequent form_slot reads see the
delta value. abort drops the delta; commit applies it and emits
the diff entry with prior/new (where prior comes from canonical
at commit-time).

still no internal callers — Task 6+ migrate world.rs / intrinsics.rs
mutation sites.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Boot wrapping — `World::new` runs in an implicit boot turn

**Files:**
- Modify: `crates/substrate/src/world.rs`

- [ ] **Step 1: Reactivate the `fresh_world_is_not_in_turn_after_construction` test**

In `crates/substrate/src/world.rs`, find the test marked `#[ignore]` from Task 2 and remove the ignore attribute:

```rust
    #[test]
    fn fresh_world_is_not_in_turn_after_construction() {
        let w = World::new();
        assert!(!w.in_turn());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p substrate world::tests::fresh_world_is_not_in_turn_after_construction`
Expected: it fails because `World::new` doesn't yet flip `in_turn` off — but actually `in_turn` defaults to `false` per Task 2's initialization (`in_turn: false`). So actually this test *passes* as-is in Task 5's pre-impl state. Recheck:

The Task 2 initialization left `in_turn: false`. So this test would already pass. The point of Task 5 is *not* to make this test pass — it's to ensure `World::new`'s mutations during construction route through the nursery (a forward-compat invariant). Reframe the failing test:

Replace the test body with one that *would* fail without Task 5's wrapping. The test should verify: after `World::new`, the `turn_watermark` is non-zero (i.e., boot allocations are committed and visible at watermark), and `nursery_deltas` is empty (boot turn committed cleanly).

Update the test:

```rust
    #[test]
    fn boot_turn_commits_cleanly() {
        let w = World::new();
        // not in a turn (boot turn committed).
        assert!(!w.in_turn());
        // watermark advanced past all bootstrap allocations.
        assert!(
            w.turn_watermark > 1,
            "watermark must include at least the bootstrap forms"
        );
        // nursery is empty (boot turn drained on commit).
        assert!(w.nursery_deltas.is_empty());
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p substrate world::tests::boot_turn_commits_cleanly`
Expected: FAIL — `turn_watermark` is 0 (set by Task 2's init), but bootstrap allocates many forms (~30 protos + globals).

- [ ] **Step 4: Wrap `World::new` body in a boot turn**

In `crates/substrate/src/world.rs`, find the `pub fn new() -> Self` method. The current shape is:

```rust
    pub fn new() -> Self {
        let mut heap = Heap::new();
        // ... lots of construction ...
        World { /* ... */ }
    }
```

Refactor to use `start_turn` / `commit_turn` around the bootstrap allocations. The simplest approach: build a partially-initialized World (with empty caches), then call methods that allocate inside a turn, then commit. But because `World::new` returns a fully-initialized World, and many bootstrap helpers (`Protos::bootstrap`, etc.) take `&mut Heap`, restructuring is invasive.

**Alternative (chosen): keep the existing construction body, but at the end (just before `World { ... }` literal), insert a manual "auto-commit boot turn" step.** Since `Protos::bootstrap` and friends mutate `heap` and `syms` directly (not via `World`), we can't use `start_turn`/`commit_turn` to wrap them. Instead, after the construction completes:
1. set `turn_watermark = heap.len() as u32` (reflects all bootstrap allocations).
2. leave `nursery_deltas` empty and `in_turn` false.

This is functionally equivalent to "boot ran in a turn, committed, and everything became canonical." The TurnDiff is implicitly discarded (no caller asked for one).

Replace the field initialization in `World { ... }` literal:

```rust
        World {
            heap,
            syms,
            // ... existing fields ...
            vm: Vm::default(),
            nursery_deltas: IndexMap::new(),
            turn_watermark: heap_len_at_boot,    // ← was 0; now reflects allocations
            in_turn: false,
            // ... cached SymIds ...
        }
```

…where `heap_len_at_boot` is computed earlier. To do this, capture the heap length right before constructing the World struct:

Find the spot just before `World { ... }` and add:

```rust
        let heap_len_at_boot = heap.len() as u32;
```

Then use `heap_len_at_boot` in the field init.

But wait — `World::new` also calls `intrinsics::install` from the outer `new_world` (in lib.rs). The intrinsics install path also performs many mutations. We need watermark to reflect those too. Re-examine:

`World::new` in world.rs is the *low-level* constructor — it sets up `Heap`, `SymTable`, `Protos::bootstrap`, the `global_env` Form, and the `Macros` Form. It's relatively self-contained.

`crate::new_world` (in `lib.rs`) does:
1. `World::new()`
2. `intrinsics::install(&mut w)` — registers many native methods, mutating proto handlers
3. bootstraps `$hash` (mutations on global env)
4. evals `lib/main.moof` (mutations everywhere)

So `World::new()`'s allocation set is a small subset. After `intrinsics::install`, a lot more mutations happen. Then `lib/main.moof` adds even more.

For Task 5, the cleanest move:
- `World::new()` sets `turn_watermark = heap.len()` at the end of its own body. This commits its own boot-time allocations.
- Subsequent mutations (intrinsics::install) still go through `heap.get_mut` directly; they don't yet route through the nursery (Task 6+ will). So they mutate canonical pre-existing forms (the protos created in `Protos::bootstrap`) without going through deltas. This is fine *during V1's migration grind* because we haven't started any user turn yet.
- When `eval_program("lib/main.moof")` runs (Task 6's wrapping), it starts an implicit turn. That turn's mutations use `heap.get_mut` directly until Task 6+ migrate them.

So Task 5 only needs to set `turn_watermark` at the end of `World::new`, after `Protos::bootstrap` and the env/macros allocs. Don't try to wrap intrinsics::install — that happens in `lib.rs::new_world`, and those mutations currently are direct heap mutations (we'll fix them in later tasks). The watermark will advance further after each future commit.

Actually for the boot-turn-commits-cleanly test to pass, watermark just needs to be > 1, which `World::new`'s own boot already achieves.

Concrete edit: in `crates/substrate/src/world.rs`'s `pub fn new()`:

Find the `World { ... }` literal at the end. Above it, add:

```rust
        // boot turn auto-commit: all allocations during World::new
        // are treated as committed canonical state. turn_watermark
        // reflects this. (the equivalent of "start_turn → bootstrap
        // → commit_turn" with the diff discarded.)
        let heap_len_at_boot = heap.len() as u32;
```

And in the literal, change `turn_watermark: 0,` to `turn_watermark: heap_len_at_boot,`.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p substrate world::tests::boot_turn_commits_cleanly`
Expected: PASS — `turn_watermark > 1`.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 422 (421 + 1 unfreeze of the previously-ignored test).

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
world: boot-turn auto-commit — turn_watermark reflects bootstrap allocs

V1.4 — World::new now sets turn_watermark to heap.len() after the
bootstrap allocations (Protos, env, Macros). functionally equivalent
to "start_turn → bootstrap → commit_turn" with the implicit boot diff
discarded.

post-boot: in_turn = false, nursery_deltas empty, turn_watermark
reflects all bootstrap-time forms. subsequent turns (eval_program's
implicit wraps in Task 6) will allocate above this watermark.

direct heap mutations during intrinsics::install / new_world's lib
loading still go to canonical (not yet routed through nursery —
Tasks 7+ migrate). this is correct mid-migration: pre-V1 every
mutation was direct canonical; we're incrementally lifting paths.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `eval_program` / `eval` implicit-turn wrap

**Files:**
- Modify: `crates/substrate/src/lib.rs`

- [ ] **Step 1: Write a failing test for raise auto-aborts**

In `crates/substrate/src/world.rs` test module (or anywhere convenient):

```rust
    #[test]
    fn raise_in_eval_program_aborts_implicit_turn_no_state_leak() {
        let mut w = crate::new_world_bare();
        // grab a reference to the global env (a form id < watermark).
        let env_id = w.global_env;
        let foo_sym = w.intern("foo");
        let snapshot_before = w.heap.get(env_id).slot(foo_sym);
        // pre-state: foo is unbound (Nil).
        assert_eq!(snapshot_before, Value::Nil);

        // try evaluating something that mutates env then raises.
        // (def foo 5) writes to global env, then (raise: 'boom) aborts.
        let result = crate::eval_program(
            &mut w,
            "(def foo 5) (raise: 'boom \"oh no\")",
        );
        assert!(result.is_err());

        // post-abort: env state preserved (foo is still unbound).
        // NOTE: this test will FAIL until Task 7 migrates env_bind
        // to use form_slot_set. before Task 7, env_bind writes
        // directly to canonical, bypassing the implicit-turn rollback.
        // we mark this test #[ignore] for now and unignore in Task 7.
    }
```

Actually, let me think carefully: in Task 6, `eval_program` wraps in implicit turns but `env_bind` still uses direct canonical writes. So the raise propagation triggers `abort_turn` in Task 6's wrap, but `abort_turn` only rolls back nursery_deltas — and `env_bind`'s mutation isn't in the nursery, it's in canonical. So state DOES leak. The test would fail.

Mark this test `#[ignore]` for Task 6, unignore in Task 7 after env_bind migration.

```rust
    #[test]
    #[ignore = "passes after Task 7 migrates env_bind to form_slot_set"]
    fn raise_in_eval_program_aborts_implicit_turn_no_state_leak() {
        let mut w = crate::new_world_bare();
        let env_id = w.global_env;
        let foo_sym = w.intern("foo");
        let snapshot_before = w.heap.get(env_id).slot(foo_sym);
        assert_eq!(snapshot_before, Value::Nil);

        let result = crate::eval_program(
            &mut w,
            "(def foo 5) (raise: 'boom \"oh no\")",
        );
        assert!(result.is_err());
        // env state preserved post-abort.
        assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);
    }
```

Add a test that's robust at this stage of the migration:

```rust
    #[test]
    fn eval_program_in_turn_state_post_eval_is_not_in_turn() {
        let mut w = crate::new_world_bare();
        // implicit turn wraps the body; on completion, in_turn is false.
        let result = crate::eval_program(&mut w, "(def x 42) x");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::Int(42));
        assert!(!w.in_turn());
    }

    #[test]
    fn eval_program_returning_error_leaves_in_turn_false() {
        let mut w = crate::new_world_bare();
        let result = crate::eval_program(&mut w, "(raise: 'boom \"x\")");
        assert!(result.is_err());
        assert!(!w.in_turn());
    }

    #[test]
    fn nested_eval_program_calls_use_outer_turn_idempotently() {
        let mut w = crate::new_world_bare();
        w.start_turn();
        // outer caller already in a turn; eval_program should NOT
        // open a nested turn (idempotent via was_in_turn).
        let _ = crate::eval_program(&mut w, "(def x 1)");
        // still in the outer turn — eval_program didn't commit.
        assert!(w.in_turn());
        w.commit_turn();
        assert!(!w.in_turn());
    }
```

These three are the active tests for Task 6.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate world::tests::eval_program_in_turn_state_post_eval_is_not_in_turn world::tests::eval_program_returning_error_leaves_in_turn_false world::tests::nested_eval_program_calls_use_outer_turn_idempotently`
Expected: at least the third one fails — current `eval_program` doesn't track `was_in_turn`, so calling it inside an existing turn would call `start_turn` again and panic. The first two should pass already (`new_world_bare` sets `in_turn=false`; `eval_program` doesn't mutate `in_turn`).

- [ ] **Step 3: Update `eval_program` and `eval` in `lib.rs`**

In `crates/substrate/src/lib.rs`, find the existing definitions:

```rust
pub fn eval(w: &mut world::World, source: &str) -> Result<value::Value, world::RaiseError> {
    let form = w
        .read(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let chunk = compiler::compile(w, form)?;
    w.run_top(chunk)
}

pub fn eval_program(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
    let forms = w
        .read_all(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let mut last = value::Value::Nil;
    for form in forms {
        let chunk = compiler::compile(w, form)?;
        last = w.run_top(chunk)?;
    }
    Ok(last)
}
```

Replace with:

```rust
pub fn eval(w: &mut world::World, source: &str) -> Result<value::Value, world::RaiseError> {
    let was_in_turn = w.in_turn();
    if !was_in_turn {
        w.start_turn();
    }
    let result = eval_inner(w, source);
    if !was_in_turn {
        match &result {
            Ok(_) => { let _ = w.commit_turn(); }
            Err(_) => { w.abort_turn(); }
        }
    }
    result
}

fn eval_inner(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
    let form = w
        .read(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let chunk = compiler::compile(w, form)?;
    w.run_top(chunk)
}

pub fn eval_program(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
    let was_in_turn = w.in_turn();
    if !was_in_turn {
        w.start_turn();
    }
    let result = eval_program_inner(w, source);
    if !was_in_turn {
        match &result {
            Ok(_) => { let _ = w.commit_turn(); }
            Err(_) => { w.abort_turn(); }
        }
    }
    result
}

fn eval_program_inner(
    w: &mut world::World,
    source: &str,
) -> Result<value::Value, world::RaiseError> {
    let forms = w
        .read_all(source)
        .map_err(|e| world::RaiseError::from_reader(&mut w.syms, e))?;
    let mut last = value::Value::Nil;
    for form in forms {
        let chunk = compiler::compile(w, form)?;
        last = w.run_top(chunk)?;
    }
    Ok(last)
}
```

Both `eval` and `eval_program` now check `was_in_turn` (idempotent: don't open a new turn if caller already started one), invoke their inner non-wrapping body, and commit-or-abort at the end based on the result.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate world::tests::eval_program_in_turn_state_post_eval_is_not_in_turn world::tests::eval_program_returning_error_leaves_in_turn_false world::tests::nested_eval_program_calls_use_outer_turn_idempotently`
Expected: all three pass.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 425 (422 + 3 new tests). One additional test (`raise_in_eval_program_aborts_implicit_turn_no_state_leak`) is `#[ignore]`-d.

If any pre-existing test fails, the issue is most likely: a test that calls `eval_program` and then *also* directly mutates `World` outside any turn. Such tests would now hit the `mutation outside a turn` panic (from Task 4's `form_slot_set`). But Tasks 6-9 haven't migrated mutation sites yet, so direct `heap.get_mut` calls in tests still work. If we hit such a regression, document it and proceed.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/lib.rs crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
lib: eval_program / eval wrap their body in implicit turns

V1.5 — both eval entry points now use the was_in_turn idempotent
pattern: if no turn is active when called, start one; on success
commit (discarding the diff for now); on RaiseError abort. if a
turn is already active (e.g., from outer test code or future
scheduler code), the inner eval_program does NOT open a nested
turn — the outer caller decides commit/abort.

internal mutations during the turn still mostly bypass the nursery
(Tasks 7+ migrate). Task 6's effect is structural: every user
program runs inside a turn, so abort-on-raise has the right shape;
the rollback semantics complete as the migration grind progresses.

raise_in_eval_program_aborts_implicit_turn_no_state_leak is
#[ignore]-d until Task 7 migrates env_bind.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Migrate `world.rs` mutation + read sites

**Files:**
- Modify: `crates/substrate/src/world.rs`

This is the first of four per-file migrations. Each migration covers BOTH writes and reads in the file at once, to maintain in-file consistency: a function that writes through the nursery must read its own writes via `form_slot` (not via `heap.get(...).slot(...)` which would see canonical and miss the delta).

- [ ] **Step 1: Audit world.rs mutation sites**

Run from the worktree root:
```bash
grep -n "heap.get_mut\|\.slots\.insert\|\.handlers\.insert\|\.meta\.insert" crates/substrate/src/world.rs
```

Expected sites (verify against current code):
1. `env_bind`: `self.heap.get_mut(env).slots.insert(name, value);`
2. `env_set` (within walk loop): `self.heap.get_mut(cur).slots.insert(name, value);`
3. `install_native`:
   - `self.heap.get_mut(method_id).meta.insert(self.source_sym, sym_v);`
   - `self.heap.get_mut(proto).handlers.insert(sel_id, Value::Form(method_id));`
4. `bump_proto_generation`: `self.heap.get_mut(proto_id).meta.insert(self.generation_sym, ...);`
5. `macro_register`: `self.heap.get_mut(self.macros_form).slots.insert(name, method);`
6. `frame_snapshot`: many `snap.slots.insert(...)` — but `snap` is a *fresh* Form being constructed before alloc. these are NOT mutations of an existing form; they're populating a new form's IndexMap before `heap.alloc(snap)`. **leave these as-is** — they're construction, not mutation.

Sites 1–5 need migration.

- [ ] **Step 2: Audit world.rs read sites that need nursery-awareness**

Run:
```bash
grep -n "heap.get(.*)\.slot\|heap.get(.*)\.handler\|heap.get(.*)\.meta_at" crates/substrate/src/world.rs
```

Expected sites:
1. `env_lookup` (within walk loop): `f.slots.get(&name).copied()` and `f.meta.get(&self.parent_sym).copied()`.
2. `env_set` (the `contains_key` check): `self.heap.get(cur).slots.contains_key(&name)` and parent walk via `.meta.get(&self.parent_sym)`.
3. `lookup_handler`: `self.heap.get(id).handler(selector)` and `self.heap.get(proto_id).handler(selector)`.
4. `lookup_handler_super`: same shape.
5. `frame_snapshot`: reads `frame.chunk` etc. but those are local variables, not heap reads. ignore.
6. `proto_generation`: `self.heap.get(proto_id).meta_at(self.generation_sym)`.
7. `macro_at`: `f.slot(name)` and `f.slot_present(name)`.

These need migration to use `form_slot` / `form_handler` / `form_meta` / `form_meta_at` style — i.e., the nursery-aware reads.

- [ ] **Step 3: Migrate `env_bind` (write site)**

Find:
```rust
    pub fn env_bind(&mut self, env: FormId, name: SymId, value: Value) {
        self.heap.get_mut(env).slots.insert(name, value);
    }
```

Replace with:
```rust
    pub fn env_bind(&mut self, env: FormId, name: SymId, value: Value) {
        self.form_slot_set(env, name, value);
    }
```

- [ ] **Step 4: Migrate `env_set` (write + read)**

Find the existing `env_set`:

```rust
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> bool {
        let mut cur = env;
        loop {
            if self.heap.get(cur).slots.contains_key(&name) {
                self.heap.get_mut(cur).slots.insert(name, value);
                return true;
            }
            let parent = self
                .heap
                .get(cur)
                .meta
                .get(&self.parent_sym)
                .copied()
                .unwrap_or(Value::Nil);
            match parent {
                Value::Form(id) => cur = id,
                _ => return false,
            }
        }
    }
```

Replace with nursery-aware version. Note: `slots.contains_key` doesn't have a direct equivalent on World — we need to check both delta and canonical. Add a helper, or inline check:

```rust
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> bool {
        let mut cur = env;
        loop {
            // check if `name` is bound in this env, considering nursery delta.
            // form_slot returns Nil for absent OR explicitly-nil. for env_set
            // semantics we need contains_key semantics, so check both delta
            // and canonical explicitly.
            let bound_in_delta = self
                .nursery_deltas
                .get(&cur)
                .map(|d| d.slots.contains_key(&name))
                .unwrap_or(false);
            let bound_in_canonical = self.heap.get(cur).slots.contains_key(&name);
            if bound_in_delta || bound_in_canonical {
                self.form_slot_set(cur, name, value);
                return true;
            }
            // walk parent — same lookup discipline.
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Form(id) => cur = id,
                _ => return false,
            }
        }
    }
```

(Note: this assumes `in_turn` is always true when `env_set` is called from user code, since user code runs inside `eval_program`'s implicit turn. If `env_set` is called outside a turn — possible during direct rust testing — the `form_slot_set` will panic. That's the right answer; substrate-wide invariant.)

- [ ] **Step 5: Migrate `env_lookup` (read site)**

Find:
```rust
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            let f = self.heap.get(cur);
            if let Some(v) = f.slots.get(&name).copied() {
                return Some(v);
            }
            let parent = f.meta.get(&self.parent_sym).copied().unwrap_or(Value::Nil);
            match parent {
                Value::Nil => return None,
                Value::Form(id) => cur = id,
                _ => return None,
            }
        }
    }
```

Replace:
```rust
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            // check delta first (if in turn AND pre-existing), then canonical.
            let in_delta = self.in_turn
                && cur.payload() < self.turn_watermark
                && self.nursery_deltas
                    .get(&cur)
                    .and_then(|d| d.slots.get(&name).copied())
                    .is_some();
            if in_delta {
                return self.nursery_deltas.get(&cur)
                    .and_then(|d| d.slots.get(&name).copied());
            }
            let f = self.heap.get(cur);
            if let Some(v) = f.slots.get(&name).copied() {
                return Some(v);
            }
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Nil => return None,
                Value::Form(id) => cur = id,
                _ => return None,
            }
        }
    }
```

(Slightly verbose because `env_lookup` distinguishes `Some(Value::Nil)` (bound to nil) from `None` (unbound), and `form_slot` collapses both. So we need the explicit dual check.)

- [ ] **Step 6: Migrate `install_native` (write sites)**

Find:
```rust
    pub fn install_native(
        &mut self,
        proto: FormId,
        selector: &str,
        native_fn: NativeFn,
    ) -> FormId {
        let sel_id = self.intern(selector);
        let method_form = Form::with_proto(Value::Form(self.protos.method));
        let method_id = self.heap.alloc(method_form);
        let sym_v = Value::Sym(sel_id);
        self.heap
            .get_mut(method_id)
            .meta
            .insert(self.source_sym, sym_v);
        self.native_fns.insert(method_id, native_fn);
        self.heap
            .get_mut(proto)
            .handlers
            .insert(sel_id, Value::Form(method_id));
        method_id
    }
```

Replace:
```rust
    pub fn install_native(
        &mut self,
        proto: FormId,
        selector: &str,
        native_fn: NativeFn,
    ) -> FormId {
        let sel_id = self.intern(selector);
        let method_form = Form::with_proto(Value::Form(self.protos.method));
        let method_id = self.heap.alloc(method_form);  // new alloc — above watermark
        let sym_v = Value::Sym(sel_id);
        // method_id is new-alloc; form_meta_set will write directly to canonical.
        self.form_meta_set(method_id, self.source_sym, sym_v);
        self.native_fns.insert(method_id, native_fn);
        // proto is pre-existing; form_handler_set buffers in delta.
        self.form_handler_set(proto, sel_id, Value::Form(method_id));
        method_id
    }
```

**However:** `install_native` is called extensively during `intrinsics::install`, which runs OUTSIDE any user turn (it runs in the boot phase, between `World::new` and the first `eval_program`). At that point, `in_turn` is `false`, and `form_meta_set` / `form_handler_set` will panic with "called outside a turn."

We need either:
- (i) wrap `intrinsics::install` in an explicit turn (in `crate::new_world`)
- (ii) keep `install_native` direct (bypassing the nursery) for the boot path; add a parallel `install_native_in_turn` for runtime

Option (i) is much cleaner. Update `crate::new_world` in `lib.rs`:

```rust
pub fn new_world() -> world::World {
    let mut w = world::World::new();  // already runs its own boot turn auto-commit
    w.transporter_root = transporter::resolve_lib_root();

    // wrap intrinsics + $hash bootstrap in an explicit turn so install_native
    // and other mutation paths see in_turn = true. commit at end.
    w.start_turn();
    intrinsics::install(&mut w);

    {
        let hash_proto = wasm::load_wasm_bytes(&mut w, HASH_MCO_BYTES, "embedded-hash")
            .unwrap_or_else(|e| {
                panic!("Hash mco bootstrap failed — substrate is broken: {}", e.message)
            });
        let new_sel = w.intern("new");
        let hash_instance = w
            .send(hash_proto, new_sel, &[])
            .unwrap_or_else(|e| {
                panic!("Hash mco [new] failed during bootstrap: {}", e.message)
            });
        let dollar_hash = w.intern("$hash");
        let global = w.global_env;
        w.env_bind(global, dollar_hash, hash_instance);
    }

    let _ = w.commit_turn();   // discard diff

    // lib/main.moof load runs in eval_program's own implicit turn.
    let root = w.transporter_root.clone().unwrap_or_else(|| { /* panic */ });
    let main_path = root.join("main.moof");
    let main_source = std::fs::read_to_string(&main_path).unwrap_or_else(|e| { /* panic */ });
    if let Err(e) = eval_program(&mut w, &main_source) {
        panic!("lib/main.moof failed to load: {}", e.message);
    }
    w
}
```

(Same for `new_world_bare`: wrap `intrinsics::install` in start/commit.)

Now `install_native`'s `form_meta_set` / `form_handler_set` calls succeed because in_turn = true.

- [ ] **Step 7: Migrate `bump_proto_generation` (write site)**

Find:
```rust
    pub fn bump_proto_generation(&mut self, proto_id: FormId) {
        let cur = self.proto_generation(proto_id);
        let next = cur.wrapping_add(1);
        self.heap
            .get_mut(proto_id)
            .meta
            .insert(self.generation_sym, Value::Int(next as i64));
    }
```

Replace:
```rust
    pub fn bump_proto_generation(&mut self, proto_id: FormId) {
        let cur = self.proto_generation(proto_id);
        let next = cur.wrapping_add(1);
        self.form_meta_set(proto_id, self.generation_sym, Value::Int(next as i64));
    }
```

And update `proto_generation` to read via `form_meta`:

```rust
    pub fn proto_generation(&self, proto_id: FormId) -> u32 {
        match self.form_meta(proto_id, self.generation_sym) {
            Value::Int(n) => n as u32,
            _ => 0,
        }
    }
```

- [ ] **Step 8: Migrate `macro_register` and `macro_at`**

Find:
```rust
    pub fn macro_at(&self, name: SymId) -> Option<Value> {
        let f = self.heap.get(self.macros_form);
        if f.slot_present(name) {
            Some(f.slot(name))
        } else {
            None
        }
    }

    pub fn macro_register(&mut self, name: SymId, method: Value) {
        self.heap
            .get_mut(self.macros_form)
            .slots
            .insert(name, method);
    }
```

`macro_at` is tricky because it uses `slot_present` which form_slot can't express. Keep the dual check:

```rust
    pub fn macro_at(&self, name: SymId) -> Option<Value> {
        // check delta first (if in turn AND pre-existing), then canonical.
        let id = self.macros_form;
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                if let Some(v) = delta.slots.get(&name).copied() {
                    return Some(v);
                }
            }
        }
        let f = self.heap.get(id);
        if f.slot_present(name) {
            Some(f.slot(name))
        } else {
            None
        }
    }

    pub fn macro_register(&mut self, name: SymId, method: Value) {
        self.form_slot_set(self.macros_form, name, method);
    }
```

- [ ] **Step 9: Migrate `lookup_handler` and `lookup_handler_super` (read sites)**

Find:
```rust
    pub fn lookup_handler(
        &self,
        receiver: Value,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        let own_id = self.effective_form_id(receiver);
        if let Some(id) = own_id {
            if let Some(handler) = self.heap.get(id).handler(selector) {
                return Some((handler, id));
            }
        }
        let mut proto = match own_id {
            Some(id) => self.heap.get(id).proto,
            None => self.proto_of(receiver),
        };
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    let f = self.heap.get(proto_id);
                    if let Some(handler) = f.handler(selector) {
                        return Some((handler, proto_id));
                    }
                    proto = f.proto;
                }
                _ => return None,
            }
        }
        None
    }
```

Replace by routing handler reads through `form_handler` and proto reads through `form_slot` (proto is in slots — wait, no. `proto` is a direct field on `Form`, not a slot. Re-check.)

Looking at `crates/substrate/src/form.rs`: `Form { proto: Value, slots: ..., handlers: ..., meta: ... }`. So `proto` is a struct field, NOT in any of the three faces. Reading `proto` doesn't go through nursery. We can keep `self.heap.get(id).proto` direct — proto isn't mutable through `form_slot_set` etc., so it stays canonical.

Actually wait — what about `set-proto!` if it existed? Per the spec / V0 commit, there's no `set-proto!` primitive at phase A; `proto` is set at allocation time and never changed. So no nursery routing needed for proto reads.

Migrate just the handler reads:

```rust
    pub fn lookup_handler(
        &self,
        receiver: Value,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        let own_id = self.effective_form_id(receiver);
        if let Some(id) = own_id {
            if let Some(handler) = self.form_handler(id, selector) {
                return Some((handler, id));
            }
        }
        let mut proto = match own_id {
            Some(id) => self.heap.get(id).proto,
            None => self.proto_of(receiver),
        };
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    if let Some(handler) = self.form_handler(proto_id, selector) {
                        return Some((handler, proto_id));
                    }
                    proto = self.heap.get(proto_id).proto;
                }
                _ => return None,
            }
        }
        None
    }
```

Same for `lookup_handler_super`:

```rust
    pub fn lookup_handler_super(
        &self,
        defining_proto: FormId,
        selector: SymId,
    ) -> Option<(Value, FormId)> {
        let mut proto = self.heap.get(defining_proto).proto;
        const MAX_PROTO_DEPTH: usize = 256;
        for _ in 0..MAX_PROTO_DEPTH {
            match proto {
                Value::Form(proto_id) => {
                    if let Some(handler) = self.form_handler(proto_id, selector) {
                        return Some((handler, proto_id));
                    }
                    proto = self.heap.get(proto_id).proto;
                }
                _ => return None,
            }
        }
        None
    }
```

- [ ] **Step 10: Unignore the raise-aborts test**

In `crates/substrate/src/world.rs`, find:
```rust
    #[test]
    #[ignore = "passes after Task 7 migrates env_bind to form_slot_set"]
    fn raise_in_eval_program_aborts_implicit_turn_no_state_leak() {
```

Remove the `#[ignore = ...]` attribute. The test is now expected to pass.

- [ ] **Step 11: Run all tests, expect 388 + V1 tests still pass**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED" | tail -20`
Expected: every test passes; no `FAILED` lines.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 426 (425 + 1 unignored test).

If a test fails, the most likely cause: a code path in another file (`intrinsics.rs`, `wasm.rs`, etc.) reads a slot via `heap.get(id).slot(k)` that was just mutated via `form_slot_set` (delta path) — see the staleness issue in §11 of the spec. The tests should still pass because:
- `eval_program`'s implicit turn means in_turn=true during user code
- env_bind etc. now write to delta
- env_lookup now reads delta first ✓
- but other read sites in intrinsics.rs (e.g., `Heap slotOf:at:`) might read canonical directly

Defer fixes in other files to subsequent tasks. If a specific test breaks now, document it inline as "expected to fail until Task N migrates the related read site."

- [ ] **Step 12: Commit**

```bash
git add crates/substrate/src/world.rs crates/substrate/src/lib.rs
git commit -m "$(cat <<'EOF'
world: migrate mutation + read sites to nursery-aware path

V1.6 — full migration of mutation and corresponding read paths in
world.rs:
- env_bind, env_set, install_native, bump_proto_generation,
  macro_register, macro_at: writes go through form_slot_set /
  form_handler_set / form_meta_set
- env_lookup, env_set's contains_key check, lookup_handler,
  lookup_handler_super, proto_generation: reads go through
  form_slot / form_handler / form_meta (with explicit dual-check
  where needed for slot_present semantics)

new_world / new_world_bare wrap intrinsics::install and the
\$hash bootstrap in an explicit start/commit pair so install_native's
form_meta_set / form_handler_set calls satisfy the in_turn invariant.

raise_in_eval_program_aborts_implicit_turn_no_state_leak test is now
unignored — env mutations during a turn that raises out roll back
cleanly. confirms the rollback semantics for env-shaped state.

intrinsics.rs / wasm.rs / compiler.rs / vm.rs mutation sites
(slotSet!, setHandler!, getOrCreateProto, mco loaders) still go
direct — Tasks 8-11 cover those.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Migrate `intrinsics.rs` mutation + read sites

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs`

- [ ] **Step 1: Audit intrinsics.rs mutation sites**

```bash
grep -n "heap.get_mut\|\.slots\.insert\|\.handlers\.insert\|\.meta\.insert" crates/substrate/src/intrinsics.rs
```

Expected counts: ~20-30 sites. Major categories:
1. `slotSet!` native — `.slots.insert(k, v)` on a target form
2. `setHandler!` native — `.handlers.insert(sel, fn)` + `bump_proto_generation` call
3. `getOrCreateProto` — `.meta.insert(name_meta, ...)` on a fresh proto-Form (new alloc, direct ok)
4. `__set-meta!` and similar — `.meta.insert(...)` mutations
5. cap installer block (`install_compiler_cap`, `install_transporter_cap`, `install_mco_cap`): `.slots.insert` on the cap form, `.meta.insert` for `:name`
6. Various per-proto installations: `install_chunk_methods` etc. that do `.handlers.insert` directly on protos for testability

For each, replace `world.heap.get_mut(id).slots.insert(k, v)` with `world.form_slot_set(id, k, v)` (and similar for handlers/meta). For new-alloc forms (just allocated, payload >= watermark), `form_*_set` writes directly to canonical anyway, so the migration is correct in both cases.

- [ ] **Step 2: Audit intrinsics.rs read sites**

```bash
grep -n "heap.get(.*)\.slot\|heap.get(.*)\.handler\|heap.get(.*)\.meta_at\|heap.get(.*)\.slot_present" crates/substrate/src/intrinsics.rs
```

Expected sites: ~20-30. Major categories:
1. `Heap slotOf:at:` native — reads slot
2. `Heap handlerOf:at:` native — reads handler
3. `Heap metaOf:at:` native — reads meta
4. `Heap slotKeysOf:` / `handlerKeysOf:` / `metaKeysOf:` natives — iterate keys (these need a different approach: nursery-aware key listing)
5. various dispatch-internal reads

For Heap singleton accessors (`slotOf:at:`, etc.), migrate to `world.form_slot(...)` etc.

For `slotKeysOf:` etc. (key iteration), need to merge nursery delta keys with canonical keys. Add a helper:

```rust
// new helper in world.rs (added in this task)
impl World {
    /// list slot keys for a form, nursery-aware. union of
    /// canonical's slot keys and nursery delta's slot keys.
    /// preserves insertion order: canonical first, then delta keys
    /// not already in canonical (preserves D5 determinism).
    pub fn form_slot_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).slots.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.slots.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
    }

    /// handler keys, nursery-aware. analogous to form_slot_keys.
    pub fn form_handler_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).handlers.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.handlers.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
    }

    /// meta keys, nursery-aware. analogous.
    pub fn form_meta_keys(&self, id: FormId) -> Vec<SymId> {
        let mut keys: Vec<SymId> = self.heap.get(id).meta.keys().copied().collect();
        if self.in_turn && id.payload() < self.turn_watermark {
            if let Some(delta) = self.nursery_deltas.get(&id) {
                for k in delta.meta.keys() {
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
        }
        keys
    }
}
```

Add these to `world.rs` (before continuing intrinsics migration), test them with a few simple unit tests, and use them from intrinsics.rs.

- [ ] **Step 3: Add the form_*_keys helpers to world.rs**

In `crates/substrate/src/world.rs`, add the three helpers above to `impl World`. Also add tests:

```rust
    #[test]
    fn form_slot_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.slots.insert(SymId(1), Value::Int(10));
        f.slots.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        // delta adds key 3 and overwrites key 2.
        let mut d = Delta::default();
        d.slots.insert(SymId(3), Value::Int(30));
        d.slots.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_slot_keys(id);
        // canonical keys 1, 2 first; then delta's new key 3.
        // key 2 is in canonical so no duplicate from delta.
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }

    #[test]
    fn form_handler_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.handlers.insert(SymId(1), Value::Int(10));
        f.handlers.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.handlers.insert(SymId(3), Value::Int(30));
        d.handlers.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_handler_keys(id);
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }

    #[test]
    fn form_meta_keys_unions_canonical_and_delta() {
        let mut w = World::new();
        let mut f = Form::default();
        f.meta.insert(SymId(1), Value::Int(10));
        f.meta.insert(SymId(2), Value::Int(20));
        let id = w.heap.alloc(f);
        w.turn_watermark = w.heap.len() as u32;
        w.start_turn();
        let mut d = Delta::default();
        d.meta.insert(SymId(3), Value::Int(30));
        d.meta.insert(SymId(2), Value::Int(99));
        w.nursery_deltas.insert(id, d);
        let keys = w.form_meta_keys(id);
        assert_eq!(keys, vec![SymId(1), SymId(2), SymId(3)]);
        w.abort_turn();
    }
```

- [ ] **Step 4: Migrate `slotSet!` in intrinsics.rs**

Find the install of `slotSet!` (search `"slotSet!"` in intrinsics.rs):

```rust
    install_global(w, "slotSet!", |w, _, args| {
        // ... arg validation ...
        let id = target_form_id(w, args[0], "slotSet!");
        let key = args[1].as_sym().ok_or(...)?;
        let value = args[2];
        w.heap.get_mut(id).slots.insert(key, value);
        Ok(value)
    });
```

Replace the mutation:
```rust
        w.form_slot_set(id, key, value);
```

- [ ] **Step 5: Migrate `setHandler!` in intrinsics.rs**

Find:
```rust
    install_global(w, "setHandler!", |w, _, args| {
        // ...
        w.heap.get_mut(id).handlers.insert(sel, method);
        w.bump_proto_generation(id);
        Ok(method)
    });
```

Replace:
```rust
        w.form_handler_set(id, sel, method);
        w.bump_proto_generation(id);
```

- [ ] **Step 6: Migrate `getOrCreateProto`**

Find (around line 2174 in intrinsics.rs):
```rust
    install_global(w, "getOrCreateProto", |w, _, args| {
        // ...
        let parent = args[1];
        let mut form = Form::with_proto(parent);
        let name_meta = w.intern("name");
        form.meta.insert(name_meta, Value::Sym(name_sym));
        let new_id = w.alloc(form);
        let v = Value::Form(new_id);
        w.env_bind(global, name_sym, v);
        Ok(v)
    });
```

The `form.meta.insert(name_meta, ...)` is on a fresh `Form` *before* it's allocated. This is construction, not mutation. Leave as-is (matches the `frame_snapshot` pattern from Task 7).

- [ ] **Step 7: Migrate Heap singleton accessors**

The `slotOf:at:` / `handlerOf:at:` / `metaOf:at:` natives currently:
```rust
.install_native(proto, "slotOf:at:", |w, _self, args| {
    // ... arg validation ...
    let v = args.first().copied().unwrap_or(Value::Nil);
    let sym = args.get(1).and_then(|s| s.as_sym()).unwrap_or(SymId::NONE);
    match w.effective_form_id(v) {
        Some(id) => Ok(w.heap.get(id).slot(sym)),
        _ => Ok(Value::Nil),
    }
});
```

Replace `w.heap.get(id).slot(sym)` with `w.form_slot(id, sym)`. Similarly for handlers and meta.

For `slotKeysOf:` (the macro-installed version), use the new `form_slot_keys` helper:

```rust
$w.install_native($proto, "slotKeysOf:", |w, _self, args| {
    let v = args.first().copied().unwrap_or(Value::Nil);
    match w.effective_form_id(v) {
        Some(id) => {
            let keys: Vec<Value> = w
                .form_slot_keys(id)        // ← was: heap.get(id).slots.keys()
                .into_iter()
                .map(Value::Sym)
                .collect();
            Ok(w.make_list(&keys))
        }
        None => Ok(Value::Nil),
    }
});
```

Similar for `handlerKeysOf:` and `metaKeysOf:`. Note: the existing macro `key_list_on_heap!` becomes unsuitable since it accesses `$field.keys()` on a `&Form`. Replace the macro invocations with three explicit installations.

- [ ] **Step 8: Migrate cap-installer mutations**

Search for cap installer blocks (`install_compiler_cap`, `install_transporter_cap`, `install_mco_cap`, etc.). Each typically:

```rust
fn install_compiler_cap(w: &mut World) {
    let cap_proto = ...;
    let cap_form = ...;
    w.heap.get_mut(cap_form).meta.insert(name_meta, Value::Sym(...));
    // ... handler installs via install_native (already migrated in Task 7)
}
```

Replace `.meta.insert` with `form_meta_set`. Same pattern for any `.slots.insert` or `.handlers.insert`.

- [ ] **Step 9: Run tests**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED" | tail -20`
Expected: every test passes; no `FAILED`. Test count climbs by ~3 (the form_*_keys helper tests).

Run the count: should be ~429 (426 + 3 helper tests).

- [ ] **Step 10: Commit**

```bash
git add crates/substrate/src/intrinsics.rs crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
intrinsics: migrate mutation + read sites to nursery-aware path

V1.7 — full migration of intrinsics.rs:
- slotSet!, setHandler!: writes through form_slot_set /
  form_handler_set
- Heap.slotOf:at:, .handlerOf:at:, .metaOf:at: reads via form_slot /
  form_handler / form_meta
- Heap.slotKeysOf:, .handlerKeysOf:, .metaKeysOf: reads via new
  form_slot_keys / form_handler_keys / form_meta_keys helpers
  (which union canonical and delta keys, preserving D5 insertion
  order)
- cap installers (transporter, compiler, mco, hash) use form_meta_set
  / form_slot_set / form_handler_set

construction-time mutations on fresh forms before alloc (e.g.,
getOrCreateProto's form.meta.insert before w.alloc(form)) stay
direct — they're populating a not-yet-heap form.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Migrate `wasm.rs` mutation sites

**Files:**
- Modify: `crates/substrate/src/wasm.rs`

- [ ] **Step 1: Audit wasm.rs mutation sites**

```bash
grep -n "heap.get_mut\|\.slots\.insert\|\.handlers\.insert\|\.meta\.insert" crates/substrate/src/wasm.rs
```

Expected: a handful of sites, primarily in the mco loader where wasm-backed methods get installed onto a freshly-created proto Form. Many are pre-alloc construction (mutating `Form` literals before `heap.alloc`), which stay direct. The post-alloc handler installs go through `install_native` (already migrated in Task 7) — no changes needed here.

If grep finds direct mutations on existing forms (not pre-alloc construction), migrate them to `form_*_set`. If all wasm.rs mutations are pre-alloc, this task is a no-op verification.

- [ ] **Step 2: Migrate any direct existing-form mutations**

For each found site, replace the mutation pattern. (Show concrete code if the audit finds anything; if the audit is clean, this step is "verify clean.")

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED" | tail -10`
Expected: all tests pass.

- [ ] **Step 4: Commit (or note no-op)**

If actual changes were needed:
```bash
git add crates/substrate/src/wasm.rs
git commit -m "$(cat <<'EOF'
wasm: migrate mco-load mutation sites to nursery-aware path

V1.8 — wasm.rs's direct existing-form mutations now go through
form_slot_set / form_handler_set / form_meta_set. construction-time
mutations on fresh wasm-backed proto forms (before heap.alloc) stay
direct — they're populating not-yet-heap forms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If no changes:
```bash
git commit --allow-empty -m "$(cat <<'EOF'
wasm: V1.8 audit pass — all wasm.rs Form mutations are pre-alloc
construction, no migrations needed

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Migrate `compiler.rs` and `vm.rs`

**Files:**
- Modify: `crates/substrate/src/compiler.rs`, `crates/substrate/src/vm.rs`

- [ ] **Step 1: Audit compiler.rs and vm.rs mutation sites**

```bash
grep -n "heap.get_mut\|\.slots\.insert\|\.handlers\.insert\|\.meta\.insert" crates/substrate/src/compiler.rs crates/substrate/src/vm.rs
```

Substrate-internal caches (chunk_ops, chunk_consts, chunk_ics, native_fns) live on `World` directly, NOT in form slots/handlers/meta. They don't go through the nursery — they're caches keyed by FormId, not user-visible form state. Distinguish those from genuine form-state mutations.

- [ ] **Step 2: Migrate found Form-state mutations**

For each mutation that touches `slots` / `handlers` / `meta` of an existing form, route through the nursery API. Substrate-internal caches stay direct.

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED" | tail -10`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/src/compiler.rs crates/substrate/src/vm.rs
git commit -m "$(cat <<'EOF'
compiler / vm: migrate Form-state mutations to nursery-aware path

V1.9 — compiler.rs and vm.rs mutations of form slots / handlers /
meta now go through form_*_set. substrate-internal caches
(chunk_ops, chunk_consts, chunk_ics, native_fns) stay direct as
they're per-FormId tables on World, not user-visible form state.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

(If audit finds nothing to migrate, do an empty commit similar to Task 9.)

---

## Task 11: Comprehensive integration tests

**Files:**
- Create: `crates/substrate/tests/nursery_e2e.rs`

- [ ] **Step 1: Create the integration test file**

```rust
//! end-to-end tests for the V1 nursery + diff machinery.
//! exercises the public turn API + the implicit-turn wrapping
//! of eval_program / eval, including rollback semantics.

use moof::nursery::FaceKind;
use moof::value::Value;

#[test]
fn explicit_turn_alloc_mutate_commit() {
    let mut w = moof::new_world_bare();
    let initial_watermark = w.turn_watermark;

    w.start_turn();

    // alloc a form (above watermark — new-alloc path).
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("hello");
    w.form_slot_set(id, key, Value::Int(42));

    // mid-turn: read sees the value (direct canonical for new alloc).
    assert_eq!(w.form_slot(id, key), Value::Int(42));

    let diff = w.commit_turn();

    // post-commit: heap is canonical-updated.
    assert_eq!(w.heap.get(id).slot(key), Value::Int(42));
    // diff lists the new alloc.
    assert!(diff.new_allocs.contains(&id));
    // new alloc's mutations don't appear in diff.mutations
    // (it had no prior state).
    assert!(diff.mutations.is_empty());
    // watermark advanced.
    assert_eq!(w.turn_watermark, initial_watermark + 1);
}

#[test]
fn explicit_turn_mutate_pre_existing_emits_diff_entry() {
    let mut w = moof::new_world_bare();

    // alloc and commit one form first.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("count");
    w.form_slot_set(id, key, Value::Int(0));
    let _ = w.commit_turn();

    // now mutate the pre-existing form in a new turn.
    w.start_turn();
    w.form_slot_set(id, key, Value::Int(99));
    let diff = w.commit_turn();

    // diff has the (id, slots, key) entry with prior=0, new=99.
    let entry = diff
        .mutations
        .get(&(id, FaceKind::Slots, key))
        .copied();
    assert_eq!(entry, Some((Value::Int(0), Value::Int(99))));
    assert_eq!(w.heap.get(id).slot(key), Value::Int(99));
}

#[test]
fn explicit_turn_abort_rolls_back_alloc_and_mutation() {
    let mut w = moof::new_world_bare();

    // first turn: alloc and commit a form.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let key = w.intern("count");
    w.form_slot_set(id, key, Value::Int(0));
    let _ = w.commit_turn();
    let watermark_after_first_commit = w.turn_watermark;

    // second turn: alloc another form, mutate the first, then abort.
    w.start_turn();
    let _id2 = w.heap.alloc(moof::form::Form::default());
    w.form_slot_set(id, key, Value::Int(99));
    w.abort_turn();

    // canonical state preserved.
    assert_eq!(w.heap.get(id).slot(key), Value::Int(0));
    // watermark unchanged (abort doesn't advance).
    assert_eq!(w.turn_watermark, watermark_after_first_commit);
    // heap was truncated — _id2 no longer exists in the Vec.
    assert_eq!(w.heap.len() as u32, watermark_after_first_commit);
}

#[test]
fn raise_in_eval_program_aborts_implicit_turn() {
    let mut w = moof::new_world_bare();
    let env_id = w.global_env;
    let foo_sym = w.intern("foo");
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);

    let result = moof::eval_program(
        &mut w,
        "(def foo 5) (raise: 'boom \"x\")",
    );
    assert!(result.is_err());

    // foo binding rolled back; canonical env unchanged.
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Nil);
}

#[test]
fn successful_eval_program_commits_state_visibly() {
    let mut w = moof::new_world_bare();
    let env_id = w.global_env;
    let foo_sym = w.intern("foo");

    let result = moof::eval_program(&mut w, "(def foo 42) foo");
    assert_eq!(result.unwrap(), Value::Int(42));

    // post-commit: canonical env has the binding.
    assert_eq!(w.heap.get(env_id).slot(foo_sym), Value::Int(42));
}

#[test]
fn mutation_outside_turn_panics() {
    let mut w = moof::new_world_bare();
    let id = w.heap.alloc(moof::form::Form::default());

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        w.form_slot_set(id, w.intern("x"), Value::Int(1));
    }));
    assert!(panicked.is_err(), "expected panic on mutation outside turn");
}

#[test]
fn diff_handles_handlers_and_meta_faces() {
    let mut w = moof::new_world_bare();

    // alloc and commit.
    w.start_turn();
    let id = w.heap.alloc(moof::form::Form::default());
    let _ = w.commit_turn();

    // mutate all three faces.
    w.start_turn();
    let k = w.intern("k");
    w.form_slot_set(id, k, Value::Int(1));
    w.form_handler_set(id, k, Value::Int(2));
    w.form_meta_set(id, k, Value::Int(3));
    let diff = w.commit_turn();

    assert_eq!(diff.mutations.len(), 3);
    assert!(diff.mutations.contains_key(&(id, FaceKind::Slots, k)));
    assert!(diff.mutations.contains_key(&(id, FaceKind::Handlers, k)));
    assert!(diff.mutations.contains_key(&(id, FaceKind::Meta, k)));
}
```

- [ ] **Step 2: Verify the test file imports work**

The test file imports `moof::nursery::FaceKind`, `moof::value::Value`, etc. Verify the public re-export path:

In `crates/substrate/src/lib.rs`, ensure these are accessible:
```rust
pub mod form;     // exposes Form, FormId
pub mod nursery;  // exposes FaceKind, Delta, TurnDiff
pub mod value;    // exposes Value
pub mod world;    // exposes World, RaiseError
```

If `form::Form` isn't accessible to integration tests, ensure the module's `pub` visibility lets it be accessed.

- [ ] **Step 3: Run the new integration tests**

Run: `cargo test --test nursery_e2e`
Expected: all 7 tests pass.

Run: `cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass=" pass }'`
Expected: 436 (429 + 7 new e2e tests).

- [ ] **Step 4: Commit**

```bash
git add crates/substrate/tests/nursery_e2e.rs
git commit -m "$(cat <<'EOF'
tests: nursery_e2e — end-to-end coverage for V1 turn machinery

V1.10 — integration tests for the public turn API:
- explicit start/commit with new-alloc forms
- explicit start/commit with pre-existing form mutations + diff
  capture
- explicit abort rolls back both new allocs (Vec::truncate to
  watermark) and mutations (deltas dropped)
- raise during eval_program aborts the implicit turn — env
  bindings rolled back
- successful eval_program commits state visibly
- mutation outside a turn panics
- diff captures all three faces (slots, handlers, meta)

7 tests in a dedicated integration test file. exercises the public
API end-to-end, complementing the unit tests in world.rs and
nursery.rs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Final verification gate

**Files:**
- (none modified — verification only)

- [ ] **Step 1: Run full test suite, count tests**

Run: `cargo test --workspace 2>&1 | tee /tmp/v1-final.txt | grep -E "^test result"`
Run: `grep -E "^test result" /tmp/v1-final.txt | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print pass }'`
Expected: 436 (or thereabouts — confirm exact count after migration).

- [ ] **Step 2: Verify no warnings**

Run: `cargo build --workspace 2>&1 | grep -E "warning|error\[" | head -20`
Expected: empty (no new warnings from V1).

- [ ] **Step 3: Verify no FAILED anywhere**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "FAILED|panicked" | head -10`
Expected: empty.

- [ ] **Step 4: Verify the boot turn invariant**

Run: `cargo test --workspace world::tests::boot_turn_commits_cleanly`
Expected: PASS.

- [ ] **Step 5: Verify rollback invariant via the e2e tests**

Run: `cargo test --test nursery_e2e raise_in_eval_program_aborts_implicit_turn`
Expected: PASS — confirms env mutations during a raising turn roll back.

- [ ] **Step 6: V1 lands**

V1's exit criteria from the spec (§22):
> - introduce nursery as a separate small heap allocated on turn-entry ✓
> - redirect `Form::alloc` and slot/handler/meta mutations to nursery during a turn ✓
> - implement read-through (nursery first, fall through to canonical) ✓
> - compute per-slot diff at turn-end ✓
> - journal diffs to a new in-memory `inputs.log` (not yet on disk) — diff is *returned* by commit_turn; storing in an inputs.log is a V9 concern (deferred per V1 spec §1)
> - exit criteria: every successful turn produces a diff; rollback drops nursery cleanly ✓

V1 complete. No final commit needed — Tasks 1–11 each committed independently.

---

## Self-Review Notes (for the planner; safe to delete after execution)

**Spec coverage:** V1 spec §13's 14 sub-tasks map to the 12 plan tasks:
- V1.0 → Task 1 ✓
- V1.1 → Task 2 ✓
- V1.2 → Task 3 ✓
- V1.3 → Task 4 ✓
- V1.4 → Task 5 ✓
- V1.5 → Task 6 ✓
- V1.6 + V1.7 + V1.12 → Task 7 (consolidated per-file) ✓
- V1.8 + V1.9 → Task 8 (consolidated per-file) ✓
- V1.10 → Task 9 ✓
- V1.11 → Task 10 ✓
- V1.13 → Task 11 ✓
- V1.14 → Task 12 ✓

**Placeholder scan:** No "TBD"/"TODO" content. Audit steps include grep commands for the engineer to follow rather than enumerating every site (which would be brittle if intermediate refactors change line numbers). Task 9 and 10 audit-then-migrate or audit-then-no-op.

**Type consistency:** `Delta`, `FaceKind`, `TurnDiff`, `form_slot`, `form_handler`, `form_meta`, `form_slot_set`, `form_handler_set`, `form_meta_set`, `form_slot_keys`, `form_handler_keys`, `form_meta_keys`, `start_turn`, `commit_turn`, `abort_turn`, `in_turn`, `turn_watermark`, `nursery_deltas` are used consistently across tasks.

**Test count progression:**
- baseline: 388
- Task 1: +8 nursery types tests = 396
- Task 2: +9 lifecycle tests, 1 ignored = 405 - 1 = 404 (with ignored)
- Task 3: +8 read-path tests = 412
- Task 4: +9 write-path tests = 421
- Task 5: +1 reactivated test (boot turn) = 422
- Task 6: +3 implicit-turn tests, 1 ignored = 425 (with ignored)
- Task 7: +3 form_*_keys helper tests, +1 reactivated raise-aborts test = 429
- Task 8: +0 (no new tests, just migrations) = 429
- Task 9: +0 = 429
- Task 10: +0 = 429
- Task 11: +7 e2e tests = 436

Final expected count: 436.
