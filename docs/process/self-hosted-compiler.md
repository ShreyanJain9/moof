# self-hosted compiler — the bootstrap dance

> **the rust compiler is a seed. it compiles `lib/compiler.moof`,
> flips a flag, and steps aside. every compilation that follows
> goes through the moof-side compiler. this memo describes the
> dance, what's irreducible in the rust line, and why.**

after track 3 (`NEXT_SESSION.md`), the rust line owns the
bootstrap. the moof line owns compilation.

## the four steps

`moof::new_world()`:

```text
        ┌───────────────────────────────┐
        │ 1. World::new()               │
        │    + intrinsics::install()    │   rust seed.
        └───────────────────────────────┘
                       │
                       ▼
        ┌───────────────────────────────┐
        │ 2. eval_program(COMPILER_SRC) │   rust seed compiles
        │                               │   compiler.moof.
        └───────────────────────────────┘
                       │
                       ▼
        ┌───────────────────────────────┐
        │ 3. w.use_moof_compiler = true │   the flip.
        └───────────────────────────────┘
                       │
                       ▼
        ┌───────────────────────────────┐
        │ 4. eval_program(BOOTSTRAP_SRC)│   moof compiler compiles
        │                               │   bootstrap.moof.
        └───────────────────────────────┘
```

step 1: rust intrinsics are installed (heap, OS i/o, arithmetic
primitives, the chunk-construction api in
`docs/reference/compiler-primitives.md`, plus `:length` on Cons /
Nil and `:initialize` on Object — both load-bearing for the moof
compiler at boot).

step 2: the rust seed compiler compiles `lib/compiler.moof`.
top-down, form by form. each `(def compile-* (fn ...))` is a
single-binding `def` whose body uses only rust-handled special
forms. once the file is loaded, `compile-form`, `compile-top`,
and the per-special-form helpers all live in the global env.

step 3: `World.use_moof_compiler = true`. from this point,
`crate::compiler::compile()` delegates to the moof-side
`compile-top` for every compilation.

step 4: the rust shim asks moof to compile each top-level form in
`bootstrap.moof`. macros (`when`, `match`, `defn`, `defmethod`,
`defproto`, `quasiquote`, ...) register through the moof
compiler's `compile-defmacro`, which uses the canonical `Macros`
slot table.

## what's irreducible in rust

the seed compiler handles **exactly seven** special forms — the
ones `compiler.moof` itself uses:

| form                          | why it can't move (yet)                         |
|-------------------------------|------------------------------------------------|
| `def name expr`               | needs `DefineGlobal` opcode                     |
| `fn (params…) body…`          | needs sub-chunk allocation + `PushClosure`      |
| `if cond then [else]`         | needs `JumpIfFalse` + `Jump` with patched offsets |
| `let ((name val)…) body…`     | desugar to `((fn …) values…)` — convenience for compiler.moof's source |
| `do e1 … eN`                  | sequence + pop intermediates — convenience      |
| `quote v`                     | needs `LoadConst`                               |
| `__send__ recv 'sel args…`    | needs `Send` / `SuperSend` / `TailSend`         |

plus the call path: `(callable args…)` → `[callable call: args…]`.

these aren't "irreducible" in any deep sense; `let` and `do`
could be desugared by hand and removed. but compiler.moof would
become noticeably uglier. seven seems like the right tradeoff.

what's **not** in the rust seed:

- **no `set!`** — compiler.moof never assigns. `set!` is a moof
  concern, handled by `compile-set` at runtime.
- **no `defmacro`** — compiler.moof never defines a macro.
  `compile-defmacro` lives in compiler.moof.
- **no user-macro lookup** — compiler.moof uses no user macros.
  the moof compiler does the `(slot Macros sym)` check.
- **no multi-clause `def`** — compiler.moof uses single-binding
  only. `compile-def` in compiler.moof rewrites multi-clause
  shapes to `(defn …)`, which is itself a moof macro.
- **no quasiquote / cascade / table / object literal** — all
  reader-emitted helpers handled by moof-side macros after step
  4.

result: the rust compiler shrinks from ~1050 LoC (pre-track-3)
to ~520 LoC of compile logic (plus tests + docs).

## what's in the moof compiler

`lib/compiler.moof` defines:

- `compile-form` — the dispatcher. literal / symbol / list,
  with macro lookup via `(slot Macros sym)` for list-heads.
- `compile-top` — alloc fresh chunk, compile in tail position,
  emit `Return`, return chunk-Form. **the entry point the rust
  shim calls when the flag is on.**
- per-special-form helpers: `compile-quote`, `compile-set`,
  `compile-def` (with multi-clause detection),
  `compile-send`, `compile-if`, `compile-fn`, `compile-do`,
  `compile-let`, `compile-defmacro`, `compile-call`.
- `compile-load-name`, `compile-const` — atomic forms.

it's ~450 LoC of moof. the special-form handlers are typically
~10–20 lines each; `compile-defmacro` is the largest at ~25 lines.

## the chicken-and-egg, addressed

> **q**: how does the rust seed compile compiler.moof when
> compiler.moof needs primitives the rust intrinsics provide?

it doesn't need them at compile time. the rust intrinsics
provide `[Chunk new: …]`, `[Opcode loadConst: …]`, `(slotSet! …)`,
etc. as *runtime* sends. the rust seed compiler doesn't invoke
them — it just emits opcodes that, when executed, will invoke
them. compiler.moof's body is a bunch of `def`s; running each
`def` binds a closure. the closures aren't called until the
moof compiler is actually used (step 4 onward).

> **q**: what if compiler.moof has a bug?

the seed stays buildable. running `cargo test --lib compiler`
exercises only the rust seed; running `cargo test --workspace`
exercises both. if the moof compiler breaks, fall back to the
seed by setting `world.use_moof_compiler = false`.

> **q**: doesn't the moof compiler's recursive walks blow the
> rust stack?

yes — the moof compiler is recursive in moof, and each `let`/`do`
in the source it's compiling expands into nested fn-calls, each
of which sends back through the rust VM's invoke path. in debug
builds (where rust frames are >10 KB), deeply nested source can
pile up MB of stack. `.cargo/config.toml` bumps `RUST_MIN_STACK`
to 32 MB for tests. release builds are fine.

`NEXT_SESSION.md` flagged this as an acceptable compile-time
perf cost (risk #3). making the moof compiler iterative (or
adopting the rust-side iterative trampoline pattern) is a
phase-G perf win, not a correctness concern.

## what comes next

phase A-self-host has two remaining moves:

1. **port the reader.** `crates/substrate/src/reader.rs` (1634
   LoC) becomes `lib/parser.moof`. the rust reader shim shrinks
   to "read just enough text to load parser.moof itself."
2. **byte-identical chunks.** track 3 verified value-equivalence
   between rust and moof compilers. byte-identity (same ops,
   same const pool, same ic count) is a stricter contract,
   useful for the diff-test that catches divergence. write
   `chunks_equal?:` and run it over every form in bootstrap.moof.

after both: the rust line owns the heap, the VM, and OS i/o.
everything else is moof — and editable from inside moof.

## see also

- `docs/reference/compiler-primitives.md` — the chunk-
  construction api compiler.moof is built on.
- `docs/laws/substrate-laws.md` L4, L5 — what self-hosting
  preserves.
- `lib/compiler.moof` — the canonical compiler.
- `crates/substrate/src/compiler.rs` — the seed.
- `NEXT_SESSION.md` — the original three-track plan.
