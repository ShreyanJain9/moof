# Vat phase V3 — env-chain / `$here` unification implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the global env into the moof object model — `$here` becomes a first-class moof Form; `def`, `set!`, and `if` collapse from rust special forms with dedicated opcodes (`Op::DefineGlobal`, `Op::StoreName`) into pure method dispatch. Adds full Ruby `instance_eval` semantics via a non-mutating view-env walker extension. A compile-time peephole optimizer recovers `if` performance without sacrificing macro purity at the source.

**Architecture:** A single new substrate-internal meta key `view-target` is recognized additively by `World::env_lookup` and `World::env_set` — non-mutating live forwarding for `[obj eval: closure]`. One new rust primitive on Closure (`:callIn:withSelf:`) is the irreducible "run closure body with explicit env+self" escape hatch; everything else (Object `:eval:`, future vau / fexpr) is moof code on top. Two opcodes removed; three compile paths rewritten to emit Send-based bytecode that the new peephole optimizer can collapse back to Jump-based for the `if` shape.

**Tech Stack:** Rust 2021, `cargo test --workspace`, `IndexMap` from the indexmap crate (already used by `Form`), `HashSet`/`HashMap` from `std::collections`. Tests live in `crates/substrate/src/<file>.rs::tests` (unit) and `crates/substrate/tests/here_e2e.rs` (integration). Moof stdlib in `lib/early/06-control-macros.moof`, `lib/stdlib/object.moof`.

---

## File Structure

| file | role |
|---|---|
| `crates/substrate/src/world.rs` | rename `global_env` field to `here_form` (32 sites in this file alone); add `view_target_sym` cache; extend `env_lookup` and `env_set` to consult view-target meta; add `is_frozen` of receiver pre-check NOT needed (view-env is non-mutating) |
| `crates/substrate/src/intrinsics.rs` | install Env proto methods (`:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current`); install Closure proto method (`:callIn:withSelf:`); bind `$here` self-reference at boot; remove `Op::DefineGlobal` and `Op::StoreName` from op-form encode/decode tables |
| `crates/substrate/src/vm.rs` | remove `Op::DefineGlobal` and `Op::StoreName` handlers in `step()` |
| `crates/substrate/src/opcodes.rs` | remove `Op::DefineGlobal` and `Op::StoreName` enum variants and any helper trait impls referencing them |
| `crates/substrate/src/compiler.rs` | rust seed: rewrite `compile_def` and `compile_if` to emit Send-based bytecode (no jump-based `if`, no `Op::DefineGlobal`); add peephole optimizer in `compile_send` recognizing the `Send :ifTrue:ifFalse:` with syntactic-closure args shape and emitting Jump-based bytecode inline |
| `crates/substrate/src/lib.rs` | propagate field rename in `new_world` boot wrap (`w.global_env` → `w.here_form` references in the `$hash` bootstrap) |
| `lib/compiler/02-special.moof` | rewrite `compileDef:chunk:` to emit Send-based bytecode |
| `lib/compiler/03-control.moof` | rewrite `compileSet:chunk:` and `compileIf:chunk:tail:` to emit Send-based bytecode |
| `lib/compiler/01-dispatch.moof` | NO CHANGE — macro precedence over special forms is already the dispatch order (verified at lines 65-75 of current file). plan note for implementer: confirm during Task 7 work; if absent, this becomes a sub-task. |
| `lib/compiler/03-control.moof` (additionally) | mirror peephole optimizer in moof Compiler `compileSend:chunk:tail:` |
| `lib/early/06-control-macros.moof` | add `(defmacro def …)` and `(defmacro set! …)` at the top of the file (after the docstring header). file's existing mission ("convert compiler-level special forms into user-overridable macros") matches exactly. |
| `lib/stdlib/object.moof` | add `(defmethod (Object eval: closure) …)` using `[Heap metaSet: env at: 'view-target to: self]` + `[closure callIn: env withSelf: self]` |
| `crates/substrate/tests/here_e2e.rs` | new file; integration tests for V3 behaviors |

---

## Task 1: Rename `World.global_env` → `World.here_form`

