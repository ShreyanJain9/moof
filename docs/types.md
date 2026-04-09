# types

every value in moof is an object. primitive values (integers,
floats, booleans, nil, symbols) are NaN-boxed immediates — 8
bytes, no heap allocation. but they delegate to their type
prototype for behavior and respond to messages like any object.

## type hierarchy

```
Object                    root of all delegation
  Nil                     the absence of value / empty list
  Boolean                 true and false
  Number                  shared numeric behavior
    Integer               exact whole numbers (48-bit signed)
    Float                 IEEE 754 doubles
  Symbol                  interned names
  Cons                    linked list pairs
  String                  immutable UTF-8 text
  Table                   mutable seq + keyed collection
  Block                   closures / lambdas
  Range                   bounded integer sequence
  Error                   structured error values
```

## Object

the root prototype. every object inherits these handlers.

### identity and reflection

```
[obj type]               => Symbol ('Integer, 'String, etc.)
[obj parent]             => the prototype this object delegates to
[obj clone]              => shallow copy
[obj equal: other]       => content equality
[obj responds: sel]      => true if handler exists for selector symbol
[obj slotNames]          => list of slot name symbols
[obj handlerNames]       => list of all handler symbols (walks chain)
[obj slotAt: 'name]      => read slot value by symbol
[obj slotAt: 'name put: val] => write slot value
[obj handle: 'sel with: handler] => install a handler
[obj handlerAt: 'sel]    => retrieve a handler value
```

### display

```
[obj describe]           => human-readable string (native)
[obj show]               => REPL display string
[obj print]              => print to stdout, return self
[obj println]            => print to stdout with newline, return nil
```

### predicates

```
[obj nil?]               => true if obj is nil
[obj some?]              => true if obj is not nil
```

### pipeline

```
[obj tap: block]         => call block with self, return self
[obj pipe: f]            => call f with self, return result
[obj then: f]            => alias for pipe:
[obj is: type]           => true if prototype chain includes type
```

### error suppression

```
[|| expr rescue: default] => try expr, return default on error
```

(defined on Object, works on blocks — calls `[self call: nil]`)

---

## Nil

the singleton absence-of-value. also serves as the empty list
(cdr terminator for cons chains).

```
[nil nil?]               => true
[nil some?]              => false
[nil empty?]             => true
[nil describe]           => "nil"
```

nil is falsy. `(list)` returns nil.

---

## Boolean

true and false. both delegate to the Boolean prototype.

```
[b not]                  => logical negation
[b ifTrue:ifFalse:]      => evaluate appropriate block
[b describe]             => "true" or "false"
```

nil and false are the only falsy values. everything else
(including 0, "", empty list) is truthy.

---

## Number

intermediate prototype between Object and Integer/Float.
holds shared numeric behavior from Comparable and Numeric
protocols.

---

## Integer

exact whole numbers. NaN-boxed as immediates (48-bit signed,
range roughly -140 trillion to +140 trillion).

### arithmetic (native)

```
[i + other]              => Integer
[i - other]              => Integer
[i * other]              => Integer
[i / other]              => Integer (truncating division)
[i % other]              => Integer (modulo)
[i negate]               => Integer
```

### comparison (native, Comparable)

```
[i < other]    [i > other]    [i <= other]    [i >= other]    [i = other]
[i between: lo and: hi]      [i clamp: lo to: hi]
[i min: other]               [i max: other]
```

### predicates (protocol + native)

```
[i zero?]    [i positive?]    [i negative?]
[i even?]    [i odd?]
```

### bit operations (native)

```
[i bitAnd: other]        => Integer
[i bitOr: other]         => Integer
[i bitXor: other]        => Integer
[i bitNot]               => Integer
[i shiftLeft: n]         => Integer
[i shiftRight: n]        => Integer
```

### iteration

```
[n times: block]         => nil (calls block with 0..n-1)
[n upto: limit do: block] => nil
[n downto: limit do: block] => nil
[n to: end]              => Range
[n to: end by: step]     => Range
```

### conversion

```
[i toFloat]              => Float
[i describe]             => String (decimal representation)
```

### math

```
[i pow: exp]             => Integer
[i gcd: other]           => Integer
[i lcm: other]           => Integer
[i digits]               => Cons (list of decimal digits)
[i factorial]            => Integer
[i inc]  [i dec]         => Integer (aliases for succ/pred)
```

---

## Float

IEEE 754 double-precision. NaN-boxed (uses the non-NaN bit space).

### arithmetic (native)

```
[f + other]    [f - other]    [f * other]    [f / other]    [f negate]
```

### comparison (native)

```
[f < other]    [f > other]    [f <= other]    [f >= other]    [f = other]
```

### trigonometry (native)

```
[f sin]    [f cos]    [f tan]
[f asin]   [f acos]   [f atan]   [f atan2: other]
```

### logarithms and exponentials (native)

```
[f log]        => Float (natural log)
[f log10]      => Float
[f log2]       => Float
[f exp]        => Float (e^self)
[f pow: exp]   => Float
```

### rounding (native)

```
[f floor]    [f ceil]    [f round]    [f sqrt]
```

### predicates (native)

```
[f nan?]       [f infinite?]    [f finite?]
[f zero?]      [f positive?]    [f negative?]
```

### constants (send to Float prototype)

```
[Float pi]        => 3.141592653589793
[Float e]         => 2.718281828459045
[Float infinity]  => +inf
[Float nan]       => NaN
```

### conversion (native)

```
[f toInteger]    => Integer (truncating)
[f describe]     => String
```

---

## String

immutable UTF-8 text. conforms to Indexable (and thus Iterable).

### access (native)

