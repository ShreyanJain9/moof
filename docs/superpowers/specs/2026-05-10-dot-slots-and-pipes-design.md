# `.foo` slot-lookup and pipe operators — design

> **status:** redesigned 2026-05-11. supersedes the previous spec
> (`2026-05-10-dot-env-lookup-design.md`, commit ac6b56c) which
> recommended a compile-time fused `LoadSelf; Send` emit. that
> direction was abandoned: it kept the method-call semantics that
> conflate slot-reads with arbitrary dispatch. the new direction
> treats `.foo` as a **pure slot read on self**, resolved via the
> env-lookup mechanism. while we are at it, this spec also covers
> two pipe operators (`|>` inside brackets and `(-> …)` threading)
> as part of the same "make moof feel less bracket-heavy" thread.
>
> **prior art:** read-time `.foo → (__send__ self 'foo)` desugaring
> currently lives in the rust reader (`crates/substrate/src/reader.rs:1218-1229`)
> and in the self-hosted parser.moof (`lib/parser/02-parser.moof:172-205`,
> commit 311a0a4). both will be **deleted**.
>
> **doc surface to update:** `docs/syntax/sigils.md` (the `.` row),
> `docs/syntax/overview.md` (the `.count ≡ [self count]` example),
> and a new short section in `docs/syntax/brackets.md` describing
> pipes. semantics change is user-visible (slot vs send) — flag it
> in the changelog.

## 1. scope and motivation

three threads converge in this spec.

**(a) the `.foo` problem.** today `.foo` is a read-time rewrite to
`(__send__ self 'foo)`. that has five problems, exhaustively listed
in the previous spec's §1 (parser magic, lost moldability, baked-in
`self`, two-reader drift, byte-equivalence fragility). they all
still apply. the previous spec's fix was to defer the rewrite to
the compiler. this spec goes further: `.foo` becomes a **slot read**,
not a **send**. method-style behavior must be requested explicitly
with `[self foo]`. the substrate already exposes `Heap slotOf:at:`
and `Heap slotSet!:at:to:` as the canonical primitives for reading
and writing a slot; `.foo` is sugar over those.

**(b) the implicit-receiver question.** today `.foo` is hardwired
to mean "the lexical `self`." but there is a useful generalization:
some scopes have a different "implicit target" — a `with` block, a
REPL focused on an object, the inspector's currently-selected
form. by making `.foo` resolve through the env-lookup mechanism, we
gain a single named hook (`with-target` env binding) where alternate
implicit receivers can be installed. no other call site changes.

**(c) the bracket-heaviness problem.** moof's `[recv sel args]` is
beautiful when one wants the smalltalk feel, but four-level chains
get noisy: `[[[[a foo] bar: 1] baz] qux]`. two surface idioms relieve
this. `|>` inside `[…]` brackets is the smalltalk-cascade-variant
that pipes the previous expression's result into the next selector.
`(-> …)` is a clojure-style threading macro that lives outside
brackets, useful for top-level pipelines. both share the
"make-the-result-flow-left-to-right" motivation. this spec defines
their reader-level and macro-level surface in lockstep with the
`.foo` change.

## 2. user-facing semantics — `.foo`

### 2.1. read

| form              | meaning                                                         |
|-------------------|-----------------------------------------------------------------|
| `.foo`            | `[Heap slotOf: self at: 'foo]` — slot read on lexical `self`    |
| `.foo` (no `self`)| raises `'no-self` at runtime                                    |
| `.foo` (no slot)  | raises `'no-such-slot` at runtime                               |
| `[self foo]`      | unchanged — explicit method send (may dispatch a handler)       |
| `[obj foo]`       | unchanged — method send on another receiver                     |
| `[self count]`    | unchanged — explicit send. *not the same as `.count` anymore*   |

the **key behavioral change** from today: `.foo` no longer falls
back to method dispatch. if `self` has a *handler* `foo` but no
*slot* `foo`, then `.foo` raises `'no-such-slot`. the user must
write `[self foo]` to invoke the handler. this is intentional:
slot-reads and method-sends are distinct operations, and `.foo`
will denote the cheaper, simpler one.

### 2.2. write

| form                  | meaning                                                                 |
|-----------------------|-------------------------------------------------------------------------|
| `(set! .foo v)`       | `[Heap slotSet!: self at: 'foo to: v]` — slot write on lexical `self`   |
| `(set! .foo v)` (no `self`) | raises `'no-self`                                                  |
| `[self foo: v]`       | unchanged — keyword-arg send (typically a setter handler)               |