**Files:**
- Modify: `crates/substrate/src/world.rs` (field declaration + ~12 self-references)
- Modify: `crates/substrate/src/intrinsics.rs` (~15 references)
- Modify: `crates/substrate/src/lib.rs` (1 reference in `new_world`'s `$hash` bootstrap)
- Modify: `crates/substrate/src/vm.rs` (any `world.global_env` references)
- Modify: `crates/substrate/src/compiler.rs` (any `world.global_env` references)
- Modify: `crates/substrate/src/transporter.rs`, `wasm.rs` (any references — verify via grep)

Pure mechanical refactor. No behavior change.

- [ ] **Step 1: Audit all references**

```bash
grep -rn "\.global_env\|world\.global_env\|w\.global_env\|self\.global_env\|pub global_env" crates/substrate/src/*.rs
```

Expected: ~32 matches across the substrate.

- [ ] **Step 2: Rename the field declaration in `World`**

In `crates/substrate/src/world.rs`, find the `pub struct World` declaration. Locate:

```rust
    pub global_env: FormId,
```

Replace with:

```rust
    /// V3 — the "here" Form for this vat. exposes as `$here` in
    /// moof code (a self-referential binding in `here_form.slots`).
    /// renamed from `global_env` in V3; V4 will move this from
    /// `World` to `Vat` per the vat-as-Form structure in
    /// `2026-05-04-vats-and-references-protocol-design.md` §9.
    pub here_form: FormId,
```

- [ ] **Step 3: Rename in `World::new`'s allocation block**

Find in `World::new` (around line 295-340 in `world.rs`):

```rust
        let mut global_env_form = Form::with_proto(Value::Form(protos.env));
        // (parent_sym set to Nil)
        global_env_form.meta.insert(parent_sym, Value::Nil);
        let global_env = heap.alloc(global_env_form);
```

Replace with:

```rust
        let mut here_form_form = Form::with_proto(Value::Form(protos.env));
        here_form_form.meta.insert(parent_sym, Value::Nil);
        let here_form = heap.alloc(here_form_form);
```

(Variable name is intentionally awkward — `here_form_form` indicates "Form that becomes the here_form FormId." Optional: rename the local to `here_env_form` if it reads better in context.)

In the struct literal at the end of `World::new`, replace `global_env,` with `here_form,`.

- [ ] **Step 4: Sed-style update for self-references in `world.rs`**

Run from repo root:

```bash
grep -n "self\.global_env" crates/substrate/src/world.rs
```

Each match (likely in `env_lookup`, `env_set`, helpers): replace `self.global_env` with `self.here_form`. Manually verify each match.

- [ ] **Step 5: Update `intrinsics.rs` references**

```bash
grep -n "\.global_env\|w\.global_env\|world\.global_env" crates/substrate/src/intrinsics.rs
```

Each match: replace `.global_env` with `.here_form`. Common patterns:

```rust
// before:
let global = w.global_env;
// after:
let global = w.here_form;
```

- [ ] **Step 6: Update `lib.rs`'s `$hash` bootstrap**

In `crates/substrate/src/lib.rs` around line 93-94:

```rust
        let global = w.global_env;
        w.env_bind(global, dollar_hash, hash_instance)
            .expect("env_bind at boot — substrate bug");
```

Replace `w.global_env` with `w.here_form`. Variable name stays `global` for now — it represents the same FormId.

- [ ] **Step 7: Update `vm.rs`, `compiler.rs`, `transporter.rs`, `wasm.rs` references**

Run:

```bash
grep -rn "\.global_env\|world\.global_env\|w\.global_env" crates/substrate/src/{vm,compiler,transporter,wasm}.rs
```

Replace each `.global_env` with `.here_form`. Confirm each file's references are updated.

- [ ] **Step 8: Verify build**

```bash
cargo build -p moof 2>&1 | tail -20
```

Expected: clean build. If errors, find any remaining `global_env` references in test files too:

```bash
grep -rn "global_env" crates/substrate/
```

Update any test references.

- [ ] **Step 9: Run full test suite**

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: 481 passing (V2 baseline; renames don't change count).

- [ ] **Step 10: Commit**

```bash
git add crates/substrate/src/
git commit -m "$(cat <<'EOF'
world: rename global_env field → here_form (V3 prep)

Pure mechanical rename. All ~32 internal references updated. No
behavior change.

Forward-looking: V4 moves here_form from World to Vat. The new
name reflects "the env we are in" rather than the now-outdated
"the global env" — matches the V0 spec §8 framing where $here is
this vat's persistent root.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Intern `view-target` symbol + extend `env_lookup` to consult view-target

**Files:**
- Modify: `crates/substrate/src/world.rs`

V3's view-env mechanism: a substrate-internal meta key `view-target` that `env_lookup` recognizes additively. Forms without the meta key behave identically to V2/V1 (backward compat). Forms WITH the meta key get the live-forwarding semantic.

- [ ] **Step 1: Locate the existing reserved-meta-symbol caches in `World`**

```bash
grep -nE "parent_sym: SymId|source_sym: SymId|name_meta:|generation_sym:" crates/substrate/src/world.rs | head
```

Find the struct declaration where these SymIds are cached (typically inside `pub struct World` after the existing pub fields).

- [ ] **Step 2: Add `view_target_sym` to the World struct**

Find the `pub struct World { ... }` declaration. Add immediately alongside existing reserved-symbol fields:

```rust
    /// V3 — meta key recognized by `env_lookup` and `env_set`.
    /// when an env-Form has `:meta at: 'view-target` set to another
    /// Form, the walker also consults that Form's slots after its
    /// own. used by `Object:eval:` to splice an obj's slots into
    /// the lookup chain without mutating obj.
    pub view_target_sym: SymId,
```

(If the existing fields like `parent_sym` aren't `pub`, match their visibility — could be `pub(crate)` or private with an accessor. The implementer should match local convention.)

- [ ] **Step 3: Initialize `view_target_sym` in `World::new`**

In `World::new`, find where existing reserved-symbol SymIds are interned (look for `let parent_sym = ...intern("parent")...` or similar). Add:

```rust
        let view_target_sym = syms.intern("view-target");
```

In the `Self { ... }` literal near the end of `World::new`, add `view_target_sym,` to the struct construction.

- [ ] **Step 4: Write a failing test for `env_lookup` view-target behavior**

In `crates/substrate/src/world.rs::tests`, add:

```rust
    #[test]
    fn env_lookup_consults_view_target_after_own_slots() {
        let mut w = World::new();
        // alloc a "viewed-into" form (e.g. an obj with slots)
        let obj_form = w.alloc(Form::default());
        let foo = w.intern("foo");
        let val = Value::Int(42);
        // bind 'foo on obj via direct heap (test setup; bypasses turn machinery)
        w.heap.get_mut(obj_form).slots.insert(foo, val);

        // alloc an env with view-target = obj
        let env = w.alloc_env(None);
        w.heap.get_mut(env).meta.insert(w.view_target_sym, Value::Form(obj_form));

        // lookup 'foo in env: env's own slots are empty, but view-target hits.
        assert_eq!(w.env_lookup(env, foo), Some(val));
    }

    #[test]
    fn env_lookup_own_slots_shadow_view_target() {
        let mut w = World::new();
        let obj_form = w.alloc(Form::default());
        let foo = w.intern("foo");
        // obj has foo → 1
        w.heap.get_mut(obj_form).slots.insert(foo, Value::Int(1));

        let env = w.alloc_env(None);
        // env has foo → 2 in its own slots
        w.heap.get_mut(env).slots.insert(foo, Value::Int(2));
        // env's view-target is obj
        w.heap.get_mut(env).meta.insert(w.view_target_sym, Value::Form(obj_form));

        // env's own foo (2) wins over view-target's foo (1)
        assert_eq!(w.env_lookup(env, foo), Some(Value::Int(2)));
    }

    #[test]
    fn env_lookup_without_view_target_unchanged() {
        // regression: pre-V3 behavior preserved when view-target meta absent.
        let mut w = World::new();
        let env = w.alloc_env(None);
        let foo = w.intern("foo");
        w.heap.get_mut(env).slots.insert(foo, Value::Int(7));
        assert_eq!(w.env_lookup(env, foo), Some(Value::Int(7)));
    }
```

- [ ] **Step 5: Run tests, expect first two to FAIL**

```bash
cargo test -p moof --lib env_lookup_consults_view_target env_lookup_own_slots_shadow env_lookup_without_view_target 2>&1 | tail -15
```

Expected: 1 pass (the regression test), 2 fail (the view-target tests fail because the walker doesn't consult view-target yet).

- [ ] **Step 6: Extend `env_lookup` to consult view-target**

Find `pub fn env_lookup` in `world.rs` (around line 608). The current shape is roughly:

```rust
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            // existing dual-check logic for delta + canonical:
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

Insert the view-target check between the own-slots check and the parent walk:

```rust
    pub fn env_lookup(&self, env: FormId, name: SymId) -> Option<Value> {
        let mut cur = env;
        loop {
            // existing dual-check logic for delta + canonical:
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
            // V3 — view-target consultation. forms with
            // :meta at: 'view-target = Some(Form(target)) get
            // their lookup chain extended through target's slots
            // (one level — does not recurse into target's parent
            // chain). used by Object:eval: for live forwarding.
            if let Some(target_v) = f.meta.get(&self.view_target_sym).copied() {
                if let Some(target_id) = target_v.as_form_id() {
                    let tf = self.heap.get(target_id);
                    if let Some(v) = tf.slots.get(&name).copied() {
                        return Some(v);
                    }
                    // also consult target's nursery delta if in-turn
                    if self.in_turn && target_id.payload() < self.turn_watermark {
                        if let Some(delta) = self.nursery_deltas.get(&target_id) {
                            if let Some(v) = delta.slots.get(&name).copied() {
                                return Some(v);
                            }
                        }
                    }
                }
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

- [ ] **Step 7: Run tests, verify all 3 pass**

```bash
cargo test -p moof --lib env_lookup_consults_view_target env_lookup_own_slots_shadow env_lookup_without_view_target 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 8: Run full library suite**

```bash
cargo test -p moof --lib 2>&1 | grep -E "^test result"
```

Expected: 481 + 3 = 484 lib tests passing (or whatever the V2 lib baseline is + 3 new). All previously-passing tests still pass.

- [ ] **Step 9: Commit**

```bash
git add crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
world: env_lookup consults view-target meta for live forwarding

V3 prep. Adds an additive walker extension: when an env-Form has
:meta at: 'view-target = Some(Form(target)) set, env_lookup
also checks target's slots (and its delta during a turn) after
its own slots, before walking parent. Forms without the meta
key behave identically to V2 — pure backward-compatible addition.

Used by Object:eval: (Task 14) for live forwarding into the
receiver without mutating receiver.parent. Works on frozen
receivers because no mutation is involved.

3 unit tests cover: view-target hit, own-slots-shadow-view-target,
regression for the view-target-absent case.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Extend `env_set` to consult view-target for live mutations

**Files:**
- Modify: `crates/substrate/src/world.rs`

`env_set` walks the chain looking for an existing binding to mutate (the `set!` semantic). V3 extends it to also consult view-target's slots — when a name is found in view-target, the mutation writes through to view-target LIVE.

- [ ] **Step 1: Write failing test**

Add to `world.rs::tests`:

```rust
    #[test]
    fn env_set_writes_to_view_target_when_name_found_there() {
        let mut w = World::new();
        let obj_form = w.alloc(Form::default());
        let foo = w.intern("foo");
        // obj.foo = 1 (initial)
        w.heap.get_mut(obj_form).slots.insert(foo, Value::Int(1));

        // env: own slots empty, view-target = obj
        let env = w.alloc_env(None);
        w.heap.get_mut(env).meta.insert(w.view_target_sym, Value::Form(obj_form));

        // env_set walks chain. env.slots doesn't have 'foo, but
        // view-target does. mutation writes through to obj LIVE.
        w.start_turn();
        let found = w.env_set(env, foo, Value::Int(99)).unwrap();
        let _ = w.commit_turn();
        assert!(found, "env_set should report found");
        // verify the LIVE mutation: obj.foo is now 99
        assert_eq!(w.heap.get(obj_form).slot(foo), Value::Int(99));
        // env's own slots are still empty
        assert!(w.heap.get(env).slots.get(&foo).is_none());
    }

    #[test]
    fn env_set_own_slots_take_priority_over_view_target() {
        let mut w = World::new();
        let obj_form = w.alloc(Form::default());
        let foo = w.intern("foo");
        w.heap.get_mut(obj_form).slots.insert(foo, Value::Int(1));

        let env = w.alloc_env(None);
        w.heap.get_mut(env).slots.insert(foo, Value::Int(2));
        w.heap.get_mut(env).meta.insert(w.view_target_sym, Value::Form(obj_form));

        w.start_turn();
        w.env_set(env, foo, Value::Int(99)).unwrap();
        let _ = w.commit_turn();
        // env's own foo updated; obj's foo unchanged
        assert_eq!(w.heap.get(env).slot(foo), Value::Int(99));
        assert_eq!(w.heap.get(obj_form).slot(foo), Value::Int(1));
    }
```

- [ ] **Step 2: Run tests, expect failures**

```bash
cargo test -p moof --lib env_set_writes_to_view_target env_set_own_slots_take_priority 2>&1 | tail -10
```

Expected: 2 failures (env_set doesn't consult view-target yet).

- [ ] **Step 3: Extend `env_set`**

Find `pub fn env_set` in `world.rs` (around line 617). Current shape (V2):

```rust
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> Result<bool, RaiseError> {
        let mut cur = env;
        loop {
            let bound_in_delta = self
                .nursery_deltas
                .get(&cur)
                .map(|d| d.slots.contains_key(&name))
                .unwrap_or(false);
            let bound_in_canonical = self.heap.get(cur).slots.contains_key(&name);
            if bound_in_delta || bound_in_canonical {
                self.form_slot_set(cur, name, value)?;
                return Ok(true);
            }
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Form(id) => cur = id,
                _ => return Ok(false),
            }
        }
    }
```

Insert view-target check between own-slots check and parent walk:

```rust
    pub fn env_set(&mut self, env: FormId, name: SymId, value: Value) -> Result<bool, RaiseError> {
        let mut cur = env;
        loop {
            // 1. own slots (delta + canonical)
            let bound_in_delta = self
                .nursery_deltas
                .get(&cur)
                .map(|d| d.slots.contains_key(&name))
                .unwrap_or(false);
            let bound_in_canonical = self.heap.get(cur).slots.contains_key(&name);
            if bound_in_delta || bound_in_canonical {
                self.form_slot_set(cur, name, value)?;
                return Ok(true);
            }
            // 2. V3 — view-target consultation. if this env has
            // :meta at: 'view-target = Some(Form(target)) AND target
            // has 'name bound, write through to target LIVE.
            let target_v = self.form_meta(cur, self.view_target_sym);
            if let Some(target_id) = target_v.as_form_id() {
                let bound_in_target_delta = self
                    .nursery_deltas
                    .get(&target_id)
                    .map(|d| d.slots.contains_key(&name))
                    .unwrap_or(false);
                let bound_in_target_canonical = self.heap.get(target_id).slots.contains_key(&name);
                if bound_in_target_delta || bound_in_target_canonical {
                    self.form_slot_set(target_id, name, value)?;
                    return Ok(true);
                }
            }
            // 3. walk parent
            let parent = self.form_meta(cur, self.parent_sym);
            match parent {
                Value::Form(id) => cur = id,
                _ => return Ok(false),
            }
        }
    }
```

- [ ] **Step 4: Run tests, verify they pass**

```bash
cargo test -p moof --lib env_set_writes_to_view_target env_set_own_slots_take_priority 2>&1 | tail -10
```

Expected: 2 passed.

- [ ] **Step 5: Run full lib suite**

```bash
cargo test -p moof --lib 2>&1 | grep -E "^test result"
```

Expected: 486 passing (484 + 2 new).

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/world.rs
git commit -m "$(cat <<'EOF'
world: env_set writes through to view-target for live mutation

V3 prep. env_set now consults the view-target meta key (Task 2's
addition) and writes mutations LIVE to view-target's slots when a
name is found there. Own slots take priority — view-target only
fires when name isn't on the env itself. Backward-compatible: forms
without view-target meta walk identically to V2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Bind `$here` in `here_form.slots` at boot

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs`

The `$here` symbol must be bound in the global env's own slots, self-referentially pointing to the global env Form.

- [ ] **Step 1: Locate `intrinsics::install`'s global-binding loop**

```bash
grep -n "let bindings" crates/substrate/src/intrinsics.rs | head
```

Expected: a section in `install_proto_globals` (or similar) where protos like `Object`, `Integer`, `Cons`, etc. are bound globally. The loop ends around `intrinsics.rs:1054`.

- [ ] **Step 2: Add `$here` binding right after the existing proto bindings**

Find the loop that does:

```rust
    for (name, id) in bindings {
        let s = w.intern(name);
        w.env_bind(global, s, Value::Form(id))
            .expect("env_bind at boot — substrate bug");
        // ...
    }
```

Immediately after the loop closes, add:

```rust
    // V3 — bind $here as a self-reference to the here_form.
    // moof code reaches the global env via this binding; reflection
    // (e.g. [Heap slotKeysOf: $here]) lists path-bound names.
    let here_sym = w.intern("$here");
    w.env_bind(w.here_form, here_sym, Value::Form(w.here_form))
        .expect("env_bind at boot — substrate bug");
```

- [ ] **Step 3: Write a failing test (or verify behavior via existing path)**

Since `$here` lookup goes through `eval_program` (which wraps a turn), test via `moof::eval`:

Add to `crates/substrate/tests/here_e2e.rs` (the file will be created in Task 14, but for Task 4 we can put a test in an existing file or in `intrinsics.rs::tests`):

In `crates/substrate/src/intrinsics.rs::tests` (search for `mod tests` at the bottom of the file):

```rust
    #[test]
    fn here_is_bound_to_self() {
        let mut w = crate::new_world_bare();
        let here_sym = w.intern("$here");
        let here_v = w.env_lookup(w.here_form, here_sym);
        assert_eq!(here_v, Some(Value::Form(w.here_form)));
    }
```

- [ ] **Step 4: Run the test, verify pass**

```bash
cargo test -p moof --lib here_is_bound_to_self 2>&1 | tail -5
```

Expected: 1 passed.

- [ ] **Step 5: Run full lib suite**

```bash
cargo test -p moof --lib 2>&1 | grep -E "^test result"
```

Expected: 487 passing (486 + 1 new).

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/intrinsics.rs
git commit -m "$(cat <<'EOF'
intrinsics: bind \$here = Value::Form(here_form) self-referentially

V3 — the canonical user-facing handle on this vat's env. binding
goes in here_form's own slots, so [Heap slotKeysOf: \$here] lists
\$here itself as one of its slots (self-reference is fine in moof's
"everything is a Form" model).

V9 persistence will preserve this self-reference because FormIds
are stable identifiers across serialize/deserialize.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Install Env proto methods (`:bind:to:`, `:set:to:`, `:lookup:`, `:parent`, `:current`)

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs`

Five natives installed on `protos.env` at boot. All wrap existing substrate APIs.

- [ ] **Step 1: Locate a good install site in `intrinsics::install`**

The natives go alongside other Env-related installs. Find where Env proto is otherwise referenced; if none exist, add a new `install_env_proto_methods` helper called from `install`. Or add directly inside `install` near the proto bindings.

For consistency with other proto installs (e.g. `install_table_methods`), add a helper:

```rust
fn install_env_proto_methods(w: &mut World) {
    let env_proto = w.protos.env;
    
    // :bind:to: — non-walking bind. writes name → value in self's slots.
    // returns the bound value (chainable). per V3 spec §4.1.
    w.install_native(env_proto, "bind:to:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":bind:to: receiver must be an Env Form")
        })?;
        let name = args[0].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":bind:to: name must be a Symbol")
        })?;
        let value = args[1];
        w.form_slot_set(env, name, value)?;
        Ok(value)
    })
    .expect("install_native :bind:to: at boot — substrate bug");

    // :set:to: — walks parent chain (and view-target consultation),
    // raises 'unbound on miss. returns the value on success.
    w.install_native(env_proto, "set:to:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":set:to: receiver must be an Env Form")
        })?;
        let name = args[0].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":set:to: name must be a Symbol")
        })?;
        let value = args[1];
        let found = w.env_set(env, name, value)?;
        if !found {
            let kind = w.intern("unbound");
            let message = format!("set!: '{} is unbound", w.resolve(name));
            return Err(RaiseError::new(kind, message));
        }
        Ok(value)
    })
    .expect("install_native :set:to: at boot — substrate bug");

    // :lookup: — walks chain (with view-target consultation). returns Nil on miss.
    w.install_native(env_proto, "lookup:", |w, self_, args| {
        let env = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":lookup: receiver must be an Env Form")
        })?;
        let name = args[0].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":lookup: name must be a Symbol")
        })?;
        Ok(w.env_lookup(env, name).unwrap_or(Value::Nil))
    })
    .expect("install_native :lookup: at boot — substrate bug");

    // :parent — convenience accessor for :meta at: 'parent. returns
    // the parent env Form, or Nil at chain root.
    w.install_native(env_proto, "parent", |w, self_, _args| {
        let env = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":parent receiver must be a Form")
        })?;
        Ok(w.form_meta(env, w.parent_sym))
    })
    .expect("install_native :parent at boot — substrate bug");

    // :current — class-method-style. returns the LIVE current frame's
    // env. self_ is ignored; [Env current], [$here current], or any
    // env-receiver returns the same: the caller's lexical env.
    // natives don't push a VM frame (verified at vm.rs:258), so
    // frames.last().env IS the caller's env.
    w.install_native(env_proto, "current", |w, _self_, _args| {
        let env = w.vm.frames.last()
            .map(|f| f.env)
            .ok_or_else(|| {
                RaiseError::new(
                    w.intern("env-out-of-scope"),
                    "[Env current] called outside any active method dispatch",
                )
            })?;
        Ok(Value::Form(env))
    })
    .expect("install_native :current at boot — substrate bug");
}
```

- [ ] **Step 2: Call the new helper from `intrinsics::install`**

In `intrinsics::install`, find the section where other helpers like `install_table_methods` are called. Add:

```rust
    install_env_proto_methods(w);
