# protocols

protocols are the type system of the objectspace. a protocol
says "if you can do X, i'll give you Y for free." implement one
handler, get dozens.

protocols are objects. you can inspect them, query them, send
them messages.

## how protocols work

### defining a protocol

protocols are created with a name, a list of required handlers,
and a Table of provided handlers:

```moof
(def Iterable (make-protocol "Iterable"
  (list 'each:)       ; requires
  #[ ... provides ... ]
  nil))                ; includes (other protocols)
```

### conforming to a protocol

install the required handler(s) on your type, then call `conform`:

```moof
(defmethod MyType each: (block)
  ...)

(conform MyType Iterable)
```

`conform` checks that all required handlers are present, then
copies the provided handlers onto the type prototype (skipping
any the type already has — your overrides take precedence).

### checking conformance

```moof
[obj conforms: Iterable]    ; => true if obj responds to required handlers
```

---

## standard protocols

### Iterable

the crown jewel. implement `each:`, get 40+ collection methods.

**requires:** `each:`

**provides:**

| category | handlers |
|----------|----------|
| transform | `map:` `flatMap:` `flat` |
| filter | `select:` `reject:` `find:` `distinct` |
| quantify | `any:` `all:` `none:` `includes:` `empty?` |
| reduce | `fold:with:` `reduce:` `sum` `product` `count` `count:` |
| access | `first` `last` `take:` `drop:` `takeWhile:` `dropWhile:` |
| order | `sort` `sortBy:` `reverse` `min` `max` `minBy:` `maxBy:` |
| group | `groupBy:` `partition:` `tally` `zip:` `intersperse:` |
| convert | `join` `join:` `toList` `toTable` |
| iterate | `each:withIndex:` |

**who conforms:** Cons, String (via Indexable), Table (via Indexable), Range

**example:**

```moof
[(list 3 1 4 1 5) sort]                 => (1 1 3 4 5)
[(list 3 1 4 1 5) distinct]             => (3 1 4 5)
[(list 1 2 3) map: |x| [x * x]]        => (1 4 9)
[(list 1 2 3 4) partition: |x| [x even?]]
  => ((2 4) (1 3))
[(list 1 2 3) fold: 0 with: |a x| [a + x]]  => 6
```

### Comparable

ordering and comparison. implement `<`, get 7 methods.

**requires:** `<`

**provides:** `>` `<=` `>=` `between:and:` `clamp:to:` `min:` `max:`

**who conforms:** Number (Integer, Float), String

**example:**

```moof
[3 between: 1 and: 5]                  => true
[7 clamp: 1 to: 5]                     => 5
["apple" < "banana"]                   => true
```

### Numeric

arithmetic predicates. implement `+` `-` `*` `negate`, get 5 predicates.

**requires:** `+` `-` `*` `negate`

**provides:** `abs` `sign` `zero?` `positive?` `negative?`

**who conforms:** Number (Integer, Float)

**example:**

```moof
[-42 abs]                               => 42
[-3 sign]                               => -1
[0 zero?]                               => true
```

### Callable

function composition. implement `call:`, get composition tools.

**requires:** `call:`

**provides:** `compose:` `>>` `partial:` `flip`

**who conforms:** Block

**example:**

```moof
(def double |x| [x * 2])
(def inc |x| [x + 1])
(def double-then-inc [double >> inc])
(double-then-inc 5)                     => 11
```

### Indexable

positional access for ordered collections. includes Iterable —
conforming to Indexable gives you Iterable for free.

**requires:** `at:` `length`

**provides:** `each:` (derived from at: + length), `first` `last`
`empty?` `indexOf:`

**includes:** Iterable (all 40+ Iterable methods come free)

**who conforms:** String, Table

---

## creating your own protocol

```moof
(def Greetable (make-protocol "Greetable"
  (list 'greet)
  #[
    'hello => (fn (this) (str "hello, " [this greet]))
    'goodbye => (fn (this) (str "goodbye, " [this greet]))
  ]
  nil))

(defmethod MyObj greet () "world")
(conform MyObj Greetable)

[{ MyObj } hello]     => "hello, world"
[{ MyObj } goodbye]   => "goodbye, world"
```

## implementation

protocols are defined in `lib/protocols.moof`. the Iterable
protocol is in `lib/iterable.moof`. per-type conformance
happens in `lib/comparable.moof`, `lib/numeric.moof`,
`lib/indexable.moof`, `lib/callable.moof`, and `lib/types.moof`.

the protocol system is built entirely in moof — Protocol is
an object, `conform` is a function, `conforms?` is a function.
no compiler support needed.
