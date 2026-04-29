# brackets

> **four bracket shapes, each with exactly one meaning. zero
> implicit modes. read the bracket, know what kind of expression.**

| shape | meaning |
|---|---|
| `(...)` | code form: fn-call, special form, or list literal when quoted |
| `[...]` | message send (smalltalk-style; binary, unary, positional, multi-keyword) |
| `{...}` | object literal |
| `#[...]` | Table literal |

each shape is a different *grammatical voice*. the user reads the
bracket first; the contents are interpreted in that voice. there are
no rules like "if the second token is a keyword, this means
something else." the bracket disambiguates.

## `(...)` — code

inside `()`:
- the head is the callable (or a special-form keyword).
- subsequent positions are arguments.

```moof
(+ 1 2)                       ; fn-call to +
(if cond then else)           ; special form: if
(let ((a 1)) body)            ; special form: let
(def name expr)               ; special form: def
(quote (a b c))               ; special form: quote
'(a b c)                      ; sugar for (quote (a b c)) — a List literal
```

`(quote (a b c))` and `'(a b c)` produce a List (cons-cell-shaped
data, `concepts/lists.md`). everything else inside `()` evaluates.

## `[...]` — message send

inside `[]`:
- the first position is the *receiver*.
- the rest is parsed as a sub-form: unary, binary, positional, or
  keyword.

### unary send

```moof
[obj read]                    ; selector :read, no args
[5 abs]
[xs length]
```

### binary send

binary operator in the second position:

```moof
[a + b]
[x < y]
[v1 ?? default]
[xs | other]
```

binary operators are symbols composed entirely of:
`! @ # $ % ^ & * + - / = < > ? | & ~`.

### positional send

```moof
[obj method arg1 arg2]
[t at-put 0 'first]
[s replace 'old 'new]
```

selector is the second position (a regular identifier). args follow.

### multi-keyword send

```moof
[dict at: 'name put: "ada"]
[window draw: rect color: 'red weight: 2]
[5 between: 1 and: 10]
```

selector is the concatenation of keyword markers (`at:`, `put:`).
args alternate after each marker.

### cascade

multiple sends to the same receiver, separated by `;`:

```moof
[transcript
   show: "hi "
   ; show: "world"
   ; newline]
```

## `{...}` — object literal

inside `{}`:
- the first position (optional) is the proto.
- followed by `name: value` slot bindings.
- followed by `[header] body` method definitions.
- the result is a fresh object with those slots and methods.

```moof
{Counter count: 0 step: 1
  [incr]    [self count: [.count + .step]]
  [read]    .count}

;; without a proto (defaults to Object)
{x: 5
 [getter] .x}
```

(`syntax/object-literals.md` for full grammar.)

## `#[...]` — Table literal

inside `#[]`:
- bare expressions are positional entries.
- `key => value` pairs are keyed entries.
- whitespace separates entries.

```moof
#[1 2 3]                      ; positional only
#['name => "ada" 'age => 30]  ; keyed only
#[1 2 3 'tag => 'urgent]      ; mixed
#[(some-key-fn) => "x"]       ; computed key
#[]                           ; empty
```

(`syntax/literals.md` for full grammar.)

## `#Tag(...)` and `#Tag literal` — tagged literals

beyond `#[...]`, the `#` prefix introduces *tagged literals*. the
tag (a Type / proto) decides how to interpret the form:

```moof
#Date "2026-04-28"
#Url  "https://moof.witch"
#Re   "[a-z]+"
#Counter(count: 0 step: 1)
```

tagged literals are extensible: any user-defined proto can register
a `:read-literal` handler.

## explicit nesting

moof has *no operator precedence* on binary sends. chained binaries
require explicit nesting:

```moof
[[a + b] * c]                 ; correct
[a + b * c]                   ; ERROR — ambiguous
```

unary chains likewise:

```moof
[[obj a] b]                   ; chain: a, then b on result
[obj a b]                     ; positional send :a with arg b
                              ; (NOT a chain)
```

(this strictness is intentional. see `concepts/sends-and-calls.md`.)

## inspirations

- `()` from lisp (mccarthy).
- `[]` for send from smalltalk-80 (kay et al.) — though smalltalk
  uses `[]` for blocks and bare juxtaposition for send. moof
  inverts: `[]` is the *send* bracket, blocks are sigil-less.
- `{}` for object/data literal: javascript / lua / clojure tradition.
  moof uses it specifically for object literals.
- `#[...]` for Table: clojure-flavored tagged-literal extension.

## see also

- `syntax/literals.md` — Table, List, and other literal syntax.
- `syntax/object-literals.md` — `{Proto …}` deep-dive.
- `syntax/methods-and-handlers.md` — method headers.
- `concepts/sends-and-calls.md` — semantics of dispatch.
