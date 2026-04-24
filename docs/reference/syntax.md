# syntax

**type:** reference

> moof's surface syntax: s-expression-based, three bracket species,
> minimal but extensible sugar.

---

## three brackets

| form | meaning |
|------|---------|
| `(f x y)` | applicative call — desugars to `[f call: x y]` |
| `[obj sel: arg]` | message send — the primitive |
| `{ Proto slot: val [handler] body }` | object literal |

use whichever reads best. they all reduce to sends.

```moof
(+ 1 2)              ; [+' call: 1 2] — applicative
[1 + 2]              ; [1 + 2] — message send (same result)
{ Point x: 3 y: 4 }  ; object literal with Point proto
```

---

## atoms and literals

```
42                   ; Integer
-7                   ; Integer (negative)
98765432345678876543456  ; BigInt — same Integer type, different storage
3.14                 ; Float
"hello, world"       ; String
'foo                 ; Symbol
'(a b c)             ; quoted list
true                 ; TRUE (Boolean)
false                ; FALSE
nil                  ; NIL
```

strings support `\n`, `\t`, `\"`, `\\`. no other escapes (by
policy — keep the reader simple).

integers that overflow i48 promote automatically to BigInt; user
sees one Integer type.

---

## message sends — the primitive

`[receiver selector: arg more-keyword: arg ...]`

- **unary**: `[obj describe]` — no colon, no args
- **binary**: `[a + b]` — single non-alphanumeric op, one arg
- **keyword**: `[list at: 3 put: 99]` — colon-suffixed parts,
  one arg each, selector is the concatenation (`at:put:`)

```moof
[x abs]                      ; unary
[3 + 4]                      ; binary
[table at: 'name put: "joe"] ; keyword
[obj foo: 1 bar: 2 baz: 3]   ; three-keyword
```

keyword selectors read as sentences. this is smalltalk's gift.

---

## applicative sugar

`(f a b)` desugars to `[f call: a b]`. closures have `call:`, so
applicative form just drives the `call:` handler. this is why
function-as-first-class works uniformly.

```moof
(f 1 2 3)     ; [f call: 1 2 3]
(map f lst)   ; [map call: f lst]
((fn (x) [x + 1]) 5)   ; [(fn (x) ...) call: 5]   ; → 6
```

---

## object literals

`{ Parent slot: val ... [handler] body ... }`

```moof
{ Point x: 3 y: 4 }                     ; slots only
{ Object [describe] "hello" }            ; handler only
{ Point x: 3 y: 4                        ; both
  [describe] (str "(" @x ", " @y ")") }
```

- the **first symbol** inside the braces is the prototype
  reference. may be `Object` (default).
- slot clauses are `name: value`.
- handler clauses are `[selector] body` (unary) or
  `[selector: args] body` (keyword/binary).
- `@name` is sugar for `[self slotAt: 'name]` — access own slots
  inside handler bodies.

---

## special forms

```
(def name value)        ; bind name to value in current env
(defn name (a b) body)  ; sugar: (def name (fn (a b) body))
(fn (args) body)        ; anonymous function
|a b| body              ; short lambda — (fn (a b) body)
(let ((x 1) (y 2)) body); local bindings
(quote x) / 'x          ; quote: returns x unevaluated
(if c a b)              ; conditional (itself a message send)
(do e1 e2 ...)          ; sequence; monadic when inner is Act/Cons/etc.
(match val (pat body)...)  ; destructure + dispatch
(vau (args) $env body)  ; fexpr — operative, sees env, unevaluated args
```

most of these are LIBRARY — written in moof on top of `vau`. the
VM only knows a few primitives (`def`, `fn`, `quote`, a couple
more). `if`, `let`, `match`, `do` — all moof-level defines.

---

## the reader extras

