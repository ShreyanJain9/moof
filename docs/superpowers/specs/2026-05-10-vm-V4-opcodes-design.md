# VM V4 — opcode set + byte encoding — design

> **status:** brainstormed 2026-05-10. ready for plan.
>
> **prior art:** V0 (FormId scope-tagging, shipped) + V1 (per-turn nursery + diff, shipped) + V2 (freezing, shipped) + V3 (env-chain unification, shipped). V4 takes the next step toward the canonical-VM end-state: a clean opcode set, byte-tagged encoding, and the substrate-honesty additions (`LoadHere`, `JumpIfTrue`, `Suspend`/`Resume`, fused sends) that the prior phases earned the right to add.
>
> **spec scope:** this document defines moof's bytecode op set, its on-disk byte encoding, the compiler emission rules, and the reflection surface — for the V4 substrate-host migration (rust seed → zig core, per the polyglot roadmap discussion of 2026-05-10).
>
> **prior reading:** `laws/substrate-laws.md` L3 (send is the universal verb), L5 (source is canonical, bytecode derived), L10 (live edit invalidates ICs), L11 (FormIds stable). `laws/determinism-laws.md` D5 (deterministic iteration). `laws/reflection-contract.md` R2 (`[m bytecodes]`).

## 1. scope and motivation

The V0-V3 wave gave us:

- a real Form heap with scope-tagged FormIds (V0)
- per-turn nursery + diff machinery for replicable mutations (V1)
- freezing (V2)
- env-as-Form / `$here` / view-target / `Object:eval:` (V3)

The substrate's *behavior* is now in good shape. The substrate's *encoding* — its bytecode — has accreted over those phases. We now have 15 opcodes encoded as a rust `enum`, sized to the largest variant (~12 bytes/op including padding), interpreted by a `match` loop. This works but isn't where we want to be:

- **enum-tagged bytecode is hard to canonical-hash** for content-addressing (phase 9). same-source → different-byte-layout-per-compiler-version isn't acceptable.
- **the existing set is missing primitives** that we already need (`LoadHere` would eliminate the most common env walk; `JumpIfTrue` would let the if-peephole avoid a `:!` send; `Suspend`/`Resume` are non-negotiable for phase D promises).
- **the enum encoding is wasteful** (~10-12 bytes/op average for an op that needs 5-8); fixing this is free dispatch perf.
- **the rust `match` dispatch loop is the bottleneck** when we benchmarked profile_new_world (1.3s in release; nearly all bootstrap time). a tail-call-threaded zig interpreter would dispatch ~3x faster.

V4 closes all four. The output is:

- **19 opcodes**, organized in 5 categories: value-load (7), stack (2), sends (7), control flow (4), closures (1), scheduling (2).
- **byte-tagged encoding**, 1-byte opcode tag + N-byte fixed-size operands. 3-8 bytes per op typical.
- **reflection-preserving fusion** — 4 fused-send opcodes (`SendSelf`, `SendHere`, `TailSendSelf`, `TailSendHere`) eliminate the most common 2-op patterns.
- **content-addressable**: canonical big-endian operand encoding makes chunk-byte-hashing deterministic.
- **zig-implementable**: each opcode handler is a small function; `@call(.always_tail)` chains them for direct-threaded dispatch performance.

V4 does **not** include: a JIT, type-specialized arithmetic (Smalltalk-style primitives with fallback), polymorphic IC promotion (that's a separate pass), or content-addressed chunk dedup (that's phase 9 work that V4 *enables*).

## 2. the V4 opcode set — overview

19 opcodes, each in one of 6 categories. detailed reference in §3.

