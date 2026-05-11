# VM V4 — polyglot substrate (parallel) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to dispatch parallel workers across the four tracks. Steps use checkbox (`- [ ]`) syntax for tracking. Multiple tracks proceed simultaneously; integration milestones gate the merges.

**Goal:** ship V4 in ONE session by going polyglot from day 1. NO rust intermediate work. Build the zig substrate + OCaml seed compiler in parallel against the V4 spec; meet at the byte-encoded bytecode boundary. Rust substrate stays running as the safety net until zig+OCaml prove themselves; then it's deleted.

**Why no rust intermediate:** every line of rust opcode work or byte-encoding work is throwaway code that delays the actual migration. Skip it. The current rust substrate already works for testing-the-stdlib's-correctness; it doesn't need V4 features to do that job. We build V4 in zig + OCaml directly, validate it parallel against rust's known-good, then flip the canonical CLI from rust → zig.

**Architecture (target end state):**

```
[ moof source ]
       ↓ (parser.moof + compiler.moof — self-hosted, post-bootstrap)
[ V4 bytecode bytes ]
       ↓
[ zig VM ] ← consumes V4 byte format directly
       ↓
[ zig Heap + GC ] ← per-vat, content-addressable

[ OCaml seed ]                       // bootstrap-only; compiles lib/
  ├─ reader.ml (parser combinators)  // once self-host completes, just
  └─ compiler.ml (form → V4 bytes)   //   regenerates the system image
```

**Tech stack:** Zig 0.16.0+ (substrate host), OCaml 5.x + dune + menhir (seed compiler), moof (everything else). Rust stays as the safety-net runtime until V4 ships; then deleted.

**Project state (HEAD):** `5be40a3` — V4 spec complete (including §10 per-vat image format), zig skeleton (`crates/zig-substrate/`) with FormId + Value + smoke test working on zig 0.16.0.

---

## strategy: four parallel tracks

```
                              ┌────────────────────┐
                              │  V4 SPEC (CONTRACT)│
                              └─────────┬──────────┘
              ┌─────────────────────────┼─────────────────────────┐
              ▼                         ▼                         ▼
       ╔══════════════╗          ╔═══════════════╗         ╔═══════════════╗
       ║  TRACK A     ║          ║  TRACK B      ║         ║  TRACK D      ║
       ║  zig         ║          ║  ocaml        ║         ║  image format ║
       ║  substrate   ║          ║  seed         ║         ║  (shared)     ║
       ╚══════╤═══════╝          ╚═══════╤═══════╝         ╚═══════╤═══════╝
              └─────────────┐            │           ┌─────────────┘
                            ▼            ▼           ▼
                          ╔═══════════════════════════╗
                          ║  TRACK C                  ║
                          ║  integration + smoke      ║
                          ╚═══════════════════════════╝
                                       │
                                       ▼
                          ╔═══════════════════════════╗
                          ║  RUST DEPRECATION (final) ║
                          ╚═══════════════════════════╝
```

**Track A (zig substrate):** the runtime. heap, value, form, sym, intrinsics, vm dispatch, world bootstrap, image deserializer. ~1500 LoC zig target.

**Track B (OCaml seed):** the compiler. reader, AST, V4 bytecode emit, image serializer, CLI. ~1500 LoC OCaml target.

**Track C (integration):** byte-format roundtrip tests, smoke tests, end-to-end pipeline (`moof-zig <(moof-seed compile foo.moof)`).

**Track D (image format):** the canonical shared format both tracks read/write. spec'd in V4 §10; both Track A's deserializer and Track B's serializer implement it.

**Rust deprecation:** ONLY happens after smoke passes end-to-end. Switch default `moof` CLI to invoke zig binary; delete `crates/substrate/src/{reader,compiler,vm}.rs` and related rust runtime code.

**Parallelism:** Tracks A, B, D can run **completely independently**. Each is several small tasks parallelizable via subagents. Track C synchronizes; integration tests reveal contract violations between tracks.

**Risk mitigation:** rust substrate keeps working throughout. If zig+OCaml stack hits a wall, we still have a functional moof. Rust deletion is the LAST step, not a prerequisite.

---

## File Structure

### Track A — Zig substrate