`set!` of a dot-prefixed name walks the env chain looking for `self`,
then writes the slot. it does **not** create the slot if absent —
`Heap slotSet!:at:to:` either updates the existing slot or raises
`'no-such-slot`, matching its native semantics today. (creating a
fresh slot via `.foo` is out of scope: that's what `defslot` is for.)

### 2.3. quote and quasi-quote — improved

| form              | today                                       | post-change                |
|-------------------|---------------------------------------------|----------------------------|
| `'.foo`           | quoted `(__send__ self 'foo)` cons-list     | quoted sym `.foo`          |
| `'(.foo bar)`     | quoted list with embedded send-list         | quoted list of two syms    |
| `` `(.foo ,x) ``  | quasi-quoted send-list                      | quasi-quoted sym-list      |

these were active bugs under the read-time-rewrite model — quoting
should mean quoting. they become tidy automatically once the rewrite
moves out of the reader.

### 2.4. examples

```moof
(defproto Counter (slots count step)
  (handlers
    [incr]
      (set! .count [.count + .step])     ;; reads .count, .step, writes .count
    [show]
      (print [.count toString])           ;; reads .count as a slot
    [doubled]
      [.count * 2]                        ;; .count → slot, * → method send
    [computed]
      [self count]))                      ;; explicit send — could dispatch
                                          ;; a handler, not just a slot
```

```moof
;; Cons:length — works identically post-change because Cons has
;; an actual `cdr` slot.
(defmethod Cons (length) [1 + [.cdr length]])

;; at the top level (self = nil):
.count                                    ;; → raises 'no-self
```

## 3. user-facing semantics — pipes

### 3.1. `|>` inside `[…]` brackets

`|>` is a **cascade-variant separator**: like `;` cascades the
*receiver*, `|>` cascades the *result of the previous segment*.

| form                            | desugars to                          |
|---------------------------------|--------------------------------------|
| `[a foo \|> bar]`               | `[[a foo] bar]`                      |
| `[a foo \|> bar \|> baz]`       | `[[[a foo] bar] baz]`                |
| `[a foo: 1 \|> bar: 2]`         | `[[a foo: 1] bar: 2]`                |
| `[a foo \|> bar: 2 baz: 3]`     | `[[a foo] bar: 2 baz: 3]`            |

each `|>` opens a fresh selector segment. within a segment the
existing `[…]` grammar applies (selector, keyword args, etc.). the
result of the preceding segment becomes the *receiver* of the next
segment.

`|>` and `;` may both appear in one bracket, with `;` taking the
saved-receiver semantics:

```moof
[a foo |> bar ; baz ; qux]
;; ≡ (let ((__r [[a foo] bar]))
;;     (do [__r baz] [__r qux] __r))
```

(this is the existing `;` cascade semantics with `[a foo |> bar]`
in the receiver position. design choice in §10.4.)

### 3.2. `(-> …)` threading macro

clojure-style first-position threading. lives at the call-form
level (outside brackets), as a regular macro.

| form                                   | desugars to                       |
|----------------------------------------|-----------------------------------|
| `(-> x)`                               | `x`                               |
| `(-> x foo)`                           | `[x foo]`                         |
| `(-> x [foo])`                         | `[x foo]`                         |
| `(-> x foo bar)`                       | `[[x foo] bar]`                   |
| `(-> x (foo a))`                       | `(foo x a)`                       |
| `(-> x [foo: 1])`                      | `[x foo: 1]`                      |
| `(-> x foo (bar 1) [baz: 2] qux)`      | `[[[(bar [x foo] 1)] baz: 2] qux]`|

each "step" after the initial value is one of:

- a bare sym `foo` — interpreted as a no-arg method send `[prev foo]`.
- a `[selector args…]` bracket — interpreted with `prev` as receiver, `[prev selector args…]`.
- a `(fn arg₁ arg₂ …)` call-form — `prev` is threaded as the **first arg**: `(fn prev arg₁ arg₂ …)`.

the macro's pleasant property is that the user reads top-to-bottom
left-to-right. compared to `|>` it works in non-bracket contexts
(e.g. as the body of a `defn`) and threads into function-calls, not
just into sends. `|>` is purely a bracket-internal sugar over the
existing `[…]` send-form.

### 3.3. `(->> …)` thread-last — provisionally yes

clojure also has `(->> …)` which threads into the **last** arg
position. moof's send-form has receivers in the *first* position,
so `->>` mostly makes sense for call-forms like `(map f xs)`:

```moof
(->> xs (filter even?) (map (fn (x) [x * 2])) (reduce +))
;; ≡ (reduce + (map (fn (x) [x * 2]) (filter even? xs)))
```

ship `->>` too. it's a sibling macro of `->`, ~5 lines of moof.
this answers open question §11.6.

## 4. implementation — `.foo` via env-resolution

### 4.1. the resolution rule

`World::env_lookup(env, name)` and `World::env_set(env, name, v)`
gain a **dot-prefix special case**, applied before the ordinary
slot-walk:

1. resolve `name` through the symbol interner; if its printed form
   starts with `.` and is longer than 1 character, take the
   **slot-read path**.
2. look up the symbol `self` in `env` via the *ordinary* env-walk
   (recursing into this rule is forbidden; `self` does not start
   with `.`).
3. if `self` is `nil` or unbound, raise `'no-self`.
4. strip the leading `.` from `name`, intern the rest as `field-sym`.
5. for `env_lookup`: call `Heap slotOf` (i.e. `heap.get(self).slots.get(field-sym)`).
   if the slot is absent on the *form itself*, raise `'no-such-slot`
   (do not consult proto chain — `.foo` is a slot-read, not a
   property-lookup).
6. for `env_set`: call the equivalent `Heap slotSet!` on `self` with
   `field-sym` and the new value.

step 5 is the key semantic point: slot-reads via `.foo` look only
at the form's own slots, *not* the proto chain. this matches
`Heap slotOf:at:`'s native behavior today (see
`crates/substrate/src/intrinsics.rs:1658-1662`). users who want
property-style "walk the chain looking for this field" should
write `[self foo]` and define a getter, or use `[Heap allSlotsOf:]`
explicitly.

