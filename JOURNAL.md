# MOOF Implementation Journal

A record of what was built, why, and what changed along the way.

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
