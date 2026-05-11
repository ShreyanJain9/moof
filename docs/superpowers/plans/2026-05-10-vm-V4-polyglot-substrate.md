# VM V4 — polyglot substrate (zig core + OCaml seed) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the full V4 migration — moof's substrate gets a new opcode set, a byte-tagged encoding, a zig host for the VM hot loop, and an OCaml seed reader+compiler. by the end, the rust substrate crate is **deleted**; moof boots from a tiny zig core + ocaml seed; the moof Compiler self-hosts; the substrate is ≤2k LoC zig + ≤1k LoC OCaml; every other byte is moof or wasm mco. content-addressable, deterministic, fast.

**Architecture (the end state):**

```
[ user moof code ]
       ↓ (parser.moof + compiler.moof)
[ V4 bytecode ]
       ↓ byte-tagged stream
[ zig VM hot loop ]  ←→  [ wasm mcos (polyglot leaves) ]
       ↓
[ zig heap + GC ]

[ OCaml seed ]                  // only used to bootstrap moof's own
  ├─ reader (parser combinators)  // parser.moof + compiler.moof; deletable
  └─ compiler (ML pattern match)  // post-self-host.
```

four phases. each is independently shippable; later phases depend on earlier ones. each phase has its own forcing function and can be in-progress / done / not-started independently of the others (subject to dependency order).

**Tech Stack:** Rust 2021 (existing seed, deleted by phase 4), Zig 0.14+ (new substrate host, phase 3+), OCaml 5.x with menhir + opam (seed compiler, phase 4), wasm (mcos, unchanged), moof (everything else).

**Project state (HEAD):** `660f855` — V4 spec drafted, no impl. V3 features all landed. const-fold, become:, perform:withArgs:, IC invalidation, doesNotUnderstand:, cycle-safe inspect all live.

---

## phase overview

| phase | output | duration estimate (real days, not robot estimates) | risk |
|---|---|---|---|
| **α** opcodes | 9 new opcodes wired in rust substrate; compilers emit them | 1-2 days | low — purely additive |
| **β** byte encoding | `Vec<u8>` chunk bodies; deterministic compile; content-hashable | 2-3 days | medium — touches dispatch + reflection + compiler |
| **γ** zig substrate | zig host for VM + heap + ICs; bytecode shared with rust seed via byte format | 1-2 weeks | high — first polyglot host swap |
| **δ** OCaml seed | reader.ml + compiler.ml; emit V4 bytecode; delete rust seed reader+compiler | 1-2 weeks | high — bootstrap-image required |

phases α and β can be done by any agent. phase γ requires zig fluency. phase δ requires OCaml fluency.

---

## File Structure (full V4 — across all phases)

| file | phase | role |
|---|---|---|
| `crates/substrate/src/opcodes.rs` | α | add 9 new enum variants |
| `crates/substrate/src/vm.rs` | α | dispatch arms for 9 new ops |
| `crates/substrate/src/intrinsics.rs` | α | reflection encode/decode for new ops + moof Opcode helpers |
| `crates/substrate/src/compiler.rs` | α, β | emit new ops; emit byte-tagged chunks |
| `lib/compiler/01-dispatch.moof` | α | moof Compiler emits LoadHere |
| `lib/compiler/02-special.moof` | α | moof Compiler emits SendSelf/SendHere/JumpIfTrue |
| `crates/substrate/src/chunk.rs` | β (new) | byte-tagged chunk encoding helpers |
| `crates/substrate/src/canonical.rs` | β (new) | canonical-encoder for chunks (content-hash input) |
| `crates/substrate/src/chunkstream.rs` | β (new) | streaming byte decoder for VM dispatch |
| `crates/zig-substrate/build.zig` | γ (new) | zig build script |
| `crates/zig-substrate/src/main.zig` | γ (new) | entrypoint + CLI |
| `crates/zig-substrate/src/heap.zig` | γ | port of heap.rs |
| `crates/zig-substrate/src/form.zig` | γ | Form struct |
| `crates/zig-substrate/src/value.zig` | γ | tagged-immediate Value |
| `crates/zig-substrate/src/sym.zig` | γ | symbol interning |
| `crates/zig-substrate/src/vm.zig` | γ | the tail-call-threaded interpreter |
| `crates/zig-substrate/src/world.zig` | γ | World struct + dispatch |
| `crates/zig-substrate/src/intrinsics.zig` | γ | native ops |
| `crates/zig-substrate/src/abi.zig` | γ | mco loader (wasm) interface |
| `crates/ocaml-seed/dune-project` | δ (new) | OCaml project setup |
| `crates/ocaml-seed/src/reader.ml` | δ | port of reader.rs |
| `crates/ocaml-seed/src/compiler.ml` | δ | port of compiler.rs (the seed) |
| `crates/ocaml-seed/src/bytecode.ml` | δ | byte-tagged bytecode emit |
| `crates/ocaml-seed/src/bin/seed.ml` | δ | CLI entry: source → V4 bytecode bytes |
| `crates/substrate/src/{reader,compiler}.rs` | δ | **DELETE** at end of phase δ |