| file | role |
|---|---|
| `crates/zig-substrate/build.zig` | (exists) build script |
| `crates/zig-substrate/src/main.zig` | (exists, will grow) CLI entrypoint |
| `crates/zig-substrate/src/sym.zig` | symbol interning |
| `crates/zig-substrate/src/form.zig` | Form struct |
| `crates/zig-substrate/src/value.zig` | tagged-immediate Value (already in main.zig — extract) |
| `crates/zig-substrate/src/heap.zig` | Heap (`std.ArrayList(Form)` + redirects) |
| `crates/zig-substrate/src/opcodes.zig` | the 24 opcode tags + decode helpers |
| `crates/zig-substrate/src/bytecode.zig` | V4 byte-tagged encoder/decoder |
| `crates/zig-substrate/src/protos.zig` | Protos struct + bootstrap |
| `crates/zig-substrate/src/world.zig` | World struct + boot |
| `crates/zig-substrate/src/vm.zig` | dispatch loop + send |
| `crates/zig-substrate/src/intrinsics.zig` | native methods |
| `crates/zig-substrate/src/image.zig` | per-vat image deserializer |

### Track B — OCaml seed

| file | role |
|---|---|
| `crates/ocaml-seed/dune-project` | project setup |
| `crates/ocaml-seed/src/dune` | library config |
| `crates/ocaml-seed/src/ast.ml` | source-Form ADT (Symbol, Int, Cons, etc.) |
| `crates/ocaml-seed/src/reader.ml` | S-expr + bracket-send parser |
| `crates/ocaml-seed/src/opcodes.ml` | the 24 opcodes as ADT |
| `crates/ocaml-seed/src/bytecode.ml` | V4 byte-encoded emit |
| `crates/ocaml-seed/src/compiler.ml` | form → bytecode (with V4 emission rules) |
| `crates/ocaml-seed/src/image.ml` | per-vat image serializer |
| `crates/ocaml-seed/bin/seed.ml` | CLI: source → image bytes |
| `crates/ocaml-seed/bin/dune` | binary build config |

### Track D — Shared format docs

| file | role |
|---|---|
| `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` | (exists) authoritative spec |

### Track C — Integration

| file | role |
|---|---|
| `fixtures/v4-bytecode/` | canonical moof programs + their expected V4 bytes |
| `scripts/v4-smoke.sh` | end-to-end pipeline tester |

### Final — Rust deprecation

| change | what |
|---|---|
| `Cargo.toml` workspace | remove `crates/substrate` member (or reduce to mco-pack + abi) |
| `crates/substrate/src/{reader,compiler,vm,intrinsics,opcodes,...}.rs` | DELETED |

---

## the parallel execution model

**stage 1 (everyone goes at once):** kick off ~12 subagents simultaneously across Tracks A, B, D. Each works in isolation against the V4 spec. None depend on each other's WIP.

**stage 2 (integration):** Track C subagent runs roundtrip + smoke tests. Failures point at which track diverged from the spec.

**stage 3 (fix loop):** dispatch targeted fix subagents. iterate until smoke passes.

**stage 4 (rust deprecation):** flip default CLI; delete rust substrate.

If we move fast and subagents return promptly, this fits in a session. If not, we ship through stage 2 (smoke passes for trivial programs); stages 3-4 spill over.

---

# Track A — Zig Substrate

Five-ish parallel sub-tracks. Each is a single .zig file with one focused job. The V4 spec is the contract; no inter-module coordination needed during stage 1.

## Task A.1: extract Value + add basic types (sym, form)

**Files:** `crates/zig-substrate/src/{value,sym,form}.zig`

extract Value from main.zig into its own file. add SymTable + Form structures.

- [ ] **Step 1:** create `src/value.zig` containing the `Value` union from main.zig. add helper methods: `isTruthy`, `isNil`, `asFormId`, `asSym`, `asInt`, `equals` (by-value equality).

- [ ] **Step 2:** create `src/sym.zig` with `SymTable`: `intern(name: []const u8) !SymId`, `resolve(SymId) []const u8`. backing: `std.StringHashMap(u32)` for interning + `std.ArrayList([]const u8)` for resolution.

- [ ] **Step 3:** create `src/form.zig` with `Form`: proto: Value, slots: ArrayHashMap, handlers: ArrayHashMap, meta: ArrayHashMap, frozen: bool. ArrayHashMap because insertion-order matters (determinism law D5).

