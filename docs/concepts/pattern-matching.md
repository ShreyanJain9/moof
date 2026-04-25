# pattern matching

**type:** concept (design doc)
**status:** designed, not built

> moof's pattern-matching has not been built yet. this doc walks the
> design space and proposes a moof-shaped synthesis. tangential to
> wave 11's definition-bundle thread but worth working out before
> we commit code.

## what we're trying to solve

today, in moof, you destructure by hand:

```moof
(if [v is: Some]
  (let ((x v.value)) [x + 1])
  (if [v is: None]
    0
    "neither"))
```

verbose, easy to forget a case, no exhaustiveness check.
defprotocol's `(provide (greet) ...)` clauses use a parser-level
shape match, but that's a one-off; users can't write patterns
of their own.

we want:

- destructure values by shape (`(Some n)` → bind n)
- check exhaustiveness against sealed types (`Option = Some | None`)
- allow user-extensible patterns (any value can be a pattern via a protocol)
- compose patterns (and-of-patterns, or-of-patterns)

without sacrificing what moof is:
- everything is a value (patterns included)
- protocols, not magic
- no compiler-only sugar where moof code suffices

## prior art (the four tastes)

### little smalltalk — Switch as an object

```smalltalk
Switch new
  case: 1 do: [:x | 'one'];
  case: 2 do: [:x | 'two'];
  default: [:x | 'other'].
```

a Switch is a value. cases are blocks. dispatch is by message
send. zero compiler magic. very moof-shaped — patterns are
already values, just objects with `case:do:` and `default:do:`.

cost: no destructuring. you check `=`, get the value, work with
it manually. and the syntax is verbose for nested cases.

### ruby — class-based ===

```ruby
case v
when Integer then "int"
when 1..10   then "in range"
when /^foo/  then "starts with foo"
when [a, b]  then "pair: #{a}, #{b}"
end
```

every class defines `===` with its own match semantics: `Integer
=== x` is `x.is_a?(Integer)`; `Range === x` is `x.between?(...)`;
`Regexp === x` is regex match; `Array === x` does pattern
destructuring (3.0+).

elegant key insight: **`===` IS the universal match protocol**.
any value answers "do you match this candidate?" — not just types.
a number is a pattern. a regex is a pattern. a class is a
pattern. an array is a pattern. it's all just `lhs === rhs`.

cost: ruby's destructuring story is bolted on; the binding side
of patterns isn't as natural as `===` itself.

### elixir — VM-level destructuring

```elixir
case v do
  {:ok, value}      -> value + 1
  {:error, reason}  -> log(reason)
  _                 -> "miss"
end
```

elixir's compiler turns each pattern into branches with bindings.
`{:ok, value}` is "does the candidate match a 2-tuple with `:ok`
in slot 0?" — if yes, bind the slot 1 value to `value`. lowercase
identifiers are binders; literals are matchers.

zero ambiguity because erlang's syntactic distinctions (atoms vs
variables) make the binder-vs-literal call lexical.

cost: needs compiler/runtime support; you can't add a new pattern
type by writing a function. (elixir bridges this with `Kernel.match?`
guards.) and the lowercase/uppercase lexical convention bleeds into
how variables look.

### haskell — algebraic types + exhaustiveness

```haskell
case v of
  Just n  -> n + 1
  Nothing -> 0
```

ADTs declare their constructors at type-definition time, so the
compiler knows every possible case and can warn on incompleteness.
patterns destructure by constructor shape, recursively.

cost: depends on a sound type system (moof has none). the compiler
needs to track which types are "closed" and what their variants are.

## what's moof-shaped

moof has:
- prototypes, not classes (but `Some`/`None` ARE prototypes)
- `[v is: Proto]` already does Ruby's `===`-by-type
- `[Option variants]` already exists (returns `(Some None)` for
  the sealed Option type) — exhaustiveness data is already in
  the image
- vau lets us build `case` as a moof form, no parser change
- Set + Cons + protocols give us the building blocks

so: combine ruby's `===`-as-universal, smalltalk's matcher-as-value,
elixir's destructuring shape, haskell's exhaustiveness. each layer
adds capability without taking from the others.

## proposed design

four layers, bottom-up.

### layer 1 — `===` everywhere

a **Matcher** is anything that can answer `===: candidate`. the
default is structural equality:

```moof
(defmethod Object === (candidate)
  [self equal: candidate])
```

prototypes match any instance via `is:`:

```moof
(defmethod Object === (candidate)
  (if [self isPrototype]      ; meta-test; needs a hook
    [candidate is: self]
    [self equal: candidate]))
```

(meta-test exists already via the existence of `:typeName` on a
proto vs. the `[v is: Proto]` walk; we'd surface a `[v isPrototype]`
helper.)

users can override `===` on their own types to declare custom
matchers — a `Range`, a `Regex`, a `Predicate`, etc.

### layer 2 — destructuring via `unmatch:`

types that want to destructure declare an **Unmatchable** protocol:

```moof
(defprotocol Unmatchable
  "Types that can be pattern-matched into a bindings table.
   Given a pattern (which encodes which positions are
   binders) and a candidate, return either a Table of
   { binder-name → value } or nil to indicate no match."
  (require (unmatch: pattern with: candidate)
    "Try to match candidate against pattern. Return Table
     of bindings on success, nil on failure."))
```

each prototype implements its own `unmatch:with:`. `Some`'s:

```moof
(defmethod Some unmatch: (pattern) with: (candidate)
  ; pattern is (Some <slot-pattern>)
  ; candidate must be a Some
  (if (not [candidate is: Some]) nil
    [self unmatch-slot:
      [[pattern cdr] car]   ; the inner pattern
      candidate.value]))
```