---

# Phase α — new opcodes (enum-based)

**forcing function:** `cargo run -- '$here'` emits `LoadHere` instead of `LoadName('$here')`; `cargo run -- '[self foo]'` (in a method) emits `SendSelf` instead of `LoadSelf;Send`. reflection round-trips through new op-names.

## Task 1: add 9 enum variants + `pushes()` updates

**Files:** `crates/substrate/src/opcodes.rs`

- [ ] **Step 1:** add 9 variants to `Op` enum (LoadHere, JumpIfTrue, SendDynamic, SendSelf, SendHere, TailSendSelf, TailSendHere, Suspend, Resume). doc-comment each per spec §3. include `selector: SymId, argc: u8, ic_idx: u16` for the Send-family variants.

- [ ] **Step 2:** add `Op::LoadHere` to `pushes()`'s match list.

- [ ] **Step 3:** `cargo build -p moof 2>&1 | tail -3` — clean build with unused-variant warnings (expected).

- [ ] **Step 4:** commit:
```
opcodes: declare V4 opcodes (LoadHere, JumpIfTrue, SendDynamic, fused sends, Suspend/Resume)

V4 phase α task 1. Pure declaration of 9 new variants. No VM
dispatch arms yet (task 2). No compiler emission yet (tasks 3-7).
Build warns about unused variants until task 2 lands.

Spec: docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md
```

## Task 2: VM dispatch arms for the 9 new opcodes

**Files:** `crates/substrate/src/vm.rs`

- [ ] **Step 1:** locate `step()`'s match block on `Op`.

- [ ] **Step 2:** add 9 match arms. For each:
  - `LoadHere` → `world.vm.stack.push(Value::Form(world.here_form))`.
  - `JumpIfTrue(offset)` → mirror `JumpIfFalse` exactly, inverting the truthiness check.
  - `SendDynamic { argc, ic_idx }` → pop selector from stack as Symbol; pop receiver + args; `send_via_ic`.
  - `SendSelf { selector, argc, ic_idx }` → receiver = `frame.self_`; pop args only; `send_via_ic`.
  - `SendHere { selector, argc, ic_idx }` → receiver = `Value::Form(world.here_form)`; pop args; `send_via_ic`.
  - `TailSendSelf { selector, argc }` → receiver = `frame.self_`; pop args; replace current frame (use existing TailSend logic).
  - `TailSendHere { selector, argc }` → receiver = `world.here_form`; pop args; replace current frame.
  - `Suspend { promise_ic: _ }` → `Err(RaiseError::new(world.intern("unimplemented"), "Suspend: phase D"))`.
  - `Resume { frame_ic: _ }` → same `'unimplemented`.

- [ ] **Step 3:** if helper extraction simplifies things, extract a `tail_send_dispatch(world, receiver, selector, args)` from the existing `Op::TailSend` arm. Have TailSendSelf/Here call it.

- [ ] **Step 4:** build clean. unused-variant warnings should be gone.

- [ ] **Step 5:** regression smokes:
  - `[1 + 2]` → `3`
  - `(if #true 1 2)` → `1`
  - `$here` → renders full env
  - `(do (def x 42) x)` → `42`

- [ ] **Step 6:** commit:
```
vm: dispatch arms for V4 opcodes

V4 phase α task 2. Adds match arms in vm.rs::step() for all 9 new
opcodes. Suspend/Resume raise 'unimplemented (phase D fills in).
Regression smokes confirm existing ops unchanged.
```

## Task 3: rust seed `compile_load_name` emits LoadHere

**Files:** `crates/substrate/src/compiler.rs`

- [ ] **Step 1:** find where the rust seed emits `Op::LoadName(sym)`.

