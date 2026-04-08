# moof standard library: the real plan

> everything follows from: objects, messages, protocols.
> if you can't send it a message, it doesn't exist.

---

## 0. the hard problem first: vau and environments

### the current break

vau receives `$e` (the caller's environment) but it's always nil.
`eval` compiles against globals only. this means:

```moof
(defmethod Integer foo: (x)
  (and [self > 0] [x > 0]))  ; BROKEN — and is a vau, eval can't see self or x
```

this blocks the ENTIRE stdlib from using `and`, `or`, `when`,
`unless` inside any method body. which is... every method.

### the fix: first-class environments

the correct fix is NOT "make and/or compiler special forms."
that's giving up on vau. the correct fix is: **make environments
real objects and make eval use them.**

how it works:

1. **environments are objects.** they already are in the
   compiler — a let/fn scope is conceptually an object with
   bindings as slots. make this REAL: when the VM enters a
   closure, it creates (or uses) an environment object.

2. **`$e` carries the real environment.** when a vau operative
   is called, `$e` is the caller's environment object — with
   locals, not just globals.

3. **`eval` takes an optional environment.** `(eval expr)` uses
   globals (current behavior). `(eval expr $e)` compiles and
   runs expr in the context of environment `$e`, where the
   local bindings are visible.

4. **the compiler emits an ENV opcode** that captures the
   current scope as a heap object. this is only emitted when
   calling a vau operative (lazy — don't create env objects
   unless a vau actually needs them).

this is a REAL change:
- new opcode: CAPTURE_ENV dst — creates an env object from
  current registers + locals
- eval form accepts 2 args: (eval expr env)
- vau call sites emit CAPTURE_ENV before calling
- the environment object is a General with locals as slots
  + parent pointing to the enclosing env

estimated effort: ~100 lines of rust across compiler + VM.
this is NOT a multi-week project. it's a day of work.

### what this enables

```moof
; and/or/when/unless work EVERYWHERE because eval sees locals
(defmethod Integer safe-div: (other)
  (when [other > 0] [self / other]))

; custom control flow that sees locals
(def my-unless (vau (test body) $e
  (if (eval test $e) nil (eval body $e))))
```

### fallback plan

if first-class envs prove too hard right now, make and/or/when/
unless compiler special forms AS A TEMPORARY MEASURE. but mark
them clearly as tech debt, not as "the answer." the vau story
is too important to abandon.

---

## 1. the Showable protocol: how things present themselves

### the principle

the REPL should NEVER use rust-side format_value or display_value
for output. it should ALWAYS send `[val show]` and print the
result. every type defines how it wants to be shown.

### the protocol

```
Showable
  requires: show
  provides: (nothing — show IS the whole protocol)
```

`show` returns a STRING. not the value itself — a string
representation suitable for the REPL.

### what show returns for each type

```
nil          => "nil"
true         => "true"
false        => "false"
42           => "42"
3.14         => "3.14"
'hello       => "'hello"
"hello"      => "\"hello\""
(1 2 3)      => "(1 2 3)"
{ x: 3 }     => "{ x: 3 }"
#[1 2]       => "#[1 2]"
<fn>         => "<fn arity:2>"
<vau>        => "<vau (test body) $e>"
<native>     => "<native: +>"
```

### the REPL change

```rust
// current (BAD):
println!("  {}", heap.display_value(val));

// correct:
// send [val show] and print the resulting string
let show_result = vm.dispatch_send(heap, val, heap.sym_show, &[])?;
let show_str = /* extract string from show_result */;
println!("  {show_str}");
```

if `show` fails (no handler), fall back to format_value. but
this should never happen once everything conforms to Showable.

---

## 2. type hierarchy

```
Object                    Showable
  Nil                     Showable
  Boolean                 Showable
  Number                  Showable, Comparable, Numeric
    Integer               + even?, odd?, times:, pow:, gcd:, ...
    Float                 + sqrt, sin, cos, floor, ceil, ...
  Symbol                  Showable
  Cons                    Showable, Iterable
  String                  Showable, Comparable, Indexable (includes Iterable)
  Bytes                   Indexable
  Table                   Showable, Indexable (includes Iterable)
  Block                   Showable, Callable
  Protocol                Showable
```

### Number (NEW)

parent of Integer and Float. created in rust
(register_type_protos). holds shared methods:

from Comparable: `>`, `<=`, `>=`, `between:and:`, `clamp:to:`,
  `min:`, `max:`
from Numeric: `abs`, `sign`, `zero?`, `positive?`, `negative?`
own: (none — everything comes from protocols)

Integer and Float keep their type-specific natives (+, -, etc.)
but INHERIT the protocol-provided methods from Number.

### creating Number in rust

```rust
let number_proto = heap.make_object(object_proto);
heap.type_protos[10] = number_proto;
// reparent Integer and Float
let int_proto = heap.make_object(number_proto);  // was: object_proto
let float_proto = heap.make_object(number_proto); // was: object_proto
```

register Number as a global so moof can add methods to it.

---

## 3. protocols: the REAL implementation

### protocol infrastructure

a Protocol is an object with:
- `name`: string
- `requires`: list of selector symbols
- `provides`: Table mapping selector → handler
- `includes`: list of other protocols

```moof
(def Comparable (protocol
  name: "Comparable"
  requires: (list '<)
  provides: #[
    '> => |self other| (not [self < other])
    '<= => |self other| (not [other < self])
    '>= => |self other| (not [self < other])
    'between:and: => |self lo hi| (if [self >= lo] [self <= hi] false)
    'clamp:to: => |self lo hi| [[self max: lo] min: hi]
    'min: => |self other| (if [self < other] self other)
    'max: => |self other| (if [self > other] self other)
  ]))
```

### conform

`(conform Type Protocol)` does:
1. check `requires` — error if any missing
2. for each entry in `provides`, if the type doesn't already
   have that handler, install it
3. for each protocol in `includes`, recursively conform

```moof
(conform Number Comparable)
; Number gets: >, <=, >=, between:and:, clamp:to:, min:, max:
; Integer and Float inherit them through Number
```

### the provides table uses Tables, not flat lists

the current implementation uses flat cons lists for provides
(selector handler selector handler ...). this is fragile.
provides should be a Table (our hashmap):

```moof
#['> => (fn ...) '<= => (fn ...) '>= => (fn ...)]
```

this makes conform cleaner — iterate the table's entries.

---

## 4. Iterable: the crown jewel

### the contract

```moof
(def Iterable (protocol
  name: "Iterable"
  requires: (list 'each:)
  provides: #[
    ; -- transforming --
    'map: => |self f| ...
    'flatMap: => |self f| ...
    'flat => |self| ...

    ; -- filtering --
    'select: => |self f| ...
    'reject: => |self f| ...
    'find: => |self f| ...

    ; -- quantifiers --
    'any: => |self f| ...
    'all: => |self f| ...
    'none: => |self f| ...
    'includes: => |self x| ...

    ; -- reducing --
    'fold:with: => |self init f| ...
    'reduce: => |self f| ...
    'sum => |self| ...
    'product => |self| ...
    'count => |self| ...
    'count: => |self f| ...

    ; -- accessing --
    'first => |self| ...
    'last => |self| ...
    'take: => |self n| ...
    'drop: => |self n| ...
    'takeWhile: => |self f| ...
    'dropWhile: => |self f| ...

    ; -- ordering --
    'sort => |self| ...
    'sortBy: => |self f| ...
    'reverse => |self| ...
    'min => |self| ...
    'max => |self| ...
    'minBy: => |self f| ...
    'maxBy: => |self f| ...

    ; -- grouping --
    'groupBy: => |self f| ...
    'partition: => |self f| ...
    'tally => |self| ...

    ; -- combining --
    'zip: => |self other| ...

    ; -- string --
    'join => |self| ...
    'join: => |self sep| ...

    ; -- other --
    'distinct => |self| ...
    'each:withIndex: => |self f| ...
    'toList => |self| ...
    'toTable => |self| ...
  ]))
```

### implementation strategy

every provided method is implemented in terms of `each:` and
`fold:with:`. the core pattern:

```moof
; fold:with: is the universal accumulator
'fold:with: => |self init f|
  (let ((acc init))
    [self each: |x| (:= acc (f acc x))]
    acc)

; everything else derives from fold
'map: => |self f|
  [[self fold: nil with: |acc x| (cons (f x) acc)] reverse]

'select: => |self f|
  [[self fold: nil with: |acc x| (if (f x) (cons x acc) acc)] reverse]

'sum => |self| [self fold: 0 with: |a x| [a + x]]

'count => |self| [self fold: 0 with: |a _| [a + 1]]

'any: => |self f|
  ; this can't short-circuit without first-class envs
  ; use fold with a flag
  [self fold: false with: |found x| (if found true (f x))]
```

### who conforms

```moof
; Cons implements each: natively (walk car/cdr)
(defmethod Cons each: (block)
  (let ((current self))
    (while (some? current)
      (block [current car])
      (:= current [current cdr]))))
(conform Cons Iterable)

; String gets each: from Indexable (iterate chars by index)
; Table gets each: from Indexable (iterate sequential part by index)
```

---

## 5. Indexable: Iterable for indexed things

```moof
(def Indexable (protocol
  name: "Indexable"
  requires: (list 'at: 'length)
  includes: (list Iterable)
  provides: #[
    ; derive each: from at: + length
    'each: => |self block|
      (let ((i 0) (len [self length]))
        (while [i < len]
          (block [self at: i])
          (:= i [i + 1])))

    'first => |self| [self at: 0]
    'last => |self| [self at: [[self length] - 1]]
    'empty? => |self| (eq [self length] 0)
    'indexOf: => |self x|
      (let ((i 0) (len [self length]) (result nil))
        (while (if [i < len] [result nil?] false)
          (if [[self at: i] equal: x] (:= result i) nil)
          (:= i [i + 1]))
        result)
  ]))
```

conforming to Indexable automatically conforms to Iterable
(via the derived `each:`). so String and Table get ALL 36
Iterable methods from just having `at:` and `length`.

---

## 6. every type, every method

### Object (root)
native: describe, slotAt:, slotAt:put:, parent, slotNames,
  handlerNames, handle:with:, handlerAt:, responds:, clone,
  type, equal:, print, println
moof: nil?, some?, tap:, pipe:, show (Showable)

### Nil
moof: nil? => true, some? => false, empty? => true, show

### Boolean
native: not, describe
moof: ifTrue:ifFalse:, ifTrue:, ifFalse:, show, toString

### Number (shared by Integer + Float)
from Comparable (P): >, <=, >=, between:and:, clamp:to:, min:, max:
from Numeric (P): abs, sign, zero?, positive?, negative?

### Integer
native: +, -, *, /, %, <, >, <=, >=, =, negate, describe
moof: even?, odd?, times:, upto:do:, downto:do:, pow:, inc,
  dec, gcd:, digits, toFloat, toString, show

### Float
native: +, -, *, /, <, >, <=, >=, =, describe, sqrt, floor,
  ceil, round, toInteger
moof: sin, cos, tan, log, exp (should be native), nan?,
  infinite?, truncate, toString, show

### Symbol
native: (describe via Object)
moof: name, toString, show

### String
native: length, at:, ++, substring:to:, split:, trim,
  contains:, startsWith:, endsWith:, toUpper, toLower,
  toInteger, describe
from Indexable (P → includes Iterable): each:, first, last,
  empty?, indexOf:, + ALL 36 Iterable methods
from Comparable (P): >, <=, >=, etc (needs native < on String)
moof: chars, lines, words, repeat:, replace:with:, reverse,
  toFloat, toSymbol, show

### Cons
native: car, cdr, length, describe
moof: first (=car), rest (=cdr), last, at:, cons:, append:,
  reverse, empty?
from Iterable (P): each: (moof-defined, walks car/cdr),
  + ALL 36 Iterable methods
moof: show
aliases: filter:=select:, collect:=map:, detect:=find:,
  inject:into:=fold:with:, size=count, do:=each:

### Table
native: at:, at:put:, push:, length, keys, values, describe,
  contains:, remove:
from Indexable (P → Iterable): each:, first, last, empty?,
  + ALL 36 Iterable methods
moof: pop, entries, has:, merge:, show

### Block/Closure
from Callable (P): compose:, curry, partial:
moof: call (no-arg), arity, show

### Protocol
moof: name, requires, provides, includes, conformers,
  describe, show

---

## 7. naming conventions (final)

| concept | primary | aliases |
|---------|---------|---------|
| transform | map: | collect: |
| keep matching | select: | filter: |
| remove matching | reject: | — |
| first matching | find: | detect: |
| accumulate | fold:with: | inject:into: |
| accumulate (no init) | reduce: | — |
| any match? | any: | — |
| all match? | all: | every: |
| no match? | none: | — |
| count | count | size |
| iterate | each: | do: |
| contains? | includes: | contains: (types with native) |
| first N | take: | first: |
| skip N | drop: | — |
| sort | sort | — |
| sort by key | sortBy: | — |
| group | groupBy: | — |
| split by pred | partition: | — |
| pair | zip: | — |
| flatten | flat | flatten |
| map+flat | flatMap: | — |
| unique | distinct | unique |
| join | join: / join | — |

predicates without args: `?` suffix (empty?, even?, nil?)
predicates with args: NO `?` (includes:, any:, all:)
conversions: `to` prefix (toFloat, toString, toList)
destructive (future): `!` suffix (sort!, reverse!)
keywords: camelCase (groupBy:, flatMap:, sortBy:)

---

## 8. file structure

```
lib/
  bootstrap.moof      (1) kernel: defn, defmethod, alias, list, match
                           and/or/when/unless (compiler forms OR vau
                           with env, depending on env fix)
  protocols.moof       (2) Protocol object, protocol constructor,
                           conform, conforms?
  showable.moof        (3) Showable protocol, show on every type,
                           REPL integration
  comparable.moof      (4) Comparable protocol, conform Number, String
  numeric.moof         (5) Numeric protocol, conform Number
                           Integer extras, Float extras
  iterable.moof        (6) THE protocol. 36 methods. conform Cons
  indexable.moof       (7) Indexable (includes Iterable). conform
                           String, Table
  callable.moof        (8) Callable protocol. conform Block
  types.moof           (9) remaining type methods: Boolean ifTrue:,
                           String chars/lines/words, Cons aliases,
                           Table extras, Symbol extras
```

load order: 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9

---

## 9. the aesthetic

the stdlib should feel like a grimoire. not in naming (we
don't call methods "incantations") but in FEEL:

- **everything just works.** `[42 even?]`, `[(list 1 2 3) sum]`,
  `["hello" reverse]` — no imports, no setup, no ceremony.
- **discovery is natural.** `[42 handlerNames]` tells you
  everything 42 can do. `[42 responds: 'even?]` checks before
  you send.
- **errors are contextual.** "42 does not understand 'foo'"
  tells you what went wrong and on what.
- **show is beautiful.** the REPL output is clean, typed,
  informative. not a debug dump.
- **consistency is absolute.** every type has show, describe,
  type, parent, handlerNames. the universe is uniform.

---

## 10. implementation order

1. **fix vau/env** — CAPTURE_ENV opcode, eval with env param.
   this unblocks everything. (~100 lines of rust)
2. **Number prototype** — create in rust, reparent Int/Float.
   (~20 lines of rust)
3. **protocols.moof** — clean protocol infrastructure using
   Tables for provides. (~40 lines of moof)
4. **showable.moof** — Showable protocol, show on all types,
   REPL uses [val show]. (~60 lines of moof + 5 lines of rust)
5. **comparable.moof** — 7 methods from <. (~30 lines)
6. **numeric.moof** — 5 methods from +/-/*/negate. (~50 lines)
7. **iterable.moof** — 36 methods from each:. THE centerpiece.
   (~150 lines)
8. **indexable.moof** — derives each: from at:/length. (~40 lines)
9. **callable.moof** — compose:/curry/partial:. (~20 lines)
10. **types.moof** — remaining type-specific methods. (~80 lines)
11. **clean up** — remove old core.moof, update REPL, test
    everything.

total moof: ~470 lines across 9 files.
total new rust: ~120 lines.
replaces: ~270 lines of current core.moof + scattered compiler code.

the result: a standard library where implementing ONE method
(each:, or <, or call:) gives you DOZENS for free, every type
participates in the object system uniformly, and the REPL shows
you exactly what everything is through the Showable protocol.
