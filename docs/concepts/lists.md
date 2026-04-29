# lists

> **linked cons-cell sequences. the substrate of code-as-data.
> distinct from Tables.**

a List is moof's classic linked sequence: a chain of cons-cell
Forms, terminated by `nil`. the empty List is `nil` is `()`.

```moof
'(1 2 3)                         ; List of three
'(a b c)                         ; List of three symbols
()                               ; nil — also the empty List
'foo                             ; not a List — a Symbol
```

## quoting

unquoted, `(...)` is a fn-call. quoted, `'(...)` is a List literal:

```moof
(foo x y)                        ; fn-call: invoke foo with x, y
'(foo x y)                       ; List with three symbols inside
```

quasiquote and unquote follow lisp tradition:

```moof
`(a ,b ,@c)                      ; quasi-quoted; ,b inserts value, ,@c splices
```

(`syntax/literals.md` for the full quoting grammar.)

## ops

```moof
[xs head]                        ; first element
[xs tail]                        ; rest of the list (a List)
[xs cons: x]                     ; new List with x prepended
[xs length]
[xs empty?]
[xs at: i]                       ; O(n)
[xs map: f]
[xs filter: pred]
[xs reduce: f from: init]
[xs append: ys]
[xs reverse]
```

cons-cell internals:

```moof
{Cons
  head: …
  args: …}                       ; structure-face: head + tail
```

a List's head/args faces are populated; its proto/slots are not used
heavily. this is where Lists differ from objects-as-data.

## why a separate type from Tables

three reasons:

1. **code-as-data is canonical.** parsed code-forms are Lists. macros
   walk Lists. quote/unquote produce Lists. having Lists be a distinct
   type with cons-cell shape preserves the lisp tradition unmodified.
2. **recursion is natural.** `[xs match |'() | ... | '(h …t) | ...]`
   reads like haskell/erlang and is the natural pattern for List
   processing. flat-array structures (Tables) don't get this for free.
3. **the user can choose.** if you want a sequence, you decide:
   linked (List) or flat (Table). different cost models, different
   ergonomics. one substrate, two collection idioms.

## Lists vs Tables, summary

`concepts/tables.md` for the table side. quick reference:

|  | List | Table |
|---|---|---|
| literal | `'(1 2 3)` | `#[1 2 3]` |
| empty | `()` = nil | `#[]` |
| structure | linked, head + tail | flat, indexed |
| immutability | conceptual default | mutable default |
| code-as-data | yes | no (Tables are data, not code) |
| typical use | macros, recursion, parsed forms | records, arrays, relations |

## protos implemented

`List` implements:

- `Iterable`
- `Sized` — `:length`, `:empty?`
- `Equatable`
- `Hashable`
- `Showable`
- `DataSource` (lazy iteration)

note: `Indexable` is implemented but `:at:` is O(n). if you need
random access, use a Table.

## inspirations

- the cons-cell list goes back to mccarthy's lisp (1958).
- the `'(…)` quote and `(quote …)` equivalence: scheme / common lisp.
- quasi-quote with `,` and `,@`: common lisp (steele 1990).
- the deliberate distinction Lists-≠-Tables: clojure (where lists are
  cons-cells, vectors are flat) — though clojure conflates the
  decision with seq abstraction. moof keeps them separately
  visible.

## see also

- `concepts/tables.md` — the rich-data alternative.
- `concepts/data-sources.md` — Lists as streams.
- `syntax/literals.md` — full quoting/list-literal grammar.