### 4.2. why env-lookup, not the compiler?

the prior spec's option C emitted a fused `LoadSelf; Send` at
compile time. that bakes `self` and the method-dispatch path into
the bytecode, which:

- prevents future "alternate implicit receiver" extension (§5)
  without a compiler-side environment-tracker (workable but ugly).
- conflates slot-reads and method-sends in the bytecode — the
  semantic distinction we want is invisible.
- couples reader/compiler change to every host (rust seed AND the
  moof self-host compiler in `lib/compiler/01-dispatch.moof`).
  the env-lookup approach localizes the change to two functions
  in the substrate, with the bytecode op (`LoadName`) unchanged.

env-lookup is the single point of name resolution. it already
handles the `view-target` chain (V3 — see `world.rs:659-681`).
adding the dot-prefix rule is the same kind of extension and lives
side-by-side with it. one switch in one function (each, for
lookup and set), per host.

### 4.3. bytecode

bytecode emits **unchanged**:

```
LoadName '.foo                  ;; pushes the lookup-result on the stack
```

the VM's `Op::LoadName` handler dispatches to `env_lookup`, which
sees `'.foo` starts with `.` and runs the rule. no new opcode. no
peephole optimization. **completely transparent at the bytecode
level.**

(see §9 on a future fused `LoadSelfSlot 'foo` op — deferred work,
optional optimization, not part of this change.)

### 4.4. `self` resolution

`self` is bound in the env by every `Op::Send` / `run_method`
prelude. it lives as an ordinary slot of the method's local-env
form (along with arguments and locals). the env-walk for `self`
inside the dot-rule is just `env_lookup(env, self_sym)` with the
ordinary path (no dot, no recursion). cost: one extra walk per
`.foo` resolution; the env chain is typically 1–3 deep at most
(`method-env → block-env → method-env`). this is negligible.

### 4.5. `set!` on top-level

`(set! .foo v)` at the top-level (where `self` is `nil` or unbound)
**raises `'no-self`**. answer to open question §11.3.

rationale: `Heap slotSet!:at:to:` on `nil` is meaningless. raising
at the env-set boundary, with a clear `'no-self` selector, gives
the user a more useful error than "`'no-such-slot 'foo on Nil'`."

### 4.6. interaction with V3 `view-target`

V3's `view-target` chain is for inspector-style "view this form's
slots as if they were locally bound." it is *not* the same as the
implicit-receiver mechanism — `view-target` rebinds *every* name
lookup, not just dot-prefixed ones.

these two features coexist cleanly: when `env_lookup` sees a
dot-name, it takes the slot-read path *first* (looks up `self`,
slots it). only if `self` is unbound does it fall through to the
normal env-walk (which would then consult `view-target`). result:
`.foo` always means "slot on lexical self." `view-target` provides
a different (orthogonal) projection of all names.

## 5. future extension — `with-target`

the design preserves the ability to add an alternate implicit
receiver later, without changing bytecode or the resolution rule
in §4.1.

**proposed extension** (not in scope for this change): a
`with-target` env-meta key. when `env_lookup` is about to look up
`self` to resolve a dot-name, it first checks for `with-target` on
the env chain. if present, that form is the implicit receiver;
`self` is consulted only if `with-target` is unbound.

usage sketch:

```moof
;; future syntax — `with` is a macro
(with someObject
  .x .y .z)              ;; .x ≡ [Heap slotOf: someObject at: 'x]
;; expands to
(let ((__t someObject))
  (let-env ((with-target __t))    ;; hypothetical env-binding form
    (do .x .y .z)))
```

this would let the **inspector**, the **REPL `>` focus mode**, and
**`with` blocks** all share one mechanism: install `with-target`
on the surrounding env, every `.foo` in scope reads that target's
slot.

**not in scope for this change.** the env-meta key plumbing and
the `let-env` (or equivalent) form are independent designs. this
spec only commits to *not closing the door*. the resolution rule
in §4.1 step 2 reads "look up `self`"; the extension changes that
to "look up `with-target`, fallback to `self`." purely additive.

## 6. implementation — `|>` inside brackets

### 6.1. reader-level desugaring

the `[recv sel args]` reader already splits on `;` (cascade
separator). it gains a similar split on `|>` symbol-tokens.

```
[ a foo |> bar ]
```

reads as three tokens between `[` and `]`: `a`, `foo`, `|>`, `bar`.
the `|>` is recognized lexically as a 2-char punctuator (like `=>`
or `:-`). the reader inserts an implicit grouping:

```
[a foo |> bar]    ≡    [[a foo] bar]
```

algorithm (rust and parser.moof, in lockstep):

1. read `[`. accumulate tokens until matching `]`.
2. split the segment on `|>` tokens at top level (not inside nested
   brackets / parens). yields a list of "send-segments."
3. left-fold the segments: each segment is parsed as a regular
   `[…]` body. the result of segment N becomes the receiver of
   segment N+1.

step 3 is the same kind of fold the cascade-separator does, but
with "pipe the result" instead of "reuse the receiver."

`;` and `|>` can both appear in one bracket. when both are present,
`|>` binds **tighter** than `;` (i.e. `;` separates top-level
cascade segments; within each cascade segment, `|>` pipes
sub-segments). see §10.4.

### 6.2. parsing precedence

`|>` precedence inside brackets:

- **above** the entire send (a `|>` opens a fresh segment, not a
  selector token).
- **below** the keyword-arg join. `[a foo: 1 bar: 2 |> baz]` means
  `[[a foo: 1 bar: 2] baz]`, not `[a foo: 1 [bar: 2 |> baz]]`.
- **above** any arg-position expression. inside `[a foo: (1 |> 2)]`
  the `|>` is not a pipe — it's just a sym inside `(…)`, which is
  a call-form. there, `|>` is the value of a sym, almost certainly
  unbound, raises at runtime. (i.e. `|>` outside `[…]` brackets has
  no special syntactic meaning — see §10.5.)

confirms open question §11.5: **left-associative, lower than
keyword-arg, scoped to `[…]` brackets only.**

### 6.3. lexer change

`|>` must be a single token, not two. the lexer (rust and the
moof `lib/parser/00-lexer.moof`) already tokenizes 2-char
punctuators like `=>`, `:-`, `->`, `::`. add `|>` to that table.

care: pipe symbols `|`, `||`, `|||` are block-opener tokens (see
`parser.moof:121-128`). `|>` should not be misread as a pipe-opener
followed by `>`. easiest fix: in the lexer's atom-reader, when
scanning a `|`, peek the next char; if it's `>`, emit `|>` as a
2-char atom and stop. otherwise behave as today (run-of-pipes
becomes a block-opener token).

### 6.4. AST shape

a `|>` chain reads as a regular cons-list of `[…]` sends. **no new
AST node.** there's nothing for the compiler to special-case. the
reader does all the work.

this is symmetric with how `;` cascade is handled: the reader emits
a `(__cascade__ recv (seg1...) (seg2...) ...)` form, and the
`__cascade__` macro (`lib/early/06-control-macros.moof:50-60`)
expands it. for `|>`, the reader can just emit nested `[…]` forms
directly — no macro needed, because the desugaring has no
"share-receiver-across-segments" requirement.

## 7. implementation — `(-> …)` threading macro

lives in `lib/early/06-control-macros.moof` next to `__cascade__`.

```moof
;; (-> x)              → x
;; (-> x form₁ form₂ …) → walk: each step gets prev threaded in.
(defmacro -> (args)
  (if [args empty?]
      [self panic: "-> requires at least one form"]
      (let ((init [args car]))
        [self threadFirst: [args cdr] starting: init])))

;; threadFirst: forms starting: prev
(setHandler! self 'threadFirst:starting:
  (fn (forms prev)
    (if [forms empty?]
        prev
        (let ((step [forms car])
              (rest [forms cdr]))
          (let ((threaded [self threadStep: step into: prev]))
            [self threadFirst: rest starting: threaded])))))

;; threadStep: step into: prev — classify and emit.
(setHandler! self 'threadStep:into:
  (fn (step prev)
    (if [step isSym?]
        ;; bare sym → [prev sym]
        `[,prev ,step]
        (if [step isBracket?]                ;; a [...] send-form
            ;; [sel args…] → [prev sel args…]
            (cons '__send__ (cons prev [step cdr]))
            (if [step isCall?]               ;; a (...) call-form
                ;; (f a b) → (f prev a b)
                (cons [step car] (cons prev [step cdr]))
                [self panic: "-> step must be sym, [...], or (...)"])))))