- [ ] **Step 2:** add `if self.world.resolve(sym) == "$here" { emit(LoadHere) }`.

- [ ] **Step 3:** smoke: `cargo run -- '(do (def f (fn () $here)) [[f bytecodes] inspect])'` — should show `LoadHere` in bytecodes.

- [ ] **Step 4:** commit.

## Task 4: reflection — encode/decode for the 9 new opcodes

**Files:** `crates/substrate/src/intrinsics.rs`, moof Opcode-helper file

- [ ] **Step 1:** find the encode (Op → Form) and decode (Form → Op) tables in `intrinsics.rs`.

- [ ] **Step 2:** add 9 encode entries. Per spec §7.1 — each op decodes as `{ Opcode op: <Symbol> operands: <list> }`.

- [ ] **Step 3:** add 9 decode entries. Each parses operands from the Form's slots back into typed fields.

- [ ] **Step 4:** find the moof Opcode helpers (`grep -rn "Opcode loadName" lib/`). add new constructors: `loadHere`, `jumpIfTrue:`, `sendDynamic:ic:`, `sendSelf:argc:ic:`, `sendHere:argc:ic:`, `tailSendSelf:argc:`, `tailSendHere:argc:`, `suspend:`, `resume:`.

- [ ] **Step 5:** smoke: `cargo run -- '(do (def f (fn () $here)) [[f bytecodes] inspect])'` — output should include `{ Opcode op: 'LoadHere operands: #[] }`.

- [ ] **Step 6:** commit.

## Task 5: moof Compiler `compileLoadName:` emits LoadHere

**Files:** `lib/compiler/01-dispatch.moof`

- [ ] **Step 1:** find `compileLoadName:chunk:`. Currently:
```moof
(if [sym is 'self] [chunk emit: [Opcode loadSelf]]
                   [chunk emit: [Opcode loadName: sym]])
```

- [ ] **Step 2:** rewrite:
```moof
(if [sym is 'self] [chunk emit: [Opcode loadSelf]]
(if [sym is '$here] [chunk emit: [Opcode loadHere]]
                    [chunk emit: [Opcode loadName: sym]]))
```

- [ ] **Step 3:** smoke + commit.

## Task 6: rust seed + moof Compiler emit SendSelf/SendHere/TailSendSelf/TailSendHere

**Files:** `crates/substrate/src/compiler.rs`, `lib/compiler/02-special.moof`

- [ ] **Step 1 (rust):** at the head of `compile_send`, check if receiver is the symbol `self` or `$here`:
```rust
let recv_kind = match elems[1] {
    Value::Sym(s) if self.world.resolve(s) == "self" => Some("self"),
    Value::Sym(s) if self.world.resolve(s) == "$here" => Some("here"),
    _ => None,
};
if let Some(kind) = recv_kind {
    // compile args only (no receiver)
    for &arg in &elems[3..] { self.compile_expr(arg, false)?; }
    let argc_u8 = (elems.len() - 3) as u8;
    let op = match (kind, tail) {
        ("self", false) => Op::SendSelf { selector: sel_sym, argc: argc_u8, ic_idx: self.next_ic() },
        ("self", true)  => Op::TailSendSelf { selector: sel_sym, argc: argc_u8 },
        ("here", false) => Op::SendHere { selector: sel_sym, argc: argc_u8, ic_idx: self.next_ic() },
        ("here", true)  => Op::TailSendHere { selector: sel_sym, argc: argc_u8 },
        _ => unreachable!(),
    };
    self.emit(op); return Ok(());
}
```

- [ ] **Step 2 (moof):** mirror in `compileSend:chunk:tail:`. before the existing const-fold/if-peephole/standard flow, check recv kind and emit the fused op directly. Use `Opcode sendSelf:argc:ic:` etc.

- [ ] **Step 3:** smoke:
  - `(do (defmethod (Object foo) [self proto]) [[[Object handlerAt: 'foo] body] bytecodes])` — should show `SendSelf` or `TailSendSelf`.
  - `(do (def f (fn () [$here parent])) [[f bytecodes] inspect])` — should show `SendHere`.

- [ ] **Step 4:** commit (one commit covering both rust and moof changes).

## Task 7: SendDynamic — replace `:perform:withArgs:` overhead (limited fuse)

**Files:** `crates/substrate/src/intrinsics.rs`

