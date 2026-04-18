# prototype-based types: the long view

prototypes ARE types in moof. this isn't a design choice — it's
a consequence of the object model. a value's "type" is the
prototype chain it delegates through. we just need to make this
visible, queryable, and trustworthy enough to build everything
on top of.

## the current state

today `[obj type]` returns the immediate parent prototype, which
is often too generic. `(Some 42)` has type `Object` — the Some
prototype's parent. there's no reliable way to ask "is this a
Some?" without walking the chain manually.

`[obj is: Proto]` exists in types.moof — it walks the chain —
but it's not yet the primary idiom. pattern matching can't
identify constructors because it can't reliably ask "what IS
this value?"

## the long-term model

### every prototype is a type

a prototype is a first-class object that other objects delegate
to. its identity (as a value) is its identity as a type. no
separate type registry, no nominal class declarations.

```moof
(def Point { x: 0  y: 0  [distance: other] ... })
(def p { Point x: 3  y: 4 })
[p is: Point]    → true
[p prototype]    → Point   ; direct parent
[p prototypes]   → (Point Object)   ; full chain
```

### the core type operations

- **`[obj prototype]`** — direct parent prototype
- **`[obj prototypes]`** — full delegation chain, nearest first
- **`[obj is: Proto]`** — is Proto anywhere in obj's chain?
- **`[obj typeName]`** — the name of obj's nearest named prototype
- **`[obj conforms: Protocol]`** — structural conformance check
- **`[obj shape]`** — slot names + handler names as a record

these work on every value: primitives, heap objects, closures,
FarRefs, Acts. primitives have their type-proto (Integer, Float,
etc.) as their prototype.

### names live on prototypes, not objects

any prototype can declare a name:

```moof
(def Point { __name: 'Point  x: 0  y: 0 })
```

instances don't carry the name — they inherit it. `[instance
typeName]` walks the chain to find the nearest `__name` slot.
this is cheap because the chain is short and cached.

anonymous prototypes (object literals without names) have no
`typeName` — they're structurally typed only.

### nominal + structural = flexible

moof should support BOTH:

**nominal:** "is this a Point?"
```moof
[p is: Point]                 ; walks prototype chain
```

**structural:** "does this have x, y, distance:?"
```moof
[p hasShape: { x: _  y: _  [distance:] }]
[p conforms: HasPosition]     ; a Protocol defining the shape
```

nominal is faster and more precise. structural is more flexible
and works with anonymous objects. both are first-class.

### constructors are prototypes

`Some` is both a prototype (parent of Some instances) AND a
constructor function. making a prototype callable via `call:` lets
this be one thing:

```moof
(def Some {
  __name: 'Some
  value: nil
  [call: v] { self value: v }    ; calling Some makes an instance
  [then: f] (f @value)
  ...
})

(Some 42)    ; → { Some value: 42 }
[(Some 42) is: Some]    ; → true
```

no separate `SomeProto` / `Some constructor` distinction. the
prototype is the type is the constructor.

### pattern matching falls out

with reliable `is:`, constructor patterns work:

```moof
(match val
  (Some x)   [x + 1]        ; matches if [val is: Some], bind x = @value
  (Err msg)  msg            ; matches if [val is: Err], bind msg = @message
  _          default)
```

the match machinery asks each value "which prototype do you
delegate to, and do you have these slots?" every type answers
the same way. no special cases.

### protocols are types too

a Protocol is a prototype with:
- required slot/handler signatures
- provided default implementations
- a conformance test

`[obj conforms: MyProtocol]` runs the test. `(defprotocol ...)`
creates the Protocol prototype. `(conform Type MyProtocol)`
installs the provided defaults on Type.

```moof
[5 conforms: Numeric]        ; → true
[5 conforms: Comparable]     ; → true
[(Some 5) conforms: Chainable] ; → true
```

### type annotations as metadata

gradual types come later, as optional annotations on slots and
handlers. the checker walks code and verifies constraints. types
NEVER affect runtime behavior — they're pure metadata, read by
the checker, the agent, the inspector.

```moof
(defserver Counter
  (slot count Integer)
  (handler [increment] -> Integer)
  ...)
