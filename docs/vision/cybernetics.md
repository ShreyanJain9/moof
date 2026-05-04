# Cybernetics and Living Systems in Moof: Deep Implementation Plan

> **Goal: Embed autopoiesis, actor-model liveness, and CRDT morphogenesis into the substrate via precise, testable phases.**

This document provides the day-by-day technical specification for evolving Moof into a living, distributed system.

## 1. Autopoiesis: Self-Hosting and Live Recompilation

A cybernetic system must be able to rewrite its own rules without restarting. We will implement hot-swappable compiler architecture.

### Phase 1.A: The `$compiler` Capability and `[become:]`
**Files:** `src/intrinsics.rs`, `lib/early/compiler-hot-swap.moof`
**The Protocol:** The compiler is no longer a static module; it is a live Form governed by the `$compiler` cap.
1. **Atomic Swap Logic:** We implement `[become:]` in `src/vm.rs`. `[formA become: formB]` updates the vat's internal indirection table such that `FormId(A)` now points to the memory of `FormId(B)`.
2. **The Moof Swap API:**
   ```moof
   (defproto CompilerUpdater
     (handlers
       [upgradeWith: sourceText]
         ;; 1. Parse and compile the new source in isolation
         (let newAst [$compiler parse: sourceText])
         (let newChunk [$compiler compileForm: newAst])
         (let newCompilerForm [$compiler execute: newChunk])
         ;; 2. Verify it fulfills the protocol
         (if (not [newCompilerForm respondsTo: :compileForm:])
             (raise 'invalid-compiler))
         ;; 3. Atomic swap
         [$compiler become: newCompilerForm]))
   ```
**Tests:** `test_compiler_upgrade_retains_identity`, `test_invalid_compiler_upgrade_fails_gracefully`.

### Phase 1.B: The Agent Optimization Loop (Vat)
**Files:** `lib/stdlib/optimizer-agent.moof`
A supervisor-level actor that runs on an interval.
1. **Introspection Query:** `(let hotMethods [$heap queryMeta: 'ic-misses > 1000])`
2. **Rewrite Logic:**
   ```moof
   (let ast [hotMethod source])
   ;; ... Agent analysis logic (e.g. inline a polymorphic call) ...
   (let newChunk [$compiler compileForm: optimizedAst])
   [hotMethod become: [$compiler execute: newChunk]]
   ```

## 2. Actor Boundaries and Liveness

Vats must be strictly isolated. Cross-vat calls must be fully asynchronous.

### Phase 2.A: The `FarRef` Pipeline and Promises
**Files:** `lib/stdlib/far-ref.moof`, `lib/stdlib/promise.moof`
1. **The FarRef Form:**
   ```moof
   (defproto FarRef
     (slots targetVatId localId)
     (handlers
       [doesNotUnderstand: selector with: args]
         ;; Intercept all sends and construct an Intent
         (let promise [Promise new])
         (let intent {Intent to: self.targetVatId target: self.localId msg: selector args: args replyTo: promise})
         [$transporter enqueue: intent]
         promise))
   ```
2. **The Reflector (`src/reflector.rs`):**
   At the end of the message turn, the Reflector iterates the Vat's outbox.
   ```rust
   for intent in vat.outbox.drain(..) {
       let canonical_bytes = canonical_encode(intent)?;
       // Network boundary
       transport_mco.send(intent.to, canonical_bytes);
   }
   ```
**Tests:** `test_far_ref_dnu_produces_intent`, `test_promise_resolves_on_receipt`.

## 3. CRDTs and Morphogenesis

Shared state across replicated vats must converge deterministically without locks.

### Phase 3.A: `CRDTTable` and the `LWWMap` Protocol
**Files:** `lib/stdlib/crdt.moof`, `lib/protocols/crdt-protocols.moof`
1. **The Protocol:**
   ```moof
   (defprotocol LWWMap
     (requires [at:] [put:withTime:] [merge:])
     (derives
       [put: key value: val]
         [self put: key withTime: [$clock logicalNow] value: val]))
   ```
2. **The CRDTTable Implementation:**
   ```moof
   (defproto CRDTTable
     (mixins LWWMap)
     (slots state) ;; {key -> {value, time}}
     (handlers
       [initialize] (set! self.state {})
       [put: k withTime: t value: v]
         (let existing [self.state at: k])
         (if (or [existing is nil] (> t [existing time]))
             [self.state put: k value: {value: v time: t}])
       [merge: otherTable]
         [otherTable forEach: |k vRecord|
           [self put: k withTime: [vRecord time] value: [vRecord value]]]))
   ```
3. **Integration with Repliaction:** When a VAT receives a state snapshot from another peer, it does not overwrite its `CRDTTable`; it sends `[localTable merge: remoteTable]`.

**Tests:** `test_crdt_table_lww_semantics`, `test_crdt_merge_is_commutative`.
