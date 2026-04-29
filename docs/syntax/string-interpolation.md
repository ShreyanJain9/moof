# string interpolation

> **ruby-style `#{expr}`. arbitrary expressions inside braces.
> `\#{` to escape.**

```moof
"hi, #{name}"
"#{count} items"
"#{if [n > 0] "positive" "non-positive"}"
"escape with \#{like-this}"
```

inside `#{...}`:
- the `...` is a complete moof expression.
- it is evaluated in the surrounding lexical scope.
- the result is converted to a string via `:to-string`.
- the string-version is spliced into the surrounding string.

## nesting

`#{...}` can contain strings, brackets, parens, anything moof
parses — *including more `#{}` interpolations*:

```moof
"name=#{[user name]} age=#{[user age]}"
"selector list: #{[handlers keys] map: |k| "#{k}"} done"
"value: #{(if [x > 0] "#{x}" "negative #{[-1 * x]}")}"
```

the parser tracks nesting; the closing `}` matches its opening
`#{`. unmatched `}` outside `#{…}` is just a literal closing brace.

## escapes

| sequence | meaning |
|---|---|
| `\n` | newline |
| `\t` | tab |
| `\\` | backslash |
| `\"` | double quote |
| `\#{` | literal `#{` (no interpolation) |
| `\u{HEX}` | unicode codepoint |

## raw strings

prefix `r"..."` to disable escape processing:

```moof
r"a\nb"                          ; the literal four characters: a \ n b
r"path: \\users\\shreyan"
```

raw strings still allow interpolation. for raw + no-interpolation,
use a `r` prefix and escape interpolations:

```moof
r"raw with \#{not interpolated}"
```

## triple-quoted

```moof
"""
line one
line two
"""
```

triple-quoted strings:
- can contain unescaped `"` (single quotes).
- dedent on parse: the indentation of the closing `"""` defines the
  zero-indent. lines with shallower indentation are an error.
- support `#{…}` interpolation by default.

raw triple-quoted: `r"""..."""`.

## conversion

`#{expr}` calls `[expr to-string]`. user types control their own
conversion:

```moof
(defproto Money
  (slots amount currency)
  (handlers
    [to-string] "$#{.currency} #{.amount}"))

"price: #{some-money}"
;; → "price: $USD 12.50"
```

if `:to-string` is not defined, falls through proto-chain to
`Object`'s default: `"<Counter at 0x12345>"` style.

## inspirations

- ruby's `"#{expr}"`: matsumoto. moof copies this directly.
- raw string `r"..."` prefix: rust.
- triple-quoted dedent: python (van rossum) and elixir.
- the recursive nesting of `#{}`: ruby and elixir.

## see also

- `concepts/strings.md` — string semantics.
- `syntax/literals.md` — full literal grammar.
