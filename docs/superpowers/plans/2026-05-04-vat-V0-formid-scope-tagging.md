# Vat phase V0 — FormId scope-tagging implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a 2-bit scope tag in the top bits of `FormId` (vat-local / shared-segment / far-ref / reserved) without changing any runtime behavior. After this lands, `FormId` is scope-aware; later phases (V1 nursery, V5 references, V6 shared segment) light up the non-vat-local scopes.

**Architecture:** A `FormId` is still 32 bits, still `Copy + Eq + Hash`, still constructed as `FormId(u32)`. The top 2 bits classify scope; the bottom 30 bits are the payload (per-scope index). Vat-local payload 0 remains the sentinel (`FormId::NONE`), so `Heap`'s placeholder slot semantics are unchanged. Constructors (`FormId::vat_local`, `FormId::shared`, `FormId::far_ref`) wrap payloads with the right tag bits. `Heap::get` / `Heap::get_mut` tag-dispatch: vat-local goes through the existing `Vec<Form>` lookup; shared and far-ref panic with stub messages (later phases implement).

**Tech Stack:** Rust 2021, no new dependencies. Tests are inline `#[cfg(test)]` modules (the existing convention). All work lives in `crates/substrate/src/form.rs` and `crates/substrate/src/heap.rs`.

**Spec reference:** `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` §5 (the FormId scheme) and §22 (V0 row).

**Series context:** This is the first of ~12 plans (V0 through V11) implementing the vat architecture spec. Each plan ships green and testable on its own. V1 (per-turn nursery) follows.

---

## File Structure

| file | role | change kind |
|---|---|---|
| `crates/substrate/src/form.rs` | `FormId` struct + accessors + `Scope` enum | modified — adds enum, tag constants, accessor methods, and constructors |
| `crates/substrate/src/heap.rs` | `Heap::alloc` / `get` / `get_mut` | modified — alloc wraps with vat-local tag and bumps capacity guard; get/get_mut tag-dispatch |

No other files need to change. `sym.rs` and `foreign.rs` use distinct ID types (`SymId`, `ForeignId`) — they are NOT affected by this refactor. Test files in `value.rs`, `form.rs`, and elsewhere construct `FormId(literal_small_int)` which lands as vat-local-tagged (top bits 0) and continues to work.

---

## Task 1: Add `Scope` enum and tag constants

**Files:**
- Modify: `crates/substrate/src/form.rs`

- [ ] **Step 1: Add a failing test for `Scope` discrimination**

In `crates/substrate/src/form.rs`, inside the existing `#[cfg(test)] mod tests` block (around line 105 onward), add:

```rust
    #[test]
    fn scope_enum_has_four_variants() {
        // The four scopes documented in the spec §5.
        let _vat = Scope::VatLocal;
        let _shared = Scope::Shared;
        let _far = Scope::FarRef;
        let _reserved = Scope::Reserved;
        assert_ne!(Scope::VatLocal, Scope::Shared);
        assert_ne!(Scope::Shared, Scope::FarRef);
        assert_ne!(Scope::FarRef, Scope::Reserved);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p substrate form::tests::scope_enum_has_four_variants`
Expected: compile error — `Scope` not defined.

- [ ] **Step 3: Add the `Scope` enum and tag constants to `form.rs`**

Insert near the top of `crates/substrate/src/form.rs`, just under the existing `use` block (around line 25, before `pub struct FormId`):

```rust
/// the four scopes a `FormId` can address. spec §5.
///
/// the top 2 bits of a 32-bit FormId encode the scope; the bottom 30
/// bits are the per-scope payload. vat-local is the only one with
/// real implementation in V0 — shared and far-ref panic until later
/// phases fill them in (V6 / V5 respectively).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Scope {
    /// `00…` — index into this vat's `Vec<Form>`.
    VatLocal,
    /// `01…` — index into the process-wide shared segment (V6).
    Shared,
    /// `10…` — index into this vat's far-ref table (V5).
    FarRef,
    /// `11…` — reserved for future use (NaN-boxed immediates,
    /// bigint pool, segmented heaps).
    Reserved,
}

/// the bit mask that selects the scope tag in a `FormId`'s u32.
pub const SCOPE_MASK: u32 = 0b11 << 30;
/// the bit mask that selects the payload in a `FormId`'s u32.
pub const PAYLOAD_MASK: u32 = !SCOPE_MASK;
/// the maximum payload value (exclusive). 2^30 ≈ 1.07 billion forms
/// per scope — vastly more than any reasonable vat needs.
pub const MAX_PAYLOAD: u32 = 1 << 30;

const TAG_VAT_LOCAL: u32 = 0b00 << 30;
const TAG_SHARED: u32 = 0b01 << 30;
const TAG_FAR_REF: u32 = 0b10 << 30;
const TAG_RESERVED: u32 = 0b11 << 30;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p substrate form::tests::scope_enum_has_four_variants`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/form.rs
git commit -m "$(cat <<'EOF'
form: add Scope enum + tag bit constants for V0 FormId scoping

