# The Form Heap as the Universal File System: Deep Implementation Plan

> **Goal: A persistent, introspectable graph replacing the traditional OS filesystem.**

This document details the exact protocols for navigating the graph, the reflection payload for agents, and the LMDB bindings that guarantee zero-save persistence.

## 1. The Semantic Namespace and Spatial Traversal

We replace string-based paths with structural traversal.

### Phase 1.A: The `PathTraversal` Protocol and Error Handling
**Files:** `lib/protocols/traversal.moof`
```moof
(defprotocol Traversible
  (requires [resolvePath:])
  (derives
    [cd: path]
      (let target [self resolvePath: path])
      (if [target is nil]
          (raise 'path-not-found {path: path at: self}))
      target))

(defproto WorldRoot
  (mixins Traversible)
  (slots users system)
  (handlers
    [resolvePath: pathList]
      (if [pathList empty?]
          self
          (let nextNode [self at: [pathList car]])
          (if [nextNode respondsTo: :resolvePath:]
              [nextNode resolvePath: [pathList cdr]]
              ;; If it's a leaf node but path has more segments
              (if [pathList cdr empty?] nextNode nil)))))
```

### Phase 1.B: Native Configuration
A configuration "file" is just a Table Form.
`[$world cd: '(system network settings)] -> {port: 8080 host: "0.0.0.0"}`
Agents use `[settings at: 'port]` directly. No JSON parsing.

## 2. Deep Introspection API

Agents need mathematically rigorous access to the running system.

### Phase 2.A: The Reflection Payload
**Files:** `lib/stdlib/reflection.moof`
The substrate guarantees specific keys in `[form meta]`:
- `:source` -> The raw string or AST Form.
- `:doc` -> Docstring.
- `:ic-stats` -> Table `{misses: 0 hits: 0}`.
- `:provenance` -> Which agent/vat created this Form.

```moof
;; Agent API
(defproto AgentTools
  (handlers
    [findBottlenecksIn: rootProto]
      (let methods [rootProto handlers])
      [methods filter: |name method|
        (> [[method meta] at: 'ic-misses default: 0] 100)]))
```

### Phase 2.B: AST Extraction and Synthesis
**Files:** `lib/early/compiler-ast.moof`
The AST is just nested `Cons` Forms.
1. `(let ast [[Cons handlers at: 'map:] source])`
2. The agent navigates the list: `[ast car] == 'fn`.
3. The agent synthesizes a new list: `(list 'fn '(a b) (list '+ 'a 'b))`.
4. `[$compiler compileForm: newAst]`.

## 3. Persistence Emergence via LMDB

Persistence must happen automatically at turn boundaries.

### Phase 3.A: The `WriteSet` and Dirty Flags
**Files:** `src/vat.rs`, `src/heap.rs`
1. The Substrate `Heap` tracks dirty Forms. `heap.set_slot(id, key, val)` adds `id` to `vat.dirty_set`.
2. At `VmStatus::Yielded` (turn end), the Reflector takes over.

### Phase 3.B: The LMDB Transaction Bindings
**Files:** `crates/mco-store-lmdb/src/lib.rs`
```rust
// In the Reflector after turn execution
let mut txn = lmdb_mco.begin_write_txn()?;

for form_id in vat.dirty_set.drain() {
    let form = heap.get(form_id);
    let canonical_bytes = canonical_encode(form)?;
    // ID is the LMDB key
    lmdb_mco.put(&mut txn, form_id.to_le_bytes(), canonical_bytes)?;
}

// Append input intent to the journal log
lmdb_mco.put(&mut txn, vat.turn_seq.to_le_bytes(), turn_intent_bytes)?;

// Atomic commit
lmdb_mco.commit(txn)?;
```
**Tests:** `test_dirty_set_populated_on_mutation`, `test_lmdb_commit_persists_canonical_bytes`, `test_boot_from_lmdb_restores_heap_state`.