**SCOPE NOTE:** the encoding mismatch (SendDynamic expects args on stack individually; `:perform:withArgs:` takes a list) means we can't trivially fuse at compile time. SendDynamic is reachable from the VM but not emitted by the canonical compiler in V4-α.

- [ ] **Step 1:** leave the `:perform:withArgs:` native in place (it works).

- [ ] **Step 2:** add a note in the spec / native body explaining that future work will emit `SendDynamic` after a list-spread or after wave C's vau makes dynamic selectors natural.

- [ ] **Step 3:** no commit needed for this task — it's a documentation acknowledgment.

## Task 8: JumpIfTrue available; no canonical emission yet

- [ ] **Step 1:** wired via task 1-2-4. no compiler emission yet.

- [ ] **Step 2:** future work: const-fold or if-peephole could emit JumpIfTrue for `(if (not c) ...)` shapes. acknowledged in plan, no commit.

## Task 9: phase-α final smoke + commit

- [ ] **Step 1:** full smoke battery:
```bash
cargo build -p moof 2>&1 | tail -3                            # clean
cargo run --quiet -p moof -- '[1 + 2]'                        # 3
cargo run --quiet -p moof -- '(if #true 1 2)'                 # 1
cargo run --quiet -p moof -- '(if #false 1 2)'                # 2
cargo run --quiet -p moof -- '(do (def x 1) (set! x 99) x)'   # 99
cargo run --quiet -p moof -- '[$here parent]'                 # nil
cargo run --quiet -p moof -- '(do (def a [Object new]) (def b [Object new]) (setHandler! a (quote m) (fn () "a")) (setHandler! b (quote m) (fn () "b")) [a become: b] [a m])'  # "b"
cargo run --quiet -p moof -- '$here'                          # renders, no overflow
```

- [ ] **Step 2:** verify new ops fire:
```bash
cargo run --quiet -p moof -- '(do (def f (fn () $here)) [[f bytecodes] inspect])'
# contains LoadHere
cargo run --quiet -p moof -- '(do (defmethod (Object zo) [self proto]) [[[Object handlerAt: (quote zo)] body] bytecodes])'
# contains SendSelf or TailSendSelf
```

- [ ] **Step 3:** benchmark:
```bash
time cargo run --quiet --release -p moof -- '(do (def x 42) x)'
# baseline pre-V4-α: ~1.3s. expect ~1.15-1.2s.
```

- [ ] **Step 4:** push.

**Exit criterion for Phase α:** all 9 opcodes exist, dispatch correctly, decode through reflection. LoadHere + SendSelf + SendHere emitted by both compilers. CLI smokes green. Suspend/Resume placeholder-raise. SendDynamic + JumpIfTrue wired but not emitted by canonical paths (future use).

---

# Phase β — byte-tagged chunk encoding

**forcing function:** chunks store bytecode as `Vec<u8>` (byte-tagged stream per spec §4) instead of `Vec<Op>`. compile twice → byte-identical output. content-hashable. reflection still works.

## Task 10: design + add the byte format

**Files:** `crates/substrate/src/chunk.rs` (new), `crates/substrate/src/canonical.rs` (new)

- [ ] **Step 1:** create `chunk.rs`. add encoder + decoder functions:
```rust
pub fn encode_op(op: Op, buf: &mut Vec<u8>) { /* big-endian; per spec §4 */ }
pub fn decode_op(buf: &[u8], pc: usize) -> (Op, usize) { /* returns (op, bytes_consumed) */ }
```
match spec §4 exactly: 1-byte tag + big-endian operands.

- [ ] **Step 2:** create `canonical.rs`. add `pub fn canonical_chunk_bytes(chunk: &Chunk) -> Vec<u8>` that produces the bytes used for content-hashing. includes: body bytes + const-pool serialized + ic-count + params.

- [ ] **Step 3:** test encoder/decoder round-trip via a small main() smoke or just careful code review.

- [ ] **Step 4:** commit.

## Task 11: dual storage — keep Vec<Op>, ADD Vec<u8>

**Files:** `crates/substrate/src/world.rs` (the chunk-side-tables)

- [ ] **Step 1:** add `chunk_bytecode: IndexMap<FormId, Vec<u8>>` alongside the existing `chunk_ops: IndexMap<FormId, Vec<Op>>`.

- [ ] **Step 2:** at chunk-creation time (compiler.rs), encode `Vec<Op>` to bytes via `chunk::encode_op` and store both. (For now, only chunk_ops is read by the VM; chunk_bytecode is just for hash verification + future use.)

