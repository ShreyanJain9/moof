# next session: opcode primitives → compiler.moof

> **mission: make `compile-form` runnable from inside moof. by
> session-end, `(compile-form '(if cond then else))` on the moof
> side returns a chunk-Form with the same opcodes the rust
> compiler emits today. once that holds for every special form,
> compiler.rs becomes a 100-line shim. the next session ports the
> reader; the one after that wires up `moof world` for real.**

---

## what stands today (commit `285a17e`)

277 tests passing. ten commits of moldability work landed in the
last session — the ledger:

| commit | landed |
|---|---|
| `fd56f53` | rust state about Forms moves onto Forms (R3, R6, D5) |
| `4b85b02` | `match` macro in moof + reader utf-8 fix + `raise:` |
| `5c8f8d0` | `\|args\| body` block sugar |
| `2c69075` | `defn` multi-clause |
| `f99b8e4` | `def` itself multi-clause |
| `a57dedc` | patterns v2 — `\|n :: T\|`, `\|n where p\|` |
| `ddd54e4` | macro precedence flip — user macros override built-ins |
| `06dc48b` | `let` → moof macro |
| `fc102c6` | `do` → moof macro (compromise: `__do__` rust shim) |
| `285a17e` | `do` → actual moof macro + `set!` walks parent chain |

### the residual rust special forms

eight forms still live as rust dispatch in `compile_expr`:

| form | what it really needs from rust |
|---|---|
| `if` | `JumpIfFalse` + `Jump` opcodes with patched offsets |
| `quote` | `LoadConst` of the (unevaluated) form-Form |
| `set!` | `StoreName` opcode |
| `fn` | chunk allocation + `PushClosure` |
| `def` | `DefineGlobal` opcode |
| `defmacro` | compile the body, register on the `Macros` Form |
| `__send__` | `Send` opcode (or `SuperSend` for `super`) |
| `do` | only the bootstrap fallback — moof macro intercepts new uses |

every one of these is a small bytecode-emission pattern. none
need ad-hoc rust logic *per form* — they all reduce to "emit
some opcodes, allocate a chunk, return a Form." that's the
opening for compiler.moof.

### where the rust line lives now

| crate file | LoC | self-host status |
|---|---:|---|
| `reader.rs` | 1634 | **target for parser.moof (session N+2)** |
| `compiler.rs` | 1049 | **target for compiler.moof (this session)** |
| `intrinsics.rs` | 2503 | legitimate — primordial natives the heap can't express |
| `vm.rs` | 757 | legitimate — bytecode interpreter hot loop |
| `world.rs` | 861 | legitimate — per-vat root + helpers |
| `lib/bootstrap.moof` | 1319 | grew +369 LoC last session as macros moved here |

once compiler.moof lands, `compiler.rs` can shrink to ~150 LoC:
the bootstrap shim that compiles compiler.moof itself, then
delegates everything else to the moof-side `compile-form`.

---

## the plan, by track

three tracks, in dependency order. each ends with
`cargo test --workspace` green before the next begins.

### track 1 — opcode-emission primitives

**why first.** compiler.moof needs to *emit bytecode*. that means
moof code must be able to (a) construct opcodes as values, (b)
collect them into a chunk's ops vector, (c) allocate a chunk-Form,
(d) register it in `world.chunk_ops` / `chunk_consts` / `chunk_ics`.
none of this is exposed to moof today — opcodes are purely a rust
construct.

**deliverables.**

- **the `Opcode` proto + per-variant constructors.** each `Op`
  variant becomes a Form on the heap with proto `Opcode`, slots
  carrying its operands. e.g. `(op-load-const 5)` returns a Form
  `{Opcode :kind 'LoadConst :idx 5}`. one constructor per variant
  in `intrinsics.rs`. ~80 LoC rust.
- **`(make-chunk params source)` global** — allocates an empty
  chunk-Form (proto: Chunk), returns its FormId. moof code then
  populates it via `(chunk-emit chunk op)`, `(chunk-add-const
  chunk val)`, `(chunk-add-ic chunk)`.
- **`(chunk-emit chunk op-form)`** — appends to
  `world.chunk_ops[chunk]`. opcode-form's slots get translated to
  the rust `Op` variant. ~50 LoC rust.