- [ ] **Step 4:** update main.zig to import these.

- [ ] **Step 5:** smoke: `zig build run` should still print the FormId/Value smoke.

## Task A.2: heap.zig

**Files:** `crates/zig-substrate/src/heap.zig`

- [ ] **Step 1:** `Heap` struct: forms: `std.ArrayList(Form)`, redirects: `std.AutoArrayHashMap(FormId, FormId)`.

- [ ] **Step 2:** methods: `init(allocator)`, `alloc(form) !FormId`, `get(id) *const Form`, `getMut(id) *Form`, `resolveId(id) FormId` (chase redirects, bounded loop), `become_(a, b) void`.

- [ ] **Step 3:** unit smoke in main.zig: alloc 3 Forms, become a→b, verify dereference chases.

## Task A.3: opcodes.zig + bytecode.zig

**Files:** `crates/zig-substrate/src/{opcodes,bytecode}.zig`

- [ ] **Step 1:** `opcodes.zig`: define `Op` as a tagged union matching V4 spec §2 (LoadConst, LoadHere, Send, SendSelf, …, Resume — all 24). define `Tag` enum with the byte values from §2.

- [ ] **Step 2:** `bytecode.zig`: `encodeOp(op: Op, writer: anytype) !void` and `decodeOp(reader: anytype) !Op`. big-endian operand encoding per V4 spec §4. exact byte layout from §3.

- [ ] **Step 3:** unit smoke: encode every opcode, decode back, verify roundtrip operand equality. (~24 tiny tests in main.zig as inline `if (op != decoded) @panic("roundtrip failed for X")`.)

## Task A.4: protos.zig + world.zig

**Files:** `crates/zig-substrate/src/{protos,world}.zig`

- [ ] **Step 1:** `Protos` struct: a flat record of FormIds for the standard protos (object, nil, bool, integer, char, sym, cons, string, bytes, method, chunk, closure, env, foreign_handle, table, frame, macros, opcode).

- [ ] **Step 2:** `bootstrap(heap)` allocates each proto as a fresh Form with `proto: Value::Form(parent)` chain. matches `crates/substrate/src/protos.rs::Protos::bootstrap`.

- [ ] **Step 3:** `World` struct: heap, syms, protos, chunk side-tables (chunk_bytecode: AutoArrayHashMap<FormId, []u8>, chunk_consts, chunk_ics), here_form: FormId, macros_form: FormId, vm: Vm.

- [ ] **Step 4:** `World.init(allocator)`: heap.init, syms.init, protos.bootstrap, alloc here_form + macros_form, bind `$here` self-referentially.

## Task A.5: vm.zig (the dispatch loop)

**Files:** `crates/zig-substrate/src/vm.zig`

- [ ] **Step 1:** `Frame` struct: chunk: FormId, pc: u32 (byte offset into chunk_bytecode), env: FormId, self_: Value, stack_base: u32, defining_proto: FormId.

- [ ] **Step 2:** `Vm` struct: stack: `std.ArrayList(Value)`, frames: `std.ArrayList(Frame)`, last_send_sel: ?SymId.

- [ ] **Step 3:** `step(world: *World) !void`: read op at frame.pc via bytecode.decodeOp, dispatch on op tag, execute. one handler per op. ~24 handlers; each ~5-15 lines.

- [ ] **Step 4:** `runTop(world: *World, chunk: FormId) !Value`: push frame, loop `step` until frame stack empty, return result.

- [ ] **Step 5:** **tail-call dispatch optimization** (optional in stage 1): replace the switch with separate handler functions chained via `@call(.always_tail, dispatch_table[next_op], .{vm, world})`. 2-3x faster but ~50% more code. **Skip in stage 1; add post-smoke.**

- [ ] **Step 6:** smoke: hand-construct a chunk that does `PushConst Int(3); Return`. Run via runTop. Verify result == Int(3).

## Task A.6: intrinsics.zig (native methods)

**Files:** `crates/zig-substrate/src/intrinsics.zig`

