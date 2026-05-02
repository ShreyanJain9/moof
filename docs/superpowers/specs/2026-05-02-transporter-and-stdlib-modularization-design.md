# Transporter + radical std-lib modularization

> Pre-MCO cleanup pass. Three threads: dynamic file loading via a
> Self-style Transporter capability, a multi-file split of the
> bootstrap so phase boundaries become first-class, and a radical
> migration of derivable methods out of `intrinsics.rs` into Moof.
> Plus a one-line REPL bug fix while we're here.

## Goals

1. **Stop hardcoding `lib/*.moof` strings into the binary.** Replace
   the two `include_str!` calls in `crates/substrate/src/lib.rs`
   with runtime file loading via a primordial `$transporter`
   capability.
2. **Treat files as transport, not source-of-truth.** The std lib
   becomes a directory tree of small thematic files. The substrate
   loads exactly one entry file (`lib/main.moof`); everything else
   threads through `[$transporter load: ...]`. Future work: round-trip
   in-image objects back to files via `[$transporter dump:toFile:]`.
3. **Move every derivable method out of Rust into Moof.** Pure
   primitives stay (heap, arithmetic, byte access, bytecode emit).
   Derivations (string trim/split/replace, char inspect, object
   identity, cons toString/inspect, all of the nil/bool/cons
   "shadow" methods that bootstrap.moof currently double-installs)
   move to Moof. Net: ~1.2k LoC out of `intrinsics.rs`.
4. **Fix the REPL nil-display bug.** `:inspect` already returns
   `"nil"`; the REPL's explicit `if !value.is_nil()` gate is what's
   suppressing it.

## Non-goals

- Hot-reload of changed `.moof` files at runtime. Reloads happen
  per-process. (Phase G, with the world-as-image work.)
- The `dump:toFile:` Transporter half. API name is reserved this
  session; implementation is later.
- Splitting `compiler.rs`. The seed compiler stays as a single
  Rust file; only the Moof side gets multi-file structure.
- Removing rust intrinsics that *aren't* derivable (arithmetic,
  heap, byte access, bytecode primitives). Those stay.
- Parser-in-Moof. Setup-only — this work clears the path; the
  port itself is a future session.

## Architecture

### The Transporter capability

`$transporter` is a primordial cap, installed by Rust intrinsics
alongside `$out`, `$err`, `$compiler`. Its proto-Form lives on
`world.protos` so it dispatches through the same machinery as
other caps.

**Methods this session:**

- `[$transporter load: pathString]` — read the file at `pathString`
  (resolved against the lib root), call `eval_program` on it,
  return the value of the last form. Raises:
  - `'tx-not-found` — file doesn't exist
  - `'tx-read-error` — IO failure during read (with errno detail)
  - `'tx-eval-error` — propagated raise from inside the file
- `[$transporter loadAll: pathList]` — convenience: walks a Cons
  of strings, calls `:load:` on each in order. Returns the value
  of the last loaded file's last form. Raises `'tx-bad-arg` if the
  argument is not a Cons of Strings (with the offending element's
  position in the message).
- `[$transporter root]` — returns the resolved lib root as a
  String. For diagnostics.

**Methods reserved for later (NOT this session):**

- `[$transporter dump: form toFile: path]` — serialize a Form's
  handlers/slots/meta back to source text using each method's
  `:source` slot. The round-trip half. Distinct doc note in the
  capability's source comment so the second half is obvious to
  the next reader.

### Lib root resolution

Implemented in `crates/substrate/src/world.rs` (or a small new
`transporter.rs` — see "module structure" below):

1. If env var `MOOF_LIB` is set → use it.
2. Else: `<dir of std::env::current_exe()>/../lib` if it exists.
3. Else: `./lib` relative to cwd, if it exists.
4. Else: raise `'tx-no-root` listing all paths tried.

The resolved root is computed once at world creation and cached
on `World`. All `[$transporter load: relPath]` calls join `relPath`
to that root. Absolute paths in `relPath` are rejected with
`'tx-bad-path` to keep the contract simple — only lib-relative
loads.

### Boot dance

Rust loads exactly one file directly. Concretely, `new_world()`:

```rust
pub fn new_world() -> world::World {
    let mut w = world::World::new();
    intrinsics::install(&mut w);
    let main_path = w.transporter_resolve("main.moof")?;
    let main_source = std::fs::read_to_string(&main_path)?;
    if let Err(e) = eval_program(&mut w, &main_source) {
        panic!("lib/main.moof failed to load: {}", e.message);
    }
    w
}
```

`main.moof` orchestrates the rest:

```moof
;; lib/main.moof — the only file the rust seed compiles directly.

;; phase 1: compiler/ — compiled by the rust seed.
[$transporter load: "compiler/00-helpers.moof"]
[$transporter load: "compiler/01-dispatch.moof"]
[$transporter load: "compiler/02-special.moof"]
[$transporter load: "compiler/03-control.moof"]

;; flip — every subsequent compile routes through Compiler singleton.
[$compiler useMoof]

;; phase 2: early/ — primitives via setHandler!, then macros.
[$transporter load: "early/00-symbol-cons-nil.moof"]
[$transporter load: "early/01-quasiquote.moof"]
[$transporter load: "early/02-control-macros.moof"]
[$transporter load: "early/03-modules.moof"]
[$transporter load: "early/04-match-defn-proto.moof"]
[$transporter load: "early/05-defmethod.moof"]

;; phase 3: stdlib/ — defmethod-driven, regular moof code.
[$transporter load: "stdlib/object.moof"]
[$transporter load: "stdlib/bool.moof"]
[$transporter load: "stdlib/nil.moof"]
[$transporter load: "stdlib/cons.moof"]
[$transporter load: "stdlib/integer.moof"]
[$transporter load: "stdlib/float.moof"]
[$transporter load: "stdlib/string.moof"]
[$transporter load: "stdlib/char.moof"]
[$transporter load: "stdlib/table.moof"]
[$transporter load: "stdlib/method.moof"]
```

Cascade (`__cascade__`) lives in `early/`, so it isn't usable in
`main.moof` itself (the seed compiler doesn't know macros, and at
the moof-compiler-flip cascade is still not yet defined). main.moof
stays in plain method-send shape. Acceptable — explicit and
bulletproof.

### `$compiler useMoof`

The current `World.use_moof_compiler` boolean flag stays internally,
but its toggle moves behind a `$compiler` capability. New cap:

- `[$compiler useMoof]` — set the flag to `true`. After this, every
  `compile()` call routes through the moof Compiler singleton.
- `[$compiler useSeed]` — for diagnostics; flips back. (Mostly used
  by tests of the seed in isolation.)

### File structure

```
lib/
  main.moof                    ;; the orchestrator (rust loads this directly)
  compiler/
    00-helpers.moof            ;; cons?:, symbol?:, bool?:, macroAt:, wrapBody:
    01-dispatch.moof           ;; compileTop:, compileForm:..., compileList:...,
                               ;;   compileSpecialOrCall:..., compileConst:...,
                               ;;   compileLoadName:...
    02-special.moof            ;; compileQuote/Set/Def/Send/Args
    03-control.moof            ;; compileIf/Fn/Do/Let/Defmacro/Call,
                               ;;   letParams, letValues, multiClauseDef?,
                               ;;   allFnForms?
  early/
    00-cons.moof               ;; setHandler!-driven Cons primitives — all
                               ;;   the current rust install_list_methods
                               ;;   ported (length, reverse, empty?, etc.)
    01-nil.moof                ;; ditto for nil — supersedes both the rust
                               ;;   shadow installs AND the bootstrap
                               ;;   defmethod nil shadows
    02-bool.moof               ;; Bool :not :and: :or: :toString
    03-string-essentials.moof  ;; the small set of String methods that
                               ;;   __decode-header transitively needs:
                               ;;   :endsWith?:, :+, :=, :length,
                               ;;   :contains?:, :all?:, :toString. NOT
                               ;;   the full string stdlib — that's stdlib/.
    04-symbol.moof             ;; Symbol :endsWithColon?, :operatorOnly?,
                               ;;   __operator-chars
    05-quasiquote.moof         ;; __qq-list?, __qq-marker?, __qq-walk-elems,
                               ;;   __qq-expand, (defmacro quasiquote)
    06-control-macros.moof     ;; when, unless, let*, let-rec
    07-modules.moof            ;; DefProto, Defn, Match singleton modules
    08-match-defn-proto.moof   ;; (defmacro match), (defmacro defn),
                               ;;   (defmacro defproto)
    09-defmethod.moof          ;; __decode-header, __decode-keyword,
                               ;;   (defmacro defmethod)
  stdlib/
    object.moof                ;; :protos, :satisfies?:, :=, :!=, :is-fallback,
                               ;;   :toString-name-fallback, :initialize
    bool.moof                  ;; :not, :and:, :or:, :toString
    nil.moof                   ;; the canonical nil methods (rust shadow gone)
    cons.moof                  ;; :length, :reverse, :map:, :filter:, :reduce:,
                               ;;   :forEach:, :take:, :drop:, :any?:, :all?:,
                               ;;   :contains?:, :sum, :product, :countWhere:,
                               ;;   :last, :count, :zip:, :scan:, :at:, :=,
                               ;;   :!=, :toString, :inspect, :append:
    integer.moof               ;; :abs, :even?, :odd?, :between?, :max:, :min:,
                               ;;   :!=, :<=, :>=
    float.moof                 ;; :abs, :max:, :min:, :!=, :<=, :>=, :asInteger,
                               ;;   :round, :floor, :ceil
    string.moof                ;; :trim, :indexOf:, :replace:with:, :split:,
                               ;;   :lines, :toString, :inspect, :asTable,
                               ;;   :startsWith?:, :endsWith?:, :contains?:,
                               ;;   :reverse, :as:
    char.moof                  ;; :inspect, :toString, :digit?, :letter?,
                               ;;   :uppercase, :lowercase
    table.moof                 ;; :size, :empty?, :nonEmpty?, :asString,
                               ;;   :toString, :inspect, :=, :as:, :forEach:
    method.moof                ;; :toString, :inspect (slot-walking)
```

