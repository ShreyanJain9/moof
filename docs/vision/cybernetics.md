# Cybernetics and Living Systems in Moof: Implementation Plan

> "A system is alive if it can regenerate its own components and maintain its boundaries against perturbations." — Francisco Varela, on Autopoiesis.

This document outlines the concrete implementation steps required to embed autopoiesis, actor-model liveness, and CRDT-based morphogenesis into the Moof substrate.

## 1. Autopoiesis: Self-Hosting and Live Recompilation

To be autopoietic, the system must be able to modify its own parsing and compilation machinery at runtime without restarting.

### Phase 1.A: The `compiler.moof` Hot-Swap Protocol
Currently, the compiler is a Moof module. We need a secure protocol to replace it.

1. **The `$compiler` Cap:** The supervisor holds the `$compiler` capability.
2. **Atomic Swap:** Introduce `[$compiler upgradeWith: newCompilerChunk]`. This method:
   - Evaluates the new chunk in a fresh sandbox environment.
   - Verifies the new compiler fulfills the `Compiler` protocol (responds to `:compileTop:`, `:compileForm:`, etc.).
   - Uses `[become:]` (identity indirection) to atomically swap the old `$compiler` instance with the new one. All subsequent `eval` calls route to the new compiler.

### Phase 1.B: The Agent Optimization Loop
We will introduce a background `OptimizerVat`.
- It periodically reads `[v meta 'ic-misses]` from heavily used methods.
- If it detects a pattern (e.g., a polymorphic call site that should be monomorphic), it retrieves the method source via `[v source]`.
- It rewrites the AST, calls `[$compiler compileForm: newAst]`, and uses `[v setHandler: newMethod for: selector]` to patch the live system.

## 2. Actor Boundaries and Liveness

Vats act as cellular boundaries. We must enforce strict asynchronous message passing to ensure one vat cannot crash another.

### Phase 2.A: Strict Intent/Receipt Routing
Currently, cross-vat calls might leak synchronous references.
1. **The `FarRef` Proto:** Define a `FarRef` in Moof: `(defproto FarRef (slots targetVatId localId))`.
2. **Intent Envelope:** When `[farRef msg: arg]` is invoked, it does *not* execute. Instead, it produces an `EffectIntent` Form:
   `{Intent to: targetVatId target: localId msg: 'msg args: (arg)}`
3. **The Reflector:** The Rust substrate (`src/reflector.rs`) sweeps the outbox at the end of the turn, serializes the Intent into Canonical Bytes, and routes it to the target Vat's inbox.
4. **The Promise Pipeline:** `[farRef msg: arg]` immediately returns a `Promise`. The target vat processes the intent and emits a `Receipt` intent back, which resolves the promise.

## 3. CRDTs and Morphogenesis

For Moofpaint to work across replicated vats concurrently, we need Conflict-Free Replicated Data Types natively supported in the Form heap.

### Phase 3.A: The LWW-Map (Last-Write-Wins) Protocol
Instead of generic Tables for shared state, we implement CRDT Tables in Moof.

```moof
(defprotocol LWWMap
  (requires [at:] [put:withTime:] [merge:])
  (derives
    [put: key value: val]
      ;; Automatically uses the turn's logical clock
      [self put: key withTime: [turn logicalNow] value: val]))

(defproto CRDTTable
  (mixins LWWMap)
  (slots state) ;; A table of key -> {value, timestamp}
  (handlers
    [put: k withTime: t value: v]
      (let existing [self at: k])
      (if (or [existing is nil] (> t [existing time]))
          [self.state at: k put: {value: v time: t}])
    [merge: otherTable]
      ;; Iterate and take highest timestamp for each key
      ...))
```

### Phase 3.B: Morphogenesis via the Input Log
When Alice and Bob edit the same Pixmap:
1. Alice's stroke is an intent `[crdtPixmap paintAt: {x y} color: 'red time: t1]`.
2. Bob's stroke is `[crdtPixmap paintAt: {x y} color: 'blue time: t2]`.
3. The Croquet-style Reflector orders these globally. Even if Bob's client processes Alice's stroke late, the CRDT `merge:` protocol guarantees both clients converge on the same pixel color based on the logical timestamps injected by the Reflector.