- [ ] **Step 1:** identify minimum-viable intrinsic set for the smoke programs:
  - arithmetic: `:+:`, `:-:`, `:*:`, `:/:` on Integer
  - comparison: `:=`, `:<`, `:>` on Integer + Bool
  - `:!!` on Object/Nil/Bool (truthiness)
  - `:proto`, `:is`, `:identity` on Object (reflection)
  - `:slot:`, `:slotSet!:` on Object (slot access)
  - `:car`, `:cdr` on Cons
  - `:bind:to:`, `:set:to:`, `:lookup:` on Env
  - `:current` (push frame env)
  - `:say:` on $out (terminal print — for hello world)

  ~30 natives, smallest viable surface.

- [ ] **Step 2:** install each as a `NativeFn` (`fn(world: *World, self_: Value, args: []const Value) anyerror!Value`) bound to the appropriate proto's handlers table.

- [ ] **Step 3:** smoke: with a chunk that does `LoadConst Int(1); LoadConst Int(2); Send :+: argc=1 ic=0; Return`, verify dispatch finds Integer:+:, calls the native, returns Int(3).

## Task A.7: image.zig (deserializer)

**Files:** `crates/zig-substrate/src/image.zig`

per V4 spec §10. read a .vat file into a fresh World.

- [ ] **Step 1:** `parseManifest(json: []const u8) Manifest` — parse the manifest.json wrapper. (could use `std.json` in zig 0.16.)

- [ ] **Step 2:** `loadVatImage(world: *World, bytes: []const u8) !void`:
  - verify magic "MVAT" + version 4
  - read header
  - read SymTableSection — intern each in order
  - read FormSection — alloc each in order; FormId payload = position
  - read ChunkSection — populate chunk_bytecode/consts/params
  - read NativeRefsSection — re-bind native methods by name (look up in process-wide intrinsics table)
  - read McoBindingsSection — load wasm bytes from mcos/ cache; instantiate
  - read FarRefsSection — populate far_ref_table (defer resolution to first use)
  - verify ImageHash footer

- [ ] **Step 3:** smoke: serialize a tiny World by hand (in code), write to bytes, deserialize, verify the deserialized World has the same heap+syms+chunks.

---

# Track B — OCaml Seed

Five parallel sub-tracks. Each subagent works from the V4 spec + a tight prompt.

## Task B.1: project skeleton + AST

**Files:** `crates/ocaml-seed/{dune-project,src/dune,src/ast.ml}`

- [ ] **Step 1:** `dune-project`:
```
(lang dune 3.0)
(name moof_seed)
```

- [ ] **Step 2:** `src/dune`:
```
(library
 (name moof_seed)
 (libraries))
```

- [ ] **Step 3:** `bin/dune`:
```
(executable
 (name seed)
 (public_name moof-seed)
 (libraries moof_seed))
```

- [ ] **Step 4:** `src/ast.ml`: the source-Form ADT. matches moof's reader output.
```ocaml
type form =
  | Nil
  | Bool of bool
  | Int of int  (* int63 — moof uses i48 *)
  | Char of int
  | Sym of string
  | Str of string
  | Cons of form * form
  | Vec of form list  (* for #[...] table literals *)
  (* ... *)

let car = function
  | Cons (h, _) -> h
  | _ -> failwith "car: not a cons"
;;
```

## Task B.2: reader.ml (S-expr + bracket parser)

**Files:** `crates/ocaml-seed/src/reader.ml`

mirrors `crates/substrate/src/reader.rs`. ~1300 lines of rust → expect ~600-800 LoC of OCaml.

- [ ] **Step 1:** lexer that handles:
  - whitespace + comments (`;` to EOL)
  - identifiers (symbols)
  - integers, floats, characters (`#\a`), strings (`"..."`)
  - parens `()`, brackets `[]` (send syntax), curlies `{}` (object literals)
  - special chars: `'` (quote), `` ` `` (quasiquote), `,` (unquote), `,@` (unquote-splice), `#[`, `#\`, `#true`, `#false`

- [ ] **Step 2:** parser that produces `ast.form` values.

- [ ] **Step 3:** smoke: read `[1 + 2]` → `Cons(Sym "__send__", Cons(Int 1, Cons(Sym "+", Cons(Int 2, Nil))))` or equivalent.

## Task B.3: opcodes.ml + bytecode.ml

**Files:** `crates/ocaml-seed/src/{opcodes,bytecode}.ml`

mirrors zig's opcodes.zig + bytecode.zig — same V4 spec contract.