**File ordering rule:** numeric prefix in `compiler/` and `early/`
indicates dependency order. `stdlib/` files are independent —
each adds methods to a distinct proto, so order is alphabetical
for predictability.

**Subtle dependency to watch:** `String:endsWith?:` currently lives
in Rust intrinsics specifically because `Symbol:endsWithColon?`
(used by `__decode-header` inside the `defmethod` macro) needs it
*before* `defmethod` runs. The migration preserves this constraint
by putting `:endsWith?:` (along with `:+`, `:=`, `:length`,
`:contains?:`, `:all?:`) in `early/03-string-essentials.moof`. The
rest of String's stdlib lives in `stdlib/string.moof` and re-defines
these later (the same shadow-pattern bootstrap.moof uses today for
`String:endsWith?:` at line 940, just made explicit by the file
structure).

### Radicality unlock — primitive free-functions for the moof compiler

The blocker: the moof compiler's `compileSend` and `compileArgs`
do method sends like `[args length]`, `[forms car]`. Those
require `Cons:length`, `Cons:car` etc. to be Moof methods. But
those methods are themselves defined via `(defmethod Cons (length)
…)` in `stdlib/cons.moof` — and compiling that file requires
`Cons:length` to exist already. Circular.

**Fix:** the moof compiler internally calls primitive **free
functions**, not methods, for the operations it bottoms out on:

```rust
// installed by intrinsics.rs as toplevel globals:
__list-length    : (List) -> Int
__list-empty?    : (Value) -> Bool       ;; #true iff value is nil
__list-car       : (List) -> Value
__list-cdr       : (List) -> Value
__list-reverse   : (List) -> List
__symbol-ends-with-colon? : (Symbol) -> Bool
__symbol-to-string : (Symbol) -> String   ;; if not already exposed
```

Inside `compiler/*.moof`, replace `[args length]` → `(__list-length
args)`, `[args is nil]` → `(__list-empty? args)`, `[forms car]` →
`(__list-car forms)`, etc. The moof compiler now uses **only**
free-function primitives plus method sends to its **own** singleton
(`[self compileForm: …]`, `[chunk emit: …]`).

Once that's done, `Cons:length`, `Cons:reverse`, `Cons:car`,
`Cons:cdr`, `Cons:empty?`, `nil:length` etc. can ALL be Moof-side
without any "needs to exist before defmethod runs" caveat.

### Method migration manifest

**Stays in Rust (intrinsics.rs):** primitives only. Approximately:

