# structural types for the objectspace

## the question

can moof have haskell-style type safety on top of its dynamic
object model? not "can we bolt on types" but "can types emerge
naturally from what moof already is?"

## what haskell has

```
algebraic data types    data Maybe a = Nothing | Just a
parametric polymorphism map :: (a -> b) -> [a] -> [b]
typeclasses             class Functor f where fmap :: (a -> b) -> f a -> f b
type inference          HM inference — types from usage, not annotations
higher-kinded types     Functor f where f :: * -> *
pattern matching        case x of Just v -> v; Nothing -> 0
```

## what moof already has (informally)

```
haskell concept     moof equivalent           gap
───────────────     ───────────────           ────
ADTs                prototype variants        open (not closed)
typeclasses         protocols                 no type params
polymorphism        duck typing               no guarantees
inference           none                      all runtime
HKTs                Chainable protocol        no type constructors
pattern matching    handler dispatch          no destructuring
```

moof has the RUNTIME behavior of all of these. it just can't
verify correctness at definition time. the question is: can we
add verification without changing how the language works?

## the proposal: structural row types

### the core idea

types describe SHAPES — which slots and handlers an object
has, and what types those expect/return. not names, not classes,
not inheritance hierarchies. shapes.

```moof
; a type is a shape description
(type Point { x: Float  y: Float })

; any object with x: Float and y: Float IS a Point
(def p { x: 3.0  y: 4.0 })
; p : Point ✓ (structural match)
```

this is structural typing (like TypeScript), not nominal typing
(like Java). you don't declare that p IS a Point — it just is,
because it has the right shape.

### row polymorphism

```moof
(type HasPosition { x: Float  y: Float  ... })
; the ... means "and possibly other slots"

(defn distance (a: HasPosition  b: HasPosition) -> Float
  ...)
```

`HasPosition` matches any object with at least `x` and `y`
slots of type Float. it could have other slots too. this is
row polymorphism — the type is open, like moof objects.

this is natural for moof because prototype delegation means
objects always have "extra" slots from their parent chain.
row types describe the minimum shape.

### handler types (method signatures)

```moof
(type Counter {
  count: Integer
  [get] -> Integer
  [increment] -> Update
  [log] -> Act
})
```

handlers have type signatures: input types → return type.
`[get] -> Integer` means "get takes no args and returns an
Integer." `[increment] -> Update` means it returns an Update
value.

### function types

```moof
(defn add (a: Integer  b: Integer) -> Integer
  [a + b])

; type of add: (Integer, Integer) -> Integer
```

function parameters and return types are annotated. the checker
verifies call sites.

### protocol types (typeclasses)

protocols already ARE typeclasses. adding type annotations:

```moof
(defprotocol Chainable
  (require (then: f: (-> a b)) -> b)
  (provide (map: f: (-> a b)) -> (Self b)))
```

`then:` takes a function from `a` to `b` and returns `b`.
`map:` takes a function from `a` to `b` and returns the same
monadic wrapper around `b`.

the type variables `a` and `b` are universally quantified.
`Self` is the implementing type.

### parametric polymorphism

```moof
(defn map (f: (-> a b)  xs: (List a)) -> (List b)
  [xs map: f])
```

`a` and `b` are type variables. `map` works for any `a` and
`b`. the checker verifies that `f` accepts elements of type
`a` and that `xs` contains elements of type `a`.

in moof, this is already how map works at runtime. types just
make the contract explicit and checkable.

## how it works with moof's object model

### prototypes as types

every prototype defines a type. `{ Counter count: 0 [get] ... }`
has the type `Counter`. objects that delegate to Counter have
type Counter. this is the nominal aspect — you can name a
prototype as a type.

but structural matching is primary. if you write a function
that accesses `@count`, the type checker infers the parameter
needs `{ count: Integer, ... }`. it doesn't care if it's a
Counter specifically — any object with `count: Integer` works.

### protocols as type constraints

```moof
(defn process (x: Chainable) -> Chainable
  [x then: |v| [v + 1]])
```

`Chainable` as a type means "any object whose prototype
conforms to Chainable." the checker verifies conformance at
the call site.

### the identity monad as default type

every moof value conforms to Chainable via Object's default
`then:`. so `Chainable` is the universal type — everything
satisfies it. narrower protocols (Iterable, Comparable) impose
stricter constraints.

### Act/Result/Option as type constructors

```
Act Integer     → an Act that resolves to an Integer
Result String   → Ok(String) or Err
Option Float    → Some(Float) or None
List Integer    → a list of Integers
```

these are type constructors: they take a type parameter and
produce a new type. `Act Integer` means "an Act whose resolved
value is an Integer."

the checker tracks type parameters through chains:
```moof
(do (x <- act)       ; act : Act Integer, so x : Integer
    [x + 1])         ; Integer + Integer → Integer
; result : Act Integer
```

### handler dispatch as pattern matching

moof's message send `[obj msg]` dispatches based on the
object's prototype — the handler comes from the proto chain.
this IS type-directed dispatch. the type determines which
implementation runs.

for exhaustive pattern matching:
```moof
(match opt
  (Some v) [v + 1]
  None     0)
```