```
'foo              ; quote: (quote foo)
`(a ,b ,@cs)      ; quasiquote + unquote + unquote-splicing
obj.slot          ; (slot access) = [obj slotAt: 'slot]
@slot             ; self slot access in handlers
#{ ... }          ; generic sugar form (rarely used; reserved)
#[ ... ]          ; empty table literal (also seq/map initializer)
<-                ; eventual send (in [obj <- sel: arg])
|a b| expr        ; short lambda — (fn (a b) expr)
|_| expr          ; one-arg lambda with ignored arg
||                ; zero-arg thunk
;  ...            ; line comment (to end of line)
```

---

## short lambdas

```
|x| [x + 1]       ; (fn (x) [x + 1])
|a b| (+ a b)     ; two args
|_| 'anything     ; ignored arg
||  42            ; no args
```

short lambdas are the 90% case. `fn` is the long form.

---

## do-notation

`(do ...)` sequences expressions. inside, `(x <- expr)` is Monadic
bind; `(x = expr)` is pure let; a bare expression is run for its
effect:

```moof
(do
  (user <- [users <- get: 'current])    ; bind — awaits Act
  (greeting = (str "hello, " user.name)) ; let — pure
  [console <- println: greeting])        ; effect — run for side effect
```

works on anything Monadic: Act, Cons, Option, Result, Update. one
syntax across them all.

---

## pattern matching

```moof
(match val
  (0 "zero")
  ((Some x) (str "some " x))
  ((Cons a b) (str "cons " a " " b))
  (_ "other"))
```

- integer literals, strings, symbols match by equality.
- `(Proto args...)` destructures by prototype + slots.
- `_` matches anything, binds nothing.
- plain symbols bind the matched value.

match is a derived form — works via `vau`. you can write your own
match if ours doesn't fit.

---

## comments

```moof
; line comment
; extends to end of line, no block form.
```

comments are stripped by the lexer. no block comments by choice —
keeps the reader simple and encourages breaking up your code.

---

## whitespace, parens, and indentation

whitespace is mostly insignificant. parentheses delimit forms.
indentation is by convention — moof doesn't use offside-rule.

formatting conventions (not enforced by the compiler):

- two-space indentation
- opening brace on the same line as the form
- closing brace on its own line for multi-line forms
- keyword sends align keywords vertically when there are many

```moof
; good:
[obj foo: 1
     bar: 2
     baz: 3]

; also fine:
[obj foo: 1 bar: 2 baz: 3]
```

---

## reserved words? not really

moof has almost no reserved words. `if`, `let`, `match`, `do`,
`fn`, `def`, `defn` are all NAMES that evaluate to operative /
applicative values. you could rebind them if you wanted (though
you usually shouldn't). `true`, `false`, `nil` are the only
literal names the reader intercepts.

this is what `vau` buys you: user code can define its own
`unless`, `while`, `switch`, whatever, and they behave identically
to built-ins.

---

## what moof's syntax explicitly is NOT

- **no infix arithmetic** beyond binary message syntax. `[1 + 2 +
  3]` is NOT `(1 + 2 + 3)` — it's `[[1 + 2 +] 3]`, which is
  ambiguous and rejected. use `(+ 1 2 3)` or `[[1 + 2] + 3]`.
- **no implicit returns.** every expression is a value; the last
  one is the body's result. no `return` statement.
- **no blocks in the C sense.** `{ ... }` is object literal,
  not statement block. use `(do ...)` for sequencing.
- **no try/catch.** failures are Result values. use `[act recover: f]`
  to handle.
- **no while.** iteration is `[n times:]` or `[coll each:]` or
  recursion.
- **no `:=`.** mutation is Updates in servers. no in-place
  assignment.

---

## the full grammar (sketch)

```
program    := expr*
expr       := atom | list | send | literal | quote | sugar
atom       := symbol | integer | float | string | boolean | nil
list       := '(' expr* ')'
send       := '[' expr expr* ']'
literal    := '{' symbol? (keyword-clause | handler-clause)* '}'
keyword-clause  := symbol ':' expr
handler-clause  := '[' selector ']' expr | '[' keyword-selector ']' expr
quote      := "'" expr | '`' expr | ',' expr | ',@' expr
sugar      := short-lambda | dot-access | eventual-send | at-slot | ...
```

this is not exhaustive — consult `crates/moof-lang/src/lang/lexer.rs`
and `parser.rs` for the authoritative grammar.

---

## what you need to know

- three brackets: `(...)`, `[...]`, `{...}`.
- message send is the primitive; applicative is sugar.
- everything is a send — there are no "statements."
- few reserved words; most "keywords" are moof-level definitions.
- sugar: quote, quasiquote, dot-access, @slot, short lambdas.
- no statements, no loops, no mutation operators — those are not
  missing features, they're rejections.

---

## next

- [../concepts/objects.md](../concepts/objects.md) — the
  semantics behind the syntax.
- [../concepts/messages.md](../concepts/messages.md) — what a
  send actually does.
- [vm.md](vm.md) — how the runtime handles it.
