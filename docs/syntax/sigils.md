# sigils

> **a one-page reference for every sigil in moof. each has one
> meaning.**

## name-prefix sigils

| prefix | meaning | example |
|---|---|---|
| `'` | symbol literal / quote | `'foo`, `'(1 2 3)` |
| `` ` `` | quasiquote | `` `(a ,b ,@c) `` |
| `,` | unquote (inside quasiquote) | `` `,b `` |
| `,@` | unquote-splice (inside quasiquote) | `` `,@xs `` |
| `.` | self.foo (slot read or unary self-send) | `.count` |
| `?` | logic variable (in queries/rules only) | `?x`, `?some-var` |
| `$` | capability convention | `$out`, `$clock` |
| `#` | tagged literal / table | `#[1 2 3]`, `#Date "..."` |
| `#\` | char literal | `#\h`, `#\space` |

## suffix conventions

| suffix | meaning | example |
|---|---|---|
| `?` | predicate method | `empty?`, `nil?` |
| `!` | mutating / unsafe method | `set!`, `swap!` |
| `:` | keyword-arg marker (only inside `[…]` sends) | `at:` |

## bracket sigils

| shape | meaning |
|---|---|
| `(...)` | code: fn-call, special form |
| `[...]` | message send |
| `{...}` | object literal |
| `#[...]` | Table literal |
| `|...|` | block parameters |

## punctuation operators

| symbol | meaning | context |
|---|---|---|
| `=>` | key → value | inside `#[...]` Tables |
| `::` | type ascription | bindings, params, expressions |
| `->` (`→`) | function-type arrow | type expressions |
| `:-` | rule body separator | `(rule head :- body…)` |
| `;` | cascade separator | inside `[…]` sends |
| `;;` | line comment | line start |
| `;:` | doc comment | line start (attaches to next def) |
| `;~` | scratch / fixme | line start |
| `#|` `|#` | block comment delimiters | spans lines |

## sigils that look special but aren't

- `:` standalone — *not* a sigil. only meaningful as keyword-arg
  marker (`name:` inside `[…]`) or as the second char of `:-`
  (datalog rule), `::` (ascription), etc.
- ascii operators `+ - * / < > = ! ?` standalone — these are *names*
  (binary operator selectors), not sigils. `+` is a symbol; `(+ 1 2)`
  uses it as a callable; `[a + b]` uses it as a binary selector.

## naming culture

### casing
- `Capitalized` — protos and types: `Counter`, `Integer`, `Comparable`.
- `lower-kebab` — everything else: `count`, `incr-by`, `do-thing`.

### special names
- `self` — receiver in method body.
- `super` — proto-chain start above defining proto.
- `nil` — the empty list = absence.
- `#true`, `#false` — booleans.

## inspirations

- `'` and the quote/quasi-quote family: lisp / scheme.
- `?` predicate suffix: lisp tradition (`null?`, `pair?`).
- `!` mutating suffix: scheme / clojure.
- `$` capability prefix: shell scripting + erlang's process registry,
  reinterpreted as cap-marker in moof.
- `#` tagged literal prefix: clojure.
- `::` type ascription: haskell, rust.
- `->` (`→`) function-type arrow: haskell, ml.
- `:-` rule body: prolog/datalog (colmerauer).
- `;` cascade and `#|...|#` block comment: smalltalk-80 / scheme.

## see also

- `syntax/overview.md` — at-a-glance.
- `syntax/literals.md` — full literal grammar.
- `syntax/brackets.md` — bracket details.
