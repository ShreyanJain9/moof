# bindings and definitions

> **`def` for top-level / proto-scope. `let` for local. multi-clause
> pattern matching everywhere. type ascription with `::`.**

## def

```moof
(def name expr)
(def name :: Type expr)

;; multi-clause / pattern matching
(def fact
  |0|     1
  |n|     [n * (fact [n - 1])])

(def length
  |'()|             0
  |'(_ …rest)|      [1 + (length rest)])

(def area
  |c :: Circle|     [PI * .radius * .radius]
  |r :: Rectangle|  [r width * r height])
```

`def` binds in the current scope (top-level: world / vat scope; inside
a proto: that proto's scope; inside a let: that let's scope, but
prefer `let` for that).

multi-clause defs: each `|pattern| body` is a clause. clauses are
tried in declaration order; first match wins. unmatched falls
through to the next proto in the chain (for methods) or raises
otherwise.

## let

```moof
(let ((a 1))                     ; single binding
  body)

(let ((a 1)                      ; multiple bindings (parallel)
      (b 2)
      (c (+ a b)))               ; ERROR: a/b not yet visible
  body)

(let* ((a 1)                     ; sequential bindings (each sees prior)
       (b a)
       (c [a + b]))
  body)

(let-rec ((fac |0| 1
                |n| [n * (fac [n - 1])]))  ; mutually recursive bindings
  (fac 5))
```

three flavors:
- `let` — parallel; bindings are independent.
- `let*` — sequential; each binding sees prior ones.
- `let-rec` — recursive; bindings can refer to each other (and
  themselves).

bindings can use patterns:

```moof
(let (((x y z) point))            ; destructure positional Tab/List
  [x + y + z])

(let ((#[name: n age: a] record)) ; destructure keyed Table
  "name=#{n}, age=#{a}")
```

## type ascription

`::` introduces a type assertion:

```moof
(def x :: Integer 42)
(def f :: (Nat) → Nat
  |0| 1
  |n| [n + (f [n - 1])])

(let ((x :: Integer 5))
  body)

|n :: Pos|                        ; in a block parameter

(if [count > 0]                   ; expressions can be ascribed too
    [v :: NonEmpty]
    nil)
```

ascriptions are checked when the analyzer is run; ignored when it's
not. (`concepts/types.md`.)

## the function-type arrow

```moof
:: (Nat) → Nat                   ; one arg
:: (Nat, Nat) → Bool             ; two args
:: () → Unit                     ; no args
:: (Nat) → ((Nat) → Nat)         ; curried
:: ($Console, Integer) → Unit    ; with caps
```

`->` (rendered as `→` in editors that support it) is the type-arrow.
left side is parameter types (parens-wrapped, comma-separated). right
side is return type.

ascii: write `->`. unicode: editor renders `→`. both parse the same.

## defop — operatives

operatives receive their args *unevaluated* (kernel: shutt 2010):

```moof
(defop unless [cond then-form]
  `(if (not ,cond) ,then-form))

(defop defproto [name & body]
  ;; user-extending the proto-definition macro
  …)
```

inside an operative body, `cond`, `then-form` are bound to the
*unevaluated source forms* the caller wrote. typical use: walk and
transform forms, then return a new form that gets evaluated in the
caller's environment.

`defop` is for adding new "special forms" to moof. user code can
make new ones. (one of moof's killer moldability features.)

## comments at definition sites

```moof
;: factorial of a non-negative integer.
;: tail-recursive form is in lib/numeric.
(def fact
  |0| 1
  |n| [n * (fact [n - 1])])
```

`;:` doc comments attach to the next definition. accessible via
reflection: `[fact meta doc]`.

## inspirations

- `def` and `let` from scheme (steele, sussman).
- `let*` and `letrec` from scheme (likewise).
- multi-clause pattern-matched defs from haskell (peyton-jones et al.)
  and erlang (armstrong et al.).
- `::` for type ascription from haskell (and rust takes it too).
- `defop` from kernel (shutt) and lisp's `defmacro` tradition.
- doc comments-as-meta from common lisp's docstrings.

## see also

- `concepts/blocks-and-patterns.md` — pattern grammar.
- `concepts/types.md` — what types can appear in `::`.
- `concepts/sends-and-calls.md` — operatives vs applicatives.
- `syntax/methods-and-handlers.md` — method definition specifically.
