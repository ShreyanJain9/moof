# protocols

**type:** concept
**specializes:** throughline 2 (constraints — handler flavor)

> a protocol is a **constraint on a value's handlers**:
> a declarative claim that the value responds to a minimum set
> of messages, and a bundle of derived behavior you get for
> free. the runtime checks the claim at construction (via
> `conform`) and at dispatch (any `responds-to:` query). same
> pattern as schemas (constraints on slots), optional types
> (constraints checked early), capabilities (constraints on
> reachability) — different phase of check.

---

## the one idea

a protocol says: **"if you implement these required handlers,
you get these derived handlers for free."**

```
Iterable:
  requires: fold:with:
  provides: map:, select:, reject:, each:, count, sum, first,
            last, reduce:, toList, toTable, take:, drop:, sort,
            group:, zip:, any:, all:, min, max, ...and more
```

this is a **constraint**: it declares what a value must respond
to, and in exchange extends what it CAN respond to. types that
satisfy the minimum contract get the derivation automatically.

if you've read [throughlines.md](../throughlines.md), this is
throughline 2: a declarative claim about a value. protocols are
the **handler flavor**. schemas (slots), optional static types
(compile-time), capabilities (reachability) are the other
flavors. same concept, different phase.

---

## why constraints-on-handlers

messages are the one operation in moof. a protocol says "this
value can participate in certain message conversations." that's
the handler flavor of "declarative claim."

contrast with:

- **schemas** (see [schemas.md](schemas.md)) — claims about the
  value's slots (shape, types). "Recipe has a `title: String`."
- **optional types** (future) — claims checked at compile time.
  "this function takes an Iterable<Integer>."
- **capabilities** — claims about reachability. "you hold this
  reference, you can send it messages."

a single type can have claims at multiple phases. Recipe might
conform to Showable (protocol), have a Shape (schema), and
check against a static annotation (future types). these don't
conflict; each checks a different axis.

---

## the syntax

```moof
(defprotocol Iterable
  "Collections. Implement fold:with: to get map, filter, each,
   sort, group, ... and ~40 others."

  (require (fold:with: init f)
    "Reduce: thread acc through every element via f.")

  (provide (each: f)
    [self fold: nil with: |_ x| (f x)])

  (provide (map: f)
    [[self fold: nil with: |acc x| (cons (f x) acc)] reverse])
  ; ...
)
```

- **require** — a handler every conformer must implement
- **provide** — a handler with a default implementation in terms
  of the required ones

types conform explicitly:

```moof
(conform Cons Iterable)
(conform Bag Iterable)
(conform Set Iterable)
```

`conform` checks that the required handlers exist, then installs
the provided ones on the target prototype. conformance is an
**additive** action — throughline 4 at work. you ADD a conformance
to a type; the type's old behavior stays, and new methods come
into scope.

---

## the ten protocols moof commits to

per [the stdlib doctrine](../laws/stdlib-doctrine.md):

| protocol | required | role |
|----------|----------|------|
| `Showable` | `show` | rendered form for humans |
| `Equatable` | `equal:` | value equality |
| `Hashable` | `hash` | stable Integer for keys |
| `Comparable` | `<` | total order |
| `Numeric` | `+`, `-`, `*`, `=`, `<` | arithmetic |
| `Iterable` | `fold:with:` | walkable sequence |
| `Indexable` | `at:`, `count` | random-access (refines Iterable) |
| `Callable` | `call:` | invokable |
| `Monadic` | `then:`, `pure:` | bind + unit |
| `Fallible` | `ok?` | can be failed |

each has ≥3 conformers. each is minimal. each exists because
the pattern recurs enough to justify abstraction. new protocols
require 3+ conformers at declaration.

---

## nominal AND structural

protocol conformance is **nominal by default** (you declare it
with `conform`) and **structurally queryable** (at runtime ask
"does this value respond to Iterable's required methods?").

- `(conform T P)` — nominal: adds T to P's conformers, installs
  provides on T.
- `[val responds-to: 'fold:with:]` — structural: would a send
  work?
- `[val is: Iterable]` — nominal check: has T been declared to
  conform?

both are useful. nominal is the default; structural is for
meta-programming and proxies.

