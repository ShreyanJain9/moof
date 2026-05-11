# self-host + rust deletion — design

> **status:** brainstormed 2026-05-10. ready for writing-plans.
> **scope:** replace rust runtime with `moof` (zig substrate) + `moof-seed`
> (ocaml build tool) via `parser.moof` and a V4-aware `compiler.moof`.
> minimal-subset bootstrap; ocaml-seed never sees macro-using code.
> **precedes:** vats-V4 (multi-vat container).

## 1. context

state at HEAD `380203b`:

- rust at build time produces `system.vat` (21.6 MB V4 vat-image)
- `moof-zig` (zig substrate) loads it — world alive but inert; doesn't yet
  run moof code
- `moof-seed` (ocaml) exists; can parse + compile simple programs; byte-
  compatible with V4 spec
- `compiler.moof` lives in `lib/compiler/*.moof` (836 LoC, 4 files); was
  V3-targeted; `[$compiler useMoof]` flip already wired in
  `lib/main.moof:17`
- no `parser.moof` yet — only rust `reader.rs` and ocaml `reader.ml`

what just shipped (vm-V4, separate from the vats numbering):

- 24-op byte-tagged V4 ISA, big-endian fixed-width operands
- per-vat image format per `2026-05-10-vm-V4-opcodes-design.md` §10
- polyglot substrate alive end-to-end

what's next (this design):

- finish the self-host story: `parser.moof` + V4-correct `compiler.moof` +
  minimal ocaml-seed bootstrap
- switch user-facing binary to `moof` (zig); demote rust to `moof-rs`
  during transition, then delete
- rust runtime deleted entirely
- precondition for vats-V4 (multi-vat container) which follows

## 2. goal

a `moof` binary written entirely in zig + moof, with `moof-seed` as a
build-only tool. rust deletes after the cycle
`moof-seed → seed.vat → moof → system.vat` works end-to-end.

post-design exit criteria:

- [ ] **W1** — `compiler.moof` emits V4-correct bytecode; matches rust
  compiler byte-for-byte on a representative form corpus
- [ ] **W2** — `parser.moof` produces Form trees identical to rust
  `reader.rs` for every file in `lib/`
- [ ] **W3** — `moof-seed` minimized to handle only the 7-special-form
  subset + V4 byte emission + seed.vat serialization
- [ ] **W4** — `moof` (zig) executes loaded chunks; `[1 + 2]` evaluates
  to `3` through stdlib protocol dispatch
- [ ] **W5** — `moof-seed → seed.vat → moof → system.vat` round-trip
  works; rust runtime deleted; user-facing binary named `moof`

## 3. end-state architecture

### 3.1 runtime — every user-facing operation

```
user source bytes (argv / stdin / file)
       ↓ moof wraps as String form
       ↓ [Parser parse: src]            ← in-image moof code
Form tree
       ↓ [Compiler compileTop: form]    ← in-image moof code
V4 chunk
       ↓ vm.runTop
result
```

zig substrate provides: heap + GC, V4 vm + decoder, intrinsic table
(~50 natives), image loader, transporter, world-serialize intrinsic.
**does not parse. does not compile.** `parser.moof` and `compiler.moof`
live as ordinary closures inside `system.vat`.

### 3.2 build-time — one-shot, dev-only

```
lib/parser/*.moof + lib/compiler/*.moof + main.moof
       ↓ moof-seed parse + compile (minimal subset only)
seed.vat   (small: Parser + Compiler + transporter chunks; no stdlib,
            no macros, no quasiquote)
       ↓ moof loads seed.vat, runs main
       ↓ [$transporter load: "..."] chain runs file-by-file through
         in-image Parser → in-image Compiler → execute
world fills up (early/*, stdlib/*, mcos/*)
       ↓ moof serializes world via :serialize-world intrinsic
system.vat   (full bootstrap, ~21 MB, ships to users)
```

`moof-seed`: parse + compile + emit V4 bytes + serialize seed.vat.
**no moof VM inside.** macros expand inside `moof` at build time using
the same compiler that runs at user-time. one compiler, two contexts.

## 4. the minimal subset

### 4.1 what ocaml-seed handles