```

(rough sketch — exact dispatch on `isBracket?` vs `isCall?` depends
on the parser's distinguishing tag. the macro just observes the
head of the form.)

`(->> …)` is the same skeleton but threads into the **last** arg:
`(->> x (f a b))` → `(f a b x)`. ship both.

### 7.1. why a macro, not reader-level?

`->` operates on call-form structure (it threads into `(f a b)`,
turning into `(f prev a b)`), which is a *post-read* operation.
the reader doesn't classify forms beyond "this is a sym" or "this
is a cons-list" — distinguishing a call-form from a bracket-form
from a bare sym is a job the macro does cleanly. macros are the
right tool here.

contrast with `|>` (§6): `|>` operates on the linear sequence of
tokens inside `[…]`, before the parser has even built the
send-form. that's reader work.

### 7.2. ordering

`(defmacro ->)` and `(defmacro ->>)` go in
`lib/early/06-control-macros.moof` after `__cascade__` /
`__table__` / etc. they have no boot-order constraints beyond
"defmacro must exist" — which it does, in
`lib/early/09-defmethod.moof` (already loaded earlier).

## 8. open questions — answers

(answers to the prompt's §"Open questions to address in spec".)

### 8.1. cons cells (`.cdr`, `.car`) — does anything break?

**no.** `Cons` has actual `cdr` and `car` *slots* (allocated by
`alloc_cons` in `world.rs`). today's `.cdr` desugars to
`(__send__ self 'cdr)` and dispatches `cdr` on a Cons, which lives
in the Cons proto's handler table as a thin wrapper that returns
the slot value. post-change, `.cdr` is `[Heap slotOf: self at: 'cdr]`,
which directly reads the slot. **same result, fewer indirections.**

(this is the most-trafficked dot-form in the codebase —
`lib/stdlib/cons.moof` has ~20 hits and `lib/early/00-cons.moof`
has more. all of these continue to work. no audit needed for Cons.)

### 8.2. types where `.foo` is a computed property — audit needed?

**yes — but the audit is bounded.** any proto where today's `.foo`
relies on a *handler* (not a slot) needs `.foo` rewritten to
`[self foo]`. the audit:

1. grep `lib/` for `\.[a-z]` occurrences.
2. for each occurrence, identify the receiver's likely proto (from
   surrounding `(defmethod ProtoName …)` context).
3. check whether `ProtoName` has an actual slot of that name. if
   yes: no change needed. if no (handler-only): rewrite to
   `[self foo]`.

candidate non-slot uses to scrutinize specifically (from a quick
scan):

- `Sym:toString` family — does `.text` resolve to a slot or to a
  handler? (a slot, today. check `world.rs` alloc_sym.)
- methods on `String` that use `.foo` — most likely slots, but
  check `String` proto setup.
- `Counter`-style examples in docs — explicitly slot-bearing.

migration commit can be a moof-only PR that follows the spec PR.
fix-forward, with the runtime `'no-such-slot` errors guiding the
audit.

### 8.3. `(set! .foo v)` at top-level — raise?

**yes — raises `'no-self`.** §4.5.

### 8.4. bytecode optimization — when?

