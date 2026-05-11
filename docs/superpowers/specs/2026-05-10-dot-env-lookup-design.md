# `.foo` — env-lookup rewrite design

> **status:** brainstormed 2026-05-10. ready for plan.
>
> **prior art:** parser-level `.foo` → `(__send__ self 'foo)` desugaring shipped in rust reader (`crates/substrate/src/reader.rs:1218-1229`) and mirrored in self-hosted parser.moof (`lib/parser/02-parser.moof:115-148`, commit 311a0a4). this spec proposes replacing both with an env-lookup-time resolution rule that keeps `.foo` as a literal sym in source.
>
> **spec reference:** `docs/syntax/sigils.md` (`.foo` row) and `docs/syntax/overview.md` (the `;; .count ≡ [self count]` example) are the user-facing docs. they will need a small note added but no semantic change — `.foo` still means "no-arg send to the implicit receiver." what changes is *when* that resolution happens.

## 1. scope and motivation

today `.foo` is a **read-time** rewrite. the reader (rust *and* moof) sees the text `.foo`, decides it starts with `.`, strips the dot, and replaces the atom with the cons-list `(__send__ self 'foo)`. by the time the parser returns, the surface symbol is gone — the source-Form tree contains a three-element list with `self` baked in.

this works but is unsatisfying for five reasons:

1. **it's parser magic.** `L5 — source is canonical` (`docs/laws/substrate-laws.md`) says the substrate must preserve the *actual source-form* a closure was compiled from. with read-time desugaring, the source-form stored on a method's `:source` slot is the post-rewrite tree, not the surface `.count` the user typed. `inspect`ing the body shows `(__send__ self 'count)` — the user's `.` sigil is structurally lost.

2. **it loses moldability.** `L3 — message dispatch is the universal verb` says every effect goes through `send`. but the rewrite happens *before* there's a vat or env in scope — there's no `Object` proto to override, no macro to redefine, no live machinery to interpose. `.foo` is not a moof concept; it's a rust quirk re-imitated in moof.

3. **it bakes `self` into the read step.** there is no notion of "the implicit receiver" — the reader always emits `self`. for the REPL, the debugger, the inspector, and (future) `with`-style blocks, the implicit receiver is *not* `self`. read-time rewrite forecloses on these.

4. **two readers, two rewrites, one drift surface.** rust's reader and parser.moof's reader both contain copies of the dot-strip logic. they were brought back into sync by the fix-loop in 311a0a4 — but the underlying duplication is exactly the kind the v4 self-host plan (`2026-05-10-self-host-and-rust-deletion-design.md`) is trying to eliminate. moving the responsibility out of the reader entirely is a structural fix.

5. **byte-equivalence is fragile.** the self-host plan compares moof-emitted V4 vat-images to rust-emitted ones byte-for-byte. any time rust's reader and parser.moof drift, byte-equivalence breaks — and the rewrite is one of the most popular drift sites because every method body uses it.

the proposed design moves `.foo` from a read-time rewrite to a **compile-time resolution**. the reader emits a raw sym (whose printed name starts with `.`). the compiler — when it sees that sym as the head of `compileLoadName` — recognizes the dot-prefix and emits a small fused bytecode sequence equivalent to a no-arg send to `self`. the source-form tree carries the literal `.foo`. reflection sees it. the user's intent is preserved through to the bytecode.

## 2. comparison with current behavior