| category | included | banned |
|---|---|---|
| atoms | `Int`, `Char`, `Sym`, `String`, `#true`/`#false`, `nil` | all others |
| structure | `(...)` cons lists, `'foo` quote, `[recv msg: arg]` send | `` ` ``, `,`, `,@`, `#[`, `;` (send-cascade) |
| special forms | `def`, `set!`, `if`, `fn`, `do`, `let`, `quote` | everything else (`cond`, `when`, `defmethod`, `defmacro`, `defproto`, `match`, pattern params, ...) |
| sends | `[recv msg: arg]` desugars at parse to `(__send__ recv 'msg arg ...)` | cascading sends |
| primitives | `setHandler!`, `cons`, `list` as ordinary primitive sends | — |
| string escapes | `\n \t \\ \" \0 \x` | unicode escapes deferred to parser.moof |
| reader extensions | `#\char`, `#true`, `#false` | `#[`, custom reader macros |

### 4.2 what this constrains

- `parser.moof`, `compiler/*.moof`, `main.moof` are written in *exactly*
  this subset
- they use raw `setHandler!` calls (no `defmethod` sugar)
- they construct cons trees by hand (no quasiquote)
- they branch with raw `if` (no `cond` / `when`)
- spartan, but every dispatch case in `parser.moof` is right there in
  the file — no macro magic anywhere in the bootstrap path

### 4.3 what this does NOT constrain

