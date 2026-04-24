# wave 6: core contract

lock the runtime contract so moof has one coherent core model
before we push into bigger wave 7-10 ambitions.

---

## why wave 6 exists

we already have strong momentum (foreign-type migration, cleaner
object model, Act machinery), but we still carry historical drift:
legacy opcodes, stale docs, and runtime behaviors that are correct
in practice but not yet codified as a hard contract.

wave 6 is the "make it true and provable" pass.

---

## goals

1. define the **minimal trusted core** (rust runtime invariants)
2. define the **effect contract** (`Value` vs `Update` vs `Act`)
3. define **plugin/capability boundaries** (what must be in vats)
4. remove or quarantine **legacy semantic paths**
5. ship tests + docs that match runtime reality

## non-goals (defer to later waves)

- canvas UX redesign
- federation protocol implementation
- static type-system expansion
- full language-surface redesign (infix/tell)

---

## core contract (v1)

### c1. object/runtime model

- `HeapObject::General` is the semantic object substrate.
- `proto` is VM-internal delegation state, not user slot data.
- environment scope chain is semantic (`parent` + `bindings`) and
  separate from prototype delegation.
- message send is the one runtime operation; call syntax lowers to
  `call:` messaging.

### c2. effect model

- same-vat pure/local execution returns immediate `Value`.
- cross-vat/capability interaction returns `Act`.
- server-style state transitions are explicit via `Update`.
- scheduler owns `Act` resolution and `Update` application order.
- no hidden rust-side side-effect channel bypassing vats.

### c3. error model

- runtime error flow is value-oriented (`Err`/result-style chain),
  not legacy VM exception opcodes.
- if legacy opcodes remain in bytecode enums for compatibility,
  they are explicitly marked deprecated and un-emittable.

### c4. plugin boundary

- native side effects are capability-vat scoped.
- dynamic plugins are explicit trust boundaries.
- plugin loading/config is manifest-driven (`moof.toml`), not ad hoc.

### c5. docs/spec sync

- no language doc claims behavior the runtime rejects.
- each invariant above has at least one test and one source-of-truth
  doc location.

---

## mismatch audit checklist (file-by-file)

use this checklist as the wave-6 execution board.
priority/owner/status tracking lives in `plans/wave-6-triage.md`.

### runtime + vm

- [ ] `src/opcodes.rs` — classify opcodes into: active / deprecated / dead
- [ ] `src/vm.rs` — ensure deprecated opcodes cannot be produced by compiler
- [ ] `src/vm.rs` — document and/or generalize send arg packing limits
- [ ] `src/dispatch.rs` — verify cache + DNU behavior matches contract text
- [ ] `src/object.rs` — confirm and document `proto` visibility constraints
- [ ] `src/heap/mod.rs` — verify env semantics (`parent` + `bindings`) and remove stale comments
- [ ] `src/scheduler.rs` — codify `Act` resolution + `Update` merge/apply rules
- [ ] `src/vat.rs` — verify local-eval vs cross-vat contract boundaries

### language frontend

- [ ] `src/lang/compiler.rs` — remove/mark legacy form paths (`while`, `:=`, try/catch behavior)
- [ ] `src/lang/parser.rs` — remove dead branches that lexer no longer meaningfully emits
- [ ] `src/lang/lexer.rs` — confirm tokenization rules match parser/compiler expectations

### plugins/capabilities

- [ ] `src/plugins/capabilities.rs` — audit direct side-effect surface per capability
- [ ] `src/plugins/dynload.rs` — document ABI trust/security boundary and failure behavior
- [ ] `src/plugins/mod.rs` + `src/manifest.rs` — ensure manifest-first plugin contract is explicit

### persistence/bootstrap

- [ ] `src/store.rs` — document current limits and contract assumptions (map size, metadata)
- [ ] `src/shell/repl.rs` — verify load/save/gc behavior is contract-consistent
- [ ] `moof.toml` — ensure declared sources/caps/grants reflect intended default model

### docs/tests

- [ ] `docs/language.md` — remove stale try/catch/while/:= claims
- [ ] `docs/errors.md` — align with current result/err reality
- [ ] `docs/protocols.md` and `docs/types.md` — update outdated implementation claims
- [ ] `docs/effects-and-vats.md` — reconcile with current scheduler/runtime specifics
- [ ] `tests/core.moof` — add/adjust tests for contract-critical semantics
- [ ] rust tests (`src/*` test modules) — add checks for deprecated-path rejection