this is the distinction haskell makes between "you wrote the
instance" and "we inferred the constraint." moof exposes both
facets as runtime queries.

---

## protocol composition

a richer protocol can REQUIRE conformance to a simpler one:

```moof
(defprotocol Indexable
  "Random-access sequence."
  (require (at: i))
  (require (count))
  (provide (first) [self at: 0])
  ; ...
  ; as a side effect of having at: + count, Indexable auto-
  ; installs Iterable by providing fold:with: in terms of them.
  (provide (fold:with: init f)
    [(range 0 [self count]) fold: init with: |acc i|
       (f acc [self at: i])]))

(conform String Indexable)
(conform Table Indexable)
```

`String` doesn't directly implement `fold:with:`; it inherits it
from Indexable's provide. that's enough to make String
`Iterable` too.

keep this shallow. one level of refinement (Indexable refines
Iterable) is the limit we intend. deeper hierarchies make
dispatch harder to follow.

---

## the protocol objects are first-class

because protocols are constraints (throughline 2), they're
first-class moof values. you can:

- pass `Iterable` as an argument
- store it in a slot
- compose constraints: `(all-of: Iterable Comparable)` (future)
- query at runtime: `[val conforms-to?: Iterable]`

a future optional-type layer (see [horizons.md](../vision/horizons.md))
uses the same protocol values as type annotations. no parallel
universe of types-vs-protocols: same values, different phase of
check.

---

## protocols and prototypes

a prototype is an object you delegate to for behavior. a
protocol is a constraint that can apply to many prototypes.

- `Cons` (prototype): every cons cell delegates to it. handlers
  on Cons become methods on cons cells.
- `Iterable` (protocol): constraint Cons implements. no cons
  cell delegates to Iterable directly.

protocols INSTALL handlers on prototypes during conformance. the
delegation chain goes through prototypes, not protocols. this
keeps dispatch fast and the proto chain short.

---

## `defmethod` vs `conform`

- `(defmethod Cons each: (block) ...)` — install one handler on
  the Cons prototype. affects every Cons.
- `(conform Cons Iterable)` — declare that Cons satisfies
  Iterable's contract; install all provides on Cons.

use `defmethod` to add one behavior. use `conform` to adopt a
bundle.

both are additive gestures (throughline 4) — they ADD to a
prototype; they don't replace.

---

## open for extension

protocols are open: anyone can define a new one. any existing
type can be retrofitted to conform via `conform`. there's no
"closed world" — you can teach existing types new behavior by
publishing a protocol and conformances.

```moof
; third-party library:
(defprotocol Renderable ...)
(conform Cons Renderable)
(conform Table Renderable)
; your existing conses and tables now render, no edit to them.
```

more powerful than haskell's orphan-instance rule (moof has no
such rule). more restrained than ruby monkey patching (because
conformance is explicit, introspectable, and can be reverted).

---

## anti-patterns

see [the stdlib doctrine](../laws/stdlib-doctrine.md) for the
full rulebook. highlights:

- **protocols with <3 conformers.** if the pattern only shows up
  once, don't abstract. write a plain defmethod.
- **"marker" protocols.** no `Lockable` protocol with zero
  methods "for typing." use a slot if you need a tag.
- **duplicate work.** if Iterable already provides `map:`, don't
  define a new protocol with a subtly different `map:`. extend
  Iterable.
- **deep hierarchies.** keep protocol hierarchies shallow.
  Indexable refines Iterable — that's the extent we want.

---

## what you need to know

- a protocol is a constraint on handlers (throughline 2).
- required methods + derived methods; conformers implement the
  first, get the second free.
- ten canonical protocols; more must justify themselves with 3+
  conformers.
- conformance is nominal + structurally queryable.
- protocols are first-class moof values — future static types
  use the same values.
- schemas ([schemas.md](schemas.md)) are the sibling constraint
  system for slots. both/and.

---

## next

- [../throughlines.md](../throughlines.md) — the constraints
  pattern this specializes
- [schemas.md](schemas.md) — sibling constraint system (slots
  instead of handlers)
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) — the
  rulebook for protocol addition
- [effects.md](effects.md) — Monadic / Fallible / Awaitable — the
  protocols that describe effectful contexts
- [objects.md](objects.md) — the material protocols apply to
