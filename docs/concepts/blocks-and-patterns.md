# blocks and patterns

> **a block is a closure literal: `|args| body`. patterns appear in
> every binding position. multi-clause definitions match patterns.**

## blocks

a block is moof's anonymous-function literal. the body is a single
expression. for sequencing, use `(do …)`.

```moof
|x| (* x x)                       ; one-arg block
|x y| [x + y]                     ; two-arg
|| (rand)                         ; nullary
|x …rest| [rest cons: x]          ; variadic — `…rest` collects extras

|x|                               ; multi-line — body still one expression
  (do
    [logger record: x]
    [x * x])
```

blocks are first-class Forms with `proto: Closure`. they capture
their lexical environment. they respond to `:call` and friends:

```moof
(let f |x| [x + 1])
(f 5)                             ; → 6 — fn-call syntax
[f call: 5]                       ; → 6 — explicit send
[f arity]                         ; → 1
[f source]                        ; → the source-form
```

(closures are reflectable like everything else; see
`laws/reflection-contract.md`.)

## the single-expression body

every block body is exactly one expression. this matches the haskell
function-clause shape (peyton-jones et al.) and lets the substrate
keep the closure-form simple. for multiple statements, wrap in
`(do …)`, which is itself an expression.

## patterns

patterns live in any binding position:

- block parameters: `|x| body`, `|0| body`, `|'(h …t)| body`
- method clauses: `[name |x|] body`
- `let` bindings: `(let ((pat expr)) body)`
- `def` clauses: `(def name |pat| body |pat| body …)`
- `match` form: `(match expr |pat| body |pat| body)`
- `try ... catch` clauses
- query rule heads: `(rule (head pattern …) :- body…)`

what a pattern can do:

```moof
|0|                  ; literal match
|'world|             ; symbol literal
|"hello"|            ; string literal
|n|                  ; bind n to anything
|_|                  ; wildcard, no binding
|n :: Nat|           ; type guard
|n where [n > 0]|    ; predicate guard
|'(h …t)|            ; destructure list: h is head, t is tail
|#[a b c]|           ; destructure table positionally
|#['name => n]|      ; destructure table by key
|{count: c step: s}| ; destructure object literal
```

patterns nest:

```moof
|#[name: n age: a address: '(street city _)]|
```

## multi-clause definitions

every `def` can have multiple clauses. the runtime tries each in
order; first match wins. fall-through to no match raises
`does-not-understand`-style error (or the next proto in the chain,
when defining methods).

```moof
(def fact
  |0|         1
  |n|         [n * (fact [n - 1])])

(def length
  |'()|             0
  |'(_ …rest)|      [1 + (length rest)])

(def area
  |c :: Circle|     [PI * .radius * .radius]
  |r :: Rectangle|  [r width * r height]
  |t :: Triangle|   [0.5 * t base * t height])

(def safe-divide
  |a 0|             nil
  |a b|             [a / b])
```

reading aloud: each `|pat|` is a clause head, the expression after is
the body. visually similar to haskell or erlang. the substrate
compiles each clause into a single multi-arity / multi-pattern
dispatch.

## the match form

an inline pattern dispatch:

```moof
(match thing
  '()                'empty
  '(x)               'single
  '(x …rest)         'many
  #[name: n]         "record with name $n"
  _                  'other)
```

`match` is an operative. it evaluates `thing` once, then tries each
pattern in order against the value, returning the body of the first
match. unmatched falls through to the wildcard or raises if none.

## patterns are forms

patterns are themselves Forms (`proto: Pattern`) with a `:match?`
handler. user-defined patterns can be added by extending the proto:

```moof
(defproto NonEmpty
  (proto Pattern)
  (handlers
    [match?: value]
      [(value length) > 0]
    [bindings]
      #[]))                         ; binds nothing

;; usage in a clause head:
(def first
  |xs :: NonEmpty|     [xs at: 0])
```

this puts the pattern matcher in user-modifiable territory — the
matcher itself is moof code (`process/docs-driven.md`). new patterns
can be added without touching rust.

## inspirations

- pattern-matched multi-clause definitions: haskell (peyton-jones
  et al.), and through it ML and erlang.
- single-expression block body: haskell (function shape) and self
  (where blocks-with-side-effects use explicit sequencing).
- the `|args|` block bracketing: original to moof, evocative of
  smalltalk's `[:x | ...]` minus the leading colon.
- patterns-as-Forms: extends self's slot/method unification to the
  pattern domain.

## see also

- `concepts/sends-and-calls.md` — how blocks are invoked.
- `concepts/types.md` — type ascriptions in patterns.
- `concepts/queries.md` — pattern matching at the relation level.
- `syntax/binding-and-defs.md` — `def`, `let`, multi-clause syntax.