| Proto | Methods kept |
|---|---|
| Object | `:proto`, `:slots`, `:handlers`, `:meta`, `:identity`, `:source`, `:dnu`, `:new`, `:handlerAt:` |
| Integer | `:+ :- :* :/ :=, :<, :>, :asFloat` |
| Float | `:+ :- :* :/, :=, :<, :>` |
| Cons | `:car, :cdr, :cons:` (heap accessors only) |
| String | `:length`, `:byteLength`, `:byteAt:`, `:at:`, `:+`, `:=`, `:slice:length:`, `:as:`, `:toList`, `:contains?:` (only because :all?: in `early/03-string-essentials.moof` uses it) |
| Table | `:new`, `:length`, `:at:`, `:at:put:`, `:push:`, `:pop`, `:keys`, `:values`, `:remove:`, `:containsKey?:` |
| Char | `:codepoint`, `:<` |
| Symbol | (none in rust today; Symbol's `:toString` flows through Object's universal toString that handles tagged immediates. Confirmed by `install_symbol_methods` being a no-op.) |
| nil | (all move to Moof) |
| Method | `:body`, `:source`, `:params`, `:consts`, `:bytecodes`, `:ics`, `:call` |
| Chunk | (all stay — bytecode primitives) |
| Console | `:emit:`, `:close`, `:next` |

Plus the new free-function primitives (`__list-length` etc.) and
the existing globals (`setHandler!`, `slotSet!`, `metaSet!`,
`globalEnv`, `intern`, `raise:`, etc.).

**Moves to Moof:** roughly 1.2k LoC across these methods (counts
include comments and blank lines, so dispersed across stdlib/ files):

- `Cons`: `:length` (~5 LoC), `:reverse` (~5), `:empty?` (1), `:null?` (1), `:toString` (~30 with helpers), `:inspect` (~30)
- `nil`: `:length, :car, :cdr, :empty?, :reverse, :append:, :proto, :toString, :inspect, :cons:` (~80 LoC, currently shadow-installed in Rust AND bootstrap)
- `String`: `:trim, :indexOf:, :replace:with:, :split:, :lines, :toString, :inspect, :asTable` (~150 LoC)
- `Object`: `:is, :=, :!=, :toString-name-fallback, :initialize` (~80 LoC)
- `Char`: `:inspect` (~30 LoC)
- `Method`: `:toString, :inspect` (~60 LoC)
- `Float`: `:asInteger`, `:!=, :<=, :>=` (~30 LoC; `:!=, :<=, :>=` already partly in bootstrap)
- `Table`: `:toString, :inspect, :asString, :=, :as:` (~120 LoC)

### REPL nil fix

In `crates/substrate/src/main.rs`:

- Line 92: remove the `if !value.is_nil() { … }` gate — let
  `:inspect` always run. With `(defmethod nil (inspect) "nil")`
  in `stdlib/nil.moof`, this prints `nil` correctly.
- Lines 42-44: same fix in one-shot mode. The user said it; the
  user gets it. `[$out say: nil]` already works since
  `(defmethod nil (toString) "nil")` exists.

The "lisp convention" comment goes away — Moof is its own thing
now and shows nils, like Smalltalk's `printNl` does.

### Rust module structure

`crates/substrate/src/`:

- `lib.rs` — `BOOTSTRAP_SOURCE` and `COMPILER_SOURCE` constants
  removed. `new_world()` rewritten to load `main.moof` via the
  new transporter. ~30 LoC delta.
- `transporter.rs` — **new file**. Houses lib-root resolution,
  `[$transporter load:]`, `[$transporter loadAll:]`,
  `[$transporter root]`, plus `dump:toFile:` stub that raises
  `'tx-unimplemented`. ~150 LoC.
- `intrinsics.rs` — major shrinkage. Methods listed above are
  removed. Free-function primitives `__list-length` etc. are
  added (~50 LoC). Net: ~3745 → ~2500 LoC.
- `world.rs` — adds `transporter_root: PathBuf` field; uses
  `crate::transporter::resolve_lib_root()` at construction.

Tests that depended on the old `BOOTSTRAP_SOURCE` / `COMPILER_SOURCE`
constants (used by some integration tests) need `MOOF_LIB` set or
the resolved-from-cwd path to work. CI implication: tests must run
with cwd at the repo root, or `MOOF_LIB=$CARGO_MANIFEST_DIR/lib`.

## Error handling

- `'tx-no-root` — no lib root could be resolved at world creation.
  Hard panic at `new_world()` since the world is unusable.
- `'tx-not-found` — `:load:` got a path that doesn't resolve.
  Caller can catch.