```
[s length]               => Integer
[s at: i]                => String (single character at index i)
[s empty?]               => Boolean
```

### building (native)

```
[s ++ other]             => String (concatenation)
[s repeat: n]            => String
```

### search (native)

```
[s contains: sub]        => Boolean
[s startsWith: prefix]   => Boolean
[s endsWith: suffix]     => Boolean
[s indexOf: sub]         => Integer or nil
```

### transform (native)

```
[s toUpper]              => String
[s toLower]              => String
[s trim]                 => String
[s reverse]              => String
[s capitalize]           => String
[s replace: old with: new]     => String (first occurrence)
[s replaceAll: old with: new]  => String (all occurrences)
```

### splitting (native)

```
[s substring: from to: to]  => String
[s split: sep]              => Cons (list of strings)
[s chars]                   => Cons (list of single-char strings)
[s lines]                   => Cons
[s words]                   => Cons
```

### conversion (native)

```
[s toInteger]            => Integer (parse)
[s toFloat]              => Float (parse)
[s toSymbol]             => Symbol (intern)
```

### comparison (native, Comparable)

```
[s < other]              => Boolean (lexicographic)
```

(Comparable protocol provides `>`, `<=`, `>=`, `between:and:`, etc.)

### iteration (via Indexable/Iterable)

String conforms to Indexable. `each:` iterates over characters.
all 40+ Iterable methods are available.

```
["hello" map: |c| [c toUpper]]    => ("H" "E" "L" "L" "O")
["hello" select: |c| [c = "l"]]   => ("l" "l")
```

---

## Symbol

interned names. two symbols with the same text are always
identical (same bits, `eq` returns true).

```
[sym name]               => String (the symbol's text)
[sym toString]           => String (alias for name)
[sym describe]           => String
```

---

## Cons

linked list pairs. the fundamental recursive data structure.
a cons cell is a pair (car, cdr). lists are chains of cons
cells terminated by nil.

### core (native)

```
[c car]    [c first]     => first element
[c cdr]    [c rest]      => rest of list
[c length]               => Integer
[c at: n]                => nth element (0-indexed)
[c last]                 => last element
[c describe]             => String (formatted list)
```

### construction

```
[c cons: x]              => prepend x, return new list
[c append: other]        => concatenate two lists
[c prepend: x]           => alias for cons:
```

### iteration (Iterable)

Cons conforms to Iterable via a native `each:` handler that
walks car/cdr. it also has a direct `fold:with:` for efficiency.
all 40+ Iterable methods are available.

```
[(list 1 2 3) map: |x| [x * 2]]       => (2 4 6)
[(list 1 2 3) select: |x| [x > 1]]    => (2 3)
[(list 1 2 3) fold: 0 with: |a x| [a + x]]  => 6
[(list 1 2 3) sum]                     => 6
[(list 3 1 2) sort]                    => (1 2 3)
[(list 1 2 3) reverse]                 => (3 2 1)
```

---

## Table

mutable collection with two parts: a sequential part (integer-
indexed, 0-based) and a keyed part (arbitrary key-value pairs).
lua-style.

### access (native)

```
[t at: key]              => value (integer index or key lookup)
[t at: key put: val]     => set by index or key
[t length]               => Integer (sequential part length)
```

### sequential (native)

```
[t push: val]            => add to end of sequential part
[t first]                => first sequential element
[t last]                 => last sequential element
```

### keyed (native)

```
[t keys]                 => Cons (list of keyed-part keys)
[t values]               => Cons (list of keyed-part values)
[t contains: key]        => Boolean
[t remove: key]          => removed value or nil
[t merge: other]         => new table combining keyed parts
```

### iteration (Iterable via Indexable)

Table conforms to Indexable (and thus Iterable).
iterates over the sequential part.

```
[#[1 2 3] map: |x| [x * 2]]      => (2 4 6)
[#[1 2 3] sum]                    => 6
```

---

## Block

closures created by `|params| body` or `(fn (params) body)`.
blocks are objects with a `call:` handler.

```
[block call: arg]        => invoke with one argument
[block call: a with: b]  => invoke with two arguments
```

### Callable protocol

```
[f compose: g]           => block: |x| (f (g x))
[f >> g]                 => block: |x| (g (f x))  (pipeline)
[f partial: arg]         => block with first arg pre-filled
[f flip]                 => block with first two args swapped
```

---

## Range

bounded integer sequences. created by `Integer#to:` and
`Integer#to:by:`.

```
[1 to: 5]               => Range (1 2 3 4 5)
[1 to: 10 by: 2]        => Range (1 3 5 7 9)
[10 to: 1 by: -1]       => Range (10 9 8 ... 1)
```

### slots

```
range.start              => Integer
range.end                => Integer
range.step               => Integer
```

### methods

```
[r each: block]          => iterate (moof, uses while loop)
[r includes: val]        => Boolean
[r size]                 => Integer
[r min]   [r max]        => Integer
[r describe]             => String: "(1 to: 5)"
```

### Iterable conformance

Range conforms to Iterable via `each:` and `fold:with:`.
all 40+ Iterable methods are available.

```
[[1 to: 5] sum]                         => 15
[[1 to: 10] select: |x| [x even?]]     => (2 4 6 8 10)
[[1 to: 5] map: |x| [x * x]]          => (1 4 9 16 25)
[[1 to: 100 by: 10] toList]            => (1 11 21 ... 91)
```

---

## Error

structured error values. created by `try`/`catch` when an
error occurs, or by `(error "msg")`.

```
[err message]            => String (the error message)
[err describe]           => String ("Error: message")
[err show]               => String ("Error: message")
```

see [errors.md](errors.md) for the full error handling guide.
