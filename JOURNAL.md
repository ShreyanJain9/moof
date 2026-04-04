# MOOF Implementation Journal

A record of what was built, why, and what changed along the way.

---

## Day 3 — TCO, mutation, and the honest object model

Three big changes that make MOOF a real system instead of a toy.

### Tail call optimization

Added `OP_TAIL_APPLY` and `OP_TAIL_CALL` opcodes. The compiler threads a `tail` flag through compilation — the last expression in a body, branches of `if`, last expression in `do`, and the final call in `let` all propagate tail position. When a function call is in tail position, the VM replaces the current frame instead of pushing a new one.

Result: `(sum-tc 100000 0)` computes without stack overflow. Tail-recursive list operations are now practical.

### The `<-` operator

Replaced `set!` with `<-` — a vau operative that inspects its target form at macro-expansion time:
- `(<- x 42)` — symbol target → walks the env chain via `[env set: target to: val]`
- `(<- obj.x 42)` — dot-access target → slot mutation via `[obj slotAt: field put: val]`
- `(<- @x 42)` — self-field target → same as dot-access on self

All three forms in one vau, purely in bootstrap.moof. The only VM addition was `set:to:` on environments (walks the parent chain to find and mutate the binding).

### Type prototypes: the honest object model

The old `primitive_send` was a lie — a giant match statement in Rust that pretended to be message dispatch. You couldn't see it, override it, or introspect it.

**Now every primitive type has a real prototype object:** Integer, Boolean, String, Cons, Nil, Symbol, Lambda, Operative, Environment. They're defined in bootstrap.moof, inherit from Object, and have real handlers.

Non-arithmetic handlers (`describe`, `applicative?`, etc.) are fn lambdas written in MOOF. Arithmetic and other performance-critical operations are **native handler lambdas** — real callable Lambda objects whose body is a single `OP_PRIM_SEND` instruction that goes directly to the VM fast path.

```
(def plus [Integer handlerAt: '+])
(type-of plus)    => 'Lambda
[plus params]     => (self a)
(plus 3 4)        => 7
```

Native handlers are real values. You can extract them, pass them around, call them. They show up in `[Integer interface]`. The fast path is an implementation detail, not a semantic difference.

**Dispatch order in `message_send`:**
1. User handlers on GeneralObjects (delegation chain)
2. Type prototype handlers (real lambdas — native or moof-defined)
3. VM fast path fallback (only during bootstrap, before prototypes are registered)
4. `doesNotUnderstand:`

### Killed: Block

Block was a separate HeapObject variant for `{ :x body }` syntax. But `{}` became object literals, and blocks were semantically identical to lambdas. Removed Block from the heap, compiler, VM, and bootstrap. One less concept, zero expressiveness lost.

### Killed: NativeHandler marker

The first attempt at integrating arithmetic used `NativeHandler` — a non-callable marker type that `message_send` would intercept and route to the fast path. It worked for introspection but was a lie: you couldn't call it, compose it, or pass it to `map`. Replaced with real lambdas using `OP_PRIM_SEND`, which bypasses handler lookup to avoid infinite recursion while being fully callable.

---

## Day 1 — The kernel lives

Built the entire runtime from scratch in one session: lexer, parser, compiler, bytecode VM, and a bootstrap library written in MOOF itself.

**Architecture decisions:**
- **Bytecode from day one.** No tree-walking interpreter phase. The cons-cell AST compiles to bytecode, and bytecode is the canonical form. This means introspection, serialization, and persistence all operate on the same representation.
- **Six kernel primitives.** `vau`, `send`, `def`, `quote`, `cons`, `eq`. Everything else is derived. `if`, `lambda`, `let`, `while` — the compiler knows about them for efficiency, but they're semantically expressible in terms of the six.
- **Bootstrap from file.** The standard library (`lib/bootstrap.moof`) is loaded at startup, not hardcoded. The image should be self-modifying — you should be able to change MOOF from within MOOF.

**What works:**
- REPL with multi-line bracket balancing
- Full prototype-delegation object model (slots + handlers)
- Vau operatives with first-class environments (the reflective tower)
- `doesNotUnderstand:` for proxies and dynamic dispatch
- `(source fn)` introspection on any lambda/operative
- `(load "file.moof")` for loading code from files
- ~200 lines of standard library written in MOOF itself