```
=== VALUE LOAD (7) ===
0x01  PushNil                         1 byte
0x02  PushTrue                        1 byte
0x03  PushFalse                       1 byte
0x04  LoadConst {idx: u16}            3 bytes
0x05  LoadSelf                        1 byte
0x06  LoadHere                        1 byte                  [NEW in V4]
0x07  LoadName {name: u32 SymId}      5 bytes

=== STACK (2) ===
0x10  Pop                             1 byte
0x11  Dup                             1 byte

=== SENDS (7) ===
0x20  Send {sel:u32, argc:u8, ic:u16}            8 bytes
0x21  TailSend {sel:u32, argc:u8}                6 bytes
0x22  SuperSend {sel:u32, argc:u8, ic:u16}       8 bytes
0x23  SendDynamic {argc:u8, ic:u16}              4 bytes      [NEW in V4]
0x24  SendSelf {sel:u32, argc:u8, ic:u16}        8 bytes      [NEW in V4]
0x25  SendHere {sel:u32, argc:u8, ic:u16}        8 bytes      [NEW in V4]
0x26  TailSendSelf {sel:u32, argc:u8}            6 bytes      [NEW in V4]
0x27  TailSendHere {sel:u32, argc:u8}            6 bytes      [NEW in V4]

=== CONTROL FLOW (4) ===
0x30  Jump {offset: i16}              3 bytes
0x31  JumpIfFalse {offset: i16}       3 bytes
0x32  JumpIfTrue {offset: i16}        3 bytes                 [NEW in V4]
0x33  Return                          1 byte

=== CLOSURES (1) ===
0x40  PushClosure {chunk: u32 FormId} 5 bytes

=== SCHEDULING (2) ===
0x50  Suspend {promise-ic:u16}        3 bytes                 [NEW in V4]
0x51  Resume {frame-ic:u16}           3 bytes                 [NEW in V4]
```

**delta from V3**: 15 ops → 19 ops. additions: `LoadHere`, `JumpIfTrue`, `SendDynamic`, `SendSelf`, `SendHere`, `TailSendSelf`, `TailSendHere`, `Suspend`, `Resume`. removals: none. note: i counted 9 additions above; that's 15+9=24 ops total. let me recount the set.

actually counting: 7 value-load + 2 stack + 7 sends + 4 control + 1 closure + 2 scheduling = **23 opcodes total**.

the V3 set had: 4 value-load (PushNil/PushTrue/PushFalse/LoadConst) + 2 (LoadName, LoadSelf) + 2 stack + 4 sends (Send, TailSend, SuperSend, plus the implicit lib-level `:perform:withArgs:`) + 4 control (Jump, JumpIfFalse, Return — only 3) + 1 closure = **15 opcodes**.

V4 adds 8: LoadHere, JumpIfTrue, SendDynamic, SendSelf, SendHere, TailSendSelf, TailSendHere, Suspend, Resume. that's 9. so V4 set is 15 + 9 = **24 opcodes**, in 6 categories.

(the count discrepancy was me triple-counting. fixing: 24 opcodes, 9 new.)

## 3. opcode reference

each opcode declared here in canonical form: byte tag, operand layout (with big-endian byte order in serialized form), stack effect, reflection shape, and execution semantics.

operand types:
- `u8`, `u16`, `u32`, `i16` — fixed-width integers, network (big-endian) byte order
- `SymId` (u32) — symbol-table index
- `FormId` (u32) — vat-local Form id (per V0 scope-tagging)

stack effect is in `pops/pushes` form. negative means "pops more than pushes."

### 3.1 value-load ops

```
0x01  PushNil
      stack: -/+1
      reflect: { Opcode op: 'PushNil operands: {} }

0x02  PushTrue
      stack: -/+1
      reflect: { Opcode op: 'PushTrue operands: {} }

0x03  PushFalse
      stack: -/+1
      reflect: { Opcode op: 'PushFalse operands: {} }

0x04  LoadConst { idx: u16 }
      operand bytes: 2
      stack: -/+1
      execution: push chunk.consts[idx]
      reflect: { Opcode op: 'LoadConst operands: { idx: N } }

0x05  LoadSelf
      stack: -/+1
      execution: push current_frame.self_
      reflect: { Opcode op: 'LoadSelf operands: {} }

0x06  LoadHere                                                [NEW]
      stack: -/+1
      execution: push Value::Form(world.here_form)
      reflect: { Opcode op: 'LoadHere operands: {} }

0x07  LoadName { name: u32 SymId }
      operand bytes: 4
      stack: -/+1
      execution: env_lookup(current_frame.env, name); push result
      raises: 'unbound if name not in chain
      reflect: { Opcode op: 'LoadName operands: { name: 'x } }
```