introduces the four-scope taxonomy (VatLocal / Shared / FarRef / Reserved)
and the bit-mask constants. no behavior change yet — accessors and
heap dispatch land in subsequent tasks.

spec ref: docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md §5

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `FormId::scope()` and `FormId::payload()` accessors

**Files:**
- Modify: `crates/substrate/src/form.rs`

- [ ] **Step 1: Add failing tests**

In the existing `#[cfg(test)] mod tests` block of `crates/substrate/src/form.rs`, add:

```rust
    #[test]
    fn vat_local_scope_extracted_from_zero_top_bits() {
        // FormId(7) has top bits 00 → vat-local payload 7.
        let id = FormId(7);
        assert_eq!(id.scope(), Scope::VatLocal);
        assert_eq!(id.payload(), 7);
    }

    #[test]
    fn shared_scope_extracted_from_01_top_bits() {
        let id = FormId(0b01 << 30 | 42);
        assert_eq!(id.scope(), Scope::Shared);
        assert_eq!(id.payload(), 42);
    }

    #[test]
    fn far_ref_scope_extracted_from_10_top_bits() {
        let id = FormId(0b10 << 30 | 100);
        assert_eq!(id.scope(), Scope::FarRef);
        assert_eq!(id.payload(), 100);
    }

    #[test]
    fn reserved_scope_extracted_from_11_top_bits() {
        let id = FormId(0b11 << 30 | 1);
        assert_eq!(id.scope(), Scope::Reserved);
        assert_eq!(id.payload(), 1);
    }

    #[test]
    fn formid_none_remains_vat_local_zero() {
        // The sentinel must remain vat-local payload 0 so existing
        // Heap placeholder semantics work unchanged.
        assert_eq!(FormId::NONE.scope(), Scope::VatLocal);
        assert_eq!(FormId::NONE.payload(), 0);
        assert!(FormId::NONE.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate form::tests`
Expected: compile errors — `scope` and `payload` methods not defined on `FormId`.

- [ ] **Step 3: Add the accessor methods**

In `crates/substrate/src/form.rs`, extend the existing `impl FormId` block (around line 36) to add:

```rust
impl FormId {
    pub const NONE: FormId = FormId(0);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// the scope tag — which of the four spaces this id addresses.
    pub fn scope(self) -> Scope {
        match self.0 & SCOPE_MASK {
            TAG_VAT_LOCAL => Scope::VatLocal,
            TAG_SHARED => Scope::Shared,
            TAG_FAR_REF => Scope::FarRef,
            TAG_RESERVED => Scope::Reserved,
            _ => unreachable!("SCOPE_MASK selects exactly 2 bits"),
        }
    }

    /// the payload (per-scope index). bottom 30 bits.
    pub fn payload(self) -> u32 {
        self.0 & PAYLOAD_MASK
    }
}
```

