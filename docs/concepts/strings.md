# strings

> **a String is a sequence of Chars. it implements Table-like indexing
> and iteration. internally optimized to UTF-8 bytes; semantically a
> Table-of-Chars.**

```moof
"hello"                          ; a String of five Chars
"hi, #{name}"                    ; with interpolation (ruby-style)
"""
multi-line
strings dedent on parse.
"""
r"raw \n is two characters"      ; raw string literal
```

## the model

a String is conceptually a sequence of Chars (codepoint-bearing
Forms). every String responds to:

- `:length` — number of Chars
- `:at:` — get the i'th Char
- `:slice:` — substring as a new String
- `:contains?:` — substring search
- `:replace:with:`, `:split:`, `:trim`, `:upcase`, `:downcase`
- `:to-list` — convert to a List of Chars
- `:as: Table` — convert to a Table of Chars
- `:lines` — return a data source over lines
- iteration through `Iterable` / `DataSource`

internally the substrate stores Strings as UTF-8 byte arrays for
efficiency. `:at: i` returns the i'th Char (codepoint), not the i'th
byte. iteration is by Char.

## chars

a Char is a Form with `proto: Char` carrying a single unicode
codepoint.

```moof
#\h                              ; literal — char "h"
#\space                          ; named — space character
#\newline                        ; named
#\u{1f496}                       ; codepoint by hex
```

Chars respond to:
- `:codepoint` — integer codepoint
- `:upcase`, `:downcase`
- `:digit?`, `:letter?`, `:whitespace?`
- `:to-string` — a String of length 1

## strings as data sources

a String is a data source: you can pipe over it.

```moof
(pipe "hello world"
  [filter: |c| [c letter?]]
  [map: |c| [c upcase]]
  [drain])                       ; → "HELLOWORLD"
```

equivalent to standard string ops, but composes with arbitrary data
source pipelines (`concepts/data-sources.md`).

## interpolation

ruby-style `#{expr}` (matsumoto, ruby 1995):

```moof
"hi, #{name}!"
"#{count} items"
"#{if [count == 1] "item" "items"}"
"escape with \#{like-this}"
```

inside `#{…}` is a full expression. parser handles nested brackets and
quotes recursively. (`syntax/string-interpolation.md`.)

## raw and triple strings

```moof
r"raw \n stays as backslash-n"
"""
triple-quoted strings dedent
according to the closing """ position
"""
r"""
raw triple — no escape processing.
"""
```

## numeric / parsing

```moof
[String parse-int: "42"]         ; → 42
[String parse-float: "3.14"]
[s reverse]
["42" as: Integer]               ; → 42 (via type coercion protocol)
```

## why a separate type from Table

a String *could* be a Table-of-Chars. semantically that's its model.
operationally, having `proto: String` distinct lets the substrate:

- pick UTF-8 byte storage internally;
- specialize regex / search / collation;
- print without `#[…]` ceremony;
- distinguish "a sequence of characters" from "a sequence of values
  that happen to be characters."

a Table-of-Chars and a String are interconvertible via `:as: String`
and `:as: Table`. they implement most of the same protocols (Iterable,
Indexable, Sized).

## protos implemented

`String` implements:

- `Iterable` (over Chars)
- `Indexable` (`:at:`, `:slice:`)
- `Sized` (`:length`)
- `Equatable` (structural; case-sensitive by default)
- `Hashable`
- `Showable`
- `Comparable` (lexicographic)
- `DataSource` (lazy stream of Chars or lines)
- `Numeric-Concat` — `+` is concatenation

## inspirations

- ruby string interpolation (matsumoto, ruby).
- triple-quoted dedent: python (van rossum) and the ruby/elixir
  triple-quoted heredoc.
- raw-string `r"..."` prefix: rust.
- the "String is a sequence of Chars with table-like interface"
  framing: haskell's `String = [Char]` (peyton-jones), with the
  pragmatic note that ours is UTF-8-stored not List-of-Char.
- `:at:` returning a Char (not a byte): swift, julia.

## see also

- `concepts/tables.md` — what a Table is and isn't.
- `concepts/data-sources.md` — String iteration as a stream.
- `syntax/string-interpolation.md` — full interp grammar.
- `syntax/literals.md` — String literal forms.