- [ ] **Step 1:** `opcodes.ml`:
```ocaml
type op =
  | PushNil
  | PushTrue
  | PushFalse
  | LoadConst of int  (* u16 idx *)
  | LoadSelf
  | LoadHere
  | LoadName of int  (* SymId u32 *)
  | Pop
  | Dup
  | Send of { selector: int; argc: int; ic_idx: int }
  | TailSend of { selector: int; argc: int }
  | SuperSend of { selector: int; argc: int; ic_idx: int }
  | SendDynamic of { argc: int; ic_idx: int }
  | SendSelf of { selector: int; argc: int; ic_idx: int }
  | SendHere of { selector: int; argc: int; ic_idx: int }
  | TailSendSelf of { selector: int; argc: int }
  | TailSendHere of { selector: int; argc: int }
  | Jump of int  (* i16 offset *)
  | JumpIfFalse of int
  | JumpIfTrue of int
  | Return
  | PushClosure of { chunk: int }  (* u32 FormId *)
  | Suspend of { promise_ic: int }
  | Resume of { frame_ic: int }
```

- [ ] **Step 2:** `bytecode.ml`: `encode_op : op -> bytes` and `decode_op : bytes -> int -> op * int`. big-endian; tags per V4 spec §3.

- [ ] **Step 3:** smoke: same as Track A's bytecode roundtrip, but in OCaml.

## Task B.4: compiler.ml

**Files:** `crates/ocaml-seed/src/compiler.ml`

the core — port `crates/substrate/src/compiler.rs` to OCaml. ~700 LoC rust → ~500-600 LoC OCaml.

- [ ] **Step 1:** chunk-building state:
```ocaml
type chunk_builder = {
  mutable ops: op list;  (* reversed during build *)
  mutable consts: value list;
  mutable ic_count: int;
  params: int list;  (* SymId list *)
  source: form;
}
```

- [ ] **Step 2:** `compile_form : form -> chunk_builder -> bool -> unit` (form + chunk + tail). dispatch on form's shape:
  - Nil/Bool/Int/Char/Str → LoadConst
  - Sym "self" → LoadSelf
  - Sym "$here" → LoadHere
  - Sym _ → LoadName
  - Cons → `compile_list`

- [ ] **Step 3:** `compile_list`: detect head:
  - `__send__` → compile_send
  - `quote` → compile_quote (LoadConst with the form)
  - `set!` → compile_set
  - `def` → compile_def
  - `if` → compile_if
  - `fn` → compile_fn
  - `defmacro` → compile_defmacro
  - `do` → compile_do
  - `let` → compile_let
  - else → compile_call (treat as `[head args...]`)