- [ ] **Step 3:** add `[chunk bytes]` reflection (new) that returns the Bytes form of the canonical bytecode. lets us call `[chunk bytes]` → `$hash` and verify content-hash determinism.

- [ ] **Step 4:** smoke: compile same source twice; both chunks' `[chunk bytes]` should produce identical bytes. (`[(c1 bytes) = (c2 bytes)]` → `#true`.)

- [ ] **Step 5:** commit.

## Task 12: switch VM dispatch to read from Vec<u8>

**Files:** `crates/substrate/src/vm.rs`

This is the actual encoding switchover. The VM `step()` becomes:
```rust
let chunk_bytes = world.chunk_bytecode.get(&chunk).unwrap();
let (op, advance) = chunk::decode_op(chunk_bytes, pc);
match op { /* same arms as before */ }
let new_pc = pc + advance;
```

- [ ] **Step 1:** rewrite step() to dispatch via byte decoding.

- [ ] **Step 2:** Jump/JumpIfFalse/JumpIfTrue offsets are now byte-offsets (i16). Make sure compiler emits them correctly (jump targets in bytes, not op-indices).

- [ ] **Step 3:** keep `chunk_ops` as a fallback / reflection-only side table for now. Delete it in task 14 once we're confident.

- [ ] **Step 4:** smoke battery: every CLI smoke from phase α should still work.

- [ ] **Step 5:** commit.

## Task 13: jump-offset rewrites in compiler

**Files:** `crates/substrate/src/compiler.rs`, `lib/compiler/`

- [ ] **Step 1:** the rust seed compiler currently uses `emit_placeholder_jump` + `patch_jump_to_here` with **op-index** offsets. Rewrite to use byte offsets.

- [ ] **Step 2:** moof Compiler `[chunk emit: [Opcode jumpIfFalse: 0]]` etc. — same byte-offset convention.

- [ ] **Step 3:** smoke: if-statements + macros that use jumps must still work.

- [ ] **Step 4:** commit.

## Task 14: drop Vec<Op> side table

**Files:** `crates/substrate/src/world.rs`, `crates/substrate/src/vm.rs`, others

- [ ] **Step 1:** remove `chunk_ops: IndexMap<FormId, Vec<Op>>` from World.

- [ ] **Step 2:** all callers either use `chunk_bytecode` directly (VM) or use the decoder to lazily produce a Vec<Op> for reflection (`intrinsics.rs::chunk_to_op_forms`).

- [ ] **Step 3:** reflection still works: `[chunk bytecodes]` decodes bytes on demand.

- [ ] **Step 4:** smoke battery.

- [ ] **Step 5:** commit.

## Task 15: content-hash + determinism verification

**Files:** `crates/substrate/src/chunk.rs`, smoke script

- [ ] **Step 1:** moof helper `[chunk hash]` returns `[$hash of: [chunk bytes]]`.

- [ ] **Step 2:** smoke: compile a method twice; `[h1 = h2]` should be `#true`.

- [ ] **Step 3:** smoke: modify source; recompile; hash differs.

- [ ] **Step 4:** commit.

**Exit criterion for Phase β:** chunks are byte-tagged. VM dispatch reads bytes directly. compiler emits canonical bytes. content-hash is stable across recompiles. reflection unchanged. CLI smokes green.

---

# Phase γ — zig substrate host

**forcing function:** `cargo build` of moof now builds a **zig substrate** crate that hosts the VM + heap. the rust substrate keeps its reader+compiler (for now); zig hosts the runtime. moof code runs through the zig VM consuming V4 bytecode produced by the rust seed compiler.

**dependency:** phase β must land first (zig consumes byte-tagged bytecode, not enum-Op).

## Task 16: zig project skeleton

**Files:** `crates/zig-substrate/` (new)

- [ ] **Step 1:** create `crates/zig-substrate/build.zig` with a buildable hello-world. produce a binary at `zig-out/bin/moof-zig`.

- [ ] **Step 2:** add to workspace via cargo build script that invokes `zig build` for this crate. (or keep them parallel — moof-rs for the seed compiler, moof-zig for runtime.)

- [ ] **Step 3:** stub `moof-zig <bytecode-file>` that reads a bytes file and prints "got N bytes".

- [ ] **Step 4:** commit.