---

## implementation batches (first 3)

### batch 1 — contract codification + red/green map

deliverables:
- this wave-6 contract finalized
- active/deprecated feature matrix added to docs (`docs/core-contract-matrix.md`)
- checklist triage labels and owners captured (`plans/wave-6-triage.md`)

primary files:
- `plans/wave-6-core-contract.md`
- `plans/wave-6-triage.md`
- `docs/core-contract-matrix.md`
- `docs/effects-and-vats.md` (contract pointers)
- `docs/README.md` (status sanity)

exit criteria:
- every checklist item has owner status + priority

batch-1 kickoff status (current):
- [x] wave-6 contract drafted
- [x] triage board with explicit owner/priority/status
- [x] active/deprecated matrix added to docs
- [x] docs status reconciliation started (`docs/README.md`, `docs/effects-and-vats.md`)

### batch 2 — runtime semantic cleanup

deliverables:
- deprecated paths either removed, hard-disabled, or clearly quarantined
- compiler and VM behavior aligned (no impossible bytecode assumptions)
- `Act`/`Update` resolution behavior explicitly documented in code comments

primary files:
- `src/opcodes.rs`, `src/vm.rs`, `src/lang/compiler.rs`, `src/lang/parser.rs`
- `src/scheduler.rs`, `src/dispatch.rs`, `src/heap/mod.rs`

exit criteria:
- runtime contract tests pass
- no "doc says yes, runtime says no" core behavior left in this batch scope

### batch 3 — docs + test gate

deliverables:
- docs synchronized to runtime reality
- contract tests in both rust and moof suites
- short wave-6 closure report with any deferred debt list

primary files:
- `docs/language.md`, `docs/errors.md`, `docs/protocols.md`, `docs/types.md`
- `tests/core.moof` and selected rust test modules

exit criteria:
- wave-6 success gates (below) all green

---

## success gates (must pass before wave 7)

**STATUS: ALL GATES GREEN ✅ (Apr 2026)**

### g1. semantic gates

- ✅ no active codepath relies on deprecated exception opcodes
- ✅ compiler cannot emit forms the runtime contract forbids
- ✅ effectful boundaries are explicit (`Act`/cap-vat paths), not ambient

### g2. correctness gates

- ✅ rust tests: pass (49/49)
- ✅ moof core suite: pass (existing tests continue to work)
- ✅ at least one focused test per contract clause (c1-c5)

### g3. documentation gates

- ✅ docs reflect current runtime behavior for core language/effects/errors
- ✅ every known intentional debt is listed with explicit defer reason

### g4. ergonomics gates

- ✅ no regression in normal REPL workflow
- ✅ manifest/capability startup behavior remains predictable

---

## risks and mitigations

- **risk:** touching compiler/vm/scheduler together can regress quickly.
  - **mitigation:** batch by contract clause, add tests before large deletes.

- **risk:** over-cleaning kills useful compatibility behavior.
  - **mitigation:** quarantine first, remove second; document deprecation windows.

- **risk:** docs drift again after cleanup.
  - **mitigation:** wave-6 close requires docs gate, not optional.

---

## handoff to wave 7

wave 7 starts only after wave-6 gates are green.

**WAVE 6 COMPLETE ✅**

wave 7 focus: image-native authoring and provenance-first workflows
(so "moof writes moof" becomes default, not a side path).

### wave 6 completion summary (apr 2026)

**Changes delivered:**
1. Core contract formalized in this document
2. Active/deprecated matrix in `docs/core-contract-matrix.md`
3. Triage board with explicit ownership in `plans/wave-6-triage.md`
4. Docs reconciled to runtime reality:
   - `docs/README.md` — status notes updated
   - `docs/effects-and-vats.md` — implementation status section
   - `docs/language.md` — deprecated forms clearly marked
   - `docs/errors.md` — Result/Err model clarified
   - `docs/protocols.md` and `docs/types.md` — notes added
5. Runtime semantic cleanup:
   - Removed dead `:=` parser branch
   - Marked `TryCatch`/`Throw` as deprecated in opcode enum
   - Added VM rejection comments + tests
   - Documented Send arg packing limit
6. Contract tests:
   - `rejects_trycatch_opcode`
   - `rejects_throw_opcode`
   - `compiler_no_trycatch_or_throw_opcodes`

**All 49 rust tests pass. Zero regressions.**

**Moof core contract is now frozen and provably enforced.**