### 3.2 stack ops

```
0x10  Pop
      stack: -1/+0

0x11  Dup
      stack: -0/+1 (effectively reads top, pushes copy)
```

### 3.3 send ops

all send ops use IC slots (except TailSend/TailSendSelf/TailSendHere which currently don't — a known wart; future work, see §6.4). IC slots are indexed into `chunk.ics`.

```
0x20  Send { sel: u32 SymId, argc: u8, ic: u16 }
      operand bytes: 7
      stack: -(1+argc)/+1
      execution: receiver = stack.pop(); args = stack.popN(argc);
                 result = dispatch_via_ic(receiver, sel, args, ic);
                 stack.push(result)
      reflect: { Opcode op: 'Send operands: { selector: 'foo: argc: 2 ic-idx: 4 } }

0x21  TailSend { sel: u32 SymId, argc: u8 }
      operand bytes: 5
      stack: -(1+argc)/+1 (replaces frame; no growth)
      execution: receiver = stack.pop(); args = stack.popN(argc);
                 dispatch to (proto, method) by lookup (no IC currently);
                 replace current frame's chunk/pc/env/self/defining_proto
                 with the dispatched method's, keeping stack contents.

0x22  SuperSend { sel: u32 SymId, argc: u8, ic: u16 }
      operand bytes: 7
      stack: -(argc)/+1 (no receiver pop — uses current self_)
      execution: receiver = current_frame.self_;
                 lookup_handler_starting_above(current_frame.defining_proto, sel);
                 result = invoke(method, receiver, args);
                 stack.push(result)
      reflect: { Opcode op: 'SuperSend operands: { selector: 'inspect argc: 0 ic-idx: 2 } }

0x23  SendDynamic { argc: u8, ic: u16 }                       [NEW]
      operand bytes: 3
      stack: -(2+argc)/+1
      execution: sel = stack.pop().as_sym();
                 receiver = stack.pop();
                 args = stack.popN(argc);
                 result = dispatch_via_ic(receiver, sel, args, ic);
                 stack.push(result)
      reflect: { Opcode op: 'SendDynamic operands: { argc: 2 ic-idx: 0 } }

0x24  SendSelf { sel: u32 SymId, argc: u8, ic: u16 }          [NEW]
      operand bytes: 7
      stack: -argc/+1 (no receiver pop)
      execution: receiver = current_frame.self_;
                 args = stack.popN(argc);
                 result = dispatch_via_ic(receiver, sel, args, ic);
                 stack.push(result)
      reflect: { Opcode op: 'SendSelf operands: { selector: 'foo: argc: 2 ic-idx: 4 } }
      semantics: equivalent to LoadSelf;Send fused for zero-pop-receiver dispatch

0x25  SendHere { sel: u32 SymId, argc: u8, ic: u16 }          [NEW]
      operand bytes: 7
      stack: -argc/+1
      execution: receiver = Value::Form(world.here_form);
                 args = stack.popN(argc);
                 result = dispatch_via_ic(receiver, sel, args, ic);
                 stack.push(result)
      reflect: { Opcode op: 'SendHere operands: { selector: 'bind:to: argc: 2 ic-idx: 8 } }
      semantics: equivalent to LoadHere;Send fused. uses substrate's
                 canonical here_form FormId; bypasses any user-level
                 rebinding of the symbol $here. see §6.5.

0x26  TailSendSelf { sel: u32 SymId, argc: u8 }               [NEW]
      operand bytes: 5
      stack: -argc/+1 (replaces frame)
      execution: receiver = current_frame.self_;
                 args = stack.popN(argc);
                 lookup + replace frame; same as TailSend with implicit
                 self-receiver.

0x27  TailSendHere { sel: u32 SymId, argc: u8 }               [NEW]
      operand bytes: 5
      stack: -argc/+1 (replaces frame)
      execution: receiver = Value::Form(world.here_form); same as TailSend.
```

### 3.4 control flow ops

```
0x30  Jump { offset: i16 }
      operand bytes: 2 (signed)
      stack: -/-
      execution: pc += offset

0x31  JumpIfFalse { offset: i16 }
      operand bytes: 2
      stack: -1/+0
      execution: v = stack.pop(); if !is_truthy(v) then pc += offset
      is_truthy: Nil and Bool(false) are falsy; everything else truthy

0x32  JumpIfTrue { offset: i16 }                              [NEW]
      operand bytes: 2
      stack: -1/+0
      execution: v = stack.pop(); if is_truthy(v) then pc += offset
      semantics: dual of JumpIfFalse — saves the compiler from inverting
                 conditions via :! when the macro generates the inverse
                 shape.

0x33  Return
      stack: -1/(frame ends)
      execution: result = stack.pop(); pop current frame;
                 caller's stack receives result; if no caller, end run.
```

### 3.5 closure ops

```
0x40  PushClosure { chunk: u32 FormId }
      operand bytes: 4
      stack: -/+1
      execution: alloc new closure-Form with proto = protos.closure;
                 set :env slot to current_frame.env;
                 set :body slot to chunk;
                 set :params slot to chunk.params;
                 push Value::Form(closure_form_id)
      reflect: { Opcode op: 'PushClosure operands: { chunk: 12345 } }
```

### 3.6 scheduling ops (phase D+)

these are reserved-and-defined in V4 but the corresponding promise/scheduler machinery isn't fully built until phase D.

```
0x50  Suspend { promise-ic: u16 }                             [NEW]
      operand bytes: 2
      stack: -1/(frame yields)
      execution: promise = stack.pop().as_form_id();
                 verify promise is a Promise-Form (or raise 'type-error);
                 enqueue current frame on promise's wait queue;
                 vat scheduler suspends this turn, schedules next promise
                 resolution as a separate turn.
      after-resume: stack.push(resolved_value); pc continues at op after
                 Suspend.

0x51  Resume { frame-ic: u16 }                                [NEW]
      operand bytes: 2
      stack: -2/+0 (or +N based on Frame's snapshot stack contents)
      execution: frame_form = stack.pop().as_form_id();
                 resume_value = stack.pop();
                 verify frame_form is a Frame-Form;
                 install frame's chunk/pc/env/self/defining_proto;
                 restore frame's saved operand stack contents (relative
                 to its stack_base);
                 push resume_value as the value the suspended op was
                 waiting for;
                 continue execution at frame.pc.
      after-resume: caller's frame is gone (replaced by the resumed one).
```

these are the substrate-level primitives for promises and continuations. moof's `[p then:]`, `await`, and (future) `call/cc` compile to these.

## 4. byte encoding

### 4.1 op tag byte

each opcode begins with one byte: the tag. ranges:

- `0x00` — reserved (never emitted; substrate panics on encounter)
- `0x01–0x0F` — value-load ops
- `0x10–0x1F` — stack ops
- `0x20–0x2F` — send ops
- `0x30–0x3F` — control flow
- `0x40–0x4F` — closures
- `0x50–0x5F` — scheduling
- `0x60–0xFF` — reserved for future expansion

the range partition is loose — it's a convention, not a load-bearing fact. ops in the same category have related semantics, making the dispatch table easy to read.

### 4.2 operand layout

operands immediately follow the tag, in declared order, in big-endian byte order. fixed-size; no LEB128 or variable-width encoding.

| operand type | bytes | range |
|---|---|---|
| `u8` | 1 | 0..255 |
| `u16` | 2 | 0..65535 |
| `u32` | 4 | 0..2^32-1 |
| `i16` | 2 | -32768..32767 |
| `SymId` | 4 | u32 |
| `FormId` | 4 | u32 (with 2-bit scope tag per V0) |

example: `Send {selector=0x1234abcd, argc=2, ic_idx=4}` encodes as:

```
0x20  0x12 0x34 0xab 0xcd  0x02  0x00 0x04
[tag]    [selector u32]    [argc][ic_idx u16]
```

total: 8 bytes.

### 4.3 chunks as serializable Forms

a chunk-Form's `:body` slot is a `Value::Bytes` containing the byte-encoded opcode stream. its `:consts` slot is a list of Values. its `:ics` slot is a list of IC-Form snapshots (each with cached_proto/cached_method/cached_generation/cached_singleton — none of these are required for execution, but their layout is part of the chunk's signature).