## Task 17: port `form.zig`, `value.zig`, `sym.zig`, `heap.zig`

**Files:** `crates/zig-substrate/src/{form,value,sym,heap}.zig`

These are the leaf data types. No dependencies on the VM.

- [ ] **Step 1:** port `form.rs::FormId` to zig: a packed struct with 2-bit scope + 30-bit payload. derive Hash/Eq.

- [ ] **Step 2:** port `value.rs::Value` to zig: tagged union (`Nil | Bool | Int | Sym | Char | Float | Form`).

- [ ] **Step 3:** port `sym.rs` to zig: a SymTable with `intern(&[]const u8) → SymId` and `resolve(SymId) → []const u8`. use `std.StringHashMap`.

- [ ] **Step 4:** port `heap.rs::Heap`: `Vec<Form>`-equivalent (`std.ArrayList(Form)`), `redirects` for become:, `alloc(form) → FormId`, `get(id) → *Form`.

- [ ] **Step 5:** unit-smoke each module via a small main() that allocs forms, interns symbols, etc.

- [ ] **Step 6:** commit.

## Task 18: port `world.zig` + `intrinsics.zig` (the native primitives)

**Files:** `crates/zig-substrate/src/{world,intrinsics}.zig`

- [ ] **Step 1:** port `World` struct: heap, syms, protos, here_form, chunk-tables (chunk_bytecode, chunk_consts, chunk_ics, native_fns).

- [ ] **Step 2:** port the intrinsic ops one by one: arithmetic, slot access, env_bind, env_lookup, env_set (with view-target), become_, freeze, perform:withArgs:. each takes `*World, Value, []const Value` and returns `Value | Error`.

- [ ] **Step 3:** mco loader / wasm runtime — use wasmtime's C api (zig has bindings) or a smaller wasm runtime like wasm3 zig.

- [ ] **Step 4:** smoke: instantiate a World via zig, install protos, call `[1 + 2]` via direct send dispatch (no VM yet — just method invocation).

- [ ] **Step 5:** commit.

## Task 19: port `vm.zig` (the dispatch loop)

**Files:** `crates/zig-substrate/src/vm.zig`

This is the heart. Tail-call-threaded dispatch.

- [ ] **Step 1:** define `Op` as an enum(u8) matching the V4 byte tags.

- [ ] **Step 2:** for each opcode, write a handler function `fn op_<name>(vm: *VM) ResumePoint`. each handler reads operands from `vm.pc`, advances pc, executes semantics, tail-calls into the next op's handler via `@call(.always_tail, dispatch_table[next_op], .{vm})`.

- [ ] **Step 3:** the dispatch table is a `[256]fn(*VM) ResumePoint` populated at comptime.

- [ ] **Step 4:** smoke: load a bytecode file produced by rust seed; run it; verify result. start with the simplest chunk (a single `LoadConst 0; Return`).

- [ ] **Step 5:** scale up to full programs.

- [ ] **Step 6:** commit.

## Task 20: bridge — rust seed produces V4 bytes, zig VM consumes

**Files:** `crates/substrate/src/main.rs` (rust) or new bridge tool

- [ ] **Step 1:** add a flag `moof --emit-bytecode <source>` that compiles source and writes V4 bytecode (+ const-pool + chunk-metadata) to a file.

- [ ] **Step 2:** `moof-zig <file>` reads it, instantiates the World, runs.

- [ ] **Step 3:** smoke: end-to-end `moof --emit-bytecode '[1 + 2]' > /tmp/p.bc; moof-zig /tmp/p.bc` → `3`.

- [ ] **Step 4:** commit.

## Task 21: stdlib bootstrap on zig

**Files:** zig substrate

- [ ] **Step 1:** the lib/main.moof bootstrap dance: read main.moof, compile (still via rust seed), produce bytecode file, hand to zig. or: have the rust seed produce a single "bootstrap image" file containing all of lib/'s compiled state.

- [ ] **Step 2:** zig substrate loads the bootstrap image at startup and serves moof code from there.

- [ ] **Step 3:** smoke: full stdlib works on zig host.

- [ ] **Step 4:** commit.

## Task 22: phase-γ exit + benchmark

- [ ] **Step 1:** full smoke battery on zig host. all V3 + V4-α features.

- [ ] **Step 2:** benchmark: `time moof-zig <bootstrap-image>` for cold boot + a representative program.

