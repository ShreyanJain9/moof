# literals

> **every literal form moof recognizes, in one place.**

## numbers

```moof
;; integers — base 10, hex, binary, octal
0, 1, -42, 1_000_000
0xff, 0xFF, 0xDEAD_BEEF
0b1010, 0b1100_0011
0o17

;; floats
1.5, .5, -3.14
1e9, 1.5e-3, 6.022e23
∞, -∞, NaN                       ; also #infinity, #-infinity, #nan

;; rationals
1/3, -7/2, 22/7

;; complex
3+4i, 0+1i, -2-3i
```

underscores `_` are allowed inside numeric literals for grouping.

## symbols

```moof
'foo                             ; standard
'hello-world                     ; kebab-case
'+                               ; operator-as-symbol
'at:put:                         ; keyword-style selectors
```

a symbol literal is `'` followed by a name. names allow:
`a-z`, `A-Z`, `0-9` (not first), `-`, `?`, `!`, `:`, plus the binary-
operator characters `+ - * / < > = etc.` for operator names.

(`concepts/forms.md`: every symbol is interned; same name ⇒ same
identity within a vat.)

## strings

```moof
"hello"
"hi, #{name}"                    ; ruby-style interpolation
"escape: \n \t \\ \" \#"
"unicode: \u{1f496}"
r"raw \n is two characters"      ; raw string
"""
multi-line strings
dedent on parse.
"""
r"""
raw triple-quoted.
"""
```

interpolation: `#{expr}` evaluates `expr` and converts the result via
`:to-string`. nested brackets and quotes inside `#{...}` are handled
recursively. (`syntax/string-interpolation.md`.)

## chars

```moof
#\h                              ; literal char
#\space                          ; named: space
#\newline
#\tab
#\u{1f496}                       ; codepoint by hex
```

## lists

```moof
'(1 2 3)
'(a b c)
'(foo (nested list) bar)
()                               ; empty list = nil
```

quasiquote / unquote / splice:

```moof
`(a ,b ,@c)                      ; quasi-quoted
;; ,b inserts the value of b
;; ,@c splices c's elements
```

(`concepts/lists.md` for List semantics.)

## tables

```moof
#[]                              ; empty
#[1 2 3]                         ; positional
#['name => "ada" 'age => 30]     ; keyed
#[1 2 3 'tag => 'urgent]         ; mixed
#[(some-key-fn) => "computed"]   ; computed key
#[nil => 5 't => 4]              ; arbitrary keys
```

inside `#[...]`:
- bare expressions are positional entries (in order).
- `key => value` pairs are keyed entries.
- whitespace separates entries; commas optional but allowed.
- nested tables and other literals are fine.

(`concepts/tables.md` for Table semantics.)

## blocks

```moof
|x| (* x x)
|x y| [x + y]
|| (rand)
|x …rest| [rest cons: x]

;; with type ascription / patterns in params
|n :: Pos| [n + 1]
|0|                  1            ; literal pattern (in multi-clause defs)
```

(`concepts/blocks-and-patterns.md`.)

## tagged literals

```moof
#Date "2026-04-28"
#Url  "https://moof.witch"
#Re   "[a-z]+"
#UUID "abc-..."
#Path "/users/shreyan/notes"
#Money(USD 12.50)
#Counter(count: 0 step: 1)       ; alternative to {Counter count: 0 step: 1}
```

`#Tag` followed by either:
- a string `"..."` (the literal payload is parsed by the proto).
- a parens `(...)` (positional + keyword args, like a constructor call).
- a Table `#[...]` (explicit table payload).

each Tag-proto registers a `:read-literal` handler that interprets
the payload. user-defined protos can register their own.

## boolean and nil

```moof
#true                            ; the boolean true
#false                           ; the boolean false
nil                              ; the empty list / "absence"
```

`nil` is the empty List — they are the same value. truthiness:
`nil` and `#false` are falsy; everything else is truthy (clojure
tradition).

## logic vars (in queries only)

```moof
?x, ?z, ?some-relation
```

`?name` is just a regular identifier. inside `(query ...)` and
`(rule ...)`, these are interpreted as logic variables. outside,
they're ordinary names (and almost certainly errors when looked up).

## comments

```moof
;; line comment
;: doc comment — attaches to following def
;~ scratch / fixme / temporary annotation

#| block comment
   spans
   multiple lines |#
```

doc-comments (`;:`) are first-class metadata: they attach to the
next definition's `meta.doc` slot, accessible via reflection.

## sigil cheat-sheet

| sigil | meaning |
|---|---|
| `'foo` | symbol literal |
| `'( … )` | List literal (quote) |
| `` ` `` | quasiquote |
| `,` | unquote |
| `,@` | unquote-splice |
| `.foo` | self.foo |
| `?foo` | logic var (in queries/rules) |
| `$foo` | capability convention |
| `#name` or `#[…]` | tagged literal / table |
| `#\h` | char literal |
| `:` (trailing) | keyword selector marker (in sends) |
| `=>` | key-value separator (in tables) |
| `::` | type ascription |
| `->` | function-type arrow |

(`syntax/sigils.md` for slightly fuller treatment.)

## see also

- `syntax/brackets.md` — bracket meanings.
- `syntax/string-interpolation.md` — `#{expr}` rules.
- `syntax/sigils.md` — sigil reference.
- `concepts/lists.md`, `concepts/tables.md`, `concepts/numbers.md`,
  `concepts/strings.md` — semantics.
