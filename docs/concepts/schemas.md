# schemas

**type:** concept

> moof supports both schema-emergent and schema-first workflows.
> you can build objects ad-hoc and let the shapes coalesce, or
> declare an explicit shape contract up front. neither mode is
> "right" — each fits different stages of thought.

---

## two modes, one substrate

**schema-emergent** (the default):

```moof
(def r1 { Recipe title: "pasta"  ingredients: (list "flour" "egg") })
(def r2 { Recipe title: "salad"  ingredients: (list "lettuce") steps: 2 })
```

`r1` and `r2` both point at the Recipe prototype but have
different slots. nothing validates; everything works; the "Recipe
shape" is whatever your recipes happen to look like.

**schema-first** (opt-in, rigorous):

```moof
(defshape Recipe
  (requires
    (title        String)
    (ingredients  (List String))
    (steps        Integer optional)))

(def r1 { Recipe title: "pasta"  ingredients: (list "flour" "egg") })
; → validated at construction; missing/mistyped slots error.
```

`Recipe` here is a **Shape**: a declared contract about slot
presence and types. construction against it is validated;
queries can use it for planning; documentation can render it.

both are legitimate. neither is forced.

---

## when to prefer which

**emergent** wins when:
- you're exploring — the shape of what you're building is still
  coalescing.
- the data is genuinely irregular (notes, scratchpads, arbitrary
  user objects).
- you want the low floor: someone who doesn't know the domain can
  still contribute.

**explicit** wins when:
- you're stabilizing — a shape has emerged, others will rely on
  it, drift is now a bug.
- the data has contract-level requirements (a user record must
  have an email).
- queries + tooling benefit from shape awareness (autocomplete,
  statics, index planning).
- serialization needs canonical ordering (schemas pin slot order
  for hashing).

the promotion path is natural: build emergent, extract a Shape
when ready, run validation retroactively.

---

## Shape is a prototype

a Shape is itself a moof object — a prototype with specific
required-slot declarations. defining one:

```moof
(defshape Recipe
  (requires
    (title        String)
    (ingredients  (List String))
    (steps        Integer optional)))
```

produces a `Recipe` prototype that:
- has a `[call: args]` handler that validates before constructing
  an instance.
- has a `[validate: obj]` handler that checks existing objects.
- has a `[slots]` handler returning the required slot specs.
- conforms to Showable (renders the schema as documentation).

---

## type annotations on slots

the second element of each require clause is a **type tag**: a
reference to a prototype the slot's value must conform to.

supported type tags (future-proof, not all implemented in the
first cut):

```
String                   ; slot must be a String
Integer                  ; must be an Integer (i48 or BigInt)
(List T)                 ; Cons where every element is T
(Option T)               ; Some<T> or None
(Either L R)             ; Ok<R> or Err<L>
(Conform P)              ; any value that conforms to protocol P
(ExactShape { ... })     ; a nested shape
Any                      ; anything (escape hatch)
```

type tags are MOOF VALUES — they're constructed and passed like
any other. `(List String)` is literally `[List call: String]` —
returns a Shape-for-list-of-string.

---

## validation

validation is explicit, not automatic. three modes:

### at construction (shapes-as-constructor)

when Shape has a `[call: args]` handler, constructing an instance
via `(Recipe title: "x" ...)` runs validation. on mismatch, you
get an Err.

### on demand

`[Shape validate: obj]` returns Ok if the object satisfies the
schema, Err with a list of problems otherwise.

### at the boundary

you can install Shape-based validation on server handlers that
receive messages, to reject malformed input at the vat edge.

validation is NEVER retroactive-and-ambient — an existing object
doesn't start failing because you added a Shape later. moof
refuses that kind of action-at-a-distance.

---

## migration

when you evolve a Shape (add a required slot, tighten a type),
existing objects that were valid under the old Shape might not be
valid under the new one. moof handles this with migrators —
user-written transformations from old shape to new:

```moof
(defmigrator Recipe v1-to-v2
  "v2 adds an author field; default to anonymous for old recipes."
  (fn (old) [old with: { author: "anonymous" }]))
```

when loading an image, moof checks each object's Shape version;
mismatches invoke the migrator; absence of a migrator fails
loading loudly (no silent corruption, same as foreign-type
migration).

---

## schemas vs protocols

these are different layers:

- **protocol** — contract about HANDLERS (methods). "this
  value responds to map:, filter:, reduce:." dispatch-level.
- **schema** — contract about SLOTS (data). "this value has a
  title slot that's a String." data-level.

a type can have both. `Recipe` conforms to `Showable` (protocol)
AND has a Shape (schema). these describe different things; they
don't conflict.

protocols are part of the type system (see
[protocols.md](protocols.md)). schemas are a data-shape
contract that sits orthogonally. most stdlib types have
protocols; schemas are for user-declared data.

---

## what schemas give you for free

once a Shape exists:

- **auto-generated form UI.** the canvas can render a Shape as an
  input form. "make a Recipe" → a form with title, ingredients,
  optional steps.
- **query autocomplete.** `[recipes where: |r| r.???]` — the
  editor knows Recipe's slots.
- **index planning.** a Shape tells the index server which fields
  to cover.
- **serialization ordering.** canonical bytes can follow the
  schema's slot order, not per-instance order.
- **documentation.** `[Recipe describe]` renders the slot list.

all of this is opt-in. emergent-mode objects don't get it; they
don't need it.

---

## what we explicitly avoid

- **whole-program static typing.** moof doesn't block you from
  using emergent objects. a Shape is a local, declared contract.
  you don't have to type everything.
- **runtime boxing gymnastics.** the type tag is a moof value;
  checking is a message send. no stratified runtime.
- **schema inheritance complexity.** schemas compose by
  reference (one slot can be `(Conform SomeProtocol)` or
  `(ExactShape ...)`), but no diamond-style multiple inheritance.
  keep schemas flat.

---

## what you need to know

- moof supports both ad-hoc emergent objects AND explicit Shapes.
- schemas are about SLOTS (data). protocols are about HANDLERS
  (behavior). they coexist.
- a Shape is a moof prototype with validation handlers.
- validation is explicit: at construction, on demand, or at the
  boundary.
- migrators handle shape evolution.
- canvas, queries, and indexing benefit from Shapes without
  requiring them.

---

## status

today: moof does NOT ship a `defshape` form. this document
describes the design. the current stdlib has only protocols.
`defshape` is on the roadmap (see
[../roadmap.md](../roadmap.md)'s beyond section). this doc
pins down what it should be.

until then, you can approximate with:
- protocols that require specific slot-accessor handlers
- user-written `validate:` handlers on a prototype
- discipline

---

## next

- [protocols.md](protocols.md) — the sibling contract system
  (for handlers).
- [objects.md](objects.md) — the material schemas describe.
- [../roadmap.md](../roadmap.md) — when `defshape` lands.