**deferred.** the env-lookup change is sufficient for V1. a fused
`LoadSelfSlot 'foo` op can be added later as a peephole pass over
`LoadName '.foo` sequences. estimated payoff: one env-walk skipped
per dot-form per execution. small but real for hot dispatch loops
(e.g. `Cons:length`'s `[1 + [.cdr length]]`).

**decision criterion:** add `LoadSelfSlot` if profiling shows
`.foo` resolution as a hot path. if `cons.moof` traversals are
~20% slower after the change (measured against today's
fused-`LoadSelf; Send` bytecode), ship the peephole.

### 8.5. `|>` precedence inside brackets — confirmed

**left-associative, lower than keyword-arg binding, scoped to
`[…]`-brackets.** §6.2.

### 8.6. `(->> …)` — shipped

**yes, ship both `->` and `->>`.** §3.3.

## 9. alternatives considered

three alternatives were rejected. (the previous spec evaluated
options A, B, C; this spec re-examines them under the new "slot
read" semantics.)

### 9.1. option α — compile-time fused emit (previous spec's option C)

emit `LoadSelfSlot 'foo` (or `LoadSelf; SlotOf 'foo`) at compile
time. detect dot-prefix in `compileLoadName`.

**rejected because:**

- requires changes in two compilers (rust seed + moof self-host).
- closes the door on `with-target` style alternate-implicit-receiver
  without compile-time scope tracking.
- doesn't help with `(eval '.foo)` dynamic resolution.
- introduces a new opcode (`LoadSelfSlot` or `SlotOf`) for marginal
  gain.

revisit later as a peephole optimization (§8.4) once the env-resolution
path is stable and profiled.

### 9.2. option β — VM `Op::LoadName` runtime check

`Op::LoadName(name)`'s handler checks the sym's first char before
dispatching to `env_lookup`. if dot, take the slot-read path
directly.

**rejected because:**

- duplicates the dot-handling between the VM op-handler and any
  other code path that calls `env_lookup` (the parser uses
  `env_lookup` for `$here` resolution; future `view-target`
  features rely on it).
- the dot rule belongs *with* the resolution mechanism, not at the
  VM-op layer. centralizing it in `env_lookup` is the right
  factoring.

### 9.3. option γ — keep the read-time rewrite, just fix to slot-read

modify the rust reader and parser.moof to desugar `.foo` to
`[Heap slotOf: self at: 'foo]` instead of `(__send__ self 'foo)`.

**rejected because:**

- still loses the surface `.foo` token in `:source` slots (the
  L5/L6 problem from previous spec's §1).
- still duplicates the rewrite across rust + moof readers (drift
  surface).
- still bakes `self` into the read step (no `with-target`
  extension point).

all three motivations for moving the rewrite *out* of the readers
still apply; this option doesn't address any of them. only the
**target** of the rewrite changes (slot read vs send), not the
fundamental problem.

### 9.4. option δ — dot-form as a struct, not a sym

introduce `Value::DotSym(SymId)` as a distinct value variant; the
compiler emits a `LoadDotSym` op; resolution is direct.

**rejected because:**

- requires plumbing the new variant through every site that
  pattern-matches on `Value` — substantial code-change blast radius.
- `Value` enum is intentionally minimal; adding a variant for one
  syntactic sigil is heavyweight.
- the env-lookup approach achieves the same end with one branch in
  one function.

## 10. edge cases — `.foo`

(these are reproduced and re-decided from the previous spec under
the new "slot read" semantics.)

### 10.1. `..foo` — two dots

today: emit `(__send__ self '.foo)` — send `.foo` (with dot) to
self.

post-change: env-lookup sees `..foo`, takes the dot rule, strips
one dot, looks up `.foo` slot on self. if there's a slot named
`.foo` (almost certainly not), reads it; otherwise `'no-such-slot`.

**proposal: keep this behavior.** strip exactly one dot. `..foo`
is exotic; document as "name your slots without leading dots."

### 10.2. `.+`, `.-` — operator-named selectors

today: `.+` sends `+` to self with no args. typically an arity
error.

post-change: `.+` looks up a slot named `+` on self. no proto has
a `+` slot (`+` is a handler, not a slot). raises `'no-such-slot`.

**this is a behavior change.** previously `.+` at least *attempted*
a no-arg dispatch. now it definitively raises. **judgment: fine.**
no one writes `.+` in real code; the change makes the semantics
crisper.

### 10.3. `.0`, `.5e3` — digit after dot

today: reader's `try_parse_number` runs first; `.5` is `Float(0.5)`.
post-change: identical. parse-number still runs before dot-prefix
detection in the reader (and the env-lookup rule never sees the
text — it only sees `Sym`s).

### 10.4. `;` cascade together with `|>` pipe

```moof
[a foo |> bar ; baz ; qux]
```

three possible parses:

1. cascade-outer, pipe-inner: `(let ((__r [[a foo] bar])) (do [__r baz] [__r qux] __r))`.
2. pipe-outer, cascade-inner: `[[a foo] (let ((__r bar)) (do [__r baz] [__r qux] __r))]` — but `(let ...)` is not a valid bracket-segment.
3. associativity-aware: `[[a foo] bar]` cascaded with `baz; qux` segments.

**parse 1 is the chosen design.** `;` segments the entire bracket
contents first; within each segment, `|>` chains. equivalent
intuition: `;` is "lower-precedence" than `|>`.

(rationale: this matches how `;` works today — it's a
*top-level-of-bracket* separator. `|>` is a finer-grained
expression-level separator. don't disrupt the existing `;` model.)

### 10.5. `|>` outside `[…]` brackets

today: `|>` lexes as a 2-char sym. the symbol's value is
almost-certainly unbound. raises `'unbound`.

post-change: identical. `|>` only has special meaning inside `[…]`
brackets. outside, it's just a sym (which one could `def`-bind to
something if one really wanted).

(future: we could introduce `|>` as a stand-alone binary syntax —
`(a |> f)` ≡ `(f a)` — but that's a separate design. not now.)

### 10.6. `'.foo` inside quote — preserved

quoting now preserves the literal dot-sym. §2.3.

### 10.7. `(def .foo 42)` — invalid

`def` names cannot be dot-prefixed. compile-time error. this is
the same as the previous spec's §11 — applies post-change too.

### 10.8. `(set! .foo v)` — handled

handled via env-set's dot rule (§4.5). not a compile-time error,
but a runtime one when `self` is nil/unbound. (different from `def`
because `set!` is dynamically resolved — `self` may be bound in a
runtime context the compiler can't see.)

## 11. migration plan

**single PR** lands the spec:

1. add dot-prefix detection to `World::env_lookup` (rust) — ~15
   lines. the dot rule branches to a new helper, e.g.
   `Self::env_lookup_dot_slot(&self, env, stripped_name) ->
   Result<Value, RaiseError>`.
2. add dot-prefix detection to `World::env_set` (rust) — ~15 lines.
   mirror the helper, e.g. `env_set_dot_slot`.
3. delete `reader.rs:1218-1229` — the dot-strip branch in
   `read_atom`.
4. delete `lib/parser/02-parser.moof:172-205` — `isDotSym?:form:`,
   `expandDotSym:atToken:`, and the `parseAtom:` branch that uses
   them.
5. add the `|>`-token lexer rules — rust (`reader.rs`) and
   `lib/parser/00-lexer.moof`.
6. add the `|>`-splitting logic to `[…]` parsing — rust and
   `lib/parser/02-parser.moof`.
7. add the `(-> …)` and `(->> …)` macros to
   `lib/early/06-control-macros.moof`.
8. update `docs/syntax/sigils.md` (the `.` row): change "slot read
   or unary self-send" to **"slot read on self."**
9. update `docs/syntax/overview.md` (the `.count ≡ [self count]`
   line): change to `.count ≡ [Heap slotOf: self at: 'count]`.
10. update `docs/syntax/brackets.md` to document `|>` cascade-variant.
11. add a short section `docs/syntax/macros.md` (or wherever stdlib
    macros are documented) for `->` and `->>`.

**audit pass (separate PR):**

12. grep `lib/` for `\.\w+` usages.
13. for each, determine whether the underlying proto has the slot;
    if not (and the dot was relying on a handler), rewrite to
    `[self xxx]`.

**zig substrate** (future, post-rust-deletion):

14. when the V4 zig substrate's env-lookup is implemented, mirror
    the dot rule. one place. (note for the zig-port spec writer.)

**rollout order rationale:**

- steps 1+2 land *before* 3+4 in the same commit. otherwise the
  reader emits raw `.foo` syms which then fail env-lookup. since
  it's one commit, this is automatic.
- step 5 (lexer for `|>`) must precede step 6 (parser using `|>`
  tokens).
- step 7 (`->` / `->>` macros) is independent of `.foo` work.
  could ship in a separate PR; cleaner together.

## 12. byte-equivalence in self-host

the V4 self-host plan compares moof-emitted vat-images to
rust-emitted ones bit-for-bit.

today the source-Form for a method using `.foo` is the desugared
3-element cons-list. post-change it is the literal sym `.foo`.
**both readers change in lockstep** (step 3 and step 4 are paired).
the source-Form byte-equivalence holds.

bytecode also changes: today the method emits `LoadSelf; Send :foo
argc=0`; post-change it emits `LoadName '.foo` (the env-lookup does
the work at runtime). both readers/compilers reach the same
new bytecode shape. byte-equivalence holds at the bytecode level
too.

(the previous spec's option C kept the bytecode identical and
moved the rewrite to compile time. this spec's bytecode is
*different from today* but *identical across rust and moof* —
which is what byte-equivalence requires. either is fine.)

## 13. performance

per `.foo` resolution adds:

- one symbol-name resolve (interner string lookup): ~1 ns.
- one first-char check: trivial.
- one extra env-walk for `self`: one or two map-lookups against a
  shallow env chain. ~tens of ns.
- one `Heap slotOf` call: one map-lookup against the form's slots.
  ~10 ns.

today's `LoadSelf; Send :foo argc=0` path:

- one frame access (`LoadSelf`): ~1 ns.
- one IC-cached send (`Send` with hot IC): ~tens of ns.

**rough parity, possibly faster.** the env-walk for `self` and the
slot read are both simpler operations than the IC machinery and
the handler-dispatch indirection. preliminary estimate: post-change
`.foo` is ~10–30% faster on hot paths (because it skips the send
overhead entirely — no IC slot, no handler-table lookup, no
runtime tail-call setup). cold paths might be slightly slower due
to the env-walk for `self`.

if profiling shows pain, mitigations:

- **fused `LoadSelfSlot` op** (§8.4): one bytecode op, one VM op,
  no env-walk for `self` (the VM reads `frame.self_` directly).
  ~5 ns per resolution.
- **symbol-interner bit:** pre-compute "is dot-prefixed" per sym
  at intern time. `env_lookup` reads the bit. saves the
  string-resolve, but not load-bearing in this hot-path.

both mitigations are future work. ship the env-resolution baseline
first, profile, then optimize.

## 14. interaction with other in-flight specs

### 14.1. `2026-05-09-vat-V3-here-form-design.md`

V3 makes `def` and `set!` into macros that resolve through `$here`
/ `[Env current]`. when those macros expand `(set! .foo v)`, they
should pass the dot-sym through as-is to `env_set`; the dot rule
in `env_set` handles the rest.

**compatibility:** the V3 macros need to *not* validate "name is
not dot-prefixed" at compile time — that validation was a
proposal in the previous spec, now superseded. `set!` of a
dot-name is meaningful (slot write); compile-time refusal would
break it.

`def` of a dot-name is still meaningless (you can't bind `.foo`
into `$here` — it's not a name). either:

- the V3 `def` macro raises `'invalid-def-name` for dot-prefixed
  names. (clean, surfaces the error at compile time.)
- `def` passes it through to `env_bind`, which doesn't have a dot
  rule and writes a slot literally named `.foo` into `$here`.
  (works, but useless slot.)

**recommend: V3 `def` macro raises `'invalid-def-name`.** matches
the previous spec's intent for `def` validation, without breaking
`set!`.

### 14.2. `2026-05-10-self-host-and-rust-deletion-design.md`

the v4 self-host plan eliminates code duplication between rust and
moof readers. this spec is a direct contribution: the dot-strip
desugaring duplication goes away (deleted in both). the `|>`-token
and `[…]`-pipe-fold *new* duplication is small (lexer-table entry
+ split-on-token loop), and unavoidable until the rust reader is
fully retired in v4.

### 14.3. `2026-05-06-typed-moof-seed.md`

the analyzer can flag `.foo` outside method scope as "almost
certainly an error." new opportunity post-change: if the analyzer
can determine `self`'s proto in scope, it can check that the slot
`foo` is declared on that proto — catching typos at type-check
time. this is a *new* capability enabled by the slot-only
semantics (under the previous send-fallback semantics, the slot
might be backed by an inherited handler, harder to type-check).

### 14.4. `project_source_canonical_value.md` (user memory)

`:source` slots now contain the literal `.foo`. the FormLoc-based
`__form-text` mechanism gives verbatim source text including `.foo`
characters. one more piece of source-preservation tidied up.

## 15. exit criteria

post-change lands when:

1. `crates/substrate/src/reader.rs:1218-1229` (the dot-strip branch)
   is **deleted**.
2. `lib/parser/02-parser.moof:172-205` (`isDotSym?:form:`,
   `expandDotSym:atToken:`, the `parseAtom:` branch) is **deleted**.
3. `World::env_lookup` recognizes dot-prefixed names and routes to
   `Heap slotOf: self at: stripped`.
4. `World::env_set` recognizes dot-prefixed names and routes to
   `Heap slotSet!: self at: stripped to: value`.
5. raises:
   - `'no-self` when `self` is unbound at top-level.
   - `'no-such-slot` when the slot doesn't exist on `self`.
6. `|>` is lexed as a single token in both lexers (rust + moof).
7. `[a foo |> bar]` parses to `[[a foo] bar]` in both parsers.
8. `(-> x foo (bar 1) baz)` expands to `[[[(bar [x foo] 1)] baz]`
   in `lib/early/06-control-macros.moof`.
9. `(->> xs (filter even?))` expands to `(filter even? xs)`.
10. doc updates in §11.
11. all existing tests pass after the audit pass (§11 step 12).
12. v4 byte-equivalence test passes (both readers emit same forms,
    both compilers emit same bytecode).
13. new tests:
    - `dot_sym_at_top_level_raises_no_self`
    - `dot_sym_on_form_with_slot_reads_slot`
    - `dot_sym_on_form_without_slot_raises_no_such_slot`
    - `dot_sym_in_quote_preserves_sym`
    - `dot_sym_set_writes_slot`
    - `dot_sym_set_at_top_level_raises_no_self`
    - `pipe_inside_bracket_left_folds`
    - `pipe_combined_with_cascade_parses_cascade_outer`
    - `thread_first_macro_threads_into_first_arg`
    - `thread_last_macro_threads_into_last_arg`
    - `method_source_preserves_dot_sym` (regression — `:source`
      slot contains the literal `.foo`).

## 16. estimated effort

| component                              | rust loc | moof loc | days |
|----------------------------------------|----------|----------|------|
| `env_lookup` dot rule                  | ~30      | n/a      | 0.5  |
| `env_set` dot rule                     | ~30      | n/a      | 0.5  |
| delete reader desugaring               | -10      | -35      | 0.1  |
| `\|>` lexer token                      | ~10      | ~15      | 0.3  |
| `\|>` parser fold (`[…]`)              | ~30      | ~30      | 0.7  |
| `(-> …)` macro                         | n/a      | ~30      | 0.4  |
| `(->> …)` macro                        | n/a      | ~30      | 0.3  |
| audit `lib/` for handler-relying `.foo`| n/a      | varies   | 0.5  |
| docs updates (`sigils.md`, etc.)       | n/a      | n/a      | 0.3  |
| new tests                              | ~150     | ~100     | 1.0  |
| **total**                              |          |          | **~5 days** |

call it one focused week.

## 17. out of scope (deferred)

- **`with-target` env-meta key and the `with` macro** — §5.
- **`LoadSelfSlot` fused opcode** — §8.4.
- **dot-sym setter syntax `.foo= v`** — not needed; `(set! .foo v)`
  covers it.
- **`|>` as a standalone binary operator outside `[…]`** — §10.5.
- **compile-time analyzer check for "dot-sym slot exists on self's
  proto"** — §14.3. natural extension of typed-moof-seed.
- **zig substrate env-lookup dot rule** — §11 step 14.

## see also

- `docs/syntax/sigils.md` — the `.` row (will be updated).
- `docs/syntax/overview.md` — the `.count` example (will be updated).
- `docs/syntax/brackets.md` — `|>` documentation added.
- `docs/laws/substrate-laws.md` — L5 (source is canonical), L6
  (reflection is total), L3 (message dispatch is universal). the
  new semantics still respect L3 (slot reads are conceptually a
  send to `Heap`, which dispatches `slotOf:at:`).
- `2026-05-10-self-host-and-rust-deletion-design.md` — v4
  self-host plan; the `.foo` dedup is one piece.
- `2026-05-09-vat-V3-here-form-design.md` — `def` and `set!` as
  V3 macros; their interaction with the dot rule (§14.1).
- `2026-05-06-typed-moof-seed.md` — analyzer can statically check
  slot existence under the new semantics (§14.3).
- `project_source_canonical_value.md` (user memory) — source-form
  preservation; `.foo` literal-in-source is another piece (§14.4).