```

the agent reads annotations to understand the server's API. the
inspector uses them to render better. the checker uses them to
flag mismatches. the runtime ignores them.

### type queries

because prototypes are values, you can query them like anything
else:

```moof
[Integer handlerNames]           ; what can Integer do?
[Integer conforms: Numeric]      ; does Integer implement Numeric?
[Numeric conformers]             ; which types implement Numeric?
[Some doc]                       ; Some's docstring
[Some slotNames]                 ; Some's slot contract
[(instances-of Counter)]         ; find all live Counter servers
```

the type system is queryable because it's JUST OBJECTS. no
separate introspection API — the same messages that work on
values work on types.

### hot type updates

since prototypes are live objects, you can modify them. adding
a handler to `Some` updates all existing Some instances
immediately (they delegate to Some at dispatch time):

```moof
[Some handle: 'debug with: || "some value"]
[(Some 42) debug]    ; → "some value" — instance got the new method
```

this is dangerous but powerful. a live typing system. for
safety, sealed prototypes disallow post-creation modifications.

### sealed vs open

by default, prototypes are OPEN — anyone can create a new object
that delegates to them. for ADTs where exhaustive matching
matters, `sealed` marks a prototype as closed:

```moof
(def Option { __sealed: true  __variants: (list Some None) })
```

the match checker can verify exhaustiveness for sealed types. open
types get an implicit wildcard requirement.

### generic types

type parameters come via shape descriptions, not special syntax.
`List[Integer]` is a List whose elements satisfy Integer's shape.
the checker tracks this through chains:

```moof
(: sum (-> (List Integer) Integer))
(defn sum (xs) [xs fold: 0 with: |a x| [a + x]])
```

at runtime, the types erase. at check time, the list's element
type flows through `fold:` and back.

## implementation path

these land incrementally:

1. **`__name` slot convention** — every major prototype gets one.
   `[obj typeName]` walks the chain looking for it. small change.

2. **`[obj is: Proto]` universally available** — already exists on
   Object, ensure it works reliably across all value types.

3. **`[obj prototypes]` returns the full chain** — helper.

4. **Pattern matching via dispatch** — `[pat match: val]` sends
   to the pattern's prototype. types define their own match
   behavior.

5. **Constructor prototypes** — prototypes can be called to
   create instances. `call:` handler on the prototype.

6. **Protocol conformance registry** — every protocol maintains
   a list of conforming types. queryable: "who implements
   Iterable?"

7. **Sealed prototypes** — opt-in closedness for exhaustive
   matching.

8. **Type annotation syntax** — `(: name Type)` and
   `(handler [sel] -> Result)` as metadata. checker reads them.

9. **Gradual type checker** — walks code, collects constraints,
   reports violations as warnings (not errors) at first.

10. **Shape queries** — `[obj shape]` returns a structured
    description for inspectors and agents.

## why this works for moof specifically

a language with static types and dynamic dispatch is haskell.
a language with nominal types and subclassing is java. a language
with structural types and classes is TypeScript. a language with
prototype-based inheritance is JavaScript.

moof is prototype-based like JavaScript, but with protocols
(like haskell typeclasses) and structural row types (like
TypeScript). the prototype IS the type. the protocol IS the
typeclass. conformance is computed, not declared.

this is coherent. it's also natively introspectable — every
question about "what is this?" has an answer in terms of
messages you can send. the inspector, the agent, the checker,
the canvas all speak the same language about types: message
sends to prototypes.

## what this buys you

1. **no separate type system** — types are objects you
   manipulate like everything else.
2. **live types** — modify a prototype, all instances update.
3. **queryable types** — find conformers, inspect shapes, get
   docs.
4. **flexible matching** — nominal when you want precision,
   structural when you want flexibility.
5. **agent-readable** — the LLM can ask "what protocols does
   this conform to?" and get a real answer.
6. **gradual** — annotate what you want checked, leave the
   rest dynamic.
7. **content-addressable** — a prototype's hash IS its identity
   for caching, distribution, versioning.

## the guardrails

1. **`__name` is convention, not magic** — it's a regular slot.
   prototypes without it have no nominal identity.

2. **prototype identity is Value identity** — two syntactically
   identical `{ x: 1 }` literals are DIFFERENT prototypes. this
   matches JavaScript semantics. content-addressing can unify
   them later.

3. **checking is opt-in** — the runtime doesn't enforce types.
   checked code is safer. unchecked code still works.

4. **sealed means sealed** — once declared sealed, you can't add
   variants. this guarantee is what makes exhaustive matching
   reliable.

5. **annotations are metadata** — they never change runtime
   behavior. you can always ignore them.

## summary

moof's type system isn't a layer on top of the language — it's
the language's own reflective capabilities used seriously. every
value knows what it is (via prototype), can be asked what it can
do (via handlers), and can be pattern-matched against shapes
(via structural inspection). the stdlib, the checker, the
agent, and the inspector all speak this same protocol.

types are values. values are objects. objects are prototypes
with state. the prototype IS the type. we just need to make
this rigorous and delightful.
