# protocols

**type:** concept

> moof's type system. a protocol is a contract declaring what
> handlers a value must respond to — and a set of handlers derived
> from those.

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

conforming types implement `fold:with:`. conformance is declared
once (`(conform Cons Iterable)`). conformers get all the provided
methods — because the provided methods are implemented in terms of
the required ones.

this is how moof has a rich collection algebra with a minimal type
surface. one required method per protocol; tens of derived methods.

---

## the syntax

protocols are declared with `defprotocol`:

```moof
(defprotocol Iterable
  "Collections. Implement fold:with: to get map, filter, each,
   sort, group, ... and ~40 others."

  (require (fold:with: init f)
    "Reduce: thread acc through every element via f.
     All other collection operations derive from it.")

  (provide (each: f)
    [self fold: nil with: |_ x| (f x)])

  (provide (map: f)
    [[self fold: nil with: |acc x| (cons (f x) acc)] reverse])

  ; ...
)
```

- **require** — a handler every conformer must implement. the
  docstring explains the contract.
- **provide** — a handler with a default implementation in terms of
  the required ones. conformers get this for free but can override
  it (usually for performance).

types conform explicitly:

```moof
(conform Cons Iterable)
(conform Bag Iterable)
(conform Set Iterable)
```

when you `(conform T P)`, the moof runtime checks that `T` has the
required handlers, then installs the provided handlers on `T`'s
prototype. after conformance, any instance of `T` responds to every
method Iterable provides.

---

## the ten protocols moof commits to

per [the stdlib doctrine](../laws/stdlib-doctrine.md), moof's stdlib
has ten protocols. each is minimal, each has ≥3 conformers, each
exists because the pattern recurs enough to justify abstraction.

| protocol | required | role |
|----------|----------|------|
| `Showable` | `show` | rendered form for humans |
| `Equatable` | `equal:` | value equality |
| `Hashable` | `hash` | stable Integer for keys |
| `Comparable` | `<` | total order |
| `Numeric` | `+`, `-`, `*`, `=`, `<` | arithmetic |
| `Iterable` | `fold:with:` | walkable sequence |
| `Indexable` | `at:`, `count` | random-access |
| `Callable` | `call:` | invokable |
| `Monadic` | `then:`, `pure:` | bind + unit |
| `Fallible` | `ok?` | can be failed |

each has its own file in `lib/data/` or `lib/kernel/` with the
defprotocol form and derived provides.

additional protocols may exist, but adding one requires 3+ concrete
conformers at declaration. no speculative protocols. no protocols
declared for one type to implement.

---

## structural and nominal

protocol conformance in moof is **primarily nominal** (you declare
it with `conform`) but also **structurally queryable** (at runtime
you can ask "does this value respond to what Iterable requires?").

- `(conform T P)` — nominal: adds T to P's known conformers,
  installs provides on T.
- `[val responds-to: 'fold:with:]` — structural: would a send work?
- `[val is: Iterable]` — nominal check: has T been declared to
  conform?

both are useful. nominal is the default; structural is for
meta-programming and proxies.

---

## protocol composition

protocols can compose. a richer protocol may REQUIRE conformance to
a simpler one:

```moof
(defprotocol Indexable
  "Random-access sequence."
  (require (at: i))
  (require (count))
  (provide (first) [self at: 0])
  ; ...
  ; as a side effect of having at: + count, Indexable auto-installs
  ; Iterable by providing fold:with: in terms of them.
  (provide (fold:with: init f)
    [(range 0 [self count]) fold: init with: |acc i|
       (f acc [self at: i])]))

(conform String Indexable)
(conform Table Indexable)
```

`String` doesn't directly implement `fold:with:`; it inherits it
from Indexable's provide. that's enough to make String `Iterable`
too.

---

## why protocols, not classes

classical OO pins behavior to class hierarchies. protocols let
unrelated types share behavior by implementing the same contract.
a Bag and a Cons have nothing in common structurally, but both
conform to Iterable — so both respond to `map:`, `filter:`,
`count`, etc.

this is haskell's typeclasses applied to a dynamic object model:
- types compose freely (a type can conform to many protocols).
- no diamond problem (there's no inherited field structure, only
  behavior).
- extension is ambient (conform an existing type to a new protocol
  at any time).
- dispatch is by receiver (one type, one handler, no ambiguity).

---

## protocols vs prototypes

a prototype is an object you delegate to for behavior. a protocol
is a contract that can apply to many prototypes.

- `Cons` (the prototype): every cons cell delegates to it.
  handlers on Cons become methods on cons cells.
- `Iterable` (the protocol): contracts that `Cons` implements. no
  cons cell delegates to Iterable directly.

protocols INSTALL handlers on prototypes during conformance. but
the delegation chain goes through prototypes, not protocols. this
keeps dispatch fast and the proto chain short.

---

## `defmethod` vs `conform`

- `(defmethod Cons each: (block) ...)` — install a handler on the
  Cons prototype. affects every Cons.
- `(conform Cons Iterable)` — declare that Cons satisfies
  Iterable's contract; install all provides on Cons.

use `defmethod` to add behavior to a single type. use `conform` to
acquire a whole bundle of behavior because you implement the
required handlers. the two are complementary.

---

## open for extension

protocols are open: anyone can define a new one. and any existing
type can be retrofitted to conform to a new protocol via
`conform`. there's no "closed world" — you can teach existing types
new behavior by publishing a protocol and conformances.

this means third-party code can say: "here's a protocol for
Renderable. here's Cons conforming to it. here's Table conforming."
— and your existing conses and tables can now render without you
changing anything.

this is more powerful than haskell's orphan-instance rule (moof
has no such rule) and more restrained than ruby's monkey patching
(because the conformance is explicit and introspectable).

---

## anti-patterns we reject

- **protocols with <3 conformers.** if the pattern only shows up
  once, don't abstract it. write a plain defmethod.
- **"marker" protocols.** no `Lockable` protocol that declares
  zero methods "for typing." use a slot if you need a tag.
- **protocols duplicating other protocols' work.** if Iterable
  already provides `map:`, don't define a new protocol with a
  subtly different `map:`. extend Iterable.
- **inheritance-style protocol hierarchies with many layers.**
  most protocols should be flat. Indexable refines Iterable —
  that's the extent of hierarchy we want.

see [the stdlib doctrine](../laws/stdlib-doctrine.md) for the full
rulebook.

---

## what you need to know

- protocols declare minimum contracts (`require`) and derived
  behavior (`provide`).
- conforming types implement the required handlers; they get
  provides installed automatically.
- moof has ten canonical protocols; new ones need ≥3 conformers.
- conformance is nominal + structurally queryable.
- protocols compose by requiring conformance to simpler protocols.
- this is moof's type system.

---

## next

- [schemas.md](schemas.md) — the sibling contract system. protocols
  constrain HANDLERS; schemas constrain SLOTS. different layer,
  same philosophy.
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) — the
  rulebook for which protocols exist and why
- [effects.md](effects.md) — Monadic, Fallible, Awaitable — the
  protocols that describe effects
- [objects.md](objects.md) — the material protocols apply to