- `'tx-read-error` — IO failure (permission denied, etc.).
- `'tx-eval-error` — wraps an error raised from inside the loaded
  file. The wrapper preserves the inner error's symbol and message
  but adds the file path to the message: `"<path>: <inner-msg>"`.
- `'tx-bad-path` — absolute path or `..`-traversal in `:load:`'s
  argument. Reject early.

## Testing

Existing tests need to keep passing — the biggest risk is the
file-split + radical migration breaking the boot dance. Strategy:

1. **Migrate REPL nil fix + transporter cap first** as a standalone
   commit. Existing `BOOTSTRAP_SOURCE` / `COMPILER_SOURCE` stay
   as the source for these tests; only the REPL behavior changes.
   New transporter unit tests cover load/loadAll/error paths.
2. **Then split compiler.moof + bootstrap.moof into the new file
   tree, keeping all current Rust intrinsics.** main.moof boots
   identically. Tests should be green.
3. **Then remove the rust shadow methods one proto at a time**,
   moving canonical implementations to `stdlib/<proto>.moof`.
   Run `cargo test --workspace` after each proto migration.

Each commit should leave `cargo test --workspace` green. Bisect
discipline.

New tests:

- `transporter_load_basic` — `[$transporter load: "stdlib/cons.moof"]` succeeds.
- `transporter_not_found` — bad path raises `'tx-not-found`.
- `transporter_eval_error` — file with a `(raise: 'foo "bar")`
  surfaces as `'tx-eval-error` with the path in the message.
- `transporter_bad_path` — absolute path rejected.
- `repl_nil_displays` — REPL eval of `nil` prints `"nil\n"`.
- For each migrated proto, a "method works after migration" test
  that exercises the migrated method through normal moof code.

## Risk register

1. **Boot dance breakage during migration.** A wrong split in
   `early/` (e.g. macro defined after first use) will fail at
   world creation, every test red. Mitigation: small commits,
   run tests after each file extraction.
   *Probability: medium. Impact: high (blocks until fixed) but
   detectable instantly.*

2. **Test cwd assumption breaks CI.** If `MOOF_LIB` resolution
   silently picks an unexpected path under CI, tests load wrong
   files. Mitigation: tests set `MOOF_LIB=$CARGO_MANIFEST_DIR/lib`
   explicitly via a test helper.
   *Probability: medium. Impact: medium.*

3. **Free-function primitives turn out to be needed in more
   places than just the compiler.** E.g. some early macro might
   need `__list-length`. Mitigation: they live as unscoped
   globals; if they leak into early/ that's fine, just slightly
   ugly. We can lint them out of stdlib/ later.
   *Probability: low. Impact: low.*

4. **Free-function primitives semantically diverge from method
   versions.** E.g. `__list-length` on a non-list raises a
   different error than `[xs length]`. Mitigation: explicit
   "what raises what" doc in `transporter.rs`'s comment block;
   tests cover the divergence cases.
   *Probability: low. Impact: low.*

5. **REPL nil fix cascades — anything else relying on the gate?**
   The gate also exists in `eval_one_shot`. Both removed
   together. Mitigation: grep for `is_nil()` after the change to
   confirm no other paths assume nil-suppression.
   *Probability: low. Impact: trivial.*

## Success criteria

- `cargo test --workspace` is 334+ green at session end.
- `lib/bootstrap.moof` and `lib/compiler.moof` no longer exist
  as single files; the new tree under `lib/` is canonical.
- `intrinsics.rs` has shrunk by ~1.2k LoC.
- `[$transporter load: "stdlib/cons.moof"]` works at the REPL
  for live reload during development. (Not full hot-reload, but
  re-running it re-installs the methods.)
- Typing `nil` at the REPL prints `nil`.
- `MOOF_LIB=/path/to/some/other/lib moof` boots from that dir.

## Out of scope, named explicitly

- Hot-reload semantics for already-installed handlers (does
  re-loading `stdlib/cons.moof` clobber existing user-installed
  Cons methods? → yes, by design, but we don't define what
  happens to in-flight closures referencing the old methods).
- Transporter's `dump:toFile:` half. Reserved.
- A `moof package` subcommand for shipping `.moof` trees as
  `.mco` bundles. (Logical next step; not this session.)
- The parser-in-moof port. This work clears the path; the port
  is a separate session.