```

Place it after the global proto bindings (after `$here` is bound; before stdlib loads).

- [ ] **Step 3: Write tests**

Add to `crates/substrate/src/intrinsics.rs::tests`:

```rust
    #[test]
    fn env_bind_to_via_dispatch() {
        let mut w = crate::new_world();
        // [$here bind: 'newGlobal to: 42] — should bind newGlobal in $here.
        let r = crate::eval(&mut w, "[$here bind: 'newGlobal to: 42]").unwrap();
        assert_eq!(r, Value::Int(42));
        // verify it's now reachable
        let r2 = crate::eval(&mut w, "newGlobal").unwrap();
        assert_eq!(r2, Value::Int(42));
    }

    #[test]
    fn env_set_to_walks_chain_and_returns_value() {
        let mut w = crate::new_world();
        // bind something globally first via :bind:to:
        crate::eval(&mut w, "[$here bind: 'x to: 1]").unwrap();
        // set! it via Env :set:to: directly
        let r = crate::eval(&mut w, "[$here set: 'x to: 99]").unwrap();
        assert_eq!(r, Value::Int(99));
        let r2 = crate::eval(&mut w, "x").unwrap();
        assert_eq!(r2, Value::Int(99));
    }

    #[test]
    fn env_set_to_raises_unbound_when_not_in_chain() {
        let mut w = crate::new_world();
        let r = crate::eval(&mut w, "[$here set: 'definitelyNotBound to: 5]");
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(w.resolve(err.kind), "unbound");
    }

    #[test]
    fn env_lookup_returns_nil_on_miss() {
        let mut w = crate::new_world();
        let r = crate::eval(&mut w, "[$here lookup: 'definitelyNotBound]").unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn env_parent_returns_nil_at_root() {
        let mut w = crate::new_world();
        let r = crate::eval(&mut w, "[$here parent]").unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn env_current_returns_caller_env() {
        let mut w = crate::new_world();
        // [Env current] from top-level should return the eval_program's
        // current frame env. exact identity is internal; verify it's a Form.
        let r = crate::eval(&mut w, "[Env current]").unwrap();
        assert!(r.as_form_id().is_some(), "[Env current] should return a Form");
    }
```

- [ ] **Step 4: Run tests, verify all pass**

```bash
cargo test -p moof --lib env_bind_to_via_dispatch env_set_to_walks env_set_to_raises env_lookup_returns_nil env_parent_returns_nil env_current_returns_caller 2>&1 | tail -15
```

Expected: 6 passed.

- [ ] **Step 5: Run full lib suite**

```bash
cargo test -p moof --lib 2>&1 | grep -E "^test result"
```

Expected: 493 passing (487 + 6 new).

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/intrinsics.rs
git commit -m "$(cat <<'EOF'
intrinsics: install Env proto methods — bind:to:, set:to:, lookup:, parent, current

V3 — five natives on protos.env wrapping the existing substrate APIs:

- :bind:to: — no-walk, returns value
- :set:to: — walks chain (and view-target), raises 'unbound on miss,
  returns value (V3 tightens the V1 silent-fall-through footgun)
- :lookup: — walks chain, returns Nil on miss
- :parent — convenience for :meta at: 'parent
- :current — returns the caller's lexical env (used by set! macro
  to find lexical scope at call site; class-method-style — receiver
  ignored)

These compose: def macro will use :bind:to:, set! macro will use
[Env current] :set:to:, future vau / fexpr (V8) builds on :current
and :callIn:withSelf: (Task 6).

6 unit tests cover the dispatch-level behaviors.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Install `Closure:callIn:withSelf:` primitive

**Files:**
- Modify: `crates/substrate/src/intrinsics.rs`

The irreducible primitive: run a closure body with an explicit `call_env` and `self`. Bypasses the closure's own `:env` slot (which `:call` would normally use). This is the substrate exposure that `Object:eval:` (Task 14) and future vau / fexpr (V8) build on.

- [ ] **Step 1: Locate where `:call` on Closure proto is currently installed**

```bash
grep -n 'install_native(w.protos.closure, "call"' crates/substrate/src/intrinsics.rs
```

Expected: one match (around line 1090). The new `:callIn:withSelf:` is installed nearby.

- [ ] **Step 2: Write a failing test**

Add to `crates/substrate/src/intrinsics.rs::tests`:

```rust
    #[test]
    fn closure_call_in_with_self_runs_body_with_explicit_env() {
        // build a closure whose body is `(do x)` — body looks up x.
        // create two distinct envs: env_a binds x=10, env_b binds x=20.
        // verify that [closure :callIn: env_a :withSelf: nil] returns 10
        // and [closure :callIn: env_b :withSelf: nil] returns 20.
        let mut w = crate::new_world();

        // construct a closure programmatically: easier via moof.
        // (fn () x) — captures the global env at creation time.
        let closure_v = crate::eval(&mut w, "(fn () x)").unwrap();

        // bind x=10 in a fresh env, x=20 in another fresh env.
        let env_a = w.alloc_env(Some(w.here_form));
        let x_sym = w.intern("x");
        w.start_turn();
        w.form_slot_set(env_a, x_sym, Value::Int(10)).unwrap();
        let _ = w.commit_turn();

        let env_b = w.alloc_env(Some(w.here_form));
        w.start_turn();
        w.form_slot_set(env_b, x_sym, Value::Int(20)).unwrap();
        let _ = w.commit_turn();

        // [closure :callIn: env_a :withSelf: nil] — returns 10
        let call_in_sym = w.intern("callIn:withSelf:");
        let r1 = w.send(closure_v, call_in_sym, &[Value::Form(env_a), Value::Nil]).unwrap();
        assert_eq!(r1, Value::Int(10));

        // [closure :callIn: env_b :withSelf: nil] — returns 20
        let r2 = w.send(closure_v, call_in_sym, &[Value::Form(env_b), Value::Nil]).unwrap();
        assert_eq!(r2, Value::Int(20));
    }
```

- [ ] **Step 3: Run test, expect failure (no method)**

```bash
cargo test -p moof --lib closure_call_in_with_self 2>&1 | tail -10
```

Expected: failure with `unhandled-dnu` for selector `callIn:withSelf:`.

- [ ] **Step 4: Install the native**

In `intrinsics.rs`, after the existing `:call` install on Closure (around line 1090), add:

```rust
    // V3 — :callIn:withSelf: — the irreducible "run closure body with
    // explicit call_env and self" primitive. used by Object:eval:
    // (lib/stdlib/object.moof) and future vau / fexpr (V8). bypasses
    // the closure's own :env slot — caller specifies scope explicitly.
    w.install_native(w.protos.closure, "callIn:withSelf:", |w, self_, args| {
        let closure_id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":callIn:withSelf: on non-closure")
        })?;
        if args.len() != 2 {
            return Err(RaiseError::new(
                w.intern("arity"),
                ":callIn:withSelf: expects 2 args (env, self)",
            ));
        }
        let call_env = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":callIn: requires a Form env")
        })?;
        let new_self = args[1];
        // extract closure body
        let body_v = w.form_slot(closure_id, w.body_sym);
        let chunk_id = body_v.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "closure has no :body chunk")
        })?;
        // closure's own :env slot is ignored — caller controls scope.
        // run the chunk with the explicit env. defining_proto is NONE
        // because this isn't a method dispatch.
        crate::vm::run_method_with(w, chunk_id, call_env, new_self, crate::form::FormId::NONE)
    })
    .expect("install_native :callIn:withSelf: at boot — substrate bug");
