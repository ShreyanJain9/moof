# compiler primitives

> **the substrate's chunk-construction api, exposed to moof. these
> primitives let moof code (specifically, `lib/compiler.moof`) emit
> bytecode without the rust line owning compilation. paired with the
> existing read-side reflection (`[m bytecodes]`, `[m consts]`,
> `[m ics]` — `laws/reflection-contract.md` R2), they make bytecode
> bidirectionally moldable: substrate-laws.md L5 ("source canonical,
> bytecode derived") holds in *both* directions.**

this is the ground floor of phase A-self-host. everything in
`compiler.moof` is built on these calls.

## the shape

per `process/docs-driven.md`, user-data ops are *methods on the
receiver*, not free functions. so the api is two protos worth of
sends:

- **`Opcode`** — class-side constructors that build opcode-Forms.
- **`Chunk`** — class-side `:new:source:` and instance-side
  `:emit:`, `:addConst:`, `:addIc`, `:jumpTarget`, `:patchJump:to:`,
  `:asClosure`.

an opcode-Form has slots `:op` (a `Symbol` like `'LoadConst`) and
`:operands` (a `Table` of positional Values). the form is just a
value: `[op slots]` works, you can quote it, store it, hand it
between vats. nothing about it is hidden.

decoding (`Op` → Form, used by `[m bytecodes]`) and encoding
(Form → `Op`, used by `[chunk emit:]`) are inverses; the
substrate guarantees `[chunk emit: [Opcode loadConst: 5]]` is
indistinguishable from a rust-side `Op::LoadConst(5)` after
finalize.

## opcode constructors

every `Op` variant in `players/rust/src/opcodes.rs` has a
matching class-side method on `Opcode`. send returns an
opcode-Form ready to feed into `[chunk emit:]`.

| send | shape of the Op |
|---|---|
| `[Opcode loadConst: idx]` | `LoadConst(idx)` |
| `[Opcode pushNil]` | `PushNil` |
| `[Opcode pushTrue]` | `PushTrue` |
| `[Opcode pushFalse]` | `PushFalse` |
| `[Opcode pop]` | `Pop` |
| `[Opcode dup]` | `Dup` |
| `[Opcode loadName: 'name]` | `LoadName(name)` |
| `[Opcode storeName: 'name]` | `StoreName(name)` |
| `[Opcode loadSelf]` | `LoadSelf` |
| `[Opcode defineGlobal: 'name]` | `DefineGlobal(name)` |
| `[Opcode send: 'sel argc: a ic: i]` | `Send { sel, argc, ic }` |
| `[Opcode tailSend: 'sel argc: a]` | `TailSend { sel, argc }` |
| `[Opcode superSend: 'sel argc: a ic: i]` | `SuperSend { … }` |
| `[Opcode pushClosure: chunk]` | `PushClosure { chunk }` |
| `[Opcode jump: off]` | `Jump(off)` |
| `[Opcode jumpIfFalse: off]` | `JumpIfFalse(off)` |
| `[Opcode return]` | `Return` |

range checks happen at *emit* time, not construct time:
`[chunk emit:]` raises if `argc > 255`, `idx > 65535`, or `off`
doesn't fit `i16`. constructors are pure value-builders.

## chunk lifecycle

a chunk goes through three stages:

```
                  [Chunk new: ps source: src]
                              │
                              ▼
                       ┌───────────┐
                       │  empty    │   :ops=[]   :consts=[]   :ics=[]
                       └─────┬─────┘
                             │
   :emit: / :addConst: / :addIc — many sends, any order
                             │
                             ▼
                       ┌───────────┐
                       │ populated │   :ops=[…]  :consts=[…]
                       └─────┬─────┘
                             │
                       :asClosure
                             │
                             ▼
                       callable closure-Form ready for `[c call: …]`
```

there is no separate "freeze" step. the chunk-Form is real and
queryable from creation; later mutations show up immediately in
`[m bytecodes]`, `[m consts]`, etc. (after wrapping in a closure).
this matches L5: source is canonical, bytecode is the derived
cache, and the cache is observable while it's being built.

### `[Chunk new: params source: source]`

class-side constructor. allocates a fresh empty chunk-Form;
returns it.

- `params`: a list of parameter symbols. used for arity checking
  on call. pass `'()` for a top-level expression chunk.
- `source`: the source-form this chunk derives from. stored in
  `:source` meta per L5. pass `nil` if the chunk is being
  constructed mechanically (compiler-internal scratch).

side effects: registers a fresh entry in the substrate's
`chunk_ops`, `chunk_consts`, `chunk_ics` side tables (each empty).

### `[chunk emit: op-form]`

append `op-form` to the chunk's ops vector. returns the position
the op was emitted at (a non-negative `Integer`). callers patch
jumps using this position.

raises `'compile-error` if `op-form` is malformed (missing or
wrong-shaped `:op` / `:operands`), `'range-error` if any
operand exceeds the bytecode's bounds.

### `[chunk addConst: value]`

append `value` to the chunk's constant pool. returns its index
(an `Integer` in `0…65535`). the index is what you pass to
`[Opcode loadConst:]`.

### `[chunk addIc]`

reserve a fresh inline-cache slot. returns its index (an
`Integer` in `0…65535`). pair it with the next `[Opcode send:…]`
or `[Opcode superSend:…]`. each `Send`/`SuperSend` op needs its
own ic slot; sharing slots between sites breaks dispatch.

### `[chunk jumpTarget]`

returns the current ops length. emit a target by:

```moof
(let ((tgt [c jumpTarget]))
  ;; … later, after emitting a forward jump:
  [c patchJump: jump-pos to: tgt])
```

backward jumps capture `jumpTarget` *first*, then emit the jump
last and patch with `[c patchJump: emitted-jump-pos to: tgt]`.

### `[chunk patchJump: pos to: target]`

overwrite the offset of the `Jump` / `JumpIfFalse` already at
`pos` in the chunk's ops vector. `target` is the ops index the
jump should land at. computes `target - pos` as the offset
(matching the VM's `(pc - 1) + off` formula, where pc has just
advanced past the jump op).

raises `'compile-error` if the op at `pos` isn't a jump variant,
`'range-error` if the offset doesn't fit `i16`.

### `[chunk asClosure]`

wrap the chunk in a Closure-Form ready to call. captures the
*global* environment (no enclosing lexical scope) and `nil` as
the captured self — equivalent to running `PushClosure { chunk }`
in a top-level frame.

returns a Value of proto `Closure`. send it `:call` with args
matching the chunk's `:params` arity.

a closure built this way is distinguishable from a closure
emitted by `Op::PushClosure` only in its captured env: this one
captures `world.global_env`, the latter captures whatever frame
it was emitted in. for compiler.moof's purposes the two are
equivalent — generated chunks are top-level.

## the smoke-test

the deliverable that proves track 1 is wired end-to-end:

```moof
(def smoke
  (let ((c [Chunk new: '() source: nil]))
    [c addConst: 1]
    [c addConst: 2]
    [c emit: [Opcode loadConst: 0]]
    [c emit: [Opcode loadConst: 1]]
    [c emit: [Opcode send: '+ argc: 1 ic: [c addIc]]]
    [c emit: [Opcode return]]
    [[c asClosure] call]))
;; smoke ≡ 3
```

if this returns `3`, every primitive is honest: opcode forms
encode correctly, the side tables update, the closure dispatches,
the VM doesn't notice the chunk was hand-built.

## what these primitives do *not* do

- **no env modeling.** `:emit:` doesn't know about lexical
  scope. `compile-form` (in moof) decides when to emit
  `LoadName` vs hoist into a const. the primitives are a level
  below scope.
- **no macroexpansion.** macro lookup is `(macroexpand …)`,
  separate primitive. compiler.moof calls it explicitly.
- **no source-loc threading.** the chunk's `:source` meta is the
  *whole* source-form. per-op source-locs are a phase-C
  follow-up (the substrate's bytecode side-tables would gain a
  parallel `chunk_locs` map).
- **no purity / effect inference.** the analyzer is
  separate; it reads `[c bytecodes]` and annotates the chunk's
  meta independently.

## doc gates

these primitives sit on the line between L5 (source canonical,
bytecode derived) and R2 ([m bytecodes] reflects). before this
file existed, both laws held *unidirectionally*: the substrate
emitted bytecode from source, and moof code could read it. with
these primitives, both directions are open: moof can also *write*
bytecode, with the substrate's blessing and the same reflection
guarantees.

R6 ("nothing the substrate knows is hidden") is preserved
because the side tables (`chunk_ops`, `chunk_consts`,
`chunk_ics`) are exposed in full via the existing reflection
methods. these primitives only widen the *write* surface; they
do not introduce hidden state.

## design note: why methods, not free functions

per `process/docs-driven.md`'s stdlib rule, the moof library
prefers *sends to receivers* over *free functions on data*.
`[c emit: op]` reads as smalltalk; `(chunk-emit c op)` reads as
scheme. moof is closer to smalltalk in spirit, so the substrate
api is too. free functions remain reserved for substrate
metaprogramming primitives (`(intern …)`, `(slot …)`,
`(macroexpand …)`) where the action is "modify the substrate's
view of this Form," not "send a message to a receiver."

## see also

- `concepts/compiled-objects.md` — chunks / methods / closures.
- `laws/substrate-laws.md` — L5.
- `laws/reflection-contract.md` — R2, R6.
- `concepts/sends-and-calls.md` — what selectors / argc / ics
  mean at dispatch time.
- `players/rust/src/opcodes.rs` — the canonical Op set.
- `lib/compiler.moof` (when written) — the consumer of these
  primitives.