(Replace the existing `impl FormId { ... }` block with the version above. Keep `NONE` and `is_none` exactly as they are; just add the two new methods.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate form::tests`
Expected: all form.rs tests pass, including the five new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/form.rs
git commit -m "$(cat <<'EOF'
form: add FormId::scope() and FormId::payload() accessors

extracts the top-2-bit scope tag and bottom-30-bit payload from a
FormId. FormId::NONE remains vat-local payload 0, preserving Heap
sentinel semantics.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add `FormId::vat_local()` / `shared()` / `far_ref()` constructors

**Files:**
- Modify: `crates/substrate/src/form.rs`

- [ ] **Step 1: Add failing tests**

In the same `#[cfg(test)] mod tests` block of `crates/substrate/src/form.rs`, add:

```rust
    #[test]
    fn vat_local_constructor_zero_top_bits() {
        let id = FormId::vat_local(42);
        assert_eq!(id.scope(), Scope::VatLocal);
        assert_eq!(id.payload(), 42);
        // raw bits: top 2 are 00, bottom 30 are 42
        assert_eq!(id.0, 42);
    }

    #[test]
    fn shared_constructor_sets_01_top_bits() {
        let id = FormId::shared(42);
        assert_eq!(id.scope(), Scope::Shared);
        assert_eq!(id.payload(), 42);
        assert_eq!(id.0, (0b01 << 30) | 42);
    }

    #[test]
    fn far_ref_constructor_sets_10_top_bits() {
        let id = FormId::far_ref(100);
        assert_eq!(id.scope(), Scope::FarRef);
        assert_eq!(id.payload(), 100);
        assert_eq!(id.0, (0b10 << 30) | 100);
    }

    #[test]
    #[should_panic(expected = "payload exceeds 30-bit limit")]
    fn vat_local_constructor_panics_on_overflow() {
        let _ = FormId::vat_local(MAX_PAYLOAD);
    }

    #[test]
    #[should_panic(expected = "payload exceeds 30-bit limit")]
    fn shared_constructor_panics_on_overflow() {
        let _ = FormId::shared(MAX_PAYLOAD);
    }

    #[test]
    #[should_panic(expected = "payload exceeds 30-bit limit")]
    fn far_ref_constructor_panics_on_overflow() {
        let _ = FormId::far_ref(MAX_PAYLOAD);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate form::tests`
Expected: compile errors — `vat_local`, `shared`, `far_ref` not defined.

- [ ] **Step 3: Add the constructors**

Extend the `impl FormId` block in `crates/substrate/src/form.rs` to add (after the existing accessors):

```rust
    /// construct a vat-local FormId. payload must fit in 30 bits.
    pub fn vat_local(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_VAT_LOCAL | payload)
    }

    /// construct a shared-segment FormId. payload must fit in 30 bits.
    pub fn shared(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_SHARED | payload)
    }

    /// construct a far-ref FormId. payload must fit in 30 bits.
    pub fn far_ref(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_FAR_REF | payload)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate form::tests`
Expected: all form.rs tests pass, including the six new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/form.rs
git commit -m "$(cat <<'EOF'
form: add FormId::vat_local / shared / far_ref constructors

scope-tagged constructors with 30-bit payload bounds checks. existing
FormId(literal_small_int) constructions remain valid as vat-local-
tagged ids, so test code is unaffected.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Update `Heap::alloc` to return vat-local-tagged ids with capacity guard

**Files:**
- Modify: `crates/substrate/src/heap.rs`

- [ ] **Step 1: Add failing test for tagged alloc**

In the existing `#[cfg(test)] mod tests` block of `crates/substrate/src/heap.rs` (around line 79), add:

```rust
    #[test]
    fn alloc_returns_vat_local_tagged_ids() {
        use crate::form::Scope;
        let mut h = Heap::new();
        let id = h.alloc(Form::default());
        assert_eq!(id.scope(), Scope::VatLocal);
        // payload starts at 1 (index 0 is the sentinel placeholder)
        assert_eq!(id.payload(), 1);
    }

    #[test]
    fn alloc_payload_increments_with_each_call() {
        use crate::form::Scope;
        let mut h = Heap::new();
        let a = h.alloc(Form::default());
        let b = h.alloc(Form::default());
        let c = h.alloc(Form::default());
        assert_eq!(a.scope(), Scope::VatLocal);
        assert_eq!(b.scope(), Scope::VatLocal);
        assert_eq!(c.scope(), Scope::VatLocal);
        assert_eq!(a.payload(), 1);
        assert_eq!(b.payload(), 2);
        assert_eq!(c.payload(), 3);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate heap::tests::alloc_returns_vat_local_tagged_ids heap::tests::alloc_payload_increments_with_each_call`
Expected: tests pass syntactically (small payloads still parse as vat-local since top bits are 0), so they may *appear* to pass already. That's expected — `FormId(id as u32)` with small `id` already produces vat-local-tagged ids by virtue of the top 2 bits being 0. Run them to confirm baseline.

If they pass without further changes, **good** — Task 4 is then about tightening the capacity guard rather than introducing tagged alloc behavior. Continue to step 3.

- [ ] **Step 3: Update `Heap::alloc` to use the typed constructor and the new bound**

In `crates/substrate/src/heap.rs`, replace the existing `alloc` method (around lines 38–46):

```rust
    pub fn alloc(&mut self, form: Form) -> FormId {
        let id = self.forms.len();
        // `usize` could in principle exceed `u32`. on 64-bit, this
        // is a 4-billion-form ceiling — way more than any real moof
        // workload should reach. enforce it explicitly.
        assert!(id < u32::MAX as usize, "heap exhausted: 4G forms allocated");
        self.forms.push(form);
        FormId(id as u32)
    }
```

with:

```rust
    pub fn alloc(&mut self, form: Form) -> FormId {
        let id = self.forms.len();
        // post-V0 the vat-local payload is 30 bits, so the per-vat
        // ceiling is ~1B forms (vs 4B before). still vastly more
        // than any real moof workload.
        assert!(
            (id as u32) < crate::form::MAX_PAYLOAD,
            "vat heap exhausted: {} forms allocated (limit {})",
            id, crate::form::MAX_PAYLOAD
        );
        self.forms.push(form);
        FormId::vat_local(id as u32)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate heap::tests`
Expected: all heap.rs tests pass, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/heap.rs
git commit -m "$(cat <<'EOF'
heap: alloc returns vat-local-tagged FormIds; bound at 30-bit payload

Heap::alloc now constructs vat-local-scoped ids explicitly via
FormId::vat_local. capacity guard tightened from u32::MAX to
MAX_PAYLOAD (~1B per vat). previously alloc happened to produce
vat-local ids by virtue of the top bits being zero; this change
makes the intent explicit and the bound spec-correct.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Add tag-dispatch to `Heap::get` and `Heap::get_mut` with stub panics for non-vat-local scopes

**Files:**
- Modify: `crates/substrate/src/heap.rs`

- [ ] **Step 1: Add failing tests**

In the `#[cfg(test)] mod tests` block of `crates/substrate/src/heap.rs`, add:

```rust
    #[test]
    #[should_panic(expected = "shared segment not yet supported")]
    fn get_on_shared_id_panics_in_v0() {
        let h = Heap::new();
        let _ = h.get(FormId::shared(1));
    }

    #[test]
    #[should_panic(expected = "far-ref table not yet supported")]
    fn get_on_far_ref_id_panics_in_v0() {
        let h = Heap::new();
        let _ = h.get(FormId::far_ref(1));
    }

    #[test]
    #[should_panic(expected = "shared segment not yet supported")]
    fn get_mut_on_shared_id_panics_in_v0() {
        let mut h = Heap::new();
        let _ = h.get_mut(FormId::shared(1));
    }

    #[test]
    #[should_panic(expected = "far-ref table not yet supported")]
    fn get_mut_on_far_ref_id_panics_in_v0() {
        let mut h = Heap::new();
        let _ = h.get_mut(FormId::far_ref(1));
    }

    #[test]
    fn get_on_vat_local_still_works() {
        let mut h = Heap::new();
        let mut f = Form::default();
        f.slots.insert(SymId(7), Value::Int(42));
        let id = h.alloc(f);
        assert_eq!(h.get(id).slot(SymId(7)), Value::Int(42));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p substrate heap::tests::get_on_shared_id_panics_in_v0 heap::tests::get_on_far_ref_id_panics_in_v0 heap::tests::get_mut_on_shared_id_panics_in_v0 heap::tests::get_mut_on_far_ref_id_panics_in_v0`
Expected: tests fail — current `get`/`get_mut` use `id.0 as usize`, which would index into `forms` with a huge offset and panic with an out-of-bounds error rather than the expected message. (The `get_on_vat_local_still_works` test should pass.)

- [ ] **Step 3: Tag-dispatch in `Heap::get` and `Heap::get_mut`**

In `crates/substrate/src/heap.rs`, replace the existing `get` and `get_mut` methods (around lines 49–59):

```rust
    pub fn get(&self, id: FormId) -> &Form {
        debug_assert!(!id.is_none(), "Heap::get on FormId::NONE");
        &self.forms[id.0 as usize]
    }

    pub fn get_mut(&mut self, id: FormId) -> &mut Form {
        debug_assert!(!id.is_none(), "Heap::get_mut on FormId::NONE");
        &mut self.forms[id.0 as usize]
    }
```

with:

```rust
    pub fn get(&self, id: FormId) -> &Form {
        use crate::form::Scope;
        debug_assert!(!id.is_none(), "Heap::get on FormId::NONE");
        match id.scope() {
            Scope::VatLocal => &self.forms[id.payload() as usize],
            Scope::Shared => panic!(
                "shared segment not yet supported (V6); got id payload {}",
                id.payload()
            ),
            Scope::FarRef => panic!(
                "far-ref table not yet supported (V5); got id payload {}",
                id.payload()
            ),
            Scope::Reserved => panic!(
                "reserved scope: id payload {}",
                id.payload()
            ),
        }
    }

    pub fn get_mut(&mut self, id: FormId) -> &mut Form {
        use crate::form::Scope;
        debug_assert!(!id.is_none(), "Heap::get_mut on FormId::NONE");
        match id.scope() {
            Scope::VatLocal => &mut self.forms[id.payload() as usize],
            Scope::Shared => panic!(
                "shared segment not yet supported (V6); got id payload {}",
                id.payload()
            ),
            Scope::FarRef => panic!(
                "far-ref table not yet supported (V5); got id payload {}",
                id.payload()
            ),
            Scope::Reserved => panic!(
                "reserved scope: id payload {}",
                id.payload()
            ),
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p substrate heap::tests`
Expected: all heap.rs tests pass, including the five new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/substrate/src/heap.rs
git commit -m "$(cat <<'EOF'
heap: tag-dispatch in get/get_mut; non-vat-local scopes panic with stub

V0 lights up scope-aware dispatch on FormId. vat-local scope continues
to index Vec<Form> directly via payload(). shared and far-ref scopes
panic with explicit "not yet supported" messages pointing at V6 / V5.
reserved scope is also handled.

this preserves existing behavior for all current code (which only ever
sees vat-local ids) while making the boundary explicit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Verify the full workspace test suite still passes

**Files:**
- (none modified — this is a verification gate)

- [ ] **Step 1: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | tee /tmp/v0-test-out.txt`
Expected: every test passes. Look for the final `test result:` line.

- [ ] **Step 2: Inspect the output**

Run: `grep -E "^test result|FAILED|error\[" /tmp/v0-test-out.txt`
Expected:
- a `test result: ok.` line for each test binary in the workspace.
- no `FAILED` or `error[` lines.
- the total count should match what was passing before V0 started (NEXT_SESSION reports 368 passing as of the most recent landing, but verify against the actual baseline by running `git stash` + `cargo test --workspace` on a known-good commit if there's any doubt).

- [ ] **Step 3: If anything failed, diagnose**

The only realistic failure mode is a code path that constructs a `FormId(literal_large_u32)` somewhere outside this plan's tracked files. Run:

```bash
grep -rn "FormId(" crates/ --include="*.rs" | grep -v "FormId::" | grep -v "//\|test\|MAX_PAYLOAD" | head -50
```

Audit each hit. Constructions of small literals (≤30 bits, top bits zero) are vat-local-tagged automatically and remain valid. Constructions with bit patterns in the top 2 bits would need updating to use `FormId::shared` or `FormId::far_ref` — but no existing code should be doing this, since shared and far-ref scopes are V0-introduced.

If you find a hit you can't explain, treat it as a bug to investigate before continuing. Otherwise this step passes.

- [ ] **Step 4: Inspect the cargo build output for new warnings**

Run: `cargo build --workspace 2>&1 | grep -E "warning|error" | head -20`
Expected: no new warnings introduced by V0. Pre-existing warnings (if any) are fine.

- [ ] **Step 5: Done — V0 lands**

V0's exit criteria from the spec (§22) are now met:

> - 2-bit scope tag on FormId
> - existing single-heap World keeps everything in `00…` (vat-local) scope
> - all `heap.get(id)` paths gain a tag-dispatch (default-cased to vat-local for now)
> - shared-segment, far-ref-table arena scopes are stubbed (panic on access)
> - exit criteria: 368 tests still pass; FormId is now scope-aware

No final commit needed — Tasks 1–5 each committed independently.

---

## Self-Review Notes (for the planner; safe to delete after execution)

**Spec coverage:** V0's spec section in §22 enumerates four bullet points. Each maps to a task:
- "introduce 2-bit scope tag on FormId" → Tasks 1+2+3 (Scope enum, tag bits, accessors, constructors)
- "existing single-heap World keeps everything in `00…`" → Task 4 (alloc returns vat-local)
- "all `heap.get(id)` paths gain a tag-dispatch" → Task 5 (get/get_mut tag-dispatch)
- "shared-segment, far-ref-table arena scopes are stubbed (panic on access)" → Task 5 (panic stubs)
- "exit criteria: 368 tests still pass" → Task 6 (workspace test verification)

Spec §5 (FormId scheme — the conceptual spec) is fully implemented at the bit level by Tasks 1–3, and the panicking stubs in Task 5 honor "the hot path stays O(1) for all three live scopes" by being a constant-time match on a 2-bit tag.

**Placeholder scan:** No "TBD", "TODO", or vague language. Every code block is complete and copy-pasteable. The audit step (Task 6 step 3) gives a concrete grep command rather than handwaving.

**Type consistency:** `FormId`, `Scope`, `MAX_PAYLOAD`, `SCOPE_MASK`, `PAYLOAD_MASK` are used consistently across tasks. Method signatures `vat_local(u32) -> Self`, `shared(u32) -> Self`, `far_ref(u32) -> Self` are stable across Tasks 3, 4, 5. The `Scope` enum's variants (`VatLocal`, `Shared`, `FarRef`, `Reserved`) appear identically in Task 1 (definition) and Task 5 (match arms).