|aspect|current (read-time)|proposed (compile-time)|
|---|---|---|
|reader output for `.foo`|`(__send__ self 'foo)` (3-elem list)|`Sym(".foo")` (a single sym)|
|`method:source` slot stores|expanded cons-list|literal sym in the cons-tree|
|`[m source]` shows user|`(__send__ self count)`|`.count`|
|compiler dispatch|generic list-with-`__send__`-head path|`compileLoadName` peephole for dot-prefix|
|bytecode emit shape|`LoadSelf; Send :count argc=0`|same — a fused 2-op sequence|
|implicit receiver|hardwired to `self` at read time|hardwired to `self` at compile time, but with one named extension point|
|reader code path|special-case in `read_atom`|none — `.foo` is just a sym|
|parser.moof code path|`isDotSym?` / `expandDotSym:atToken:`|none — `.foo` is just a sym|
|number-collision (`.5`, `1.5`)|already handled (numeric tried first)|already handled (same — reader's `try_parse_number` runs first)|

the **bytecode-level effect is identical**. what changes is *where* the rewrite happens and what the source-Form looks like in between.

## 3. where does the special-case lookup happen?

three options. the spec evaluates each.

### 3.1. option A — in `World::env_lookup`

every env lookup checks: does the printed form of `name` start with `.`? if yes, strip the dot, look up `self` in the same env, then dispatch `send(self, stripped-name, &[])`.

**pros:**

- minimal compiler change. the compiler emits the same `LoadName` op it always did. `LoadName` calls `env_lookup`. `env_lookup` does the dot check.
- no separate "dot-sym" path in bytecode. simpler op table.
- works in dynamic contexts — e.g. `(eval '.foo)` evaluates the dot-sym in whatever env it's given.

**cons:**

- every env lookup pays the dot-check cost (one symbol-table indirection + first-char-of-name check). there are *many* env lookups in a hot loop.
- `env_lookup` is meant to return the *bound value*. `.foo` resolution is not "the value of `.foo`" — it's "send a message and return the result." conflating these two is a category error: lookup turns into dispatch, which can have effects (mutation, raise, recursion, infinite loop). violates `env_lookup`'s contract as a pure read.
- harder to reason about for replication / determinism — `env_lookup` becomes a dispatch site, which means inline-cache state, journaling implications, raise paths.
- the lookup happens inside `env_lookup`, but the *receiver* is `self`, which is bound at the frame level, not the env level. `env_lookup` would have to consult `vm.frames.last().self_` — coupling two abstraction layers that were previously independent.

verdict: **rejected**. mixing reads and dispatches inside `env_lookup` blows past too many invariants. the perf cost is incidental; the conceptual mess is the real problem.

### 3.2. option B — in VM's `LoadName` op handler

the bytecode op stays the same. `Op::LoadName(name)`'s handler in `vm.rs` checks the sym's first char; if `.`, dispatches a send to `self` instead of doing an env lookup.

**pros:**

- compiler unchanged — same `LoadName` op, same emit path.
- no `env_lookup` invariant violation — `env_lookup` stays a pure read.
- frame-level `self` is right there (`vm.frames[frame_idx].self_`).

**cons:**

- every `LoadName` execution pays a `world.resolve(name).starts_with('.')` check, OR pays a symbol-id cache lookup ("is this sym known to start with dot?"). either way, hot-path overhead on *every* name lookup.
- the symbol-id-cache mitigation requires interning bookkeeping: when a sym is first interned, check if its name starts with `.`, and set a bit. the VM checks the bit. still O(1) per LoadName, but adds state to the symbol table.
- bytecode is still "load a name" but actually executes "send a message" — readers of bytecode see `LoadName .count` and have to know dispatch is implied. complicates `[method bytecodes]` reflection (`L6 — reflection is total`).

verdict: **viable but inferior to option C**. moves the magic from compile to run time without buying anything.

### 3.3. option C — in the compiler (recommended)

`compileLoadName:chunk:` (lib/compiler/01-dispatch.moof:50 + rust seed equivalent) inspects the sym before emitting. if the sym's printed name starts with `.` and has length > 1, it emits `LoadSelf; Send :rest argc=0` (using the existing two ops — no new opcode). otherwise it emits the standard `LoadName name` (or `LoadSelf` for the bare `self` sym, which is already special-cased).

**pros:**

- **zero runtime cost.** the check happens once at compile time. bytecode has the fused shape but uses two existing ops the VM already knows.
- `env_lookup` stays a pure read. no contract change.
- `LoadName` op stays semantically "look up a binding" — no dispatch hidden inside.
- `[method bytecodes]` shows `LoadSelf; Send :count argc=0` — true to what the VM does. reflection over the *source* slot still shows `.count` — true to what the user wrote. both faces are honest.
- the change is small and local: one branch in `compileLoadName:`, mirrored in the rust seed's symbol-emit path. ~10 lines each.
- it composes with future "alternate implicit receiver" mechanisms (§5) — the compiler can be parameterized on "what does `.foo` resolve against in this scope?" much more easily than the runtime can.

**cons:**

- changes the bytecode shape from `LoadName .count` (current — emitted today via the desugared cons-list flowing through `compile_send`) to `LoadSelf; Send :count argc=0`. wait — actually that's exactly what the current `__send__`-cons-list-flowing-through-`compile_send` path produces too. so this is bytecode-identical to today. no change.
- the *source*-Form tree changes: from a 3-element list to a single sym. introspection tools that pattern-match on the old shape need updating. there is one such site in the codebase — see §10.

verdict: **recommended.** ships option C.

## 4. what about `self` being bound?

at the moment `Op::LoadSelf` executes (vm.rs:429), it reads `world.vm.frames[frame_idx].self_`. this is populated by every dispatch path — `run_method`, `send_via_ic`, the bytecode `Send`, etc. — and contains the receiver of the current message.

option C's emit is:

```
LoadSelf                       ;; pushes frames.last().self_
Send :count argc=0             ;; pops receiver, dispatches
```

no new mechanism is needed. `self` resolution piggybacks on the existing `Op::LoadSelf` semantic, which is already the canonical way to access the implicit receiver. this is the same thing the current `(__send__ self 'count)` cons-list compiles to (because `self` is a bare sym in that list, which `compileLoadName` already recognizes and emits as `LoadSelf`). bytecode-identical.

## 5. what about contexts where `self` isn't bound?

in some contexts there is no method-frame `self`:

- **top-level code in `lib/main.moof`** — the boot orchestration runs at world-toplevel. `self` is bound to `Nil` by convention (verified in `vm.rs` — `frames[0].self_` is `Value::Nil` for the boot frame).
- **REPL evaluation** — same: REPL frame's `self_` is `Nil`.
- **`(eval form env)` invoked from a method whose `self` is something** — `self` carries through; the inner eval frame inherits the outer frame's `self_`.

current behavior: `.foo` at top-level expands to `(__send__ self 'foo)`, which compiles to `LoadSelf; Send :foo argc=0`. `LoadSelf` pushes `Nil`. `Send :foo argc=0` dispatches `:foo` on `Nil`, which `protos.nil` does NOT implement, raising `'no-handler`.

**proposed behavior under option C:** identical. `.foo` at top-level compiles to the same `LoadSelf; Send :foo argc=0` sequence. raises `'no-handler` on Nil.

**should this raise or be a *static* error?** the compiler could check, at emit time, whether the surrounding chunk is a method or a top-level chunk, and raise a compile-time error if `.foo` appears in a non-method context. but moof's surface has no rigid notion of "this chunk is/isn't a method" — `(fn () .x)` is a closure, and whether it's used as a method depends on installation. so the compile-time check would have false positives.

**decision:** keep the runtime-raise behavior. `.foo` at top-level is a runtime error (`'no-handler`), matching current behavior. defer compile-time checking until we have more confidence in what we'd be catching.

(future: the analyzer / type-checker — `2026-05-06-typed-moof-seed.md` — can flag `.foo` outside a method context as a likely error. but that's analysis, not enforcement.)

## 6. what about non-self implicit receivers?

future use cases:

- **`with` block:** `(with obj .x .y .z)` — `.x` and `.y` resolve against `obj`, not lexical `self`. ruby's `tap` / smalltalk's `Object>>with:`.
- **inspector / debugger:** "the object I'm looking at" is the implicit receiver. clicking around in the inspector should let `.foo` mean "field of the focused object."
- **REPL `>` mode:** in a "focused" REPL session, `.foo` means a field of the focus target.

option C is **friendly** to these. the compiler's dot-sym recognition is a single function. that function can consult a compile-time context that says "what is the implicit receiver in this scope?" by default it's `self` (i.e. emit `LoadSelf`). but `with` (when implemented as a special form / macro) can locally rebind the implicit receiver:

```
;; inside compileWith:chunk:tail: (future)
;; for (with target body...) — rebind "the implicit receiver" to target
;; in the body's compile context
(let ((savedImplicit [self implicitReceiver]))
  [self setImplicitReceiver!: 'target-local-var]
  [self compileForms: body chunk: chunk]
  [self setImplicitReceiver!: savedImplicit])
```

then `compileLoadName` for a dot-sym emits:

```
LoadName <implicit-receiver-name>     ;; default: LoadSelf
Send :rest argc=0
```

this gives `with` a clean implementation path without touching the VM or env-lookup.

option A or B could also accommodate this, but more awkwardly — they'd need a per-frame "implicit receiver" slot, which is more invasive.

**decision:** ship option C with `LoadSelf` hardwired for V1 (`with` is out of scope). leave the compiler factored so the implicit-receiver source is a single named function — adding `with` later is mechanical.

## 7. performance

option C imposes **zero runtime cost**. the dot check happens at compile time, once per source occurrence of `.foo`. compile is amortized across many runs; for cached bytecode (the common case post-bootstrap), it's free.

compile-time cost: one extra `world.resolve(sym).starts_with('.')` per `compileLoadName` call. resolve is O(1) (interner is a `Vec` of `String`s, indexed by `SymId.payload()`). first-char check is O(1). a hot loop in the compiler running 1000 syms pays an extra 1000 nanosecond-class operations — negligible.

**mitigation if needed:** the symbol interner can pre-compute a "dot-prefixed" bit per sym at intern time. then `compileLoadName` reads the bit instead of resolving the string. ships only if profiling shows the resolve+starts_with is hot, which it won't be (compile is already dominated by env-allocation and bytecode-list-cons).

## 8. byte-equivalence in self-host

the v4 self-host plan compares moof-emitted vat-images to rust-emitted ones for bit-identity. today, both readers emit the desugared cons-list for `.foo`. byte-equivalence holds.

if we move the rewrite out of *both* readers and into the compiler, byte-equivalence at the source-Form level **changes** — the source-Form now contains the literal `.foo` sym instead of the cons-list. but the *bytecode* is unchanged (option C emits the same `LoadSelf; Send :rest argc=0` shape that the old cons-list compiled to). so:

- **vat-image at the source-slot level:** changes. methods' `:source` slots now contain the unmodified surface form.
- **vat-image at the bytecode level:** unchanged.
- **byte-equivalence between rust-emitted and moof-emitted v4 images:** preserved, as long as BOTH change in lockstep.

**migration plan:**

1. land the compiler change in moof (lib/compiler/01-dispatch.moof — `compileLoadName:chunk:`) AND the rust seed compiler (the compile-symbol-as-name path in crates/substrate/src/compiler.rs or wherever the seed's `LoadName` is emitted). both emit option C's two-op sequence when the sym is dot-prefixed.

2. remove the read-time rewrite from rust reader (crates/substrate/src/reader.rs:1218-1229). reader emits the raw `.foo` sym.

3. remove the read-time rewrite from parser.moof (lib/parser/02-parser.moof:172-205, the `isDotSym?` / `expandDotSym:atToken:` block and the test in `parseAtom:`). parser.moof emits the raw sym.

4. order: do steps 1 + 2 + 3 in one commit. neither rust nor moof can be in an intermediate state where readers emit raw `.foo` syms but compilers haven't been updated yet — they'd hit `'unbound name '.foo` at runtime.

5. run the full test suite under both rust-host and self-host paths. v4 byte-equivalence test should still pass.

a transition strategy that allows incremental rollout: keep the read-time rewrite in place AS a fallback path in the *compiler* for a release cycle, so that older code with serialized post-rewrite source can still be recompiled. but moof is pre-1.0; no serialized source is in the wild. ship it all at once.

## 9. reflection — inspecting method bodies

L6 — reflection is total. method `:source` slots carry the actual source-form. today, that's the post-rewrite cons-list. post-change, it's the literal form the user typed:

```moof
(defmethod (Counter incr)
  [self count: [.count + .step]])

;; today (current behavior):
[Counter handlerAt: 'incr] :source
;; → (defmethod (Counter incr) [self count: [(__send__ self 'count) + (__send__ self 'step)]])
;; — `.count` is gone; `(__send__ self count)` is in its place

;; post-change (option C):
[Counter handlerAt: 'incr] :source
;; → (defmethod (Counter incr) [self count: [.count + .step]])
;; — verbatim
```

this matters for:

- **interactive inspection** in the REPL and inspector — what the user sees matches what they wrote.
- **printing methods** — `inspect` of a method body shows surface syntax, not the desugared cons-tree.
- **doc comments** that depend on the source position — surface positions are preserved.
- **`__form-text`** — the FormLoc-based "verbatim source text of this form" mechanism (`project_source_canonical_value.md` in user memory) finally tells the truth about `.foo` lines.

**verdict: option C is the only design that preserves the source-form face of L5/L6.** options A/B preserve it too, but only if they ALSO change the reader; once the reader changes, option C is essentially free, and option C's other properties dominate.

## 10. bytecode emit shape

option C emits:

```
LoadSelf
Send :rest argc=0
```

(or whatever the implicit-receiver source is — see §6.)

**no new opcode** is introduced. `Op::LoadSelf` exists (vm.rs:429). `Op::Send` exists. inline-cache slot allocation works the same way as any other send. tail-position dispatch (`TailSend`) is *not* used here — dot-form is usually a sub-expression, not in tail position. (if a method body is *just* `.foo`, the dispatch wrapper around it puts the result in tail position; the chunk's last op is the Send, which will be flagged for tail by whatever surrounds it.)

interestingly, this is **bytecode-identical to today**. today the compiler sees `(__send__ self 'count)` and emits `LoadSelf; Send :count argc=0` via `compile_send` (rust seed) or `compileSend:` (moof). post-change, it sees `Sym(".count")` and emits `LoadSelf; Send :count argc=0` via `compileLoadName`. **same two ops, same constants, same arity, same IC slot allocation pattern.** the change is purely upstream of bytecode emission.

(this means the V4 vat-image byte-equivalence at the bytecode level is unchanged — see §8.)

## 11. interaction with `def` / `set!`

three sub-questions:

### 11.1. `(def .foo 42)`

today: the reader rewrites `.foo` to `(__send__ self 'foo)` *before* `def` dispatch sees it. so `(def .foo 42)` becomes `(def (__send__ self 'foo) 42)` — which is a malformed `def` (its first argument isn't a sym) and raises a compile error.

post-change: `(def .foo 42)` reaches `compileDef:` with `.foo` intact. should `def` accept it?

**proposal: no.** `def` binds a name in `$here` (after V3, via the def macro: `[$here bind: 'name to: value]`). a dot-prefixed sym is not a "name" — it's a *send sugar*. binding the literal sym `.foo` into `$here` would be confusing and most likely a typo. `compileDef:` should validate: first arg is a sym, sym does NOT start with `.`. raise `'invalid-def-name` otherwise.

(this is a tighter validation than today's de-facto behavior. but it's the right one — under the rewrite-at-read model, `(def .foo 42)` failed at compile time anyway, just with a less informative error.)

### 11.2. `(set! .foo 42)` → keyword assignment?

ruby has `obj.foo = 42` meaning `obj.foo=(42)`, a setter call. moof's analogue would be `(set! .foo 42)` → `[self foo: 42]`. tempting.

**proposal for V1: out of scope.** `set!` already has a precise meaning (mutate a lexical binding, post-V3 raises if unbound). overloading it for setter calls conflates two distinct operations. and moof already has a clean setter idiom: `[self foo: 42]` is the canonical way to write a setter call. there is no syntactic gap to close.

if this is later wanted, the cleanest path is a `.foo=` sigil (dot + name + equals) parsed as a keyword-setter symbol, or a `(.= self foo 42)` form. **but not by overloading `set!`.**

`compileSet:` should validate the first arg the same way `compileDef:` does — first arg is a sym, not dot-prefixed. raise `'invalid-set-name` otherwise.

### 11.3. summary

|form|behavior|
|---|---|
|`(def .foo 42)`|raise `'invalid-def-name` at compile|
|`(set! .foo 42)`|raise `'invalid-set-name` at compile|
|`.foo`|→ `[self foo]` at compile (option C)|
|`[self foo: 42]`|standard setter call (unchanged)|

## 12. edge cases

### 12.1. `..foo` — two dots

today: `..foo` is a single atom; the reader's dot-strip says "name starts with `.`, length > 1, ok" → emits `(__send__ self '.foo)` — a send to self with selector `.foo`. weird but not erroneous.

post-change: `..foo` is a sym whose name is `..foo`. `compileLoadName` checks "starts with `.`, length > 1, name is `..foo`" → strip one dot, emit `LoadSelf; Send :.foo argc=0`. **same as today.** the selector is `.foo`, which is a valid selector (selectors can be any sym). dispatching `.foo` on self looks up handler `'.foo` on self's proto.

**proposal: keep this behavior.** `..foo` strips exactly one dot. anyone writing it is doing something exotic (e.g. methods named `.x` for historical reasons), and the literal "strip one dot" rule is predictable. document as a curiosity, not a feature.

### 12.2. `.+`, `.-` — operator-named selectors

today: `.+` reads as the atom `.+`. dot-strip → emit `(__send__ self '+)` — a send to self with the `+` selector. so `.+` means "the value of `[self +]` with zero args" — which usually errors because `+` is binary. but the rewrite *succeeds* at read; the error is at dispatch time (insufficient arity).

post-change: identical. `compileLoadName` sees `.+`, strips, emits `LoadSelf; Send :+ argc=0`. runtime raises arity error.

**proposal: keep.** any selector-named-after-an-operator works syntactically; whether it makes semantic sense is the user's problem.

### 12.3. `.0` — digit after dot

today: reader's `try_parse_number` runs FIRST. `.0` parses as Float(0.0). dot-strip never sees it. so `.0` is the float zero, not a send.

post-change: `try_parse_number` order is preserved (it's in the reader, not the compiler; the reader runs whether we strip or not). `.0` is still a Float. **no collision.**

`.5e3` likewise parses as Float (500.0).

`.foo5` — name is `.foo5`, not numeric (Float parse would fail on the letter), so dot-strip applies, sends `:foo5` to self. unchanged from today.

### 12.4. `.` alone

today: reader has `if !rest.is_empty() && rest != "."` — i.e., `.` alone is *not* dot-stripped; it's emitted as the sym `'.'`. `compileLoadName` would then look up `.` as a name, find it unbound, raise `'unbound`. (or, if a user has `(def . something)` defined, it succeeds. probably a bad idea.)

post-change: identical. the compile-time dot-recognition rule has the same guard: length > 1.

**proposal: keep `.` alone as an ordinary (and almost-always-unbound) sym.**

### 12.5. `.|`, `.;`, `.[` — punctuation after dot

`|`, `;`, `]`, `)`, `}`, `[`, `(`, `{`, `,` — all delimiters. the lexer stops the atom at them. so `.;` is two tokens: `.` and `;`. `.|` is two tokens: `.` and `|`. these cases don't form a single dot-prefixed atom; not a concern.

### 12.6. `.foo` followed by `:` — keyword-style selector?

`.foo:` — does the reader produce one atom or two? `:` is the keyword-arg marker INSIDE `[…]` sends. as an atom-char, `:` is not a delim — it can appear in atom names. so `.foo:` reads as a single atom `.foo:`.

post-change: `compileLoadName` sees `.foo:`, strips, emits `LoadSelf; Send :foo: argc=0`. dispatches `foo:` on self with zero args. zero-arg dispatch of a keyword-selector almost certainly raises arity error — `foo:` expects one arg.

context: `.foo:` outside a `[…]` bracket is meaningless today and meaningless post-change. inside `[obj .foo:]` it's an arity error. **no semantic change; document as "don't do this."**

### 12.7. `'.foo` — quoted dot-sym

today: reader sees `'`, expects a form, reads `.foo`, applies dot-strip → emits `'(__send__ self 'foo)` (a quoted three-element list). the user wrote `'.foo` expecting "the symbol whose name is `.foo`" and got a quoted dispatch-form instead. surprising.

post-change: reader sees `'`, expects a form, reads `.foo`, emits `'(.foo)` — wait, more precisely it emits `(quote .foo)` where `.foo` is a sym. quoting it preserves the sym verbatim. user gets what they wanted: the literal sym `.foo`.

**this is a meaningful behavior improvement.** users can now write `'.foo` and get the sym. e.g. `[obj sendSelector: '.foo argc: 0]` is now sensibly expressible.

### 12.8. `.foo` inside `'(.foo bar)` — quoted list with dot-sym

today: the dot-rewrite happens at read time and is unconditional. so `'(.foo bar)` becomes a quoted list `((__send__ self 'foo) bar)` — meaning the user's quoted DATA contains a dispatch-form. extremely surprising.

post-change: `'(.foo bar)` is a quoted list of two syms: `.foo` and `bar`. as data. as the user wrote. **this is the bigger win** — quoting now means quoting, even for dot-prefixed names.

### 12.9. backquote `` `(.foo ,x) ``

same as 12.8 — quasi-quote now preserves dot-syms as data. unquote at `,x` is unchanged. **win.**

## 13. boot order

no new boot-order concern. the compiler change lives in `lib/compiler/01-dispatch.moof` — already loaded before any stdlib that uses `.foo`. the rust seed compiler change is in-tree (compiled directly into the rust binary). reader changes are pure-deletion (removing the rewrite arm in both `crates/substrate/src/reader.rs:1218-1229` and `lib/parser/02-parser.moof:177-205`).

the order during a single commit:

1. add the dot-sym recognition arm to rust seed's `compile_load_name` (or equivalent).
2. add the dot-sym recognition arm to `lib/compiler/01-dispatch.moof::compileLoadName:chunk:`.
3. delete the dot-strip arm from rust reader.
4. delete the dot-strip arms from parser.moof.

steps 1+2 must precede 3+4. but all four can ship in one commit. the test suite is the gate.

## 14. exit criteria

post-change lands when:

1. `crates/substrate/src/reader.rs:1218-1229` — the `if let Some(rest) = text.strip_prefix('.')` block — is **deleted**. dot-prefixed atoms become regular syms.
2. `lib/parser/02-parser.moof:172-205` — `isDotSym?:form:`, `expandDotSym:atToken:`, and the conditional in `parseAtom:` — are **deleted**. parser.moof emits the raw sym.
3. rust seed compiler's "compile a sym as LoadName" path (find with grep `compileLoadName\|compile_load_name\|LoadName(name)` in `crates/substrate/src/compiler.rs` or `seed.rs`) gains a dot-prefix check: emits `LoadSelf; Send :stripped argc=0` (two ops) when the sym starts with `.` and is longer than 1 char. `self` and bare-self handling unchanged.
4. `lib/compiler/01-dispatch.moof::compileLoadName:chunk:` gains the same check. structure mirrors:

```moof
(setHandler! Compiler 'compileLoadName:chunk:
  (fn (sym chunk)
    (if [sym is 'self]
        [chunk emit: [Opcode loadSelf]]
        (if [self dotSym?: sym]
            (do
              [chunk emit: [Opcode loadSelf]]
              [chunk emit: [Opcode send: [self stripDot: sym] argc: 0]])
            [chunk emit: [Opcode loadName: sym]]))))
```

5. `compileDef:` and `compileSet:` validate that their name argument is non-dot-prefixed; raise `'invalid-def-name` / `'invalid-set-name` at compile time otherwise.
6. all existing tests pass. method `:source` slots for any handler containing `.foo` now show the literal `.foo` (verify: pick `Cons:length` whose body is `[1 + [.cdr length]]` — its `:source` slot should contain the literal `.cdr`, not `(__send__ self 'cdr)`).
7. new tests:
   - `dot_sym_compiles_to_load_self_then_send` — chunk's bytecode after compiling `.count` contains `LoadSelf` followed by `Send :count argc=0`.
   - `dot_sym_in_quote_preserves_sym` — `(quote .foo)` evaluates to the sym `'.foo`, not a cons-list.
   - `dot_sym_in_quoted_list_preserves_sym` — `'(.foo bar)` evaluates to a 2-element list of syms `(.foo bar)`.
   - `dot_sym_with_no_self_raises_no_handler` — `.foo` at top-level (where `self` is `Nil`) raises `'no-handler`.
   - `def_with_dot_name_raises_invalid_def_name` — `(def .foo 42)` raises `'invalid-def-name` at compile.
   - `set_with_dot_name_raises_invalid_set_name` — `(set! .foo 42)` raises `'invalid-set-name` at compile.
   - `method_source_preserves_dot_sym` — for a method whose body contains `.count`, the `:source` slot's printed form contains the literal `.count`.
   - `dot_dot_sym_sends_dot_prefixed_selector` — `..foo` compiles to `LoadSelf; Send :.foo argc=0`.
   - `dot_digit_parses_as_float` — `.5` is a Float, not a send (regression).
8. byte-equivalence test (v4 self-host): rust-emitted vat-image and moof-emitted vat-image match bit-for-bit. (the source-Form changed shape consistently in both readers; bytecode is unchanged.)

## 15. test plan (sketch)

unit tests in `crates/substrate/src/compiler.rs::tests` (or wherever the seed compiler's unit tests live):

- `compile_dot_sym_emits_load_self_and_send`
- `compile_bare_self_emits_load_self`
- `compile_ordinary_sym_emits_load_name`
- `compile_def_with_dot_name_raises`
- `compile_set_with_dot_name_raises`

unit tests in `crates/substrate/src/reader.rs::tests`:

- `reader_emits_dot_sym_as_single_sym` — read `.foo`, get `Sym(".foo")`, not a cons-list.
- `reader_preserves_dot_sym_in_quoted_list` — read `'(.foo bar)`, get a 2-element list of syms.
- `reader_parses_dot_digit_as_float` — read `.5`, get Float(0.5). (regression.)

integration tests in `crates/substrate/tests/dot_sym_e2e.rs`:

- `dot_sym_dispatches_to_self_in_method` — `(defproto Counter (slots c) (handlers [g] .c))`; `[c g]` returns the slot value.
- `dot_sym_raises_at_top_level` — running `.foo` at top-level (the boot frame) raises `'no-handler`.
- `dot_sym_inspect_preserves_surface` — `[m source]` for a method using `.foo` shows `.foo`, not the desugared cons-list.
- `dot_sym_quoted_is_pure_data` — `(quote .foo)` returns the literal sym; `[sym toString]` is `".foo"`.

self-host byte-equivalence test:

- `v4_image_byte_equivalent_with_dot_sym_methods` — load any stdlib file containing `.foo` usage; compare rust-emitted and moof-emitted v4-images byte-for-byte.

regression: full existing test suite. priority files to scrutinize:

- `lib/stdlib/cons.moof` (72 hits of `.foo` per the bootstrap grep — heaviest user).
- any method introspection / debugger tests.

## 16. out of scope (deferred)

- **`with` block and alternate implicit receiver.** §6 sketches the design. ship the compiler-side hook (one named function returning the implicit-receiver sym) so that `with` can later parameterize it without further compiler refactoring.
- **`.foo=` setter sigil.** §11.2. cleaner than overloading `set!`; deferred until a use case appears.
- **compile-time analysis for "dot-sym outside method context."** the analyzer / typer (typed-moof-seed) is the right home. defer.
- **`.@foo` or `.$foo` — composite sigils.** moof has clean rules for what each prefix means; combining them creates ambiguity. not pursued.
- **`Op::SendSelf foo argc`** — a single fused opcode. discussed under option B's relative; rejected because LoadSelf + Send is already two ops, the IC machinery works as-is, and adding one more opcode increases the substrate surface for no perf win. revisit if profiling shows the IC slot reuse is worth a fused op.
- **changes to `__send__` itself.** `__send__` remains the internal marker for `[…]`-bracket sends emitted by both readers. nothing in this spec changes that. `__send__` and `.foo` were correlated only because the read-time rewrite *also* produced `__send__`-cons-lists; with the rewrite gone, they're independent.

## see also

- `docs/syntax/sigils.md` — `.foo` row; update text to clarify that `.foo` is "a sym whose name starts with `.`, recognized at compile time as a no-arg self-send."
- `docs/syntax/overview.md` — the `.count ≡ [self count]` example is correct; add a footnote that the rewrite is compile-time, not read-time.
- `docs/laws/substrate-laws.md` — L5 (source is canonical) and L6 (reflection is total) — both newly honored for `.foo` cases post-change.
- `2026-05-10-self-host-and-rust-deletion-design.md` — v4 self-host plan. the dot-sym change is one element of removing duplication between rust and moof readers.
- `2026-05-09-vat-V3-here-form-design.md` §7-8 — `def` and `set!` becoming macros via `[$here bind:to:]` / `[Env current]`. this spec's §11 tightens their compile-time validation.
- `project_source_canonical_value.md` (user memory) — the source-as-canonical-value design that L5 implements. `.foo`-as-surface-sym is one more piece of source preservation.