```
chunk-Form = {
  Chunk
  :source <Form>           ; the moof form this was compiled from
  :body <Bytes>            ; the byte-encoded opcode stream
  :consts <Cons list>      ; constant pool, indexed by LoadConst
  :ics <Cons list>         ; IC slot snapshots (count determines size)
  :params <Cons list>      ; (for closure chunks) the parameter symbols
}
```

deserialization: the substrate parses `:body` byte-by-byte using the tag table, walks operands per the fixed-width schema, and reconstructs the in-memory `Vec<Op>` for fast dispatch. or — depending on the substrate's host language — keeps the bytes as the canonical form and threads dispatch directly off byte indices.

### 4.4 content-addressing implications

a chunk's *canonical hash* is computed from:
- the byte-encoded `:body`
- the const-pool layout (each Value's canonical bytes — see V0 canonical-encoder spec)
- the params + source Form's canonical bytes

with this scheme, **two chunks have identical hash iff they were compiled from identical source by identical compiler version with identical canonicalization rules**. content-addressing dedup is meaningful; cross-vat code sharing is real.

V4 doesn't *enable* dedup directly — that's phase 9 work that requires LMDB persistence. but V4 makes the math add up.

## 5. compiler emission rules

### 5.1 standard send dispatch

```
when compileSend: emits a send:
  if receiver is the symbol `self`:
    if tail position: emit TailSendSelf
    else: emit SendSelf
  else if receiver is the symbol `$here`:
    if tail position: emit TailSendHere
    else: emit SendHere
  else if receiver is unknown-at-compile-time:
    compile receiver normally (emits a receiver-load op);
    if tail position: emit TailSend
    else: emit Send
```

the fusion rule fires at compile time. it's the only place fused ops are produced. mid-bytecode rewriting (by a future JIT, e.g.) is not allowed at V4 phase.

### 5.2 if-shape peephole — uses JumpIfTrue when shorter

the V3 if-shape peephole detects `(__send__ (__send__ c '!!) 'ifTrue:ifFalse: (fn () t) (fn () e))` and emits Jump-based bytecode. with `JumpIfTrue` available, the peephole can avoid emitting the `!!` send when the cond is already a Bool literal — direct branch.

(also: future peepholes for `if-not` shape would use JumpIfTrue directly without inverting.)

### 5.3 LoadHere replaces LoadName($here)

```
when compileLoadName: receives the symbol `$here`:
  emit LoadHere instead of LoadName($here)
```

LoadHere skips the env walk entirely. user-level rebinding of `$here` in env is not honored — see §6.5 below.

### 5.4 SendDynamic for `:perform:withArgs:`

```
when compileSend: receives the selector `perform:withArgs:`:
  compile receiver;
  compile sel arg;
  compile args arg;
  emit SendDynamic {argc=<args-count>, ic=fresh}
```

current implementation routes `:perform:withArgs:` through the `Object:perform:withArgs:` native. with `SendDynamic`, the compiler can short-circuit: the substrate handles dynamic selectors directly, bypassing the native's overhead.

## 6. specifics and edge cases

### 6.1 IC slot sharing across fused-send variants

the IC slot at `chunk.ics[ic_idx]` caches `(proto, method, generation, singleton)`. *the receiver source doesn't affect the IC layout* — the IC caches on the receiver's *proto*. so:

- `SendSelf {sel, argc, ic=4}` and `Send {sel, argc, ic=4}` would share the same IC slot if compiled to the same site, regardless of which op variant fires.
- different sites use different `ic_idx` values — independent caches.

practically: the compiler assigns IC slot indices monotonically per chunk; each Send variant gets its own slot. there's no IC-slot collision.

### 6.2 TailSend variants currently lack ICs

`TailSend`, `TailSendSelf`, `TailSendHere` don't have an `ic` field — they each do a full lookup. for tail-recursive code (which goes through tail-send repeatedly), this is a perf wart. **flagged for future work**: extend the IC machinery to tail-position calls.

### 6.3 SuperSend uses self as receiver implicitly

`SuperSend` already pops args and uses `current_frame.self_` as receiver. there's no need for `SuperSendSelf` — that would be redundant. similarly, `SuperSend`'s dispatch starts at `current_frame.defining_proto`'s parent in the proto chain.

### 6.4 stack-effect declarations

each opcode has a declared stack effect (`pops/pushes`). the compiler maintains a running stack-balance check at compile time; mismatches indicate compiler bugs. (current rust seed: not yet implemented; flagged as a hygiene improvement.)

stack effects are total — no opcode "may push or may not push depending on context."

### 6.5 SendHere bypasses user $here rebinding

if moof code does `(set! $here other-env)`, the *slot* `$here` in some env gets a new value. that does NOT change `world.here_form` (the substrate-canonical here_form FormId).

- `LoadName('$here')` followed by `Send`: observes the user-level rebinding (env walk finds the user-set value).
- `SendHere`: ignores the rebinding (uses world.here_form directly).

documentation: `SendHere` targets the substrate-level here_form. emit `LoadName('$here'); Send` (or use a non-fused path) if your code's semantic depends on user-level `$here` rebinding.

### 6.6 nil-receiver-self-send at top level

at top-level (not inside a method dispatch), `current_frame.self_` is `Value::Nil`. `SendSelf` therefore sends `Nil :sel argc=N`. cleanly defined; dispatches to nil's handlers; no panic.

### 6.7 reading-pc during execution

each handler reads its operands from `pc` and advances. the read+advance is atomic with respect to the executing thread (single-threaded substrate per V0); no race. the canonical encoding makes operand layout fixed-size; no parsing ambiguity.

## 7. reflection layer

### 7.1 `[chunk bytecodes]` returns the decoded list

```moof
[some-chunk bytecodes]
;; → (
;;   { Opcode op: 'LoadHere operands: {} }
;;   { Opcode op: 'LoadConst operands: { idx: 0 } }
;;   { Opcode op: 'LoadName operands: { name: 'x } }
;;   { Opcode op: 'SendHere operands: { selector: 'bind:to: argc: 2 ic-idx: 0 } }
;;   { Opcode op: 'Return operands: {} }
;; )
```

each Opcode-Form has `:op` (the op-name symbol) and `:operands` (a Table of name → value). the decoder reads bytes from `chunk:body`, dispatches on tag, builds the structured form.

new opcodes have new names: `LoadHere`, `JumpIfTrue`, `SendDynamic`, `SendSelf`, `SendHere`, `TailSendSelf`, `TailSendHere`, `Suspend`, `Resume`.

### 7.2 disassembly text

a `Opcode :toString` renderer produces smalltalk-style disassembly:

```
LoadHere
LoadConst idx=0
LoadName name='x
SendHere :bind:to: argc=2 ic=0
Return
```

users can pretty-print a chunk via `[chunk disassemble]` (a moof helper, not a substrate primitive).

### 7.3 chunk-byte access for content-hashing

`[chunk bytes]` returns the raw `:body` Bytes (the byte-encoded stream). `[chunk hash]` returns the chunk's content-address (via `$hash`).

```moof
[chunk1 hash] = [chunk2 hash]  ;; same source → same hash (deterministic compile)
```

## 8. boot order — bytecode bootstrap

`new_world()`'s sequence with V4:

1. `World::new()` — allocates protos, sym table, here_form.
2. `intrinsics::install` — installs natives (including new ops' implementations on the VM dispatch table).
3. embed bootstrap chunks — the rust seed compiles `lib/compiler/*.moof` with the V4 emission rules (LoadHere, fused sends, etc.). these chunks become canonical.
4. `[$compiler useMoof]` — moof Compiler takes over, emitting V4 bytecode for everything else.
5. `lib/early/*.moof` and `lib/stdlib/*.moof` load via the moof Compiler.

no change to the boot dance. the rust seed compiler emits V4 bytecode from the start.

## 9. determinism — same source, same bytecode

content-addressable bytecode requires deterministic compile. V4 makes this explicit:

- the compiler emits ops in canonical order (no reordering based on heap state).
- IC slot indices are assigned monotonically per chunk (deterministic).
- const-pool entries are deduplicated by value equality, then assigned monotonically.
- nested chunks are compiled depth-first and assigned monotonically.

with these rules, two compiler runs on the same source produce byte-identical bytecode. that's a substrate-laws.md D1+D4 strengthening.

(open detail: SymId stability across vats — currently the symbol table is in-memory and not deterministic across cold boots. canonical SymId is "interning order"; we'd need either a stable canonical-symbol-encoding format, or a SymId-at-link-time resolution layer. flagged for phase B work.)

## 10. exit criteria

V4 ships when:

- [ ] all 24 opcodes are defined with byte encodings.
- [ ] the rust seed compiler emits V4 bytecode (or — if rust seed retires per the polyglot plan — the new zig/ocaml seed compiler does).
- [ ] the VM dispatcher handles all 24 opcodes correctly (smoke tests via CLI).
- [ ] `[chunk bytecodes]` decodes V4 bytecode into Opcode-Forms (existing reflection works).
- [ ] `LoadHere`, `JumpIfTrue`, `SendSelf`, `SendHere`, `TailSendSelf`, `TailSendHere`, `SendDynamic` all fire in their target patterns.
- [ ] same source → same bytecode (deterministic compile).
- [ ] `Suspend`/`Resume` are present as opcodes but the promise/scheduler machinery is phase D+; testing them is "round-trips through reflection."
- [ ] no warnings in `cargo build` (or whatever the new host build is).
- [ ] the bootstrap profile improves (`new_world()` goes from 1.3s to ~0.8s expected; sub-second is the target).

## 11. test plan (sketch)

- **encoding round-trip**: encode every opcode, decode back, verify operand values match.
- **reflection round-trip**: emit chunk, `[chunk bytecodes]`, parse to AST, re-emit, verify byte-identical.
- **fused-send correctness**: `[self foo]` and `[$here foo]` produce same result via SendSelf/SendHere as via LoadSelf/LoadHere+Send.
- **deterministic compile**: compile twice; verify byte-equal bytecodes.
- **content-hash**: compile a chunk; hash; modify source; recompile; verify hash changed; revert; verify hash recovers.
- **CLI smokes**: `[1 + 2]`, `(if #true 1 2)`, `(do (def x 1) (set! x 99) x)`, `[Object new]` all still work.

(no rust unit tests — we're test-free for now per project policy. these are CLI-level confidence checks.)

## 12. out of scope (deferred)

- **JIT / type-specialized arithmetic** (Smalltalk-80-style primitives with fallback). phase G+.
- **polymorphic IC** (4-entry hash on miss; mono-IC works fine until profiling demands more).
- **mid-bytecode rewriting** for self-modifying optimization. compiler emits; substrate executes; no in-between rewriting.
- **content-addressed chunk dedup** (phase 9 — requires LMDB).
- **canonical SymId encoding** (phase B — required for cross-vat chunk transport).
- **multiple-tag-byte ops** (e.g. extended ops beyond 0xFF). 24 opcodes leaves us ~240 spare slots; we'd add a `Extended` prefix only if we actually run out.

## see also

- `2026-05-09-vat-V3-here-form-design.md` (V3 — `$here`, the predecessor of `LoadHere`/`SendHere`)
- `laws/substrate-laws.md` L3 (send is universal), L5 (source canonical), L10 (live edit IC), L11 (FormIds stable)
- `laws/reflection-contract.md` R2 (`[m bytecodes]`)
- `laws/determinism-laws.md` D1, D4, D5 (deterministic alloc + iteration)
- the 2026-05-10 brainstorm/conversation: polyglot host plan (rust → zig substrate; OCaml seed compiler), opcode fusion design
