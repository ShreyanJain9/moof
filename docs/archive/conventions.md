# documentation conventions

how we refer to moof things in prose and documentation.

## referring to handlers (methods)

use the `Type#selector` notation. the `#` separates the type
from the handler selector. this tells you "this handler lives
on (or is accessible via delegation from) this type prototype."

```
Integer#negate          unary handler on Integer
String#length           unary handler on String
Cons#map:               keyword handler (one arg)
Table#at:put:           keyword handler (two args)
Object#slotAt:          inherited handler (on Object, available to all)
```

for protocol-provided handlers, use the protocol name:

```
Iterable#map:           provided by Iterable protocol
Comparable#between:and: provided by Comparable protocol
```

when it matters whether a handler is native (rust) or moof-defined,
annotate:

```
Integer#+ (native)      implemented in rust
Cons#map: (protocol)    provided by Iterable conformance
Range#each: (moof)      defined in lib/range.moof
```

## referring to types

type names are PascalCase. always.

```
Integer  Float  String  Cons  Table  Boolean  Nil
Symbol   Object  Number  Range  Error  Block
```

`Block` is the type name for closures/lambdas/blocks — values
created by `|x| expr` or `(fn (x) expr)`.

## referring to protocols

protocol names are PascalCase. protocols are types too (they're
objects), but they serve a different role — they describe contracts.

```
Iterable   Comparable   Numeric   Callable   Indexable
```

## code examples

always use actual moof syntax. send syntax for method calls,
parenthetical syntax for function calls.

```moof
; message send
[42 + 1]
[(list 1 2 3) map: |x| [x * 2]]
[pt slotAt: 'x]

; function call
(list 1 2 3)
(def x 42)
(defn greet (name) (str "hello, " name))

; object literal
{ Point x: 3 y: 4 }

; table literal
#[1 2 3 "name" => "alice"]

; block (closure)
|x| [x + 1]
|x y| [x + y]
|| "thunk"
```

## selectors in prose

when mentioning a selector in running text, use backticks and
include the colons:

- `map:` takes a block and returns a new collection
- `fold:with:` takes an initial value and a two-arg block
- `at:put:` sets a value at a key

unary selectors have no colon: `length`, `negate`, `reverse`.

keyword selectors always end with a colon per keyword part:
`map:`, `fold:with:`, `at:put:`, `between:and:`.

## signatures

document handler signatures like this:

```
[receiver selector: arg]  =>  return-type
```

examples:

```
[coll map: block]            => Cons
[coll fold: init with: block] => any
[n to: end]                  => Range
[n to: end by: step]         => Range
```

for multi-line documentation of a handler:

```
Integer#to:
  [start to: end] => Range

  creates a Range from start to end (inclusive), step 1.

  [1 to: 5]         => (1 to: 5)
  [[1 to: 5] sum]   => 15
```

## file references

refer to source files relative to the project root:

```
src/vm.rs           the bytecode interpreter
src/heap.rs         object storage and symbol table
lib/iterable.moof   the Iterable protocol and its 40+ methods
```

## naming conventions in moof code

these are the naming rules for moof code itself (not docs):

- predicates (no args): `?` suffix — `empty?`, `even?`, `nil?`
- predicates (with args): no `?` — `includes:`, `any:`, `contains:`
- conversions: `to` prefix — `toFloat`, `toString`, `toList`
- destructive (future): `!` suffix — `sort!`, `reverse!`
- keyword selectors: camelCase — `groupBy:`, `flatMap:`, `startsWith:`
- unary selectors: lowercase — `length`, `reverse`, `sum`
- protocol names: PascalCase — `Comparable`, `Iterable`
- type prototype names: PascalCase — `Object`, `Integer`, `String`
- global functions: lowercase — `list`, `str`, `range`, `not`
- vau operatives: lowercase — `and`, `or`, `when`, `unless`, `defn`
