# The Form Heap as the Universal File System: Implementation Plan

> **Discarding the hierarchy of bytes for a persistent graph of living objects.**

This document details how Moof replaces the traditional filesystem with a traversed Form graph, and how agents use deep introspection APIs to navigate and rewrite this living system.

## 1. The Semantic Namespace and Spatial Traversal

Filesystems use strings (`/usr/bin`). Moof uses message sends through Placements.

### Phase 1.A: The `PathTraversal` Protocol
We introduce a standard way to query the graph, analogous to filesystem paths but structural.

```moof
(defprotocol Traversible
  (requires [resolvePath: list]))

(defproto WorldRoot
  (mixins Traversible)
  (slots users system)
  (handlers
    [resolvePath: pathList]
      (if [pathList empty?]
          self
          (let nextNode [self at: [pathList car]])
          [nextNode resolvePath: [pathList cdr]])))
```

A user (or agent) typing `cd users/alice/tools` is actually executing the Moof code:
`[[[$world resolvePath: '(users alice tools)]] openInHalo]`

### Phase 1.B: Native Data Representation
Because everything is a Form, there is no serialization/deserialization step for configuration.
If an agent wants to read Alice's color preference:
`[$world resolvePath: '(users alice prefs color)] -> 'blue`
It returns the Symbol `'blue`, not a string `"blue"` that needs parsing from a JSON file.

## 2. Deep Introspection API

To allow agents to rewrite the system, the Reflection contract must be mathematically rigorous.

### Phase 2.A: The Meta-Level API
Every Form has a `meta` slot (a Table) managed by the substrate.
```moof
;; Agent queries a method's performance metrics
(let method [[String handlers] at: 'trim])
(let metrics [method meta at: 'performanceStats])
(if (> [metrics icMisses] 1000)
    [AgentOptimizer flagForRewrite: method])
```

### Phase 2.B: The AST Extraction and Rewriting Protocol
An agent identifies duplicated code across `Cons` and `String`.

1. **Extract AST:**
   `(let ast [method source])` -> Returns the exact `(fn (self ...) ...)` Form tree.
2. **Agent Analysis:** The agent traverses the AST (which is just a nested `Cons` list of Forms), recognizes the `reduce:` pattern.
3. **Synthesis:** The agent synthesizes a new Protocol Form AST.
   `(let newProtocolAst '(defprotocol Enumerable ...))`
4. **Compilation:**
   `(let newProtocolChunk [$compiler compileForm: newProtocolAst])`
   `(let newProtocolForm [$compiler execute: newProtocolChunk])`
5. **Hot-Patching:**
   The agent rewrites the target prototypes to include the new mixin.
   `[Cons addMixin: newProtocolForm]`
   `[String addMixin: newProtocolForm]`

## 3. Persistence Emergence

Agents do not "save" files. The Reflector (Rust `src/reflector.rs`) and the Canonical Encoder (`core/canonical-encoder.mco`) handle this natively.

### Phase 3.A: The LMDB Turn Commit
1. During a turn, if an agent evaluates `[AlicePrefs setHandler: newMethod for: 'draw]`, the `FormId` of `AlicePrefs` is marked dirty in the Vat's local `WriteSet`.
2. At the end of the turn, the Vat serializes the dirty Forms into Canonical Bytes.
3. The substrate executes an LMDB transaction:
   `txn.put(form_id_bytes, canonical_bytes)`
4. The system is inherently crash-proof. The "filesystem" is exactly the state of the heap at the end of the last successful message turn.