- [ ] **Step 4:** each compile_X follows V4 emission rules from spec §5:
  - compile_def emits SendHere :bind:to: (since def is the V3 macro expansion)
  - compile_set emits Send :current then SendDynamic :set:to: (or compile to the [[Env current] set: 'name to: value] shape)
  - compile_if emits the V3 Send-based shape with the Task 13 peephole
  - compile_send detects self / $here receivers and emits SendSelf / SendHere

- [ ] **Step 5:** include the V3 const-fold peephole (`[1 + 2] → LoadConst 3`).

- [ ] **Step 6:** smoke: compile `[1 + 2]` → emit `LoadConst 3; Return`.

## Task B.5: image.ml (serializer)

**Files:** `crates/ocaml-seed/src/image.ml`

per V4 spec §10 — write a per-vat .vat file.

- [ ] **Step 1:** `vat_image`: record holding all the sections we need to write (heap forms, sym table, chunks, native refs, mco bindings, far-refs).

- [ ] **Step 2:** `serialize_vat (vat: vat_image) : bytes` — produce the full file matching V4 spec §10.3 byte layout. include the magic "MVAT", version 0x0004, header, sections, footer hash.

- [ ] **Step 3:** smoke: serialize a tiny World by hand, write bytes, hash matches expected blake3.

## Task B.6: seed CLI

**Files:** `crates/ocaml-seed/bin/seed.ml`

- [ ] **Step 1:** parse argv:
  - `seed compile <file.moof>` — print the V4 bytecode for the file
  - `seed build-image --root lib/ --entry main.moof --output system.vat` — full bootstrap-image build

- [ ] **Step 2:** `compile <file>` reads the file, parses, compiles each top-level form, prints disassembly + hex dump of bytecode.

- [ ] **Step 3:** `build-image` (more elaborate — multi-task subagent):
  - read main.moof + transitively load all referenced files via `[$transporter load: ...]` (just trace the load chain statically)
  - compile each form in order; populate a virtual World
  - serialize the World as a .vat file via image.ml

- [ ] **Step 4:** smoke: `seed compile /tmp/test.moof` shows V4 bytes for `[1 + 2]`.

---

# Track D — Image Format (shared)

The image format is fully specified in V4 §10. This track has minimal NEW work:

## Task D.1: confirm spec is implementable

- [ ] read V4 §10 carefully. flag any ambiguity:
  - byte layout for Value union (per V4 §4)
  - exact ordering rules for slots/handlers/meta (insertion order per D5)
  - blake3 hashing scope (footer excluded? yes, per §10.3)
  - native-ref re-binding rules (look up by name in process-wide intrinsics table)
  - mco-binding hash format (32-byte blake3)

if anything is unclear: pause, write a one-paragraph clarification, push to the spec. don't start coding around an ambiguity.

## Task D.2: produce a reference fixture

- [ ] hand-craft a tiny vat-image (smallest non-trivial: a vat with one Form, one chunk, one sym). encode the bytes by hand following V4 §10.3.

- [ ] commit to `fixtures/v4-bytecode/tiny.vat` + `tiny.vat.json` (the manifest) + `tiny.expected.txt` (a human-readable description of what's in it).

- [ ] both Track A's deserializer and Track B's serializer will validate against this fixture as their first integration test.

---

# Track C — Integration & Smoke

C tasks BLOCK on A + B having shipped their minimum viable.

## Task C.1: bytecode roundtrip (A.3 ↔ B.3)

- [ ] **Step 1:** OCaml seed emits the bytecode for `LoadConst 0; Send :+: argc=1 ic=0; Return` (with a const-pool entry of `Int 2`). hex-dump to a file.

- [ ] **Step 2:** Zig substrate reads the same bytes, decodes via bytecode.zig, verifies each op matches.

- [ ] **Step 3:** Reverse: Zig encodes the same op stream, OCaml decodes, matches.

- [ ] **Step 4:** smoke roundtrip for every opcode. fail-fast on any mismatch.

## Task C.2: tiny-program smoke (A + B end-to-end)

- [ ] **Step 1:** `echo '[1 + 2]' | moof-seed compile - > /tmp/p.bc`

- [ ] **Step 2:** `moof-zig /tmp/p.bc` → output: `3`

- [ ] **Step 3:** repeat for: `(do (def x 42) x)`, `(if #true 1 2)`, `[$here parent]`, `[Object new]`. each must produce the expected result.

## Task C.3: stdlib bootstrap smoke

- [ ] **Step 1:** `moof-seed build-image --root lib/ --entry main.moof --output /tmp/system.vat`

- [ ] **Step 2:** `moof-zig /tmp/system.vat` boots successfully (no panics; system vat loaded).

- [ ] **Step 3:** `echo '(do (def x [Object new]) [x foo])' | MOOF_VAT=/tmp/system.vat moof-zig` runs against the loaded stdlib.

**This is the BIG integration moment. Expect bugs.** Iterate.

---

# Final — Rust Deprecation

ONLY after Track C smoke passes:

## Task Final.1: switch default CLI

- [ ] **Step 1:** add a top-level `moof` wrapper script (or alias) that invokes `moof-zig` with the bundled `system.vat`.

- [ ] **Step 2:** old `moof` (rust) becomes `moof-rs` for transition period.

- [ ] **Step 3:** smoke: `moof '[1 + 2]' → 3` via zig.

## Task Final.2: delete the rust substrate runtime

- [ ] **Step 1:** delete `crates/substrate/src/{reader,compiler,vm,opcodes,nursery,world,intrinsics,table,wasm,transporter}.rs`. that's the runtime; the data types (form, value, sym, foreign, heap) might also go.

- [ ] **Step 2:** keep `crates/mco-pack/` and `crates/abi/` + `crates/abi-rust/` — those are mco utilities, not runtime.

- [ ] **Step 3:** update `crates/Cargo.toml` workspace to remove `substrate` member.

- [ ] **Step 4:** smoke: all CLI tests still pass via moof-zig.

- [ ] **Step 5:** commit + push:
```
moof: delete rust substrate; moof-zig + ocaml-seed are canonical
```

---

## execution plan: parallelization map

**stage 1 — kick off ~10-12 parallel subagents simultaneously:**

| agent | track | task | scope |
|---|---|---|---|
| α1 | A | A.1 | value+sym+form extracted; main.zig updated |
| α2 | A | A.2 | heap.zig + become roundtrip |
| α3 | A | A.3 | opcodes.zig + bytecode.zig + roundtrip smoke |
| α4 | A | A.4 | protos + world init |
| α5 | A | A.5 | vm + dispatch + smoke `LoadConst Int(3); Return` |
| α6 | A | A.6 | intrinsics (~30 natives) |
| α7 | A | A.7 | image.zig deserializer |
| β1 | B | B.1+B.2 | project skeleton + reader |
| β2 | B | B.3 | opcodes.ml + bytecode.ml + roundtrip |
| β3 | B | B.4 | compiler.ml |
| β4 | B | B.5+B.6 | image.ml + seed CLI |
| δ1 | D | D.2 | reference fixture (tiny.vat) |

**stage 2 — sequential integration after stage 1:**

| step | task |
|---|---|
| C.1 | bytecode roundtrip A.3 ↔ B.3 |
| C.2 | tiny-program smoke A + B |
| C.3 | stdlib bootstrap (the big one) |

**stage 3 — fix loop (parallel as needed):**

dispatch fix subagents for any integration failures.

**stage 4 — sequential final:**

| step | task |
|---|---|
| Final.1 | switch default CLI to zig |
| Final.2 | delete rust substrate |

---

## risks + mitigations

1. **zig 0.16 stdlib churn.** every API used in Track A could break in a point release. *mitigation:* pin to 0.16.0 in `build.zig`. document the version. don't update zig casually.

2. **OCaml seed scope creep.** moof has a lot of macros and sugar. seed might not handle every edge case. *mitigation:* seed compiles ENOUGH for `lib/main.moof` to bootstrap. once the stdlib loads, `parser.moof` and `compiler.moof` take over for everything else. seed never sees user code post-bootstrap.

3. **bytecode contract drift.** A and B independently implement V4 §3. they might disagree on edge cases (jump offset signedness, big-endian-ness, etc.). *mitigation:* Track C.1 catches this immediately. fix and rerun.

4. **stdlib bootstrap is the long pole.** the moof stdlib has 23 files with 1000+ forms, macros that emit complex bytecode, and proto-chain setup. *mitigation:* tackle simplest files first (early/00-cons, early/01-nil); ship those green; iterate.

5. **wasm mco instantiation in zig.** wasmtime has a C api; zig has bindings. *mitigation:* use wasm3 or wasmer instead if wasmtime is hard. lazy-instantiate (V4 §10 open question 4).

6. **the rust substrate stops working mid-session.** *mitigation:* DON'T touch rust until Track C.3 smoke passes. it's the safety net. delete it LAST.

7. **session timeout.** *mitigation:* aim for end-of-stage-2 (smoke for tiny programs) as the session goal. stages 3-4 spill to next session.

---

## exit criteria for this session

minimum viable session goal:

- [ ] zig substrate: heap + vm + intrinsics + image deserializer working
- [ ] OCaml seed: reader + compiler + image serializer working
- [ ] roundtrip smoke: encode in OCaml, decode in zig, match (all 24 opcodes)
- [ ] tiny-program smoke: `moof-zig <(moof-seed compile '[1 + 2]')` → 3
- [ ] CLI verification: `[1 + 2]`, `(if #true 1 2)`, `(do (def x 1) x)` all work end-to-end

stretch session goals (if time):

- [ ] stdlib bootstrap smoke (Track C.3 passes)
- [ ] rust deprecation initiated (Final.1)

absolute session goals (probably next session):

- [ ] full rust deletion
- [ ] tail-call-threaded dispatch
- [ ] performance benchmark

---

## see also

- spec: `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` (the contract for all tracks)
- V3 plan: `docs/superpowers/plans/2026-05-09-vat-V3-here-form.md` (predecessor wave)
- the 2026-05-10 conversation: polyglot language choices, opcode fusion design, image format brainstorm