- [ ] **Step 3:** expect: zig host is ~3x faster than rust match-dispatch on typical programs (per spec rationale). bootstrap time should be sub-second.

- [ ] **Step 4:** push.

**Exit criterion for Phase γ:** zig substrate compiles and runs moof code. all features from phases α + β + V3 work. rust substrate's VM + heap + intrinsics are formally deprecated (still exist, but the zig binary is the canonical runtime). benchmark beats rust dispatch.

---

# Phase δ — OCaml seed reader + compiler

**forcing function:** the rust seed reader (`reader.rs`) and seed compiler (`compiler.rs`) are deleted. moof source → V4 bytecode goes through an OCaml binary. zig substrate consumes the bytecode unchanged. once parser.moof + compiler.moof self-host, OCaml is throw-away scaffolding.

**dependency:** phase γ should land first (zig host stable). phase δ can be parallel after that.

## Task 23: OCaml project skeleton + reader

**Files:** `crates/ocaml-seed/{dune-project,src/reader.ml}`

- [ ] **Step 1:** `dune-project` for OCaml 5.x with menhir + alcotest.

- [ ] **Step 2:** port `reader.rs`'s s-expression parser to OCaml. use menhir or hand-written parser combinators. produce an AST that mirrors moof's source-form structure.

- [ ] **Step 3:** smoke: read `[1 + 2]` → AST equivalent to `(__send__ 1 + 2)`.

- [ ] **Step 4:** commit.

## Task 24: OCaml seed compiler

**Files:** `crates/ocaml-seed/src/{compiler,bytecode}.ml`

- [ ] **Step 1:** define ADT for `Op` matching the V4 spec.

- [ ] **Step 2:** port `compiler.rs`'s logic to OCaml: `compile_form`, `compile_send`, `compile_if`, etc. emit ops with byte-offset jumps.

- [ ] **Step 3:** include all V4 emission rules: LoadHere for $here, SendSelf/SendHere fusions, JumpIfTrue (when applicable).