the checker verifies exhaustiveness: all variants of the
type are covered. for Option, that means Some and None. for
Result, that means Ok and Err.

## the tension: open vs closed

haskell ADTs are CLOSED — `Maybe` is exactly `Nothing | Just a`.
you can't add a third variant. the compiler guarantees
exhaustive matching because it knows all variants.

moof prototypes are OPEN — anyone can create an object that
delegates to SomeProto. you can add new "variants" freely.
this means exhaustive matching can't be guaranteed — a new
variant might exist that the match doesn't cover.

### resolution: sealed prototypes

```moof
(deftype Option
  (sealed)
  (variant (Some value: a))
  (variant None))
```

`(sealed)` means no new objects can delegate to Option outside
this definition. the variants are fixed. the checker can
guarantee exhaustive matching.

unsealed types (the default) get an implicit wildcard case:
```moof
(match obj
  (Some v) [v + 1]
  None     0
  _        (err "unexpected variant"))
```

this is the pragmatic middle ground: sealed when you want
guarantees, open when you want extensibility.

## inference

### what can be inferred

```moof
(defn double (x) [x * 2])
; inferred: x : Numeric, return : Numeric
; because * requires Numeric conformance

(defn greet (name) (str "hello " name))
; inferred: name : (has describe), return : String
```

the checker walks the body and collects constraints:
- `[x * 2]` → x must respond to `*` → x : Numeric
- `(str ...)` → args must respond to `describe`

### what can't be inferred

```moof
(defn process (x) [x frobnicate])
; inferred: x : { [frobnicate] -> ? }
; the return type of frobnicate is unknown
```

when the checker can't determine the full type, it produces a
partial type with unknowns. annotations fill the gaps:

```moof
(defn process (x: Frobnicable) -> Integer
  [x frobnicate])
```

## gradual typing

the system is GRADUAL — unannotated code is dynamically typed.
annotated code is checked. the boundary is explicit.

```moof
; untyped — no checking, fully dynamic
(defn f (x) [x + 1])

; typed — checked at call sites
(defn g (x: Integer) -> Integer [x + 1])

; mixed — x is checked, y is dynamic
(defn h (x: Integer y) [x + y])
```

at the boundary between typed and untyped code, the checker
inserts implicit "trust" — it assumes the untyped value matches.
runtime errors catch mismatches.

## implementation sketch

### type representation

types are moof objects:

```moof
(def IntegerType { name: "Integer" kind: 'primitive })
(def OptionType {
  name: "Option"
  kind: 'adt
  params: (list 'a)
  variants: (list SomeType NoneType)
})
```

types are values. they're inspectable, queryable, composable.
the type checker is a moof program that manipulates type
objects. it's not a separate system — it's moof all the way
down.

### where checking happens

1. **at definition time** — `defn`, `defserver`, `defprotocol`
   with type annotations are checked when defined.
2. **at call sites** — arguments are checked against parameter
   types. return values are checked against declared return
   types.
3. **in do-notation** — the checker tracks type parameters
   through monadic chains.
4. **never at runtime** (for typed code) — types are erased
   after checking. the runtime is unchanged.

### integration with protocols

protocols become type constraints. conformance is checked
structurally: does the object have the required handlers with
compatible types?

```moof
(defprotocol Numeric
  (require (+: other: Self) -> Self)
  (require (*: other: Self) -> Self)
  (require (negate) -> Self))

; Integer conforms because it has +, *, negate with matching types
; the checker verifies this at conform time
```

## what this buys you

1. **catch errors early** — `[3 + "hello"]` is flagged at
   check time, not at runtime.
2. **documentation** — types ARE documentation. the agent
   reads them to understand APIs.
3. **IDE support** — autocomplete, hover types, go-to-definition
   all work because types describe the shape.
4. **protocol verification** — conformance is checked at
   definition time, not just at first use.
5. **exhaustive matching** — sealed types guarantee all cases
   are covered.
6. **refactoring safety** — change a type, see all the places
   that need updating.

## what this doesn't change

1. **runtime behavior** — types are erased. the VM is unchanged.
2. **prototype delegation** — still works. types describe the
   shape, not the implementation path.
3. **duck typing** — still works for untyped code. gradual.
4. **repl experience** — types are optional. the REPL is still
   dynamic and exploratory.
5. **existing code** — nothing breaks. types are additive.

## implementation priority

this is a phase 3+ feature. it requires:
- type representation as moof objects
- constraint inference engine
- checking at definition/call sites
- integration with defprotocol
- sealed variant support
- error reporting

the infrastructure (protocols, Chainable, structural matching)
already exists. the type system is a LAYER on top, not a
rewrite underneath.

## summary

1. **structural, not nominal** — types describe shapes, not names.
2. **row polymorphism** — `{ x: Float, ... }` matches open objects.
3. **protocols as typeclasses** — same mechanism, with type params.
4. **gradual** — annotate what you want checked, leave the rest.
5. **inference** — the checker infers types from handler usage.
6. **sealed types** — opt-in closed variants for exhaustive matching.
7. **types are values** — type objects are inspectable moof data.
8. **runtime unchanged** — types erase. the VM doesn't know about them.
