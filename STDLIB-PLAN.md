# moof standard library design

> everything follows from: objects, messages, protocols.
> implement one method, get dozens free.
> if you can't send it a message, it doesn't exist.

---

## 0. prerequisite: vau and first-class environments

vau receives `$e` (the caller's environment) but it is currently nil.
`eval` compiles against globals only. this means `and`, `or`, `when`,
`unless` cannot see local variables inside method bodies. this blocks
the entire stdlib.

the fix: make environments real objects, make eval use them.

1. environments are objects with bindings as slots and a parent pointer
2. `$e` carries the real environment when calling a vau operative
3. `eval` takes an optional environment: `(eval expr $e)`
4. the compiler emits CAPTURE_ENV only when a vau operative needs it

estimated: ~100 lines of rust. this unblocks everything.

fallback: make and/or/when/unless compiler special forms temporarily.
mark them as tech debt.

---

## 1. type hierarchy

```
Object                          root of all delegation
  Nil                           the absence of value
  Boolean                       true and false
  Number                        shared numeric behavior (NEW intermediate)
    Integer                     exact whole numbers (NaN-boxed)
    Float                       IEEE 754 doubles (NaN-boxed)
  Symbol                        interned names
  Cons                          linked list pairs
  String                        immutable UTF-8 text
  Bytes                         immutable byte sequences
  Table                         mutable seq + keyed collection
  Block                         closures / lambdas
  Stream                        lazy sequences (future)
  Range                         bounded iteration (future)
  Set                           unordered unique collection (future)
  Error                         structured error values (future)
  Protocol                      protocol descriptor objects
  Environment                   first-class lexical scopes
  Vat                           concurrency unit (future)
  Promise                       eventual value (future)
  Membrane                      send interception (future)
  Continuation                  captured control state (future)
```

### Number (new intermediate prototype)

created in rust between Object and Integer/Float.

```rust
let number_proto = heap.make_object(object_proto);
// reparent:
let int_proto = heap.make_object(number_proto);
let float_proto = heap.make_object(number_proto);
```

Number holds all shared numeric behavior from Comparable and Numeric
protocols. Integer and Float inherit it, only overriding type-specific
native ops.

### prototype chain examples

```
42        -> Integer -> Number -> Object
3.14      -> Float   -> Number -> Object
\"hello\"   -> String  -> Object
(1 2 3)   -> Cons    -> Object
#[1 2]    -> Table   -> Object
true      -> Boolean -> Object
nil       -> Nil     -> Object
'foo      -> Symbol  -> Object
|x| x     -> Block   -> Object
```

---

## 2. protocols

### Showable

how things present themselves in the REPL.

- **requires:** `show`
- **provides:** (none -- show is the whole protocol)
- **includes:** (none)
- **who conforms:** every type. Object provides a default.

`show` returns a string suitable for REPL display:
- `[nil show]` => `\"nil\"`
- `[42 show]` => `\"42\"`
- `[3.14 show]` => `\"3.14\"`
- `[true show]` => `\"true\"`
- `[\"hello\" show]` => `\"\\\"hello\\\"\"`
- `['foo show]` => `\"'foo\"`
- `[(list 1 2 3) show]` => `\"(1 2 3)\"`
- `[#[1 2] show]` => `\"#[1 2]\"`
- `[|x| x show]` => `\"<fn arity:1>\"`

the REPL must send `[val show]` instead of using rust-side format_value.

### Comparable

ordering and comparison.

- **requires:** `<`
- **provides:**
  - `[a > b]` -- true if b < a
  - `[a <= b]` -- true if not (b < a)
  - `[a >= b]` -- true if not (a < b)
  - `[a between: lo and: hi]` -- true if a >= lo and a <= hi
  - `[a clamp: lo to: hi]` -- constrain a to [lo, hi]
  - `[a min: b]` -- the lesser of a and b
  - `[a max: b]` -- the greater of a and b
  - `[a <=> b]` -- returns -1, 0, or 1 (spaceship operator)
- **includes:** (none)
- **who conforms:** Number (Integer, Float inherit), String, Symbol

### Numeric

arithmetic and numeric predicates.

- **requires:** `+`, `-`, `*`, `negate`
- **provides:**
  - `[x abs]` -- absolute value (negate if negative)
  - `[x sign]` -- returns -1, 0, or 1
  - `[x zero?]` -- true if x equals zero
  - `[x positive?]` -- true if x > 0
  - `[x negative?]` -- true if x < 0
- **includes:** (none)
- **who conforms:** Number (Integer, Float inherit)

### Iterable

the crown jewel. ruby's Enumerable for moof.

- **requires:** `each:`
- **provides:**

  transforming:
  - `[coll map: f]` -- apply f to each element, collect results as list
  - `[coll flatMap: f]` -- map then flatten one level
  - `[coll flat]` -- flatten one level of nesting
  - `[coll collectConcat: f]` -- alias for flatMap:

  filtering:
  - `[coll select: f]` -- keep elements where f returns true
  - `[coll reject: f]` -- keep elements where f returns false
  - `[coll find: f]` -- first element where f returns true, or nil
  - `[coll distinct]` -- remove duplicates (preserves first occurrence)

  quantifiers:
  - `[coll any: f]` -- true if f returns true for any element
  - `[coll all: f]` -- true if f returns true for every element
  - `[coll none: f]` -- true if f returns false for every element
  - `[coll includes: x]` -- true if any element equals x
  - `[coll one?: f]` -- true if f returns true for exactly one element
  - `[coll empty?]` -- true if collection has no elements
  - `[coll any]` -- (no arg) true if collection is not empty

  reducing:
  - `[coll fold: init with: f]` -- accumulate with initial value
  - `[coll reduce: f]` -- accumulate using first element as init
  - `[coll sum]` -- fold with + starting from 0
  - `[coll product]` -- fold with * starting from 1
  - `[coll sumBy: f]` -- sum of (f element) for each element
  - `[coll count]` -- number of elements
  - `[coll count: f]` -- number of elements where f returns true

  accessing:
  - `[coll first]` -- first element or nil
  - `[coll last]` -- last element or nil
  - `[coll first: n]` -- list of first n elements
  - `[coll last: n]` -- list of last n elements
  - `[coll take: n]` -- first n elements
  - `[coll drop: n]` -- all but first n elements
  - `[coll takeWhile: f]` -- leading elements while f returns true
  - `[coll dropWhile: f]` -- skip leading elements while f returns true
  - `[coll sample]` -- random element (requires native support)
  - `[coll shuffle]` -- randomized order (requires native support)

  ordering:
  - `[coll sort]` -- sort using < (elements must be Comparable)
  - `[coll sortBy: f]` -- sort by key function f
  - `[coll reverse]` -- reverse order
  - `[coll min]` -- smallest element by <
  - `[coll max]` -- largest element by <
  - `[coll minBy: f]` -- element with smallest (f element)
  - `[coll maxBy: f]` -- element with largest (f element)
  - `[coll minmax]` -- pair of (min, max) in one pass
  - `[coll minmaxBy: f]` -- pair of (minBy, maxBy) by key function

  grouping:
  - `[coll groupBy: f]` -- table mapping (f element) to list of elements
  - `[coll partition: f]` -- pair of (matching, non-matching) lists
  - `[coll tally]` -- table mapping element to count of occurrences
  - `[coll chunk: n]` -- list of n-sized sublists
  - `[coll eachSlice: n]` -- same as chunk:
  - `[coll sliding: n]` -- list of overlapping n-sized windows
  - `[coll eachCons: n]` -- same as sliding:

  combining:
  - `[coll zip: other]` -- pair elements positionally as cons cells
  - `[coll zip: other with: f]` -- combine paired elements with f
  - `[coll intersperse: sep]` -- insert sep between each element

  converting:
  - `[coll join]` -- concatenate element descriptions into string
  - `[coll join: sep]` -- join with separator string
  - `[coll toList]` -- convert to cons list
  - `[coll toTable]` -- convert to table (sequential part)

  iteration variants:
  - `[coll each: f withIndex: g]` -- iterate with index (alt signature)
  - `[coll each:withIndex: f]` -- f receives (element, index)
  - `[coll each:withObject: f obj]` -- f receives (element, obj), returns obj
  - `[coll cycle: n]` -- repeat the collection n times as a list

- **includes:** (none)
- **who conforms:** Cons (native each:), String (via Indexable), Table (via Indexable), Stream, Range, Set

### implementation strategy

every provided method builds on `each:` and `fold:with:`.

```moof
'fold:with: => |self init f|
  (let ((acc init))
    [self each: |x| (:= acc (f acc x))]
    acc)

'map: => |self f|
  [[self fold: nil with: |acc x| (cons (f x) acc)] reverse]

'select: => |self f|
  [[self fold: nil with: |acc x| (if (f x) (cons x acc) acc)] reverse]

'sum => |self| [self fold: 0 with: |a x| [a + x]]

'count => |self| [self fold: 0 with: |a _| [a + 1]]

'any: => |self f|
  [self fold: false with: |found x| (if found true (f x))]
```

### Indexable

positional access for ordered collections.

- **requires:** `at:`, `length`
- **provides:**
  - `[coll each: f]` -- derived from at: + length, iterates by index
  - `[coll first]` -- element at index 0
  - `[coll last]` -- element at (length - 1)
  - `[coll empty?]` -- true if length is 0
  - `[coll slice: from to: to]` -- sub-sequence from index from to to (exclusive)
  - `[coll indexOf: x]` -- index of first occurrence, or nil
  - `[coll lastIndexOf: x]` -- index of last occurrence, or nil
  - `[coll contains: x]` -- true if indexOf: returns non-nil
  - `[coll reverse]` -- reversed collection
  - `[coll sample]` -- random element by index
- **includes:** Iterable (each: derived from at: + length)
- **who conforms:** String, Table, Bytes

conforming to Indexable gives you Iterable for free. so String and
Table get all ~50 Iterable methods from just having `at:` and `length`.

### Callable

anything invocable.

- **requires:** `call:`
- **provides:**
  - `[f compose: g]` -- returns block that applies g then f
  - `[f >> g]` -- returns block that applies f then g (pipeline order)
  - `[f curry]` -- returns curried version (if arity known)
  - `[f partial: arg]` -- returns block with first arg pre-filled
  - `[f flip]` -- returns block with first two args swapped
  - `[f arity]` -- number of parameters (native support needed)
- **includes:** (none)
- **who conforms:** Block, any object with a call: handler

### Hashable

identity for collection membership.

- **requires:** `hash`
- **provides:** (enables use as Table key, Set member)
- **includes:** (none)
- **who conforms:** Integer, Float, String, Symbol, Boolean, Nil

note: hash returns an integer. two objects that are equal: must return
the same hash. native implementations for all primitive types.

---

## 3. Object methods

every object inherits these from the Object prototype.

### identity and reflection

- `[obj type]` -- symbol naming the type (native: 'Integer, 'String, etc.)
- `[obj parent]` -- the prototype this object delegates to (native)
- `[obj clone]` -- shallow copy (native; primitives return self)
- `[obj equal: other]` -- content equality (native)
- `[obj identical: other]` -- reference identity (same object id)
- `[obj responds: sel]` -- true if obj has handler for selector symbol (native)
- `[obj conforms: protocol]` -- true if obj conforms to protocol
- `[obj protocols]` -- list of protocols this object conforms to

### slots

- `[obj slotAt: 'name]` -- read slot value by symbol (native)
- `[obj slotAt: 'name put: val]` -- write slot value (native)
- `[obj slotNames]` -- list of slot name symbols (native)

### handlers

- `[obj handlerNames]` -- list of all handler selector symbols (native, walks chain)
- `[obj handle: 'sel with: handler]` -- install a handler (native)
- `[obj handlerAt: 'sel]` -- retrieve a handler by selector (native)
- `[obj doesNotUnderstand: sel args: args]` -- called when no handler found

### display

- `[obj describe]` -- basic string representation (native)
- `[obj show]` -- REPL display string (Showable protocol)
- `[obj print]` -- print describe to stdout, return self (native)
- `[obj println]` -- print describe to stdout, return nil (native)
- `[obj inspect]` -- detailed string with type, slots, handlers

### predicates

- `[obj nil?]` -- true if obj is nil (overridden on Nil)
- `[obj some?]` -- true if obj is not nil

### pipeline

- `[obj tap: block]` -- call block with self, return self (for side-effects in chains)
- `[obj pipe: f]` -- call f with self, return result (transform in chain)
- `[obj then: f]` -- alias for pipe: (reads well in chains)
- `[obj yield: sel]` -- send sel to self, return self (like tap but with a message)

### conversion

- `[obj toList]` -- wrap in a one-element list: (cons obj nil)
- `[obj freeze]` -- return an immutable view (future)

### comparison

- `[obj is: type]` -- true if obj's prototype chain includes type
- `[obj isNot: type]` -- negation of is:

---

## 4. Nil

nil is the singleton absence-of-value.

- `[nil nil?]` -- returns true (overrides Object)
- `[nil some?]` -- returns false
- `[nil empty?]` -- returns true (nil is the empty list)
- `[nil describe]` -- returns \"nil\"
- `[nil show]` -- returns \"nil\"
- `[nil type]` -- returns 'Nil
- `[nil ifNil: block]` -- evaluates and returns block result
- `[nil ifNil: nilBlock ifSome: someBlock]` -- evaluates nilBlock
- `[nil toList]` -- returns nil (nil IS the empty list)
- `[nil each: f]` -- does nothing (nil is iterable as empty collection)
- `[nil map: f]` -- returns nil
- `[nil select: f]` -- returns nil
- `[nil fold: init with: f]` -- returns init
- `[nil count]` -- returns 0

nil is falsy. in boolean contexts (if, and, or, when, unless), nil
behaves like false.

note: nil serves double duty as the empty list (the cdr terminator for
cons chains). this is the lisp tradition. `(list)` returns nil.

---

## 5. Boolean

true and false are the two boolean values. both delegate to Boolean.

- `[b not]` -- logical negation (native)
- `[b ifTrue: block]` -- evaluate block if true, return result or nil
- `[b ifFalse: block]` -- evaluate block if false, return result or nil
- `[b ifTrue: tBlock ifFalse: fBlock]` -- evaluate appropriate block (native)
- `[b and: other]` -- logical and (short-circuit via block: `[b and: || expr]`)
- `[b or: other]` -- logical or (short-circuit via block)
- `[b xor: other]` -- exclusive or
- `[b describe]` -- \"true\" or \"false\" (native)
- `[b show]` -- \"true\" or \"false\"
- `[b toString]` -- \"true\" or \"false\"
- `[b toInteger]` -- 1 for true, 0 for false
- `[b type]` -- 'Boolean

both true and false are falsy/truthy as expected. nil and false are
the only falsy values. everything else is truthy.

---

## 6. Number (shared by Integer and Float)

Number is the intermediate prototype between Object and Integer/Float.
it holds all behavior from Comparable and Numeric protocols.

### from Comparable protocol (requires < on Integer and Float)

- `[n > other]` -- greater than
- `[n <= other]` -- less than or equal
- `[n >= other]` -- greater than or equal
- `[n between: lo and: hi]` -- true if n >= lo and n <= hi
- `[n clamp: lo to: hi]` -- constrain to range [lo, hi]
- `[n min: other]` -- the lesser value
- `[n max: other]` -- the greater value
- `[n <=> other]` -- comparison: -1, 0, or 1

### from Numeric protocol (requires +, -, *, negate on Integer and Float)

- `[n abs]` -- absolute value
- `[n sign]` -- -1, 0, or 1
- `[n zero?]` -- true if n equals 0
- `[n positive?]` -- true if n > 0
- `[n negative?]` -- true if n < 0

### own methods

- `[n toFloat]` -- coerce to float
- `[n toInteger]` -- coerce to integer (truncate for float)
- `[n isNaN]` -- false for integers, checks for float
- `[n isInfinite]` -- false for integers, checks for float
- `[n isFinite]` -- true for integers, checks for float
- `[n divmod: other]` -- returns (quotient, remainder) as a cons pair
- `[n nonzero?]` -- true if not zero

---

## 7. Integer

exact whole numbers, NaN-boxed as immediates.

### native arithmetic (in rust)

- `[i + other]` -- addition
- `[i - other]` -- subtraction
- `[i * other]` -- multiplication
- `[i / other]` -- integer division (truncates toward zero)
- `[i % other]` -- modulo
- `[i < other]` -- less than
- `[i > other]` -- greater than
- `[i <= other]` -- less than or equal
- `[i >= other]` -- greater than or equal
- `[i = other]` -- numeric equality
- `[i negate]` -- unary negation

### arithmetic (moof)

- `[i ** exp]` -- alias for pow:
- `[i pow: exp]` -- exponentiation by repeated multiply
- `[i gcd: other]` -- greatest common divisor (euclidean algorithm)
- `[i lcm: other]` -- least common multiple via gcd
- `[i divmod: other]` -- cons pair of (quotient, remainder)
- `[i floorDiv: other]` -- floor division (rounds toward negative infinity)
- `[i modulo: other]` -- modulo that matches floorDiv sign convention

### bit operations (need native support)

- `[i bitAnd: other]` -- bitwise and
- `[i bitOr: other]` -- bitwise or
- `[i bitXor: other]` -- bitwise exclusive or
- `[i bitNot]` -- bitwise complement
- `[i shiftLeft: n]` -- left shift by n bits
- `[i shiftRight: n]` -- arithmetic right shift by n bits

### predicates

- `[i even?]` -- true if divisible by 2
- `[i odd?]` -- true if not even
- `[i zero?]` -- true if 0 (inherited from Number)
- `[i positive?]` -- true if > 0 (inherited)
- `[i negative?]` -- true if < 0 (inherited)
- `[i between: lo and: hi]` -- inherited from Comparable
- `[i multiple?: other]` -- true if i % other == 0
- `[i prime?]` -- true if i is a prime number (trial division)
- `[i power?: n]` -- true if i is a perfect nth power

### iteration

- `[i times: block]` -- call block with 0, 1, ... i-1
- `[i upto: limit do: block]` -- call block with i, i+1, ... limit
- `[i downto: limit do: block]` -- call block with i, i-1, ... limit
- `[i to: limit]` -- return a Range from i to limit (inclusive)
- `[i to: limit by: step]` -- return a Range with step

### conversion

- `[i toFloat]` -- convert to float
- `[i toString]` -- decimal string representation
- `[i toRadix: base]` -- string in given base (2, 8, 16, etc.)
- `[i toChar]` -- unicode character for this codepoint
- `[i describe]` -- native: returns string of decimal representation

### math

- `[i abs]` -- absolute value (inherited)
- `[i sign]` -- -1, 0, or 1 (inherited)
- `[i digits]` -- list of decimal digits (e.g. 123 => (1 2 3))
- `[i digits: base]` -- list of digits in given base
- `[i factorial]` -- n! (error if negative)
- `[i sqrt]` -- integer square root (floor of float sqrt)

### successor/predecessor

- `[i succ]` -- i + 1
- `[i next]` -- alias for succ
- `[i pred]` -- i - 1
- `[i prev]` -- alias for pred
- `[i inc]` -- alias for succ
- `[i dec]` -- alias for pred

---

## 8. Float

IEEE 754 double-precision floats, NaN-boxed.

### native arithmetic (in rust)

- `[f + other]` -- addition
- `[f - other]` -- subtraction
- `[f * other]` -- multiplication
- `[f / other]` -- division
- `[f < other]` -- less than
- `[f > other]` -- greater than
- `[f <= other]` -- less than or equal
- `[f >= other]` -- greater than or equal
- `[f = other]` -- numeric equality

### native math (in rust)

- `[f sqrt]` -- square root
- `[f floor]` -- round toward negative infinity
- `[f ceil]` -- round toward positive infinity
- `[f round]` -- round to nearest integer
- `[f toInteger]` -- truncate to integer

### math functions (need native support)

- `[f sin]` -- sine (radians)
- `[f cos]` -- cosine (radians)
- `[f tan]` -- tangent (radians)
- `[f asin]` -- arcsine
- `[f acos]` -- arccosine
- `[f atan]` -- arctangent
- `[f atan2: other]` -- two-argument arctangent
- `[f log]` -- natural logarithm
- `[f log: base]` -- logarithm in given base
- `[f log10]` -- base-10 logarithm
- `[f log2]` -- base-2 logarithm
- `[f exp]` -- e raised to this power
- `[f pow: exp]` -- raise to arbitrary power
- `[f hypot: other]` -- sqrt(self*self + other*other)

### rounding

- `[f truncate]` -- round toward zero (alias: [f toInteger] toFloat)
- `[f round: places]` -- round to n decimal places
- `[f roundUp]` -- alias for ceil
- `[f roundDown]` -- alias for floor

### special values

- `[f nan?]` -- true if NaN
- `[f infinite?]` -- true if positive or negative infinity
- `[f finite?]` -- true if not NaN and not infinite
- `[Float nan]` -- class method: returns NaN
- `[Float infinity]` -- class method: returns positive infinity
- `[Float negativeInfinity]` -- class method: returns negative infinity
- `[Float epsilon]` -- class method: smallest representable difference from 1.0
- `[Float pi]` -- class method: pi
- `[Float e]` -- class method: euler's number

### conversion

- `[f toInteger]` -- truncate to integer (native)
- `[f toString]` -- string representation
- `[f describe]` -- native string representation
- `[f show]` -- REPL display

### predicates

- `[f zero?]` -- true if 0.0 (inherited)
- `[f positive?]` -- true if > 0.0 (inherited)
- `[f negative?]` -- true if < 0.0 (inherited)
- `[f approx: other]` -- true if within Float epsilon
- `[f approx: other within: tolerance]` -- true if within tolerance

---

## 9. String

immutable UTF-8 text. conforms to Indexable (which includes Iterable).

### native (in rust)

- `[s length]` -- number of characters
- `[s at: i]` -- character at index i (as single-char string)
- `[s ++ other]` -- concatenation
- `[s substring: from to: to]` -- substring by index range
- `[s split: sep]` -- split by separator string, returns cons list
- `[s trim]` -- strip whitespace from both ends
- `[s contains: sub]` -- true if sub is found
- `[s startsWith: prefix]` -- true if starts with prefix
- `[s endsWith: suffix]` -- true if ends with suffix
- `[s toUpper]` -- uppercase copy
- `[s toLower]` -- lowercase copy
- `[s toInteger]` -- parse as integer
- `[s describe]` -- returns self (native)

### access (moof)

- `[s empty?]` -- true if length is 0
- `[s first]` -- first character (via Indexable)
- `[s last]` -- last character (via Indexable)
- `[s first: n]` -- first n characters as string
- `[s last: n]` -- last n characters as string
- `[s slice: from to: to]` -- alias for substring:to:

### search

- `[s indexOf: sub]` -- index of first occurrence, or nil
- `[s lastIndexOf: sub]` -- index of last occurrence, or nil
- `[s count: sub]` -- number of non-overlapping occurrences

### transform

- `[s capitalize]` -- uppercase first character, lowercase rest
- `[s swapcase]` -- swap upper/lower case
- `[s reverse]` -- reversed string
- `[s strip]` -- alias for trim
- `[s trimLeft]` -- strip leading whitespace
- `[s trimRight]` -- strip trailing whitespace
- `[s center: width]` -- center in field of width, pad with spaces
- `[s center: width with: pad]` -- center with custom pad character
- `[s ljust: width]` -- left-justify in field of width
- `[s rjust: width]` -- right-justify in field of width
- `[s squeeze]` -- collapse runs of same character to one
- `[s squeeze: chars]` -- collapse runs of chars in given string
- `[s delete: chars]` -- remove all occurrences of chars
- `[s tr: from to: to]` -- character transliteration

### building

- `[s repeat: n]` -- repeat n times
- `[s prepend: other]` -- other ++ s
- `[s insert: sub at: i]` -- insert sub at index i
- `[s replace: old with: new]` -- replace first occurrence
- `[s replaceAll: old with: new]` -- replace all occurrences (gsub)

### splitting

- `[s chars]` -- cons list of single-character strings
- `[s lines]` -- split on newlines
- `[s words]` -- split on whitespace
- `[s bytes]` -- cons list of byte values (integers)
- `[s partition: sep]` -- split into (before, sep, after) triple

### matching (future, when regex lands)

- `[s match: pattern]` -- first match or nil
- `[s scan: pattern]` -- all matches as list
- `[s gsub: pattern with: replacement]` -- global substitution

### conversion

- `[s toInteger]` -- parse as integer (native)
- `[s toFloat]` -- parse as float
- `[s toSymbol]` -- intern as symbol
- `[s toList]` -- alias for chars
- `[s encode: encoding]` -- encode to Bytes (future)

### from Indexable (protocol)

- `[s each: f]` -- iterate over characters
- plus all ~50 Iterable methods via the protocol chain

### from Comparable (requires native `<` for strings)

- `[s > other]`, `[s <= other]`, `[s >= other]`, etc.

note: String needs a native `<` handler comparing lexicographically.
this is not currently implemented in the compiler.

---

## 10. Cons (linked list)

the fundamental recursive data structure. a cons cell is a pair
(car, cdr). lists are chains of cons cells terminated by nil.

### native (in rust)

- `[c car]` -- first element of the pair
- `[c cdr]` -- rest of the pair
- `[c length]` -- walk the chain, count cells
- `[c describe]` -- formatted list representation

### aliases

- `[c first]` -- alias for car
- `[c rest]` -- alias for cdr

### access

- `[c at: n]` -- nth element (0-indexed)
- `[c last]` -- last element (walk to end)
- `[c second]` -- element at index 1
- `[c third]` -- element at index 2
- `[c empty?]` -- always false (nil is the empty list)

### construction

- `[c cons: x]` -- prepend x, return new list
- `[c append: other]` -- concatenate two lists
- `[c prepend: x]` -- alias for cons:
- `[c push: x]` -- append x at end (builds new list)

### iterable (native each:, protocol provides the rest)

- `[c each: f]` -- walk car/cdr, call f on each car
- `[c map: f]` -- apply f to each element, collect as new list
- `[c select: f]` -- keep elements where f is true
- `[c reject: f]` -- keep elements where f is false
- `[c find: f]` -- first element where f is true
- `[c fold: init with: f]` -- accumulate from init
- `[c reduce: f]` -- accumulate using first element
- `[c any: f]` -- true if any element satisfies f
- `[c all: f]` -- true if all elements satisfy f
- `[c none: f]` -- true if no element satisfies f
- `[c includes: x]` -- true if x is in the list
- `[c count]` -- number of elements
- `[c count: f]` -- number of elements satisfying f
- `[c sum]` -- sum of elements
- `[c product]` -- product of elements
- `[c min]` -- smallest element
- `[c max]` -- largest element
- `[c minBy: f]` -- element with smallest f(element)
- `[c maxBy: f]` -- element with largest f(element)
- `[c sort]` -- sorted copy (quicksort)
- `[c sortBy: f]` -- sort by key function
- `[c reverse]` -- reversed list
- `[c take: n]` -- first n elements
- `[c drop: n]` -- all but first n
- `[c takeWhile: f]` -- leading elements while f is true
- `[c dropWhile: f]` -- skip leading elements while f is true
- `[c zip: other]` -- pair elements as cons cells
- `[c flat]` -- flatten one level of nested lists
- `[c flatMap: f]` -- map then flatten
- `[c distinct]` -- unique elements
- `[c partition: f]` -- split into (matching, non-matching)
- `[c groupBy: f]` -- table mapping key to list
- `[c tally]` -- table mapping element to count
- `[c join]` -- concatenate element descriptions
- `[c join: sep]` -- join with separator
- `[c chunk: n]` -- list of n-sized sublists
- `[c sliding: n]` -- overlapping windows of size n
- `[c intersperse: sep]` -- insert sep between elements
- `[c each:withIndex: f]` -- f receives (element, index)
- `[c toList]` -- returns self
- `[c toTable]` -- convert to table sequential part

### smalltalk aliases

- `filter:` = `select:`
- `collect:` = `map:`
- `detect:` = `find:`
- `inject:into:` = `fold:with:`
- `size` = `count`
- `do:` = `each:`

---

## 11. Table

mutable collection with two parts: sequential (integer-indexed, 0-based)
and keyed (arbitrary key-value pairs). lua-style.

### native (in rust)

- `[t at: key]` -- read by integer index or key
- `[t at: key put: val]` -- write by integer index or key
- `[t push: val]` -- append to sequential part
- `[t length]` -- length of sequential part
- `[t keys]` -- list of keyed-part keys
- `[t values]` -- list of keyed-part values
- `[t contains: key]` -- true if key exists in seq or keyed part
- `[t remove: key]` -- remove keyed entry, return old value
- `[t describe]` -- formatted representation

### from Indexable (protocol)

- `[t each: f]` -- iterate sequential part by index
- all ~50 Iterable methods on the sequential part

### sequential operations (moof)

- `[t empty?]` -- true if sequential length is 0
- `[t first]` -- first sequential element
- `[t last]` -- last sequential element
- `[t pop]` -- remove and return last sequential element (native needed)
- `[t shift]` -- remove and return first sequential element (native needed)
- `[t unshift: val]` -- prepend to sequential part (native needed)
- `[t insert: val at: i]` -- insert at index (native needed)
- `[t removeAt: i]` -- remove at index (native needed)
- `[t slice: from to: to]` -- sub-table of sequential range
- `[t reverse]` -- reversed sequential part as new table
- `[t sort]` -- sorted sequential part as new table
- `[t sortBy: f]` -- sort by key function

### keyed operations (moof)

- `[t has: key]` -- true if keyed part contains key
- `[t entries]` -- list of (key, value) cons pairs
- `[t merge: other]` -- new table combining keyed parts (other wins conflicts)
- `[t update: key with: f]` -- apply f to current value, store result
- `[t getOrDefault: key default: val]` -- return value or default
- `[t getOrPut: key default: val]` -- return value or store and return default

### iteration over keyed part

- `[t eachEntry: f]` -- f receives (key, value) for keyed part
- `[t mapValues: f]` -- new table with f applied to each value
- `[t selectEntries: f]` -- keep entries where f(key, val) is true
- `[t rejectEntries: f]` -- remove entries where f(key, val) is true

### query operations (Queryable protocol)

- `[t where: f]` -- filter rows (select: for collections of objects)
- `[t orderBy: key]` -- sort by slot name (symbol)
- `[t groupBy: key]` -- group by slot value
- `[t join: other on: key equals: otherKey]` -- inner join
- `[t aggregate: spec]` -- compute aggregates (count, sum, avg)
- `[t distinct]` -- unique elements
- `[t limit: n]` -- first n rows
- `[t offset: n]` -- skip n rows

### conversion

- `[t toList]` -- sequential part as cons list
- `[t show]` -- REPL display

---

## 12. Block / Closure

blocks are closures created with `|params| body` or `(fn (params) body)`.
they are objects with a `call:` handler.

### core

- `[block call]` -- invoke with no arguments
- `[block call: arg]` -- invoke with one argument
- `[block call: a with: b]` -- invoke with two arguments (variadic)
- `[block value]` -- alias for call (smalltalk style)
- `[block value: arg]` -- alias for call:
- `[block arity]` -- number of parameters (native support needed)

### from Callable protocol

- `[block compose: other]` -- returns block: |x| [self call: [other call: x]]
- `[block >> other]` -- pipeline: |x| [other call: [self call: x]]
- `[block curry]` -- convert multi-arg block to chain of single-arg blocks
- `[block partial: arg]` -- pre-fill first argument
- `[block flip]` -- swap first two arguments

### utility

- `[block apply: list]` -- call with list elements as arguments (native needed)
- `[block repeat: n]` -- call n times, return last result
- `[block while: condition]` -- call while condition block returns true
- `[block until: condition]` -- call until condition block returns true
- `[block describe]` -- \"<fn arity:N>\"
- `[block show]` -- \"<fn arity:N>\"

---

## 13. Symbol

interned names. symbols are immediate values (no heap allocation).
two symbols with the same text are always identical.

- `[sym name]` -- the string name of the symbol (native needed)
- `[sym toString]` -- alias for name
- `[sym describe]` -- returns the symbol's name as string
- `[sym show]` -- returns \"'name\" (quoted form)
- `[sym < other]` -- lexicographic comparison (for Comparable)
- `[sym = other]` -- identity comparison (symbols are interned)
- `[sym type]` -- 'Symbol

---

## 14. IO (capability objects)

IO is done through capability objects. if your scope does not hold a
reference to Console, you cannot print. no ambient authority.

### Console

the standard IO capability.

- `[Console println: val]` -- print val's describe string with newline
- `[Console print: val]` -- print without newline
- `[Console readLine]` -- read a line from stdin (blocking)
- `[Console readLine: prompt]` -- read with prompt
- `[Console write: str]` -- write raw string to stdout
- `[Console error: str]` -- write to stderr
- `[Console errorln: str]` -- write to stderr with newline
- `[Console clear]` -- clear the terminal (ANSI escape)
- `[Console newline]` -- print a bare newline

### Filesystem (future)

capability for file operations.

- `[FS read: path]` -- read file contents as string
- `[FS write: path contents: str]` -- write string to file
- `[FS append: path contents: str]` -- append to file
- `[FS exists: path]` -- true if file exists
- `[FS delete: path]` -- delete file
- `[FS list: path]` -- list directory entries
- `[FS mkdir: path]` -- create directory
- `[FS stat: path]` -- file metadata object

### Clock (future)

capability for time.

- `[Clock now]` -- current timestamp (float, seconds since epoch)
- `[Clock monotonic]` -- monotonic time for benchmarking
- `[Clock sleep: seconds]` -- pause execution
- `[Clock measure: block]` -- time a block, return (result, elapsed) pair
- `[Clock date]` -- current date as object { year: month: day: }
- `[Clock time]` -- current time as object { hour: minute: second: }

### Network (future)

capability for HTTP.

- `[HTTP get: url]` -- fetch URL, return response object
- `[HTTP post: url body: data]` -- POST request
- `[HTTP put: url body: data]` -- PUT request
- `[HTTP delete: url]` -- DELETE request

### Random (future)

capability for randomness.

- `[Random integer: max]` -- random integer in [0, max)
- `[Random float]` -- random float in [0.0, 1.0)
- `[Random between: lo and: hi]` -- random integer in [lo, hi]
- `[Random sample: collection]` -- random element from collection
- `[Random shuffle: collection]` -- shuffled copy

---

## 15. Error handling

### the model

errors are objects. `doesNotUnderstand:` is a message, not a crash.
the handler-not-found path sends `doesNotUnderstand:` to the receiver.
if the receiver has no doesNotUnderstand: handler, the vat reports
the error and continues.

### Error objects

```moof
{ Error message: \"division by zero\" selector: '/ receiver: 0 }
```

- `[err message]` -- human-readable error string
- `[err selector]` -- the selector that failed (or nil)
- `[err receiver]` -- the object that received the failed send (or nil)
- `[err describe]` -- formatted error string
- `[err show]` -- REPL display

### try/catch

```moof
(try
  [dangerous operation]
  catch: |err| [handle the error])

; returns result of operation, or result of catch block
```

`try` is a vau-based form. it establishes an error boundary. if the
body raises an error (doesNotUnderstand:, division by zero, etc.),
the catch block receives the Error object.

### rescue (lightweight)

```moof
[risky-operation rescue: default-value]
```

sends the message; if it errors, returns default-value instead.
implemented as a handler on Object:

```moof
(defmethod Object rescue: (default)
  (try [self call] catch: |_| default))
```

### signaling errors

```moof
[Error signal: \"something went wrong\"]
; or
(error \"something went wrong\")
```

`error` is a kernel form or vau that creates an Error object and
triggers the nearest try/catch boundary.

### doesNotUnderstand:

```moof
(defmethod MyProxy doesNotUnderstand: (sel args)
  ; forward to delegate, log, transform, etc.
  [delegate send: sel with: args])
```

this enables: proxies, delegation, DSLs, method_missing-style
metaprogramming. the default doesNotUnderstand: on Object reports
the error and returns nil (or signals to try/catch).

---

## 16. Streams (lazy sequences)

streams are lazy, potentially infinite sequences. a stream is an
object with a `next` handler that returns the next value (or a
sentinel for exhaustion).

### creating streams

```moof
; from a block that generates values
(def naturals [Stream generate: 0 with: |n| [n + 1]])

; from an iterable (lazy wrapper)
(def lazy [(list 1 2 3) lazy])

; from explicit next handler
(def fibs [Stream new: { a: 0 b: 1 } next: |state|
  (let ((val state.a))
    [state slotAt: 'a put: state.b]
    [state slotAt: 'b put: [val + state.b]]
    val)])
```

### Stream methods

- `[s next]` -- produce and return next value
- `[s take: n]` -- materialize first n values as list
- `[s drop: n]` -- skip n values, return new stream
- `[s takeWhile: f]` -- take while f is true (lazy)
- `[s dropWhile: f]` -- skip while f is true (lazy)
- `[s map: f]` -- lazy transform (returns new stream)
- `[s select: f]` -- lazy filter (returns new stream)
- `[s reject: f]` -- lazy reject (returns new stream)
- `[s zip: other]` -- lazy pairwise combine
- `[s flat]` -- lazy flatten
- `[s flatMap: f]` -- lazy map + flatten
- `[s each: f]` -- materialize and iterate (CAUTION: infinite streams!)
- `[s toList]` -- materialize to list (CAUTION: infinite streams!)
- `[s force]` -- alias for toList
- `[s lazy]` -- returns self (already lazy)
- `[s cycle]` -- infinite repetition of a finite stream
- `[s intersperse: val]` -- lazy intersperse

### constructors

- `[Stream empty]` -- empty stream
- `[Stream of: val]` -- single-element stream
- `[Stream repeat: val]` -- infinite stream of same value
- `[Stream iterate: seed with: f]` -- seed, f(seed), f(f(seed)), ...
- `[Stream generate: init with: f]` -- alias for iterate:with:
- `[Stream range: from to: to]` -- lazy range
- `[Stream unfold: seed with: f]` -- f returns (value, nextSeed) or nil

---

## 17. Range

ranges represent bounded sequences of values. used for iteration
and bounds checking.

### creating ranges

```moof
[1 to: 10]            ; 1 through 10, step 1
[1 to: 10 by: 2]      ; 1, 3, 5, 7, 9
[10 downTo: 1]         ; 10, 9, 8, ... 1
[1 to: 100 by: 10]    ; 1, 11, 21, ... 91
```

### Range slots

- `range.start` -- first value
- `range.end` -- last value (inclusive)
- `range.step` -- step increment

### Range methods

- `[r each: f]` -- iterate through values (conforms to Iterable)
- `[r includes: val]` -- true if val is within bounds and on step
- `[r toList]` -- materialize to list
- `[r size]` -- number of values
- `[r reverse]` -- reversed range
- `[r map: f]` -- via Iterable
- `[r select: f]` -- via Iterable
- all other Iterable methods
- `[r min]` -- start or end depending on direction
- `[r max]` -- end or start depending on direction
- `[r describe]` -- \"(1 to: 10)\"
- `[r show]` -- \"(1 to: 10)\"

---

## 18. Set

unordered collection of unique elements. elements must be Hashable.

### creating sets

```moof
[Set new]                     ; empty set
[Set from: (list 1 2 3 2)]   ; => {1, 2, 3}
[(list 1 2 3 2) toSet]       ; => {1, 2, 3}
```

### Set methods

- `[s add: element]` -- add element, return self
- `[s remove: element]` -- remove element, return self
- `[s includes: element]` -- true if element is in set
- `[s size]` -- number of elements
- `[s empty?]` -- true if size is 0
- `[s union: other]` -- new set with all elements from both
- `[s intersection: other]` -- new set with shared elements
- `[s difference: other]` -- new set with elements not in other
- `[s symmetricDifference: other]` -- elements in one but not both
- `[s subset?: other]` -- true if all elements are in other
- `[s superset?: other]` -- true if all of other's elements are in self
- `[s disjoint?: other]` -- true if no shared elements
- `[s each: f]` -- iterate elements (conforms to Iterable)
- all Iterable methods
- `[s toList]` -- elements as list
- `[s toTable]` -- elements as table sequential part
- `[s describe]` -- \"{1, 2, 3}\"
- `[s show]` -- \"{1, 2, 3}\"

---

## 19. JSON

parse and serialize JSON. JSON is a global capability object.

### methods

- `[JSON parse: str]` -- parse JSON string into moof values
  - JSON object => Table (keyed part)
  - JSON array => Table (sequential part)
  - JSON string => String
  - JSON number => Integer or Float
  - JSON true/false => Boolean
  - JSON null => nil
- `[JSON serialize: val]` -- convert moof value to JSON string
  - Table with keyed part => JSON object
  - Table with sequential part => JSON array
  - Cons list => JSON array
  - String => JSON string
  - Integer/Float => JSON number
  - Boolean => JSON true/false
  - nil => JSON null
- `[JSON prettyPrint: val]` -- serialize with indentation
- `[JSON prettyPrint: val indent: n]` -- custom indent width

---

## 20. Testing

### test framework

tests are written as moof code using a simple assertion library.

```moof
(test \"integer arithmetic\"
  (assert [2 + 3] equals: 5)
  (assert [10 / 3] equals: 3)
  (assert [7 even?] equals: false))

(test \"list operations\"
  (assert [(list 1 2 3) sum] equals: 6)
  (assert [(list 3 1 2) sort] equals: (list 1 2 3))
  (assert [(list 1 2 3) includes: 2] equals: true))
```

### assertion methods

- `(assert expr equals: expected)` -- check equality
- `(assert expr)` -- check truthiness
- `(assertNil expr)` -- check nil
- `(assertError expr)` -- check that expr raises an error
- `(assert expr responds: 'sel)` -- check handler existence
- `(assert expr conforms: Protocol)` -- check protocol conformance
- `(assertRaises: block)` -- check that block signals error

### test runner

- `(test name body...)` -- define a named test
- `(testSuite name tests...)` -- group related tests
- `(runTests)` -- run all registered tests, report results
- output: test name, pass/fail, failure details

### implementation

`test` is a vau form that wraps the body in a try/catch. assertions
signal errors on failure. the runner collects results.

```moof
(def test (vau (name . body) $e
  (try
    (do . body)
    (test-pass name)
    catch: |err| (test-fail name err))))
```

---

## 21. Documentation

### self-documenting objects

objects document themselves through a `doc:` handler and a help
system.

```moof
; adding documentation to a method
(defmethod Integer factorial ()
  \"returns the factorial of self. self must be non-negative.\"
  (if [self <= 1] 1 [self * [[self - 1] factorial]]))

; querying documentation
[Integer doc: 'factorial]
; => \"returns the factorial of self. self must be non-negative.\"
```

### help system

- `[obj doc: 'selector]` -- return the doc string for a handler
- `[obj help]` -- print all handlers with their doc strings
- `[obj help: 'selector]` -- print doc for specific handler
- `[Protocol help]` -- print protocol requires/provides with docs
- `(help obj)` -- top-level help function

### how it works

when defmethod encounters a string literal as the first form of the
body, it stores it as metadata on the handler. the `doc:` handler
retrieves it. this mirrors python docstrings and elixir @doc.

---

## 22. Regular expressions (future)

pattern matching on strings via a Regex type.

### creating

```moof
(def pat [Regex new: \"[0-9]+\"])
; or with literal syntax (future):
; /[0-9]+/
```

### Regex methods

- `[pat match: str]` -- first match as Match object, or nil
- `[pat matchAll: str]` -- list of all Match objects
- `[pat test: str]` -- true if any match
- `[pat replace: str with: replacement]` -- replace first match
- `[pat replaceAll: str with: replacement]` -- replace all matches
- `[pat split: str]` -- split string on matches
- `[pat describe]` -- \"/pattern/\"

### Match object

- `[m text]` -- the matched text
- `[m start]` -- start index
- `[m end]` -- end index
- `[m group: n]` -- nth capture group
- `[m groups]` -- list of all capture groups

### String integration

- `[str match: regex]` -- delegates to regex
- `[str scan: regex]` -- all matches as list
- `[str gsub: regex with: replacement]` -- global substitution
- `[str sub: regex with: replacement]` -- first substitution

---

## 23. Naming conventions

### predicates

- without arguments: `?` suffix -- `empty?`, `even?`, `nil?`, `nan?`, `prime?`
- with arguments: NO `?` suffix -- `includes:`, `any:`, `all:`, `none:`, `contains:`
- rationale: `?` reads naturally at the end of a bare word; with args the colon already signals \"question\"

### conversions

- `to` prefix, camelCase target type -- `toFloat`, `toString`, `toList`, `toTable`, `toInteger`, `toSymbol`, `toSet`

### destructive operations (future)

- `!` suffix -- `sort!`, `reverse!`, `push!`, `pop!`
- only for mutations that modify the receiver in place
- the non-`!` version always returns a new object

### selectors

- camelCase for multi-word -- `groupBy:`, `flatMap:`, `sortBy:`, `startsWith:`
- no underscores
- colons separate keyword parts -- `fold:with:`, `at:put:`, `between:and:`

### protocol names

- PascalCase -- `Comparable`, `Iterable`, `Indexable`, `Callable`, `Hashable`, `Showable`, `Numeric`

### type prototype names

- PascalCase -- `Object`, `Integer`, `Float`, `String`, `Cons`, `Table`, `Boolean`, `Nil`, `Symbol`, `Block`, `Stream`, `Range`, `Set`

### global functions

- lowercase, hyphenated -- `for-each`, `nil?`, `some?`
- these are bootstrap-level; prefer methods over globals

### aliases table

| concept            | primary       | aliases                    |
|--------------------|---------------|----------------------------|
| transform          | map:          | collect:                   |
| keep matching      | select:       | filter:                    |
| remove matching    | reject:       | --                         |
| first matching     | find:         | detect:                    |
| accumulate         | fold:with:    | inject:into:               |
| accumulate (no init)| reduce:      | --                         |
| any match?         | any:          | --                         |
| all match?         | all:          | every:                     |
| no match?          | none:         | --                         |
| count elements     | count         | size, length (on Cons)     |
| iterate            | each:         | do:                        |
| contains?          | includes:     | contains: (native types)   |
| first n            | take:         | first:                     |
| skip n             | drop:         | --                         |
| sort               | sort          | --                         |
| sort by key        | sortBy:       | --                         |
| group              | groupBy:      | --                         |
| split by predicate | partition:    | --                         |
| pair               | zip:          | --                         |
| flatten            | flat          | flatten                    |
| map + flatten      | flatMap:      | collectConcat:             |
| unique             | distinct      | unique                     |
| join               | join: / join  | --                         |

---

## 24. Implementation priority

### phase 0: unblock (rust, ~100 lines)
1. CAPTURE_ENV opcode, eval with env parameter
2. or: and/or/when/unless as temporary compiler special forms

### phase 1: foundation (rust, ~50 lines)
3. Number prototype -- create between Object and Integer/Float
4. native `<` on String (lexicographic comparison)
5. native `hash` on Integer, Float, String, Symbol, Boolean, Nil
6. native `name` on Symbol

### phase 2: protocols (moof, ~100 lines)
7. protocols.moof -- clean Protocol object, protocol constructor using Tables for provides, conform, conforms?
8. showable.moof -- Showable protocol, show on every type, REPL integration

### phase 3: core protocols (moof, ~200 lines)
9. comparable.moof -- Comparable protocol, conform Number and String
10. numeric.moof -- Numeric protocol, conform Number
11. iterable.moof -- THE centerpiece. ~50 provided methods from each:
12. indexable.moof -- Indexable protocol (includes Iterable), conform String and Table
13. callable.moof -- Callable protocol, conform Block

### phase 4: type enrichment (moof, ~150 lines)
14. integer.moof -- gcd:, lcm:, digits, factorial, toRadix:, prime?
15. float.moof -- trig, log, exp, nan?, infinite?, constants
16. string.moof -- capitalize, center:, squeeze, tr:to:, replace:with:
17. boolean.moof -- and:, or:, xor:
18. nil.moof -- ifNil:, empty list behavior
19. cons.moof -- remaining aliases, second, third
20. table.moof -- entries, merge:, eachEntry:, mapValues:, query ops
21. block.moof -- repeat:, while:, until:, apply:

### phase 5: new types (rust + moof, ~200 lines)
22. Range -- native Range type, to:, to:by:, Iterable conformance
23. Set -- native Set type backed by hash table
24. Stream -- lazy sequences with next handler
25. Error -- structured errors, try/catch form

### phase 6: capabilities (rust + moof, ~150 lines)
26. Console object -- readLine, error:, clear
27. Clock object -- now, monotonic, measure:, sleep:
28. Random object -- integer:, float, sample:, shuffle:
29. JSON -- parse:, serialize:

### phase 7: quality of life (moof, ~100 lines)
30. Testing framework -- test, assert, runTests
31. Documentation system -- doc:, help, docstrings in defmethod
32. Regular expressions -- Regex type (needs rust regex crate)

### estimated totals

- new rust: ~500 lines across phases 0-1, 5-6
- new moof: ~750 lines across phases 2-4, 7
- replaces: ~270 lines of current core.moof
- the result: ~50 methods from implementing each:, ~8 from <, ~5 from negate

### critical path

```
vau/env fix  -->  protocols.moof  -->  iterable.moof  -->  everything else
     |                                      |
     v                                      v
Number proto  -->  comparable.moof    indexable.moof
     |                                      |
     v                                      v
String <     -->  string enrichment   table enrichment
```

the vau/env fix is the single gating item. nothing else can start
without either the real fix or the special-forms fallback. after that,
protocols and Iterable are the highest-leverage work: one protocol
implementation gives dozens of methods to every conforming type.

---

## file structure

```
lib/
  bootstrap.moof      (1) kernel: defn, defmethod, alias, list, match
                           and/or/when/unless
  protocols.moof       (2) Protocol object, protocol(), conform, conforms?
  showable.moof        (3) Showable protocol, show on every type
  comparable.moof      (4) Comparable protocol, conform Number, String
  numeric.moof         (5) Numeric protocol, conform Number
  iterable.moof        (6) ~50 methods from each:. the crown jewel.
  indexable.moof       (7) Indexable (includes Iterable). conform String, Table
  callable.moof        (8) Callable protocol. conform Block
  types/
    object.moof        (9a) Object: inspect, is:, then:, yield:
    nil.moof           (9b) Nil: ifNil:, empty list behavior
    boolean.moof       (9c) Boolean: and:, or:, xor:
    integer.moof       (9d) Integer: gcd:, lcm:, digits, factorial, etc.
    float.moof         (9e) Float: trig, log, exp, constants
    string.moof        (9f) String: capitalize, center:, tr:to:, etc.
    cons.moof          (9g) Cons: aliases, second, third
    table.moof         (9h) Table: merge:, entries, query ops
    block.moof         (9i) Block: repeat:, while:, apply:
    symbol.moof        (9j) Symbol: toString, show
  stream.moof          (10) Stream type and methods
  range.moof           (11) Range type and methods
  set.moof             (12) Set type and methods
  error.moof           (13) Error type, try/catch
  io/
    console.moof       (14a) Console capability enrichment
    clock.moof         (14b) Clock capability
    random.moof        (14c) Random capability
  json.moof            (15) JSON parse/serialize
  testing.moof         (16) test framework
  docs.moof            (17) documentation system
```

load order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9a-j -> 10-17
```

---

### Critical Files for Implementation

- /Users/shreyan.jain27/Stuff/moof/src/lang/compiler.rs (register_type_protos function, lines 873-1653 -- where Number prototype insertion, native hash/String-<, and bit ops must be added)
- /Users/shreyan.jain27/Stuff/moof/lib/bootstrap.moof (kernel forms, protocol infrastructure, and and/or/when/unless -- the vau/env fix gates everything)
- /Users/shreyan.jain27/Stuff/moof/lib/core.moof (current type methods to be replaced/reorganized into the new file structure)
- /Users/shreyan.jain27/Stuff/moof/VISION.md (the design vision that this stdlib plan implements -- protocol definitions, type hierarchy, query model)
- /Users/shreyan.jain27/Stuff/moof/src/dispatch.rs (handler dispatch and doesNotUnderstand: path -- critical for error handling design)"}],"stop_reason":"end_turn","stop_sequence":null,"stop_details":null,"usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":50330,"output_tokens":16998,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard"}},"requestId":"req_011CZqzRLfyVE25fAqsXBxor","type":"assistant","uuid":"41abae12-20c7-4605-a4ca-d4f5be1d74a0","timestamp":"2026-04-08T08:35:03.112Z","userType":"external","entrypoint":"cli","cwd":"/Users/shreyan.jain27/Stuff/moof","sessionId":"8a2b1891-c8aa-4596-9999-1df3ae868eb3","version":"2.1.94","gitBranch":"master","slug":"frolicking-honking-boole"}