- [ ] **Step 4:** include the V3 const-fold peephole (or skip — it's a moof-Compiler optimization, not strictly needed at seed level).

- [ ] **Step 5:** smoke: compile `[1 + 2]` to bytes matching what rust seed produced.

- [ ] **Step 6:** commit.

## Task 25: OCaml-seed CLI

**Files:** `crates/ocaml-seed/src/bin/seed.ml`

- [ ] **Step 1:** `seed <source-file> --emit-bytecode <output-file>` mirrors rust's emit-bytecode flag.

- [ ] **Step 2:** smoke: `seed /tmp/test.moof --emit-bytecode /tmp/p.bc; moof-zig /tmp/p.bc` end-to-end works.

- [ ] **Step 3:** commit.

## Task 26: switch the bootstrap to use OCaml-seed

**Files:** the moof boot dance

- [ ] **Step 1:** change `moof-zig`'s startup to invoke OCaml-seed to compile lib/main.moof (and friends) into a bootstrap image.

- [ ] **Step 2:** or: have a separate `moof-build` tool that orchestrates the seed-compile + image-bundle, producing a single binary embedding the bootstrap image. zig substrate loads it at startup.

- [ ] **Step 3:** smoke: full stdlib bootstrap, all features.

- [ ] **Step 4:** commit.

## Task 27: delete rust seed reader + compiler

**Files:** `crates/substrate/src/{reader,compiler}.rs` (and many call sites)

- [ ] **Step 1:** delete `reader.rs` and `compiler.rs`.

- [ ] **Step 2:** delete `lib.rs::eval`, `lib.rs::eval_program` (or move to a different layer if still needed).

- [ ] **Step 3:** delete the now-orphaned helpers in `intrinsics.rs` (`compileTop:`, `compileForm:`, etc — wait, those are the moof Compiler's. those stay.) actually only delete rust-side seed compilers.

- [ ] **Step 4:** the `crates/substrate` crate is now... what? a leaf crate with only the rust-side native primitives? or fully obsolete?

- [ ] **Step 5:** if obsolete, delete the entire `crates/substrate` crate. update workspace `Cargo.toml`. remove rust dependencies.

- [ ] **Step 6:** smoke: full moof stack runs via `moof-zig` + OCaml-seed.

- [ ] **Step 7:** commit.

## Task 28: phase-δ exit + integration

- [ ] **Step 1:** the moof workspace now has three (or four) crates:
  - `crates/zig-substrate` — the runtime
  - `crates/ocaml-seed` — the seed compiler
  - `crates/mco-pack` — utility
  - `crates/abi` + `crates/abi-rust` — the mco ABI (may consolidate)

- [ ] **Step 2:** the rust substrate is **GONE**.

- [ ] **Step 3:** count LoC: target zig ≤2k + ocaml ≤1.5k. rust ≤500 (just mco-pack + abi).

- [ ] **Step 4:** push the world-changing moment.

**Exit criterion for Phase δ:** rust substrate is deleted. moof boots via zig + OCaml. all V3 + V4 features work. moof is a polyglot project — zig for substrate, OCaml for compiler, moof for everything else.

---

# Phase ε (post-V4) — what comes next

once V4 lands, the path to phase B (single-vat persistence) is clear:

- canonical bytecode + content-addressed chunks (already done in V4-β!) → cross-vat code sharing
- byte-tagged Form serialization → heap snapshot to LMDB
- the zig substrate already has the GC discipline (turn-boundary; no mid-step alloc; etc.)
- intent/receipt model → cap calls are journaled separately

phase B becomes "wire LMDB + add journal-tail bootloader + write the canonical encoder for Forms." most of the substrate work is done.

---

## Final exit criteria for V4 (the whole thing)

- [ ] all 24 opcodes work in the zig substrate.
- [ ] byte-tagged chunk encoding; deterministic compile; content-hashable.
- [ ] zig substrate host: heap, GC, VM, mcos, intrinsics. ≤2k LoC.
- [ ] OCaml seed compiler: reader + compiler emitting V4 bytecode. ≤1.5k LoC.
- [ ] rust substrate crate **deleted** (`crates/substrate` no longer exists, or is reduced to mco utilities only).
- [ ] all V3 features work end-to-end on the new stack.
- [ ] CLI smokes pass: arithmetic, control flow, def/set!, live edit, become:, `$here`, reflection.
- [ ] benchmark: cold-boot moof from scratch ≤ 0.8s (vs current ~1.3s).
- [ ] benchmark: hot loop dispatch ~3x faster than rust match.
- [ ] documentation: spec doc updated; this plan marked complete.

---

## see also

- spec: `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`
- V3 spec + plan (the predecessor wave; reference patterns)
- substrate-laws.md (L3, L5, L10, L11)
- determinism-laws.md (D1-D6)
- reflection-contract.md (R2)
- the 2026-05-10 polyglot conversation (the "why" for the language choices)

---

## a note on phase ordering + parallelization

phases α and β must land in rust before γ starts (γ depends on the new opcodes + byte encoding being in rust, even if just to validate the encoding before re-implementing in zig).

phase γ should land before δ STARTS, BUT δ can be developed in parallel after γ has a stable bytecode-consumer story. an experienced agent could start δ once phase β is committed — but the integration moment (δ's "delete rust substrate") requires γ to be functional.

if working in a multi-agent setting, phases γ and δ can split between two agents after β lands. they meet at task 26-27.

---

## known risks + mitigations

1. **bootstrap dance breakage in phase β.** the moof Compiler self-compiles its own code via byte-emit. if the byte format is wrong, the bootstrap fails opaquely. *mitigation:* keep `Vec<Op>` alongside `Vec<u8>` (task 11) until everything works on bytes; flip the dispatch only after bytes are verified. delete `Vec<Op>` last.

2. **zig compile errors.** zig is a moving target. *mitigation:* pin to a specific zig version in `build.zig`'s `.minimum_zig_version`. document the version in the README.

3. **OCaml seed bootstrap.** OCaml has its own toolchain. *mitigation:* dune + opam are stable; pin to OCaml 5.x. produce a static binary so users don't need OCaml installed.

4. **mco loader portability.** wasmtime in zig vs wasmtime in rust. *mitigation:* use a stable wasm runtime that has zig bindings (wasm3, wasmer-c). or: port the wasm loader carefully, validating with existing mco binaries.

5. **deletion of rust substrate is irreversible.** once gone, recovering it from git history is doable but painful. *mitigation:* the rust substrate is a separate crate; we can leave it in the workspace as "legacy/" until phase δ + 30 days confirms the zig host is robust.

6. **performance regression in zig.** zig MIGHT not actually beat rust on dispatch — depends on inlining + tail-call success. *mitigation:* benchmark early (after task 19) before committing fully to the migration.
