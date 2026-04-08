# moof standard library plan

> inspired by ruby's Enumerable, smalltalk's collections,
> elixir's Enum, and the principle: implement one, get thirty.

## the rule

methods on objects. NOT standalone functions. you write
`[(list 1 2 3) map: |x| [x * 2]]` not `(map |x| [x * 2] list)`.

standalone functions exist for convenience (like `map` calling
`[list map: ...]`) but the PRIMARY interface is message sends.

protocols are the mechanism. implement `each:` on your type,
conform to Iterable, get 30+ methods for free. implement `<`,
conform to Comparable, get 7 methods.

---

## Object (root — everything inherits these)

```
describe        "human-readable string"
print           output to stdout, return self
println         output + newline, return nil
type            return type symbol ('Integer, 'String, etc.)
equal:          content equality (strings compare by content)
parent          prototype object
slotNames       list of slot symbols
handlerNames    list of all handler symbols (walks chain)
slotAt:         read a slot by symbol
slotAt:put:     write a slot value
handle:with:    add/replace a handler
responds:       true if handler exists for selector
clone           shallow copy (new object, same slots+handlers)
tap:            call block with self, return self (debug pipe)
pipe:           [x pipe: f] = (f x) — threading helper
```

## Integer

```
; arithmetic
+ - * / % negate abs
pow:            [2 pow: 10] => 1024

; comparison (Comparable protocol)
< > <= >= = between:and: clamp:to: min: max:

; predicates
zero? positive? negative? even? odd?

; conversion
toFloat toString toChar

; iteration
times:          [5 times: |i| [i print]]
upto:do:        [1 upto: 10 do: |i| ...]
downto:do:      [10 downto: 1 do: |i| ...]

; math
gcd: lcm: digits
```

## Float

```
; arithmetic (same as Integer)
+ - * / negate abs

; comparison (Comparable)
< > <= >= =

; math
sqrt sin cos tan log exp
floor ceil round truncate
toInteger

; predicates
nan? infinite? zero? positive? negative?
```

## String

```
; access
length at: empty?

; building
++ repeat:

; searching
contains: startsWith: endsWith: indexOf:

; transforming
toUpper toLower reverse trim trimLeft trimRight
replace:with: split: substring:to:

; decomposing
chars lines words

; converting
toInteger toFloat toSymbol

; Iterable (iterate characters)
each:           [str each: |c| [c print]]
; → gets map:, select:, fold:with:, join, etc. from Iterable
```

## Boolean

```
not
ifTrue:ifFalse:   [cond ifTrue: || "yes" ifFalse: || "no"]
```

## Cons (linked list)

```
; core
car / first     first element
cdr / rest      rest of list
last            last element
length          count elements
empty?          true if nil
at:             nth element

; building
cons:           prepend: [list cons: x] => (x . list)
append:         join two lists

; Iterable (THE protocol — implement each:, get everything)
each:           iterate elements
; all Iterable methods available after conform
```

## Table

```
; access
at:             integer index OR key lookup
at:put:         set by index or key
length          sequential part length

; sequential
push: pop first last

; map
keys values entries
has: / containsKey:
remove:
merge:

; Iterable (iterates sequential part)
each:
; all Iterable methods

; Queryable (for structured data — objects as rows)
where:          filter by predicate
orderBy:        sort by key
groupBy:        group by key
```

## Block / Closure

```
call:           invoke (receives args as list)
call            invoke with no args
arity           number of params
compose:        [f compose: g] = |x| (f (g x))
>>              alias for compose: (pipe-forward)
curry           partial application
```

---

## Protocols

### Iterable (requires: each:)

the big one. ruby's Enumerable. implement `each:` and get:

```
map:            transform each element
select:         keep matching (ruby name)
reject:         remove matching
fold:with:      accumulate: [list fold: 0 with: |a x| [a + x]]
reduce:         fold without initial: [list reduce: |a x| [a + x]]
any:            any match predicate?
all:            all match predicate?
none:           none match predicate?
count           number of elements
count:          count matching predicate
find:           first matching (smalltalk: detect:)
first           first element (or first: N)
last            last element
take:           first N
drop:           skip first N
takeWhile:      take while predicate true
dropWhile:      drop while predicate true
sort            natural order (needs Comparable elements)
sortBy:         sort by key function
min / max       smallest/largest (needs Comparable)
minBy: / maxBy: by key function
sum / product   numeric accumulation
reverse         reverse order
join: / join    join with separator / join with nothing
groupBy:        group into table by key
partition:      split into [matches, non-matches]
zip:            pair with another iterable
flat            flatten nested lists
flatMap:        map then flatten
includes:       contains element?
tally           count occurrences => Table
distinct        remove duplicates
each:withIndex: iterate with index
toList          materialize as cons list
toTable         materialize as table
```

**~35 methods from implementing ONE handler.**

### Comparable (requires: <)

```
>               (not [other < self])
<=              (not [self > other])
>=              (not [self < other])
between:and:    [x between: lo and: hi]
clamp:to:       [x clamp: lo to: hi]
min:            [a min: b]
max:            [a max: b]
```

### Numeric (requires: + - * negate)

```
abs             [x abs]
sign            -1, 0, or 1
zero?           [x = 0]
positive?       [x > 0]
negative?       [x < 0]
```

### Callable (requires: call:)

```
compose:        [f compose: g] = |x| (f (g x))
>>              alias
curry           partial application
```

---

## lib/ file structure

```
lib/
  bootstrap.moof    kernel syntax: and, or, when, unless,
                    defn, defmethod, list, match
  
  core.moof         Object enhancements: responds:, clone,
                    tap:, pipe:. Boolean: ifTrue:ifFalse:.
                    Integer: times:, upto:do:, etc.
                    String: chars, lines, words, etc.
  
  iterable.moof     THE protocol. each: => 35 methods.
                    conform Cons, String, Table.
  
  comparable.moof   < => 7 methods. conform Integer, Float,
                    String.
  
  protocols.moof    Numeric, Callable, other protocols.
```

## loading order

1. bootstrap.moof (kernel syntax)
2. core.moof (type enhancements)
3. iterable.moof (the big protocol)
4. comparable.moof
5. protocols.moof

---

## naming conventions

- predicates end with `?`: `empty?`, `even?`, `nil?`
- destructive operations end with `!`: `sort!`, `reverse!` (future)
- keyword selectors use camelCase: `startsWith:`, `groupBy:`
- unary selectors are lowercase: `length`, `reverse`, `sum`
- the method name should read like english:
  `[list select: |x| [x > 0]]` — "list, select where x > 0"
  `[str startsWith: "hello"]` — "str starts with hello"
  `[5 times: |i| [i print]]` — "5 times, do i print"

## what stays as standalone functions

some things make more sense as functions than methods:

```
(list 1 2 3)        variadic list construction
(str a b c)         variadic string building
(range n)           generate 0..n
(not x)             boolean negation
(and a b)           short-circuit (vau)
(or a b)            short-circuit (vau)
(when test body)    conditional eval (vau)
(unless test body)  conditional eval (vau)
(match ...)         pattern matching (vau)
(defn ...)          function definition (vau)
(defmethod ...)     handler definition (vau)
(apply f args)      apply function to list
(eval expr)         runtime eval
```

these are either vau-based (need unevaluated args) or are
constructors (building new values, not operating on existing
ones).