Cons's would handle list patterns; Table's would handle dict
patterns; user-defined types extend it.

`unmatch-slot:` is the recursive entry: it dispatches on the
inner pattern's shape — a binder symbol, a literal, a nested
constructor pattern.

### layer 3 — the `case` vau

```moof
(case v
  ((Some n)            [n + 1])
  (None                0)
  ((list a b)          (str a "-" b))
  (Integer             "an int")            ; type match via ===
  ([x where: |x| [x > 100]]  "big")          ; predicate via ===
  (_                   "miss"))
```

`case` is a vau: receives the value form + clause forms unevaluated.
for each clause:
- if pattern is `_` → matches anything
- if pattern is a literal (number, string, nil, true/false) → match via `===`
- if pattern is a bare symbol (lowercase) → it's a binder; matches anything, binds
- if pattern is a bare symbol (uppercase, resolves to a prototype) → match via `===`
- if pattern is a cons `(Constructor args...)` → invoke `[Constructor unmatch: pattern with: candidate]`
- otherwise → eval the pattern, match via `===`

generated code per clause:

```moof
(let ((bindings (...try-match...)))
  (if (some? bindings)
    (eval body in (env + bindings))
    (next-clause)))
```

binder-detection (lowercase vs uppercase symbol) is the one
syntactic convention we'd need. it matches moof's existing
informal practice (Some/None are types, x/n are values).
alternative: explicit binder marker like `?n` if we don't want
the convention to be load-bearing.

### layer 4 — exhaustiveness via `:variants`

`(defprotocol ... [sealed?] true)` already lets a type declare
itself sealed, and `Option` lists its variants. when `case` runs
against a sealed type, it can check that every variant has a
clause and warn (compile-time? at-eval-time? both possible).

```moof
(case (Some 5)
  ((Some n) ...)
  ; case complains: "missing variant: None"
  )
```

this is moof's flavor of haskell's exhaustiveness — opt-in per
type, not compiler-mandated.

## what's nice about this

- **patterns are values.** you can `(def my-pattern (Some _))` and
  use it in many cases.
- **`===` is the surface.** any object can be a matcher by
  implementing one method.
- **destructuring extends per-type.** add a new prototype, write
  its unmatch:, get pattern matching for free.
- **case is a vau.** no parser change. lives in lib/, editable.
- **exhaustiveness rides on `:variants`.** no separate type system.

## what's hard

- **the lowercase-binder convention.** ambiguous for symbols that
  are both lowercase AND globally bound (e.g. user has
  `(def n 5)` and writes pattern `(Some n)` — does n bind or
  match-by-equality with 5?). resolution: pattern position is
  always a binder unless it's already a known prototype. or use
  explicit binder marker.
- **nested matching cost.** evaluating clauses sequentially is
  O(n) per case. acceptable for now. compilation could later
  decision-tree it.
- **moof has no static analysis** for "did you cover every
  variant?" — runtime check on the first un-covered candidate is
  the workable substitute.
- **the `===`-Object default.** every value answers `===: c`. we'd
  default to `equal:`. no surprise unless someone overrides
  `===` on Object (which they shouldn't).

## smaller starting point

before all four layers, just layer 1 + a dumb case form:

```moof
(case v
  (Some        [v.value + 1])     ; type match via ===
  (None        0)
  (_           "miss"))
```

no destructuring; users access `v.value` manually. ~30 lines of
moof. proves the shape. layer 2 (unmatch:) lands when there's a
clear use that's painful enough to want it.

## related — what already exists

- **`:source.form` carries pattern-shaped AST already** — we have
  cons cells in slot values. no new data shape needed.
- **`Sealed` protocol exists** for `:sealed?` and `:variants`.
- **`is:` walks prototype chains** — already half of `===`.
- **Result/Option** are the natural test bed; they already have
  `then:` / `recover:` which IS pattern matching, just on a fixed
  schema. a generalized `case` would let us write more without
  inventing new methods per type.

## sequencing if we build this

1. **layer 1** — define `===` on Object as `equal:`. add `===`
   for Cons, Set, Table (structural). add `===` for prototype
   values (delegates to `is:`).
2. **a minimal `case`** — vau that walks clauses, calls `===`,
   no destructuring. wildcard `_`. ~50 lines.
3. **destructuring (layer 2)** — Unmatchable protocol; conform
   Some/None/Ok/Err/Cons; case starts handling
   constructor-shaped patterns.
4. **exhaustiveness** — when the matched value's prototype has
   `[:sealed?]` true, warn at runtime if a variant is uncovered.
5. **Switch as data** — let users build patterns programmatically
   via a Switch value, not just via the `case` form.

phase 1+2 would be a solid afternoon. phase 3 is a real spike.
phases 4–5 are polish.

## open questions

- do we want patterns to be **values** (a la Smalltalk Switch)
  AND a vau form (a la Elixir case)? probably both: the vau is
  the convenience surface, the value is the substrate. case
  desugars to a Switch.
- is `===` the right name? it's ruby's. moof might prefer
  `matches?:` to read as a method send. `[Some matches?: v]`
  reads naturally. could expose both — `===` as an alias for
  `matches?:` on Object.
- does `(Some n)` pattern parse as a regular cons cell, with
  `case` interpreting it? yes — minimum surprise. no new syntax.
- what about guards (`((Some n) where: [n > 0] ...)`)? probably
  layer ~2.5: a pattern can have a `[where: predicate]` qualifier.

---

## decision

park this until wave 11.4 or wave 12. what we have today (manual
`is:` + `then:`) is functional; the design is captured, ready to
build when the right use case demands it. small starting point
(layer 1 + minimal case) is 1–2 sessions of focused work whenever
the priority lines up.