every file loaded *after* the seed (early/*, stdlib/*, mcos/*, user
code) uses the full moof surface. macros, quasiquote, send-cascade,
defmethod, pattern-match params — all available, because they're
handled by in-image `Parser` + `Compiler`, not by `moof-seed`.

## 5. workstream decomposition

### W1 — `compiler.moof` V4 audit

**goal:** make `lib/compiler/*.moof` emit V4-correct bytecode.

read each `(setHandler! Compiler 'compileX:...)` clause in
`00-helpers.moof`, `01-dispatch.moof`, `02-special.moof`,
`03-control.moof`. for each emission:

1. opcode byte-tag correct per V4 spec §3?
2. operands encoded big-endian fixed-width per spec §4?
3. emission rule right per spec §5 — especially for Send / TailSend /
   SuperSend / SendDynamic / SendSelf / SendHere?
4. peepholes preserved: const-fold (`[1 + 2]` → `LoadConst 3` when both
   operands literal), if-jump peephole

deliverable: byte-differ test. compile a corpus of ~30 representative
forms via rust compiler AND moof compiler; assert byte-identical
output.

files: `lib/compiler/00-helpers.moof`, `01-dispatch.moof`,
`02-special.moof`, `03-control.moof`. plus a small differ tool (rust
side, ~30 LoC) printing first divergent op.

estimated effort: ~3–4h. mechanical.

### W2 — `parser.moof`

**goal:** new files in `lib/parser/`. ~800 LoC total. produces Form
trees from String input.

suggested structure:

| file | role |
|---|---|
| `lib/parser/00-lexer.moof` | char-by-char string → token stream. tokens are simple cons-cells `(type . value)`. |
| `lib/parser/01-tokens.moof` | token-type predicates, position tracking (`FormLoc`). |
| `lib/parser/02-parser.moof` | token-stream → Form tree. recursive-descent. one method per syntactic category. |
| `lib/parser/03-bootstrap.moof` | `Parser` singleton; `[Parser parse: src]` entry point; `[$reader useMoof]` flip. |

constraints:

- written in the minimal subset (§4)
- output Form tree byte-comparable to rust `reader.rs` on identical input
- attaches `:source-loc` meta to each Form (for error reporting)
- handles every syntactic form used anywhere in `lib/` (audit by reading
  every file in `early/`, `stdlib/`, `mcos/`)

files: `lib/parser/*.moof` (new), `lib/main.moof` (add load step before
the compiler load — parser must be available so `[$reader useMoof]`
flips before `[$compiler useMoof]`).

estimated effort: ~6–8h. **biggest workstream.** subdivides into
lexer / token-utils / parser / bootstrap agents.

### W3 — `moof-seed` minimization

**goal:** strip `crates/ocaml-seed/src/` to handle only the minimal
subset.

retain:

- atoms + cons + `[...]` send + quote (parse + compile)
- 7 special forms (`def`, `set!`, `if`, `fn`, `do`, `let`, `quote`)
- V4 byte emission (audit: must be V4, not V3)
- image serializer producing seed.vat

delete (or stub):

- macro expansion paths
- quasiquote / unquote handling
- `defmethod` / `defproto` / `cond` / `when` / pattern params
- send-cascade `;` desugaring
- string interpolation
- any VM / interpreter code (ocaml-seed never executes moof; `moof`
  does that)

add:

- `moof-seed build-seed --root lib/ --output seed.vat` — compiles the
  minimal-bootstrap files; writes seed.vat

estimated effort: ~4–5h. mostly mechanical reduction, plus V4 byte-
emission audit (mirror rust's `v4_export` byte layout exactly).

### W4 — `moof` (zig) proof-of-life

**goal:** zig substrate executes loaded chunks. the world stops being
inert. (this is "track A" from `NEXT_SESSION.md`.)

steps:

1. `moof exec <vat-file> <chunk-id>` command. instantiates the chunk's
   frame, runs `vm.runTop`, prints result.
2. fix bugs that surface: chunk side-tables, IC slot init, dispatch
   against re-bound natives, SymId alignment.
3. smoke: `moof exec system.vat <chunk-of-(1+2)>` outputs `3` via
   `Integer:+:`.
4. add `:serialize-world` intrinsic — emits current World as V4 vat-
   image bytes. needed for W5's seed-to-system.vat step.

files: `crates/zig-substrate/src/{main,vm,intrinsics,image}.zig`.

estimated effort: ~4–5h. **riskiest workstream** — bug surface is
unknown until execution starts hitting real chunks. budget fix-loop
time.

### W5 — integration + rust deletion + rename

**goal:** wire it all together. flip the canonical binary. delete rust.

steps (binary names below use the transitional `moof-zig` until the
rename in step 5):

1. `moof-seed build-seed --root lib/ --output seed.vat` succeeds.
2. `moof-zig run seed.vat` boots: loads seed.vat, runs main,
   transporter chain pulls in `early/*` + `stdlib/*` + `mcos/*`, world
   fills.
3. `moof-zig serialize <output>` dumps the loaded world as system.vat
   (uses the `:serialize-world` intrinsic added in W4).
4. `system.vat` produced this way is functionally equivalent to rust's
   current export (corpus passes against both; hash-compare both
   images).
5. **rename binaries.** `moof-zig` → `moof` (the user-facing runtime).
   rust's current `moof` → `moof-rs` for one transition session, then
   deleted. update any scripts / docs / `NEXT_SESSION.md` references.
6. delete `crates/substrate/src/{reader,compiler,vm,opcodes,intrinsics,
   nursery,transporter,...}.rs`. retain `crates/mco-pack/` and
   `crates/abi/` if still useful for mco builds.
7. `Cargo.toml` workspace: remove `substrate` member.
8. commit + push.

estimated effort: ~3–4h.

## 6. parallelization map for subagents

### stage 1 — kick off simultaneously

| agent | stream | scope |
|---|---|---|
| α | W1 | `compiler.moof` V4 audit + byte-differ test |
| β1 | W2 | `parser/00-lexer.moof` |
| β2 | W2 | `parser/01-tokens.moof` + `02-parser.moof` skeleton |
| β3 | W2 | `parser/03-bootstrap.moof` + main.moof wiring + cover-every-syntax audit |
| γ | W3 | ocaml-seed minimization + V4 byte-emission audit |
| δ | W4 | zig substrate `exec` + `:serialize-world` intrinsic |

stage 1 = **6 agents in parallel**. all independent. the V4 spec is
the shared contract.

### stage 2 — integration, sequential

| agent | task |
|---|---|
| ε | W5: round-trip + rust deletion + binary rename |

stage 2 = 1 agent. blocks on all of stage 1.

### fix loop

if integration surfaces divergences, dispatch targeted fix agents per
failure mode (byte mismatch, Form-tree mismatch, dispatch bug, etc.).

## 7. risks + mitigations

1. **V4 byte mismatch invisible without differ.**
   *mitigation:* byte-differ tool is part of W1's exit. ~30 LoC rust to
   print first divergent op when corpus diff fails.

2. **`parser.moof` source-loc tracking.** rust reader produces
   `FormLoc` meta. `parser.moof` must too, from day 1, or we lose
   error messages.
   *mitigation:* include in W2's exit criteria — every Form's `:meta`
   has `:source-loc` populated identically to rust output.

3. **minimal-subset linter.** if any minimal-bootstrap file uses a
   banned form (quasiquote, defmethod, etc.) the subset breaks.
   *mitigation:* tiny linter (~50 LoC moof or ~30 LoC shell+grep)
   scans `lib/parser/*.moof + lib/compiler/*.moof + lib/main.moof`
   for banned forms. runs as part of W2 and W3 exit checks.

4. **W4 bugs are unknown unknowns.** chunk side-table alignment, IC
   slot init, sym-table re-interning, native re-binding by name.
   *mitigation:* W4 is the riskiest stream; allocate extra fix-loop
   budget. start it earliest so bugs surface while other streams run.

5. **system.vat byte-equivalence between rust-built and polyglot-built
   paths.** insertion order of slots/handlers/meta is determinism law
   D5. if the two builds produce different bytes, debugging is hard.
   *mitigation:* hash-compare final `system.vat` after both builds; if
   different, dispatch a differ agent that walks both images section
   by section.

6. **`moof-seed build-image` currently stubbed.** NEXT_SESSION.md
   flagged this; current ocaml-seed only writes a SKELETON .vat.
   *mitigation:* W3 explicitly replaces it with `build-seed` (just the
   minimal subset, not the full bootstrap). full bootstrap is `moof`'s
   job, not ocaml-seed's.

7. **rust deletion regret window.** main branch loses the rust
   runtime. if a bug surfaces post-deletion, hard to recover.
   *mitigation:* keep a `rust-fallback` branch retained for at least
   one session after deletion. main branch: deleted.

## 8. testing strategy

### per workstream

- **W1:** byte-differ corpus. for each of ~30 representative forms,
  compile via rust → bytes_a, via moof → bytes_b. assert
  `bytes_a == bytes_b`.
- **W2:** Form-tree differ. for each file in `lib/`, parse via rust
  → tree_a, via parser.moof → tree_b. assert structural equality
  (insertion-order-aware, including `:source-loc` metadata).
- **W3:** seed.vat reproducibility. build seed.vat twice; hash-compare.
  assert identical bytes (determinism law D5).
- **W4:** moof exec smokes:
  - `[1 + 2]` → `3`
  - `(do (def x 42) x)` → `42`
  - `(if #true 1 2)` → `1`
  - `[Object new]` → `<object>` (non-nil reflection)
  - each runs end-to-end with real native dispatch through stdlib.
- **W5:** end-to-end. `moof-seed → seed.vat → moof boots → transporter
  loads stdlib → moof serializes system.vat → moof system.vat → REPL
  works`. round-trip the entire stdlib through the polyglot stack.

### integration

byte-equivalence test: rust-built system.vat and polyglot-built
system.vat should be hash-comparable. if they differ, why? (insertion
order? FormId allocation order? blake3 footer scope?) — that's where
the differ agent earns its keep.

## 9. vats-V4 preconditions

self-host done unlocks vats-V4 (multi-vat container) because:

- single substrate (zig) to refactor `World` → `World + Vat` in, not two
- no rust runtime to keep in sync during the carve-up
- in-image `Compiler` means live-editing the compiler at a vat boundary
  is feasible — moldability survives across vats
- per-vat `parser.moof` is the natural shape for sandboxed parsing in
  worker vats

specific things vats-V4 will lean on:

- the per-vat image format (already V4 spec'd, §10)
- the `:serialize-world` intrinsic added in W4 (becomes per-vat-
  serialize during the W→W+V carve-up)
- the in-image `Compiler` (so each vat can have its own — moldability
  across vats)

vats-V4 spec gets written *after* W5 lands, against the post-rust-
deletion codebase. ~1 day brainstorm + 2–3 sessions implementation
per the vats-spec §22 phasing.

## 10. open questions

- **Q1:** should `parser.moof` include a `read-from-port` style
  streaming API, or always slurp a full String? rust reader supports
  both. for V1 self-host, slurp is fine; streaming is a nice-to-have.
  *recommendation:* slurp-only for now.

- **Q2:** does `[$reader useMoof]` need an inverse for debugging?
  `[$compiler useMoof]` doesn't have one; rust runs both code paths
  in-process before the flip. once flipped, no going back.
  *recommendation:* same pattern. no inverse.

- **Q3:** should the byte-differ tool live in rust (where both
  compilers can be invoked) or moof (where we already have the runtime
  reflection)? rust-side is simpler since rust still exists during
  W1-W4.
  *recommendation:* rust-side. delete it alongside rust in W5.

## see also

- `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` — V4
  opcode + image format contract
- `docs/superpowers/plans/2026-05-10-vm-V4-polyglot-substrate.md` —
  predecessor implementation plan
- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md` —
  vats roadmap (next big thing after this)
- `NEXT_SESSION.md` — current state snapshot at HEAD `380203b`
- `docs/roadmap.md` — phase A-self-host + phase B context