## Day 4 — String operations and the rest-param fix

### String ops

Full string manipulation suite on the String prototype: `substring:to:`, `at:`, `indexOf:`, `split:`, `trim`, `startsWith:`, `endsWith:`, `contains:`, `toUpper`, `toLower`, `toSymbol`, `toInteger`, `chars`, `replace:with:`. All registered as native handler lambdas, all introspectable via `[String interface]`.

Symbol gets `asString`/`name` for extracting the raw name without the `'` prefix. Integer gets `asString`. Concatenation (`++`) now works with any type, not just strings — falls back to `toString`.

Added `str` helper to bootstrap: `(str "count: " 42 " done")` → `"count: 42 done"`. Variadic, clean.

### The rest-param fix

Found that `call_lambda` was using `list_to_vec` + positional indexing for param binding, which broke rest params. `(fn args body)` (bare symbol = capture all args) was silently only binding the first arg. Fixed all three call paths (call_lambda, OP_TAIL_APPLY, OP_TAIL_CALL) to use `bind_params`, which correctly handles positional, rest, and destructuring patterns.

This also fixed dotted-rest params like `(fn (a b . rest) ...)` in the TCO paths.

---

## Day 2 — The great syntax reform

Tackled two design problems that were making the language awkward to use day-to-day.

### Problem 1: Symbol syntax was redundant

Three ways to say "the symbol x": `'x`, `#x`, bare `x`. Two too many.

**Resolution:** Kill `#name` entirely. `'sym` is the only symbol literal. Removed `HashSymbol` from the lexer and all references in bootstrap/geometry.

### Problem 2: Objects were ceremony-heavy

Creating an object required `(object Parent #x 10)` with separate `(handle! obj #method (lambda (self) ...))` calls. Not declarative, too much plumbing.

**Resolution — a suite of changes:**

1. **Object literals with `{}`** — `{ Parent x: 10 y: 20 }` for thin data objects. Shape-based method detection: `key: value` is a slot, `key: (params) body...` is a method (auto-injects `self`).

2. **`obj.x` dot access** — Parser desugars tight dots (no whitespace) to `(%dot obj 'field)`, new `OP_SLOT_GET` bytecode. All slots are public. Had to disambiguate tight dots (field access) from loose dots (dotted pairs in `(a . b)`) at the lexer level — tight dots emit `DotAccess` token, loose dots emit `Dot`.

3. **`@x` self-field sugar** — `@x` desugars to `(%dot self 'x)`. One character to read your own slots inside a method body.

4. **`fn` as a vau operative** — Not compiler sugar. `fn` is defined in `bootstrap.moof` as a vau that constructs and evals a `lambda` form. Lives in MOOF, not the compiler. This is important: it means `fn` could be redefined by the user.

5. **`defmethod`** — Also a vau operative in bootstrap. `(defmethod Point describe () "a Point")` auto-prepends `self` to the param list and desugars to `handle!`.

### The dot ambiguity bug

The dot-access postfix logic in `parse_expr` was greedily consuming the `.` in dotted pairs like `(a . b)`. The parser saw `a.b` (field access) instead of `a . b` (dotted pair tail). Fixed by tracking whitespace in the lexer: tight dots (no preceding whitespace) emit `DotAccess`, loose dots emit `Dot`. Parser uses `DotAccess` for field chains and `Dot` for dotted pairs. Clean separation.

### Before and after

```lisp
; BEFORE
(def Point (object Object))
(handle! Point #x (lambda (self) [self slotAt: #x]))
(handle! Point #y (lambda (self) [self slotAt: #y]))
(handle! Point #describe (lambda (self) "a Point"))
(handle! Point #distanceTo:
  (lambda (self other)
    (let ((dx [[self x] - [other x]])
          (dy [[self y] - [other y]]))
      [[dx * dx] + [dy * dy]])))
(def make-point
  (lambda (x y) (object Point #x x #y y)))
(def pt (make-point 3 4))

; AFTER
(def Point { Object
  describe: () "a Point"
  distanceTo: (other)
    (let ((dx [@x - other.x])
          (dy [@y - other.y]))
      [[dx * dx] + [dy * dy]])
})
(def pt { Point x: 3 y: 4 })
```

Night and day.