- **`(chunk-add-const chunk value)`** — appends to
  `world.chunk_consts[chunk]`, returns the index (a u16) so callers
  can pair it with a subsequent `LoadConst` emit.
- **`(chunk-add-ic chunk)`** — bumps the IC count, returns the
  fresh IC index.
- **`(finalize-chunk chunk)`** — currently a no-op (the side
  tables are already mutated in place); reserved for any
  one-shot post-processing the rust compiler does (currently
  none; finalize just builds the Form, which is already alloc'd).
- **`(jump-target chunk)`** — returns the current ops length, so
  callers can patch jumps after emitting their target. plus
  `(patch-jump chunk pos target)` to fix the offset.

**rust delta.** ~+250 LoC in `intrinsics.rs` (most of it is the
mechanical opcode-form ↔ Op-variant marshalling).

**moof delta.** ~+0 — this track is *about* exposing primitives;
no moof-side compiler yet.

**forcing function.** a hand-written moof program that
constructs a 3-op chunk (push 1, push 2, send `+`), allocates it
as a closure, and calls it — getting `3` back.

```moof
(let ((c (make-chunk '() nil)))
  (chunk-add-const c 1)
  (chunk-add-const c 2)
  (chunk-emit c (op-load-const 0))
  (chunk-emit c (op-load-const 1))
  (chunk-emit c (op-send '+ 1 (chunk-add-ic c)))
  [(make-closure c) call])  ; → 3
```

if that runs, the runway is open.

**doc gates.** `docs/laws/substrate-laws.md` L5 ("source is
canonical, bytecode is derived"). `docs/laws/reflection-contract.md`
R2 (chunks expose `:bytecodes`). after this track, those laws
hold *bidirectionally* — moof code can both *read* and *write*
bytecode.

---

### track 2 — port special forms to moof, smallest first

**why now.** with track 1 exposing the primitives, each special
form becomes a moof function ~30 LoC. ports happen one at a time;
each ends green; if a port introduces a bug, only that form is
suspect.

**order.** ranked by leverage and risk:

1. **`quote`** — simplest. `(compile-quote form chunk)` →
   `(chunk-add-const chunk form) (chunk-emit chunk
   (op-load-const idx))`. ~10 LoC moof. proves the primitives
   work end-to-end. *the smoke test for track 1.*
2. **`set!`** — `(compile-set form chunk)` → compile rhs,
   `(chunk-emit chunk (op-store-name name))`. ~15 LoC.
3. **`def`** — like set! but emits `DefineGlobal`. ~15 LoC. has
   to handle the multi-clause shape; that's already a moof macro
   (`defn`) that emits a single-binding `(def name (fn …))`, so
   the moof-side `compile-def` only handles single-binding.
4. **`__send__`** — emit `Send`/`SuperSend` opcodes. ~30 LoC
   moof. handles the `super` receiver case.
5. **`if`** — first non-trivial: needs the patch-jump dance.
   `(compile-if cond then else chunk)` emits cond, jump-if-false
   to a placeholder, then-branch, jump to end, patch first jump
   to here, else-branch, patch second jump to here. ~40 LoC.
6. **`fn`** — allocates a sub-chunk and emits `PushClosure`.
   ~50 LoC. recursive: the body compiles via the same
   `compile-form` we're building.
7. **`let`** — already a moof macro emitting `((fn …) …)`,
   so it falls out of `fn` + the call path. *no new code.*
8. **`do`** — already a moof macro. *no new code.*
9. **`defmacro`** — compile body, register on the Macros Form
   via `macro-register`. ~20 LoC.
10. **the call path** (`(callable arg…)`) — compile head, compile
    args, emit `Send :call argc`. ~25 LoC.

**the dispatcher** — `(compile-form form chunk)` is a giant
`match` over heads:

```moof
(def compile-form
  |form chunk|
  (match form
    ;; literals
    f where [[f proto] is Integer]   (compile-const f chunk)
    f where [[f proto] is Float]     (compile-const f chunk)
    …
    ;; symbol — env lookup
    sym where (__match-symbol? sym)  (compile-load-name sym chunk)
    ;; list — special form dispatch
    '(quote x)                       (compile-quote x chunk)
    '(set! n v)                      (compile-set n v chunk)
    '(if c t e)                      (compile-if c t e chunk)
    '(fn ps b)                       (compile-fn ps b chunk)
    '(def n v)                       (compile-def n v chunk)
    '(defmacro n p b)                (compile-defmacro n p b chunk)
    '(__send__ recv sel …args)       (compile-send recv sel args chunk)
    ;; macro? expand and recurse.
    '(name …args) where (registered-macro? name)
                                     (compile-form (macroexpand form) chunk)
    ;; otherwise: function call.
    '(callable …args)                (compile-call callable args chunk)))
```

note the `|form chunk|` block-sugar + `match` make this readable
in a way the rust version isn't.

**rust delta.** the rust special-form branches in `compile_expr`
go from "do the work" to "delegate to the moof `compile-form`."
when compiler.moof is registered, the rust path becomes a thin
"is the moof compiler ready? yes → call it. no (during bootstrap)
→ run the rust fallback" check. ~+30 LoC of dispatcher, ~-300 LoC
of special-form handlers (the rust ones become unreachable for
post-bootstrap compiles and can stay as bootstrap fallbacks).

**moof delta.** ~+500 LoC of `lib/compiler.moof`.

**forcing function.** every test in
`crates/substrate/tests/doc_alignment.rs` and
`phase_a_forcing_function.rs` compiles via the moof-side
compiler. concretely: a feature flag `World::use_moof_compiler`
(default off during bootstrap, on after). when on, every
`compile_expr` call routes through moof. tests pass either way.

**doc gates.** `docs/concepts/compiled-objects.md` (chunks),
`docs/syntax/binding-and-defs.md` (the special forms),
`docs/laws/substrate-laws.md` L4 ("eval is itself a send" —
already true for user code; this makes it true for the substrate's
compile path too).

---

### track 3 — the bootstrap dance + cleanup

**why last.** we've now got two compilers: rust (fallback during
bootstrap) and moof (post-bootstrap). they must agree. this
track tightens the integration.

**deliverables.**

- **byte-identical chunks** between rust and moof compilers for
  every form in `bootstrap.moof`. compare via a new
  `chunks_equal?:` helper that walks ops + consts + ics and
  asserts pairwise equality.
- **flip the flag.** `World::use_moof_compiler` defaults to
  `true` after bootstrap finishes. measure: how much of bootstrap
  itself can route through the moof compiler? ideally, after the
  last `(defmacro …)` line, every subsequent compile uses moof.
- **trim rust special-form handlers.** anything no longer
  reachable becomes `#[deprecated]` + delete. compile_expr's
  inner block shrinks substantially.
- **a `docs/process/self-hosted-compiler.md`** memo: the
  bootstrap order, how the dance works, what rust still owns and
  why.

**rust delta.** ~-300 LoC (deletions outweigh additions).
**moof delta.** ~+0 (cleanup only).

**forcing function.** `cargo test --workspace` green, AND a new
diff-test that compiles every form in bootstrap.moof through both
compilers and asserts byte-identical chunks.

---

## what is NOT in scope this session

| deferred | why |
|---|---|
| parser.moof | session N+2. requires compiler.moof to compile parser.moof itself. |
| persistence (lmdb, journal) | session N+3 onward. orthogonal to self-hosting. |
| canonical encoding + canonical hash | session N+4 (phase D). |
| vats + scheduler + far-refs | session N+5. |
| moofpaint demo | session N+6+. |
| GC | minimal mark-sweep at turn boundaries; deferred unless heap pressure shows up. |
| frame-as-Form (option A) | the snapshot path (R3) honors the contract; full conversion is a phase-B journaling concern. |
| chunk slot-canonical | already R6-honest as L5-permitted derivation cache. |

---

## the read-the-docs-first discipline

before each track:

1. **re-read its doc gate.** every track above cites the relevant
   files; the contract is in those files, not in this plan.
2. **ask: does the doc cover what you're about to do?** if no —
   that is the bug. fix the doc *first*, then implement.
3. **forcing function before writing code.** the test exists
   before the impl exists.
4. **277 → growing.** each track ends with green
   `cargo test --workspace`. tracks compose; no debt.

`docs/process/docs-driven.md` is the rule. drift between docs
and code is the v3 mistake — we don't repeat it.

---

## risk register

ranked by likelihood × impact:

1. **rust-vs-moof compiler agreement.** subtle differences in
   ic-slot allocation order, constant-pool dedup, or jump-offset
   computation will cause divergence. mitigation: the diff-test
   at the end of track 3 catches it. before flipping the flag,
   run the diff-test on every form in bootstrap.moof.
   *probability: high. impact: medium (caught by test).*

2. **macro recursion in self-hosted compile.** compile-form
   itself uses `match`, which expands to `let` + `if` + `fn`. if
   any of those expansions trigger compile-form recursively *at
   compile time of compile-form*, we infinite-loop. mitigation:
   bootstrap.moof keeps the rust compile path until compile-form
   is fully registered; only THEN flip the flag.
   *probability: medium. impact: high (deadlock).*

3. **opcode marshalling overhead.** every emit goes through a
   moof send + heap alloc per opcode. compiling a 200-op chunk
   means 200 sends. probably 100x slower than rust. acceptable
   until perf measurements demand a fast path.
   *probability: high. impact: low (compile-time only).*

4. **set!-walks-parent-chain regressions.** the recent fix
   changed lexical-scope semantics in ways that may affect
   user code that relied on shadow-bind behavior.
   already shipped + tested; the residual risk is pre-existing
   user code outside the test suite.
   *probability: low. impact: low.*

---

## the ladder of acceptable session-end states

if the session goes ideally, all three tracks land with the moof
compiler running everything. if not, here's the descending ladder:

1. **tracks 1–3 done.** moof compiler handles every form;
   rust compiler is bootstrap-only. **target: this.**
2. **tracks 1–2 done; track 3 deferred.** both compilers exist;
   diff-test reveals a discrepancy. session N+2 closes it.
3. **tracks 1–2 done with one or two forms still rust-only.**
   `if` and `fn` are the hardest; if they slip, that's a real but
   bounded loss. **fallback: this.**
4. **track 1 done; track 2 ports `quote` and `set!` only.** the
   smoke test passes; the runway is established but barely
   walked. session N+2 walks the rest.
5. **track 1 partial.** opcode constructors exist but
   `make-chunk`/`chunk-emit` not wired. fix in N+2.

below ladder rung 5 the session is judged "we learned but did
not ship." that's also fine.

---

## the inputs to the session

before this session starts:

- `git pull` to current state. confirm `cargo test --workspace`
  shows 277 / 277 passing. (it does as of `285a17e`.)
- read `docs/concepts/compiled-objects.md` end-to-end.
- read `docs/laws/substrate-laws.md` L1, L4, L5 carefully.
- skim the existing `compiler.rs` — note which special forms
  share structural shape (most emit a few ops + a chunk
  finalize). that structure becomes the moof code.

---

## post-session: the next sessions, briefly

| session | scope | end-state |
|---|---|---|
| **session N+1 (this one)** | tracks 1–3 — opcode primitives, port special forms to moof, bootstrap dance | moof compiler runs everything; rust compiler is bootstrap-only |
| **session N+2** | parser.moof — port `reader.rs` to moof. the rust shim reads only enough to load parser.moof itself. | parser is moof; phase A-self-host complete |
| **session N+3** | phase B foundations — vats, mailbox, scheduler, lmdb persistence | `moof world ./worlds/test/` runs; state survives reboot |
| **session N+4** | phase D foundations — canonical encoding, canonical hash, replicated-vat mode | the 2-replica convergence test passes |
| **session N+5** | phase E — terminal renderer, `$canvas` / `$pointer`, single-user world | `moof world ./worlds/test/` shows a navigable 3D space |
| **session N+6** | phase F — websocket transport, `moof world join wss://…` | the manifesto's demo is real |

six sessions to the demo. this session removes the parser/compiler
from the rust line. that's the beating heart of "the language is
theirs" — once compiler.moof exists, redefining a special form is
no longer a special privilege of the substrate. it's a moof edit.

---

## final note

the docs are the source of truth. when the implementation
diverges from a doc, the doc is the bug to fix first — *unless*
the doc is the bug, in which case the doc is the bug to fix
first. either way the doc moves before the code.

the session begins by re-reading
`docs/concepts/compiled-objects.md` and
`docs/laws/substrate-laws.md` end-to-end. then a doc-update if
either is silent on something this plan needs (track 1's `make-
chunk` / `chunk-emit` API has no doc home yet — write one in
`docs/reference/compiler-primitives.md` before the rust impl).

`>.<` softly. let's get the language out of rust. ૮ ◞ ﻌ ◟ ა
