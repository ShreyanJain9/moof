# syntax overview

> **a one-page survey of moof's surface. four bracket shapes, a few
> sigils, ruby-style string interpolation, smalltalk-style message
> sends.**

```moof
;; ────────────────────────────────────────────────────────
;; LITERALS
;; ────────────────────────────────────────────────────────

'foo                            ; symbol
"hi, #{name}"                   ; string with interpolation
1, 1.5, 1/3, 3+4i, 0xff         ; numbers
'(1 2 3)                        ; List literal (cons-cell)
()                              ; empty List = nil
#[1 2 3]                        ; Table — array
#['a => 1 'b => 2]              ; Table — map
#[1 2 3 'tag => 'urgent]        ; Table — mixed
{Counter count: 0 step: 1       ; object literal
  [incr] (...)
  [read] .count}
|x| (* x x)                     ; block — single-expr body
#Date "2026-04-28"              ; tagged literal — extensible

;; ────────────────────────────────────────────────────────
;; CALLS, SENDS, BINDINGS
;; ────────────────────────────────────────────────────────

(foo x y)                       ; fn-call
(if cond then else)              ; special form
(let ((a 1) (b 2)) body)
(def name expr)
(def name :: Type expr)
(def name |0| body |n| body)    ; multi-clause pattern matched

[5 + 3]                         ; binary send
[obj read]                      ; unary send
[obj method arg1 arg2]          ; positional send
[dict at: 'foo put: 5]          ; multi-keyword send
[obj a ; b ; c: 5]              ; cascade

self                            ; receiver in method body
.count                          ; ≡ [self count]
[self count: 5]                 ; setter

;; ────────────────────────────────────────────────────────
;; SIGILS, AT A GLANCE
;; ────────────────────────────────────────────────────────

'foo            symbol
"foo"           string
.foo            self.foo (slot read or unary self-send)
?foo            just a name; meaningful only inside (query)/(rule)
$foo            capability convention
#Tag(...)       tagged literal
`,@             quote / unquote / splice
::              type ascription
->              fn-type arrow
=>              key→value in #[...]
|x|             block params
:-              datalog rule body separator
```

## brackets

| shape | meaning |
|---|---|
| `(...)` | code: fn-call, special form, list literal when quoted |
| `[...]` | message send |
| `{...}` | object literal |
| `#[...]` | Table literal |

(see `syntax/brackets.md` for the formal grammar.)

## the four reading modes

### lispy fn-call rhythm

```moof
(map |x| [x * x] xs)
(if cond
    'yes
    'no)
```

### smalltalk binary

```moof
[a + b]
[x ?? default]
```

### smalltalk keyword

```moof
[dict at: 'name put: "ada"]
[window draw: rect color: 'red weight: 2]
```

### object/data construction

```moof
{Counter count: 0 step: 1
  [incr] [self count: [.count + .step]]}

#[1 2 3 'tag => 'urgent]
```

freely mixed in one expression:

```moof
(if [count > threshold]
    [alarm ring: 3 with: 'urgent]
    [counter incr])
```

## a complete example

```moof
(defproto Counter
  (slots count step)
  (handlers
    [incr]              [self count: [.count + .step]]
    [incr-by: n]        [self count: [.count + n]]
    [decr]              [self count: [.count - .step]]
    [read]              .count
    [reset-with: |s :: Pos|]
                        (do
                          [self count: 0]
                          [self step: s])))

(let
  ((c {Counter count: 0 step: 1}))
  (do
    [c incr]
    [c incr]
    [c incr-by: 3]
    (println "$.read")))             ; → 5
```

(`syntax/brackets.md`, `syntax/literals.md`,
`syntax/binding-and-defs.md`, `syntax/methods-and-handlers.md`,
`syntax/object-literals.md`, `syntax/string-interpolation.md`,
`syntax/sigils.md` for the deeper specs.)

## conventions

- **predicate methods** end in `?`: `empty?`, `nil?`, `even?`.
- **mutating methods** end in `!`: `set!`, `push!`, `swap!`.
- **type / proto names** are `Capitalized`.
- **everything else** is `lower-kebab-case`.
- **caps** start with `$`: `$clock`, `$out`.
- **slots** are `lower-kebab-case` like other names.

## inspirations

surface syntax draws from: lisp/scheme (s-exprs, quoting), smalltalk
(`[]` send brackets, keyword messages, cascades), self (slot/method
unification), kernel (operatives), io (Message-as-tree spirit), ruby
(string interpolation, `?`/`!` suffixes, friendliness), haskell
(pattern-matched clauses, `::` type ascription), lua (Tables),
clojure (tagged literals via `#`).

## see also

each detail has its own doc:

- `syntax/brackets.md` — bracket meanings and grammar.
- `syntax/literals.md` — every literal form.
- `syntax/binding-and-defs.md` — `def`, `let`, multi-clause.
- `syntax/methods-and-handlers.md` — method definition.
- `syntax/object-literals.md` — `{Proto …}`.
- `syntax/string-interpolation.md` — `#{expr}` rules.
- `syntax/sigils.md` — quick sigil reference.