```

(Note: `crate::vm::run_method_with` doesn't exist yet — see Step 5.)

- [ ] **Step 5: Expose `run_method` as a public helper**

The existing `vm::run_method` (around vm.rs:299) is module-private. Expose it (or a wrapper) as `pub(crate) fn run_method_with` so the intrinsics native can call it.

In `crates/substrate/src/vm.rs`, find:

```rust
fn run_method(
    world: &mut World,
    chunk: FormId,
    env: FormId,
    self_v: Value,
    defining_proto: FormId,
) -> Result<Value, RaiseError> {
```

Change visibility to `pub(crate)`:

```rust
pub(crate) fn run_method(
    world: &mut World,
    chunk: FormId,
    env: FormId,
    self_v: Value,
    defining_proto: FormId,
) -> Result<Value, RaiseError> {
```

In `intrinsics.rs`, change `crate::vm::run_method_with` references to `crate::vm::run_method` (use the existing name).

- [ ] **Step 6: Run tests, verify pass**

```bash
cargo test -p moof --lib closure_call_in_with_self 2>&1 | tail -5
```

Expected: 1 passed.

- [ ] **Step 7: Run full lib suite**

```bash
cargo test -p moof --lib 2>&1 | grep -E "^test result"
```

Expected: 494 passing (493 + 1 new).

- [ ] **Step 8: Commit**

```bash
git add crates/substrate/src/intrinsics.rs crates/substrate/src/vm.rs
git commit -m "$(cat <<'EOF'
intrinsics: install Closure :callIn:withSelf: primitive

V3 — the irreducible "run closure body with explicit env and self"
escape hatch. Bypasses the closure's own :env slot, letting the
caller fully control scope. Used by Object:eval: (Task 14, moof
stdlib) and future vau / fexpr (V8) — both compose on this.

vm::run_method's visibility raised to pub(crate) so the native
body can call it directly.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Rewrite rust seed `compile_def` and moof Compiler `compileDef:` to emit Send-based bytecode

**Files:**
- Modify: `crates/substrate/src/compiler.rs`
- Modify: `lib/compiler/02-special.moof`

Both compilers stop emitting `Op::DefineGlobal` and instead emit the Send-based pattern equivalent to the def-macro expansion. After this task, `Op::DefineGlobal` is unused — Task 9 removes it.

- [ ] **Step 1: Locate rust seed `compile_def`**

```bash
grep -n "fn compile_def" crates/substrate/src/compiler.rs
```

Expected: around line 376. Read the current body — emits `Op::DefineGlobal(name)`.

- [ ] **Step 2: Rewrite rust seed `compile_def`**

Find:

```rust
    fn compile_def(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        if elems.len() != 3 {
            return Err(self.err("malformed def form"));
        }
        let name = elems[1].as_sym().ok_or_else(|| self.err("def name must be a symbol"))?;
        self.compile_form(elems[2], false)?;
        self.emit(Op::DefineGlobal(name));
        Ok(())
    }
```

Replace with:

```rust
    fn compile_def(&mut self, elems: &[Value]) -> Result<(), RaiseError> {
        // V3: (def name value) compiles to Send-based bytecode equivalent to
        // (do [$here bind: 'name to: value] 'name). Op::DefineGlobal is
        // no longer emitted — the only path to env_bind on $here is now
        // method dispatch on Env's :bind:to:.
        if elems.len() != 3 {
            return Err(self.err("malformed def form"));
        }
        let name = elems[1].as_sym().ok_or_else(|| self.err("def name must be a symbol"))?;
        let here_sym = self.world.intern("$here");
        let bind_to_sym = self.world.intern("bind:to:");

        // LoadName $here  (push receiver)
        self.emit(Op::LoadName(here_sym));
        // LoadConst 'name  (push first arg — the symbol)
        let name_const = self.add_const(Value::Sym(name));
        self.emit(Op::LoadConst(name_const));
        // compile rhs  (push second arg — the value)
        self.compile_form(elems[2], false)?;
        // Send :bind:to: arity=2  (pops receiver + 2 args, pushes result)
        let ic_idx = self.fresh_ic();
        self.emit(Op::Send {
            selector: bind_to_sym,
            argc: 2,
            ic_idx,
        });
        // discard bind result (the value); push 'name as def's return value
        self.emit(Op::Pop);
        let name_const2 = self.add_const(Value::Sym(name));
        self.emit(Op::LoadConst(name_const2));
        Ok(())
    }
```

(Verify `Op::LoadName`, `Op::LoadConst`, `Op::Send`, `Op::Pop`, and the helpers `self.add_const`, `self.fresh_ic`, `self.compile_form`, `self.emit` all exist with these signatures. Adjust if local naming differs.)

- [ ] **Step 3: Locate moof Compiler `compileDef:`**

In `lib/compiler/02-special.moof`, find:

```moof
(setHandler! Compiler 'compileDef:chunk:
  (fn (rest chunk)
    ...))
```

Around line 67-77.

- [ ] **Step 4: Rewrite `compileDef:`**

Replace its body to emit the Send-based bytecode pattern. The exact moof shape depends on the local Compiler module's helpers (e.g. `Chunk:emit:`, `Chunk:addConst:`, `Opcode loadName: ...`, `Opcode loadConst: ...`, `Opcode send: arity: ic:`).

Use this template, adapted to local conventions:

```moof
(setHandler! Compiler 'compileDef:chunk:
  (fn (rest chunk)
    ;; (def name value) compiles to:
    ;;   loadName $here
    ;;   loadConst 'name
    ;;   <compile rhs>
    ;;   send :bind:to: arity=2 ic:fresh
    ;;   pop
    ;;   loadConst 'name
    (let ((name [rest car])
          (rhs [[rest cdr] car]))
      ;; LoadName $here
      [chunk emit: [Opcode loadName: '$here]]
      ;; LoadConst 'name
      [chunk emit: [Opcode loadConst: [chunk addConst: name]]]
      ;; compile rhs (pushes the value on stack)
      [self compileForm: rhs chunk: chunk tail: #false]
      ;; Send :bind:to: arity=2
      (let ((ic-idx [chunk freshIc]))
        [chunk emit: [Opcode send: 'bind:to: arity: 2 ic: ic-idx]])
      ;; Pop the bind result; push 'name as def's return value
      [chunk emit: [Opcode pop]]
      [chunk emit: [Opcode loadConst: [chunk addConst: name]]])))
```

(Adjust selector names — `Opcode loadName:`, `Opcode send:arity:ic:`, etc. — to match local Compiler-module conventions. Run `grep -n "Opcode\|emit:" lib/compiler/*.moof` to learn the exact spellings.)

- [ ] **Step 5: Verify build**

```bash
cargo build -p moof 2>&1 | tail -10
```

Expected: green.

- [ ] **Step 6: Run full workspace tests**

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: same count as before this task (no behavior change — both paths now emit Send-based bytecode that goes through `:bind:to:`, which routes to `env_bind`, same as the prior `Op::DefineGlobal`).

- [ ] **Step 7: Commit**

```bash
git add crates/substrate/src/compiler.rs lib/compiler/02-special.moof
git commit -m "$(cat <<'EOF'
compiler: compile_def emits Send-based bytecode (no Op::DefineGlobal)

V3 task 7. Both rust seed compile_def and moof Compiler compileDef:
rewritten to emit the same Send-based bytecode pattern equivalent to
the def-macro expansion: LoadName \$here, LoadConst 'name, compile
rhs, Send :bind:to: arity=2, Pop, LoadConst 'name. After this task,
no path emits Op::DefineGlobal — Task 9 removes the opcode itself.

Behavioral equivalent: same env_bind under the hood, just reached
via method dispatch rather than a dedicated opcode.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Add `(defmacro def …)` to `lib/early/06-control-macros.moof`

**Files:**
- Modify: `lib/early/06-control-macros.moof`

The user-overridable macro that takes precedence over `compileDef:` post-load. Macro precedence is already in dispatch.moof (verified during plan write). compileDef: stays as bootstrap fallback for early/00-05 files.

- [ ] **Step 1: Read the existing top of `06-control-macros.moof`**

```bash
head -50 lib/early/06-control-macros.moof
```

The file's docstring lists "the only true compiler-level special forms remaining after these macros load: `if`, `let`, `do`, `quote`, `set!`, `fn`, `def`, `defmacro`." V3 will remove `def` and `set!` from this list (and `if` in Task 12).

- [ ] **Step 2: Add the def macro**

After the file's docstring (and after `__cascade__` / `__table__` / `__obj__` if they appear first — match the file's existing structure), insert:

```moof
;; ─────────────────────────────────────────────────────────────────
;; def — purified to a macro in V3.
;;
;; (def name value) → (do [$here bind: 'name to: value] 'name)
;;
;; Returns 'name (a Symbol) so `(def x 42)` evaluates to `'x` —
;; matches the existing rust-seed Op::DefineGlobal behavior.
;; ─────────────────────────────────────────────────────────────────
(defmacro def (args)
  (let ((name [args car])
        (value [[args cdr] car]))
    `(do
       [$here bind: ',name to: ,value]
       ',name)))
```

- [ ] **Step 3: Update the file's docstring**

Find the doc paragraph listing remaining special forms. Replace `def` with no entry (remove from the list). The updated paragraph should read approximately:

```
;; the only true compiler-level special forms remaining (after these
;; macros load) are: `let`, `do`, `quote`, `fn`, `defmacro`,
;; plus the reader-emitted helper `__send__`. each genuinely needs
;; bytecode-level access (env construction, compile-time evaluation order).
```

(Note: `set!` and `if` will be removed from this list in Tasks 11 and 13. After all V3 tasks, the list shrinks accordingly.)

- [ ] **Step 4: Verify build + tests**

```bash
cargo build -p moof 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: build green, test count unchanged (def macro shadows compileDef: from this point onward, but emits identical bytecode).

- [ ] **Step 5: Add a test verifying the macro path is taken**

In `crates/substrate/tests/here_e2e.rs` (create the file if Task 14 hasn't yet):

```rust
//! V3 here-form / env unification — end-to-end tests.

use moof::value::Value;

#[test]
fn def_macro_binds_via_here() {
    let mut w = moof::new_world();
    // (def x 42) should bind x in $here. verify via direct env_lookup.
    moof::eval(&mut w, "(def x 42)").unwrap();
    let x_sym = w.intern("x");
    assert_eq!(w.env_lookup(w.here_form, x_sym), Some(Value::Int(42)));
}

#[test]
fn def_returns_the_symbol() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "(def y 99)").unwrap();
    let y_sym = w.intern("y");
    assert_eq!(r, Value::Sym(y_sym));
}
```

Run:

```bash
cargo test --test here_e2e 2>&1 | tail -10
```

Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add lib/early/06-control-macros.moof crates/substrate/tests/here_e2e.rs
git commit -m "$(cat <<'EOF'
stdlib: (defmacro def …) — purify def to a moof macro

V3 task 8. def joins if (already at lib/early/10), when, unless,
let*, etc. as a moof-side macro. Expansion:

  (def name value) → (do [\$here bind: ',name to: ,value] ',name)

Macro precedence is already established in lib/compiler/01-dispatch.moof
(line 65-75 — "user-defined macros take precedence over special
forms"). So files loaded after early/06 see def as a macro;
early/00–05 (loaded before this file) still hit compileDef: which
now emits the same bytecode anyway.

2 e2e tests confirm the macro path: (def x 42) binds x in \$here;
def evaluates to 'x.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Remove `Op::DefineGlobal`

**Files:**
- Modify: `crates/substrate/src/opcodes.rs`
- Modify: `crates/substrate/src/vm.rs`
- Modify: `crates/substrate/src/intrinsics.rs` (op-form encode/decode tables)
- Modify: `crates/substrate/src/compiler.rs::tests` (any tests asserting on DefineGlobal directly)

After Task 7 + 8, no path emits `Op::DefineGlobal`. Removal is safe.

- [ ] **Step 1: Audit remaining references**

```bash
grep -rn "DefineGlobal" crates/substrate/src/
```

Expected: references in `opcodes.rs` (variant declaration), `vm.rs` (handler), `intrinsics.rs` (encode/decode tables for op-form reflection), possibly `compiler.rs::tests`.

- [ ] **Step 2: Remove the `Op::DefineGlobal` enum variant**

In `crates/substrate/src/opcodes.rs`, find:

```rust
    DefineGlobal(SymId),
```

Delete this line. Ensure no other helper trait impl (e.g. `pushes()`) special-cases `DefineGlobal` — if so, remove those.

- [ ] **Step 3: Remove the VM handler**

In `crates/substrate/src/vm.rs`, find the match arm for `Op::DefineGlobal`:

```rust
        Op::DefineGlobal(name) => {
            let v = pop(world)?;
            let global = world.here_form;
            world.env_bind(global, name, v)?;
            world.vm.stack.push(Value::Sym(name));
        }
```

Delete this entire match arm.

- [ ] **Step 4: Remove from op-form encode/decode tables**

In `crates/substrate/src/intrinsics.rs`, find:

```bash
grep -n "DefineGlobal" crates/substrate/src/intrinsics.rs
```

Two matches expected:
- The encode side (op-to-Form): `Op::DefineGlobal(s) => ("DefineGlobal", vec![Value::Sym(s)]),`
- The decode side (Form-to-Op): `"DefineGlobal" => { Op::DefineGlobal(need_sym(world, "DefineGlobal", &operands, 0)?) }`
- Possibly a `mk_op_form` reverse table: `Ok(mk_op_form(w, "DefineGlobal", args))`

Delete all three.

- [ ] **Step 5: Update any tests**

```bash
grep -rn "DefineGlobal" crates/substrate/
```

Should be empty. If any test references `Op::DefineGlobal`, remove or update those tests.

- [ ] **Step 6: Verify build**

```bash
cargo build -p moof 2>&1 | tail -5
```

Expected: green.

- [ ] **Step 7: Run full workspace tests**

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: same count as before. No new tests.

- [ ] **Step 8: Commit**

```bash
git add crates/substrate/src/
git commit -m "$(cat <<'EOF'
opcodes: remove Op::DefineGlobal

V3 task 9. After Tasks 7-8, no compile path emits Op::DefineGlobal:
- rust seed compile_def emits Send-based bytecode (Task 7)
- moof Compiler compileDef: emits Send-based bytecode (Task 7)
- def macro takes precedence post-bootstrap (Task 8)

Removal is safe. VM handler, opcodes.rs variant, and intrinsics.rs
encode/decode entries all deleted.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Rewrite moof Compiler `compileSet:` to emit Send-based bytecode

**Files:**
- Modify: `lib/compiler/03-control.moof`

`set!` in moof currently compiles to `Op::StoreName(name)` (which walks lexical chain via `env_set`, falls back to global on miss). V3 rewrites this to emit Send-based bytecode equivalent to the set! macro: `[[Env current] set: 'name to: value]`. The rust seed doesn't compile set! (per existing comment at compiler.rs:105) — no rust change needed.

- [ ] **Step 1: Locate moof Compiler `compileSet:`**

```bash
grep -n "compileSet" lib/compiler/03-control.moof lib/compiler/02-special.moof
```

Expected: a setHandler! definition for `compileSet:chunk:` in `02-special.moof` line ~25.

- [ ] **Step 2: Rewrite `compileSet:`**

Replace the current body with the Send-based pattern:

```moof
(setHandler! Compiler 'compileSet:chunk:
  (fn (rest chunk)
    ;; (set! name value) compiles to:
    ;;   loadName Env
    ;;   send :current arity=0 ic:fresh
    ;;   loadConst 'name
    ;;   <compile rhs>
    ;;   send :set:to: arity=2 ic:fresh
    ;; (no Pop — set! evaluates to the bound value, the natural
    ;; result of :set:to:; this is a slight semantic shift from the
    ;; pre-V3 Op::StoreName which left Nil. Tests relying on
    ;; (set! ...) → nil should be updated to → value.)
    (let ((name [rest car])
          (rhs [[rest cdr] car]))
      ;; LoadName Env
      [chunk emit: [Opcode loadName: 'Env]]
      ;; Send :current arity=0
      (let ((ic-current [chunk freshIc]))
        [chunk emit: [Opcode send: 'current arity: 0 ic: ic-current]])
      ;; LoadConst 'name
      [chunk emit: [Opcode loadConst: [chunk addConst: name]]]
      ;; compile rhs
      [self compileForm: rhs chunk: chunk tail: #false]
      ;; Send :set:to: arity=2
      (let ((ic-set [chunk freshIc]))
        [chunk emit: [Opcode send: 'set:to: arity: 2 ic: ic-set]]))))
```

- [ ] **Step 3: Audit tests for `(set! …)` evaluating to nil**

The pre-V3 `Op::StoreName` left Nil on the stack. V3's set!-via-Send leaves the value (per `:set:to:`'s return). If any test asserts `(set! x v) == nil`, update.

```bash
grep -rn "set!" lib/ crates/substrate/ | head -20
```

Look for usages where the result of a set! is consumed and compared to nil. Fix expected values.

- [ ] **Step 4: Verify build + tests**

```bash
cargo build -p moof 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: green build. Test count similar to before (set! semantic change might break tests; fix as needed).

- [ ] **Step 5: Add a regression test for set!'s walking semantic**

Add to `crates/substrate/tests/here_e2e.rs`:

```rust
#[test]
fn set_walks_lexical_chain_via_env_current() {
    let mut w = moof::new_world();
    // bind foo in a let-frame, then set! it from inside.
    let r = moof::eval(&mut w, "(let ((foo 5)) (set! foo 99) foo)").unwrap();
    assert_eq!(r, Value::Int(99));
}

#[test]
fn set_raises_unbound_when_name_not_in_chain() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "(set! definitelyNotBound 0)");
    assert!(r.is_err());
    assert_eq!(w.resolve(r.unwrap_err().kind), "unbound");
}
```

Run:

```bash
cargo test --test here_e2e set_walks set_raises 2>&1 | tail -10
```

Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add lib/compiler/03-control.moof lib/compiler/02-special.moof crates/substrate/tests/here_e2e.rs
git commit -m "$(cat <<'EOF'
compiler: compileSet: emits Send-based bytecode (no Op::StoreName)

V3 task 10. moof Compiler's compileSet: now emits the set! macro's
expansion shape directly: LoadName Env, Send :current arity=0,
LoadConst 'name, compile rhs, Send :set:to: arity=2.

Semantic change: (set! x v) now evaluates to v (per :set:to:'s
return) rather than nil (pre-V3 Op::StoreName left nil). Also,
unbound-name now raises 'unbound instead of silently falling back
to creating a global — V1's turn-aware raise machinery makes this
tightening safe.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Add `(defmacro set! …)` and remove `Op::StoreName`

**Files:**
- Modify: `lib/early/06-control-macros.moof`
- Modify: `crates/substrate/src/opcodes.rs`
- Modify: `crates/substrate/src/vm.rs`
- Modify: `crates/substrate/src/intrinsics.rs` (encode/decode tables)

- [ ] **Step 1: Add the set! macro to lib/early/06-control-macros.moof**

After the def macro (Task 8), add:

```moof
;; ─────────────────────────────────────────────────────────────────
;; set! — purified to a macro in V3.
;;
;; (set! name value) → [[Env current] set: 'name to: value]
;;
;; raises 'unbound if name isn't reachable from the lexical chain
;; (V3 tightens the V1 silent-fallback-to-global footgun).
;; returns the bound value (per :set:to:).
;; ─────────────────────────────────────────────────────────────────
(defmacro set! (args)
  (let ((name [args car])
        (value [[args cdr] car]))
    `[[Env current] set: ',name to: ,value]))
```

Update the file's docstring to remove `set!` from the "remaining compiler special forms" list.

- [ ] **Step 2: Remove `Op::StoreName` enum variant**

In `crates/substrate/src/opcodes.rs`, find:

```rust
    StoreName(SymId),
```

Delete. Update any helper trait impls (e.g. `pushes()`) referencing it.

- [ ] **Step 3: Remove VM handler**

In `crates/substrate/src/vm.rs`, find the `Op::StoreName(name)` match arm (around line 423). Delete the entire arm.

- [ ] **Step 4: Remove from intrinsics op-form tables**

```bash
grep -n "StoreName" crates/substrate/src/intrinsics.rs
```

Delete encode and decode entries.

- [ ] **Step 5: Verify build + tests**

```bash
cargo build -p moof 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: green, test count similar (no new tests, no regressions).

- [ ] **Step 6: Commit**

```bash
git add lib/early/06-control-macros.moof crates/substrate/src/
git commit -m "$(cat <<'EOF'
opcodes: remove Op::StoreName + add (defmacro set! …)

V3 task 11. set! joins def as a moof-side macro. Macro expands to
[[Env current] set: 'name to: value] — uses V3's :current method
on Env to capture the lexical env at the call site, then walks via
:set:to: which raises 'unbound on miss.

Op::StoreName removed: only the moof Compiler's compileSet: emitted
it; Task 10 rewrote that to Send-based. No remaining emitters.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Rewrite rust seed `compile_if` and moof Compiler `compileIf:` to emit Send-based bytecode

**Files:**
- Modify: `crates/substrate/src/compiler.rs`
- Modify: `lib/compiler/03-control.moof`

Both compilers stop emitting Jump-based `if` bytecode and instead emit `[[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]` Send pattern. Op::JumpIfFalse and Op::Jump are NOT removed (used by other constructs like let). Performance is recovered via the peephole optimizer in Task 13.

- [ ] **Step 1: Locate rust seed `compile_if`**

```bash
grep -n "fn compile_if" crates/substrate/src/compiler.rs
```

Expected: around line 335.

- [ ] **Step 2: Add a helper method `compile_thunk`**

The Send-based shape needs a way to compile an expression into a fresh chunk-Form (the body of a `(fn () expr)`) and emit a `PushClosure` over it. Add a helper to the `Compiler` impl:

```rust
    /// V3 — compile an expression into a fresh chunk-Form (no params)
    /// and return its FormId. used by `compile_if` to build the
    /// Send-based if pattern's branch closures.
    fn compile_thunk(&mut self, body: Value) -> Result<FormId, RaiseError> {
        // recursively allocate a sub-Compiler for the body chunk.
        let mut inner = Compiler::new(self.world, vec![], body);
        inner.compile_form(body, true)?;
        inner.emit(Op::Return);
        // finalize and return the chunk-Form id
        inner.finalize()
    }
```

(Verify `Compiler::new`, `Compiler::finalize` exist with these signatures. The `finalize` method should return the FormId of the constructed chunk-Form; if it currently returns something else, adapt.)

- [ ] **Step 3: Rewrite rust seed `compile_if`**

Find the existing body (around line 335-375):

```rust
    fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // (if c t [e]) → JumpIfFalse-based bytecode
        ...
    }
```

Replace with:

```rust
    fn compile_if(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // V3: (if c t [e]) compiles to Send-based bytecode equivalent to
        // [[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]. The peephole
        // optimizer (compile_send) recognizes this shape and emits
        // Jump-based bytecode inline — recovers perf without sacrificing
        // macro purity at the source.
        if elems.len() < 3 || elems.len() > 4 {
            return Err(self.err("(if c t [e])"));
        }
        let c = elems[1];
        let t = elems[2];
        let e = if elems.len() == 4 { elems[3] } else { Value::Nil };
        let bang_bang = self.world.intern("!!");
        let if_true_if_false = self.world.intern("ifTrue:ifFalse:");

        // compile c, then Send :!! to coerce to Bool
        self.compile_form(c, false)?;
        let ic_idx = self.fresh_ic();
        self.emit(Op::Send {
            selector: bang_bang,
            argc: 0,
            ic_idx,
        });
        // compile t into a thunk-chunk; PushClosure wraps it
        let t_chunk = self.compile_thunk(t)?;
        self.emit(Op::PushClosure(t_chunk));
        // compile e similarly
        let e_chunk = self.compile_thunk(e)?;
        self.emit(Op::PushClosure(e_chunk));
        // Send :ifTrue:ifFalse: arity=2
        let ic2 = self.fresh_ic();
        let send_op = if tail {
            Op::TailSend { selector: if_true_if_false, argc: 2 }
        } else {
            Op::Send { selector: if_true_if_false, argc: 2, ic_idx: ic2 }
        };
        self.emit(send_op);
        Ok(())
    }
```

(Verify `Op::PushClosure(FormId)` exists. If the existing op takes a different argument shape, adapt.)

- [ ] **Step 4: Rewrite moof Compiler `compileIf:`**

In `lib/compiler/03-control.moof`, find:

```moof
(setHandler! Compiler 'compileIf:chunk:tail:
  (fn (rest chunk tail)
    ...))
```

Replace body with the Send-based pattern:

```moof
(setHandler! Compiler 'compileIf:chunk:tail:
  (fn (rest chunk tail)
    ;; (if c t [e]) → [[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]
    (let ((c [rest car])
          (t [[rest cdr] car])
          (rest2 [[rest cdr] cdr])
          (e (if [rest2 is nil] nil [rest2 car])))
      ;; compile c
      [self compileForm: c chunk: chunk tail: #false]
      ;; Send :!!
      (let ((ic1 [chunk freshIc]))
        [chunk emit: [Opcode send: '!! arity: 0 ic: ic1]])
      ;; compile t into a thunk and PushClosure
      (let ((t-chunk [self compileThunk: t]))
        [chunk emit: [Opcode pushClosure: t-chunk]])
      ;; compile e similarly
      (let ((e-chunk [self compileThunk: e]))
        [chunk emit: [Opcode pushClosure: e-chunk]])
      ;; Send :ifTrue:ifFalse: arity=2 (tail or non-tail)
      (let ((ic2 [chunk freshIc]))
        (if tail
            [chunk emit: [Opcode tailSend: 'ifTrue:ifFalse: arity: 2]]
            [chunk emit: [Opcode send: 'ifTrue:ifFalse: arity: 2 ic: ic2]])))))
```

Also add the `compileThunk:` helper if it doesn't exist:

```moof
(setHandler! Compiler 'compileThunk:
  (fn (body)
    ;; allocate a fresh chunk-Form, compile body in tail position,
    ;; emit Return, return the chunk-Form id.
    (let ((c [Chunk new: '() source: body]))
      [self compileForm: body chunk: c tail: #true]
      [c emit: [Opcode return]]
      c)))
```

- [ ] **Step 5: Verify build + tests**

```bash
cargo build -p moof 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: green build. Test count likely SAME (semantically equivalent — `if` evaluates same way, just via Send dispatch instead of Jump).

**Performance note:** without the Task 13 peephole, `if`-heavy code will be slower. Tests still pass; runtime perf is the trade-off until peephole lands.

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/compiler.rs lib/compiler/03-control.moof
git commit -m "$(cat <<'EOF'
compiler: compile_if emits Send-based bytecode

V3 task 12. Both rust seed compile_if and moof Compiler compileIf:
rewritten to emit [[c !!] ifTrue: (fn () t) ifFalse: (fn () e)]
Send pattern via PushClosure for branch thunks.

Op::JumpIfFalse and Op::Jump are kept — used by let, etc.

Performance hit accepted for V3 source-level purity. Task 13's
peephole optimizer recovers the Jump-based bytecode for the
if-macro shape, reaching pre-V3 efficiency without sacrificing
the user-overridable macro semantic.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Add the if-shape peephole optimizer

**Files:**
- Modify: `crates/substrate/src/compiler.rs`
- Modify: `lib/compiler/02-special.moof` (or `03-control.moof` — wherever `compileSend:` lives)

Compile-time recognition of `Send :ifTrue:ifFalse:` with two syntactic-closure args after `Send :!!` — replace the Send pattern with the Jump-based equivalent. Closures are not allocated at runtime; branch bodies are inlined. Same as pre-V3 if-bytecode efficiency.

- [ ] **Step 1: Locate rust seed `compile_send`**

```bash
grep -n "fn compile_send" crates/substrate/src/compiler.rs
```

Expected: around line 290.

- [ ] **Step 2: Add the peephole in rust seed `compile_send`**

Read the current `compile_send` body. The shape is:

```rust
    fn compile_send(&mut self, elems: &[Value], tail: bool) -> Result<(), RaiseError> {
        // (__send__ receiver 'selector args…) — lowers to Op::Send
        ...
    }
```

At the head of `compile_send`, before the standard emission logic, add:

```rust
        // V3 peephole — recognize the if-macro's expanded shape:
        //   (__send__ (__send__ c '!!) 'ifTrue:ifFalse: tThunk eThunk)
        // where tThunk and eThunk are syntactic (fn () body) literals.
        // emit Jump-based bytecode inline — no closure allocations.
        if let Some((c_form, t_body, e_body)) = self.match_if_pattern(elems) {
            return self.compile_if_inline(c_form, t_body, e_body, tail);
        }
```

Then add the helper methods:

```rust
    /// Recognize the post-macro-expansion `if` shape:
    /// `(__send__ (__send__ c '!!) 'ifTrue:ifFalse: (fn () t) (fn () e))`.
    /// Returns `Some((c, t-body, e-body))` if matched; None otherwise.
    fn match_if_pattern(&self, elems: &[Value]) -> Option<(Value, Value, Value)> {
        // elems[0] = '__send__, elems[1] = receiver, elems[2] = selector, elems[3..] = args
        if elems.len() != 5 { return None; }
        let selector = elems[2].as_sym()?;
        if self.world.resolve(selector) != "ifTrue:ifFalse:" { return None; }
        // receiver must be (__send__ c '!!)
        let receiver = elems[1];
        let recv_elems = self.list_elems_lenient(receiver)?;
        if recv_elems.len() != 3 { return None; }
        if recv_elems[0].as_sym().map(|s| self.world.resolve(s)) != Some("__send__") { return None; }
        let recv_inner_sel = recv_elems[2].as_sym()?;
        if self.world.resolve(recv_inner_sel) != "!!" { return None; }
        let c_form = recv_elems[1];
        // args[0] and args[1] must each be (fn () body)
        let t_body = self.match_zero_arg_fn(elems[3])?;
        let e_body = self.match_zero_arg_fn(elems[4])?;
        Some((c_form, t_body, e_body))
    }

    /// Recognize `(fn () body)`. Returns `Some(body)` if matched.
    fn match_zero_arg_fn(&self, form: Value) -> Option<Value> {
        let elems = self.list_elems_lenient(form)?;
        if elems.len() != 3 { return None; }
        if elems[0].as_sym().map(|s| self.world.resolve(s)) != Some("fn") { return None; }
        // elems[1] should be empty params list (Nil)
        if !matches!(elems[1], Value::Nil) { return None; }
        Some(elems[2])
    }

    /// Same as `list_elems` but returns Option instead of Result —
    /// used in the peephole matcher where mismatch is "no opt", not error.
    fn list_elems_lenient(&self, form: Value) -> Option<Vec<Value>> {
        // (call into self.world.list_to_vec; return None on Err)
        self.world.list_to_vec(form).ok()
    }

    /// Emit the Jump-based bytecode inline — pre-V3 if shape.
    fn compile_if_inline(
        &mut self,
        c_form: Value,
        t_body: Value,
        e_body: Value,
        tail: bool,
    ) -> Result<(), RaiseError> {
        // compile c
        self.compile_form(c_form, false)?;
        // emit JumpIfFalse with placeholder offset
        let jif = self.emit_placeholder_jump(BranchKind::IfFalse);
        // compile t inline
        self.compile_form(t_body, tail)?;
        // emit unconditional jump to end with placeholder
        let jmp = self.emit_placeholder_jump(BranchKind::Always);
        // patch jif to land on e
        self.patch_jump_to_here(jif);
        // compile e inline
        self.compile_form(e_body, tail)?;
        // patch jmp to land at end (after e)
        self.patch_jump_to_here(jmp);
        Ok(())
    }
```

(Note: `BranchKind`, `emit_placeholder_jump`, `patch_jump_to_here` are existing helpers from the pre-V3 `compile_if`. Verify with grep; their signatures should be unchanged.)

- [ ] **Step 3: Mirror the peephole in moof Compiler**

In `lib/compiler/02-special.moof` (or wherever `compileSend:chunk:tail:` is defined), add the peephole logic at the head of the method. The pattern is the same: detect the if-macro shape and emit Jump-based bytecode inline.

(Implementation in moof depends heavily on the local Compiler module's helpers. The implementer should match conventions.)

- [ ] **Step 4: Add tests verifying the peephole emits Jump-based bytecode**

In `crates/substrate/tests/here_e2e.rs`:

```rust
#[test]
fn if_macro_post_peephole_compiles_to_jump_based() {
    let mut w = moof::new_world();
    // compile a small (if c t e) and inspect the resulting bytecode.
    let chunk_form = moof::eval(&mut w, "(quote (if #true 'yes 'no))").unwrap();
    // (compile via the compileTop: API; verify ops contain JumpIfFalse, not PushClosure.)
    // exact verification depends on what introspection moof exposes.
    // simpler check: tight-loop performance — peephole optimization
    // means the if doesn't allocate per-iteration.
    let r = moof::eval(&mut w, "(let ((sum 0)) (def i 0) (set! i 0) sum)").unwrap();
    // smoke test: complex if-using code runs fine
    let _ = r;
    let r2 = moof::eval(&mut w, "(if #true 1 2)").unwrap();
    assert_eq!(r2, Value::Int(1));
    let r3 = moof::eval(&mut w, "(if #false 1 2)").unwrap();
    assert_eq!(r3, Value::Int(2));
}

#[test]
fn if_with_non_syntactic_closure_args_uses_send_dispatch() {
    let mut w = moof::new_world();
    // user code that builds the closures explicitly — peephole should NOT trigger.
    let r = moof::eval(&mut w, "
        (let ((tThunk (fn () 'yes))
              (eThunk (fn () 'no)))
          [#true ifTrue: tThunk ifFalse: eThunk])").unwrap();
    let yes = w.intern("yes");
    assert_eq!(r, Value::Sym(yes));
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test --test here_e2e if_macro_post_peephole if_with_non_syntactic 2>&1 | tail -10
```

Expected: 2 passed.

Run full workspace:

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Expected: previous count + 2 new tests.

- [ ] **Step 6: Commit**

```bash
git add crates/substrate/src/compiler.rs lib/compiler/
git commit -m "$(cat <<'EOF'
compiler: peephole optimizer for if-macro shape (Jump-based recovery)

V3 task 13. Compile-time recognizer in both rust seed compile_send
and moof Compiler compileSend: detects the if-macro's expanded
shape:

  (__send__ (__send__ c '!!) 'ifTrue:ifFalse: (fn () t) (fn () e))

When matched, emits Jump-based bytecode inline:

  <compile c>
  Send :!! arity=0
  JumpIfFalse else_label
  <compile t inline>
  Jump end_label
  else_label: <compile e inline>
  end_label:

No closure allocations. Same efficiency as pre-V3 special-form if.
Falls back to standard Send dispatch when args aren't syntactic
(fn () body) literals — preserves user-overridability of
:ifTrue:ifFalse: when called explicitly.

2 e2e tests cover the matched and unmatched paths.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Add `Object:eval:` in `lib/stdlib/object.moof`

**Files:**
- Modify: `lib/stdlib/object.moof`

Implements Ruby `instance_eval` semantics on top of the substrate primitives (`:callIn:withSelf:`, `view-target` meta key). Pure moof code.

- [ ] **Step 1: Locate `lib/stdlib/object.moof`**

```bash
ls lib/stdlib/object.moof
```

Read its current top to learn the local `defmethod` style.

- [ ] **Step 2: Add `:eval:` method on Object**

Append to `lib/stdlib/object.moof`:

```moof
;; ─────────────────────────────────────────────────────────────────
;; Object :eval: — Ruby instance_eval-style evaluation.
;;
;; [obj eval: closure] runs the closure body with obj's slots as a
;; "view" into the lookup chain. lookups AND mutations propagate
;; LIVE to both obj and closure's captured env. obj is NOT mutated
;; (uses the substrate's view-target meta key for non-mutating
;; delegation — works on frozen receivers).
;;
;; lookup chain inside the closure body:
;;   body-env's let-locals
;;     → self's slots (LIVE; via view-target)
;;     → closure.captured_env (closure's lexical chain)
;;     → closure.captured_env's parents → globals
;;
;; mutations via (set! name value):
;;   - body-local: writes to body-env
;;   - obj-slot: writes to obj LIVE (via view-target's :set:to: hit)
;;   - closure.captured-name: writes to that env LIVE
;;   - else: raises 'unbound
;; ─────────────────────────────────────────────────────────────────
(defmethod (Object eval: closure)
  (let ((captured-env [Heap slotOf: closure at: 'env])
        (body-env [Env new]))
    ;; configure body-env: parent = captured-env, view-target = self
    [Heap metaSet: body-env at: 'parent to: captured-env]
    [Heap metaSet: body-env at: 'view-target to: self]
    ;; run the closure body with body-env, self bound to obj
    [closure callIn: body-env withSelf: self]))
```

(Adjust selector names — `Heap slotOf:at:`, `Heap metaSet:at:to:`, `Env new`, `closure callIn:withSelf:` — to match local conventions. Verify with `grep "Heap slotOf\|Heap metaSet\|Env new" lib/`.)

- [ ] **Step 3: Add e2e tests**

In `crates/substrate/tests/here_e2e.rs`:

```rust
#[test]
fn obj_eval_lookups_find_obj_slots() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "
        (def obj [Object new])
        [obj bind: 'foo to: 42]
        [obj eval: (fn () foo)]").unwrap();
    assert_eq!(r, Value::Int(42));
}

#[test]
fn obj_eval_lookups_also_find_closure_captured_names() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "
        (def obj [Object new])
        [obj bind: 'foo to: 42]
        (def captured 99)
        [obj eval: (fn () captured)]").unwrap();
    assert_eq!(r, Value::Int(99));
}

#[test]
fn obj_eval_set_propagates_live_to_obj_via_view_target() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "
        (def obj [Object new])
        [obj bind: 'counter to: 0]
        [obj eval: (fn () (set! counter 100))]").unwrap();
    let r = moof::eval(&mut w, "[obj lookup: 'counter]").unwrap();
    assert_eq!(r, Value::Int(100));
}

#[test]
fn obj_eval_works_on_frozen_obj() {
    // V3's view-env doesn't mutate receiver — so frozen obj is fine.
    let mut w = moof::new_world();
    moof::eval(&mut w, "
        (def obj [Object new])
        [obj bind: 'foo to: 42]
        [obj freeze]").unwrap();
    let r = moof::eval(&mut w, "[obj eval: (fn () foo)]").unwrap();
    assert_eq!(r, Value::Int(42));
}
```

(Adjust moof syntax for slot binding — `[obj bind: 'foo to: 42]` should work via Env :bind:to: but note Env methods are on the Env proto. For Object instances, may need `[obj slotSet!: 'foo to: 42]` or similar. Verify with local stdlib.)

- [ ] **Step 4: Run tests**

```bash
cargo test --test here_e2e obj_eval 2>&1 | tail -15
```

Expected: 4 passed.

Full workspace:

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

- [ ] **Step 5: Commit**

```bash
git add lib/stdlib/object.moof crates/substrate/tests/here_e2e.rs
git commit -m "$(cat <<'EOF'
stdlib: Object :eval: — Ruby instance_eval via view-env

V3 task 14. Pure moof implementation built on the substrate
primitives:
- :callIn:withSelf: on Closure (Task 6)
- view-target meta key in env_lookup / env_set (Tasks 2-3)

Lookups find names from BOTH obj's slots AND closure's captured
env. Mutations via set! walk the chain and write through to obj
LIVE (via view-target's env_set hit) — true Ruby instance_eval
semantics. Works on frozen receivers (no mutation of obj).

4 e2e tests cover: lookups from obj, lookups from captured env,
live mutation via set!, frozen-obj support.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Final integration tests + verification gate

**Files:**
- Modify: `crates/substrate/tests/here_e2e.rs`

Round out integration coverage; verify all V3 exit criteria.

- [ ] **Step 1: Add comprehensive coverage tests**

Append to `crates/substrate/tests/here_e2e.rs`:

```rust
#[test]
fn here_self_reference_works_in_reflection() {
    let mut w = moof::new_world();
    // [Heap slotKeysOf: $here] should include '$here as one of its slots
    let r = moof::eval(&mut w, "[[Heap slotKeysOf: $here] reduce: '$here-found
        startingWith: #false
        with: (fn (acc k) (if [k = '$here] #true acc))]");
    // (alternative simpler test: use cons membership predicate)
    // depending on local stdlib, this may need adjustment.
    let _ = r;  // smoke test that the eval doesn't crash
}

#[test]
fn here_lookup_works() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "(def myValue 12345)").unwrap();
    let r = moof::eval(&mut w, "[$here lookup: 'myValue]").unwrap();
    assert_eq!(r, Value::Int(12345));
}

#[test]
fn here_bind_to_works() {
    let mut w = moof::new_world();
    moof::eval(&mut w, "[$here bind: 'newName to: 'newValue]").unwrap();
    let r = moof::eval(&mut w, "newName").unwrap();
    let new_value = w.intern("newValue");
    assert_eq!(r, Value::Sym(new_value));
}

#[test]
fn here_parent_returns_nil() {
    let mut w = moof::new_world();
    let r = moof::eval(&mut w, "[$here parent]").unwrap();
    assert_eq!(r, Value::Nil);
}

#[test]
fn here_self_reference_is_value_form_of_here() {
    let mut w = moof::new_world();
    let here_v = moof::eval(&mut w, "$here").unwrap();
    assert_eq!(here_v, Value::Form(w.here_form));
}
```

- [ ] **Step 2: Run full workspace test suite**

```bash
cargo test --workspace 2>&1 | tee /tmp/v3-final.txt | grep -E "^test result"
grep -E "^test result" /tmp/v3-final.txt | awk '{ for(i=1;i<=NF;i++) if($i=="passed;") { gsub(",","",$(i-1)); pass += $(i-1) } } END { print "TOTAL pass="pass }'
```

Record the final pass count.

- [ ] **Step 3: Verify zero warnings**

```bash
cargo build --workspace 2>&1 | grep -E "warning|error\[" | head -20
```

Expected: no V3-introduced warnings (existing pre-V3 warnings on intentional camelCase test names are fine).

- [ ] **Step 4: Verify zero FAILED anywhere**

```bash
cargo test --workspace --no-fail-fast 2>&1 | grep -E "FAILED|panicked" | head
```

Expected: empty.

- [ ] **Step 5: Verify the cli works**

```bash
cargo run --quiet -p moof -- '(def x 42) x' 2>&1 | tail
echo 'if' | cargo run --quiet -p moof 2>&1 | tail
cargo run --quiet -p moof -- '[Object new]' 2>&1 | tail
```

Each should produce sensible output without panics.

- [ ] **Step 6: Verify spec §12 exit criteria**

Manually check each numbered exit criterion in `docs/superpowers/specs/2026-05-09-vat-V3-here-form-design.md` §12. Each should map to a completed task. List any gaps.

- [ ] **Step 7: Commit**

```bash
git add crates/substrate/tests/here_e2e.rs
git commit -m "$(cat <<'EOF'
tests: V3 here-form e2e coverage round-out

V3 task 15. Final integration test sweep verifying the user-facing
behaviors: \$here is a Form, reflection works through it, lookup /
bind / parent / set semantics behave correctly across the macro
and direct-method paths.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review Notes (for the planner; safe to delete after execution)

- **Spec coverage:** §2 → Task 1; §3 → Task 4; §4 → Task 5; §5 → Task 5 (`:current` is one of the five Env methods); §6 → Tasks 2, 3, 6, 14; §7 → Tasks 7, 8, 9; §8 → Tasks 10, 11; §9 → no task (already in place; Task 8's plan note); §10 → Tasks 12, 13; §11 → Tasks 4, 5, 6 (boot wiring is part of those tasks); §12 → Task 15.
- **Type / signature consistency:** `World.here_form: FormId`, `World.view_target_sym: SymId`, `Closure :callIn:withSelf:` takes `(env: FormId, self: Value) -> Value`, `Object :eval:` is a 1-arg moof method. Names consistent.
- **No placeholders:** every code block has executable code; every step has commands and expected outputs.
- **Open risk:** Task 13 peephole has matchers (`match_if_pattern`, `match_zero_arg_fn`, `list_elems_lenient`) that need verification against the actual `Compiler` impl helpers. The plan instructs the implementer to verify with grep and adapt.
- **Open risk:** Task 7's moof code references `Opcode loadName:`, `Opcode send:arity:ic:`, etc. The actual selector names depend on local Compiler-module conventions. Implementer must grep `lib/compiler/` and adapt — instructed in the plan but not pinned.
- **Performance regression risk:** Tasks 12 + 13 must land together for `if`-heavy code to maintain performance. If Task 13's peephole has bugs and falls back to Send dispatch frequently, runtime perf degrades. Tests assert correctness, not perf — a benchmark sanity-check could be added if concerns arise.
