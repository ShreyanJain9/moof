# objects

**type:** concept
**role:** the material every throughline operates on

> everything in moof is an object. there is one semantic type —
> Object. this document says what that means.
>
> objects are the material the [throughlines](../throughlines.md)
> act on. contexts (1) wrap values that are objects. constraints
> (2) describe objects. walks (3) traverse object graphs.
> additive authoring (4) adds to objects. canonical form (5) is
> their byte representation. understanding objects is
> understanding the substrate.

---

## the shape of an object

every object has three things:

1. **a prototype** — another object it delegates to when a handler
   isn't found locally. the VM follows this chain during dispatch.
2. **slots** — public data. fixed-shape at creation. each slot is
   a name (symbol) paired with a value. slots are how you read
   and write data.
3. **handlers** — open behavior. each handler is a selector
   (symbol) paired with a function or native reference. handlers
   respond to messages.

that's the whole object. no classes. no methods-vs-functions. no
hidden metadata. an object is prototype + slots + handlers.

```
{
  Point             ← prototype reference (usually a type)
  x: 3              ← slot
  y: 4              ← slot
  [distanceTo: p]   ← handler
    (sqrt (+ (sq (- @x [p x])) (sq (- @y [p y]))))
}
```

## slots vs handlers

the split matters:

- **slots are data.** public, directly readable as `obj.name`.
  fixed at creation time — you can't add a slot after construction.
  slot names and positions are part of the object's identity.
- **handlers are behavior.** public, reachable via message send
  `[obj name: arg]`. open — you can add or replace them on any
  prototype anytime.

why the split? slots are fast and serializable; handlers are late-
bound and extensible. separating them lets us serialize an object
without serializing its behavior (the behavior lives on the
prototype) and lets us extend behavior without growing data.

reading a slot:
```
obj.x            ; sugar for [obj slotAt: 'x]
```

sending a message:
```
[obj foo: 3]     ; find 'foo:' handler, invoke with receiver=obj, args=(3)
```

if a handler is missing, it's not a crash — it's a message:
`[obj doesNotUnderstand: 'foo: args]`. this enables proxies,
auto-generation, dynamic DSLs. (see [messages.md](messages.md).)

---

## prototypes, not classes

moof has no classes. objects delegate to other objects through their
prototype slot. the prototype is itself an object — it has its own
prototype, which has its own, and so on until we reach `Object`
(whose prototype is nil).

```
a point instance
  ↓ .proto
Point              ← the type-prototype
  ↓ .proto
Object             ← the root prototype
  ↓ .proto
nil
```

message dispatch walks this chain:

1. look for handler on the instance itself.
2. if not found, look on its prototype.
3. continue up until found or until proto is nil.
4. if nil reached, send `doesNotUnderstand:`.

this is all there is to "inheritance." prototypes are values, so
you can:

- use one prototype as another's prototype (single inheritance)
- compose prototypes dynamically at runtime
- clone a prototype and modify it (a common pattern for variants)
- introspect the proto chain from moof code

there's no class/metaclass distinction. "the class of X" is just
"X's prototype." a Point instance's prototype is the Point object.
Point's prototype is Object. Object's prototype is nil. no tower.

---

## how objects are made

three ways, all building on the same primitive.

### object literal

```
{ Parent x: 3 y: 4 [foo] (+ @x @y) }
```

creates a new object with:
- prototype = `Parent`
- slots = `(x: 3, y: 4)`
- handlers = `([foo] → (+ @x @y))`

this is the primitive. everything else sugar.

### with:

`[existing with: { x: 99 }]` returns a new object like `existing`
but with `x` replaced. non-destructive. structural sharing: the
other slots and handlers aren't copied, they're referenced.

### type constructors

a type-prototype usually defines a `call:` handler that constructs
instances:

```
{ Point [call: args] { Point x: [args car] y: [[args cdr] car] } }
```

then `(Point 3 4)` → `[Point call: '(3 4)]` → a Point instance.
this is how `(list 1 2 3)`, `(Integer 5)`, `(Url "moof:/...")`
work. they're all just `call:` on a prototype.

---

## object identity

objects have identity by reference — two objects are the same object
only if they're literally the same heap cell. `[a identical: b]`
tests this. `[a equal: b]` tests VALUE equality (slot-by-slot, or
via a custom `equal:` handler).

**immutable objects** (Integer, String, Bytes, Cons, Table, Symbol,
BigInt) have structural identity: two Integers with the same value
are `equal:`, and if the VM can prove it, they're `identical:` too
(it often can — small Integers are canonical).

**mutable objects** have reference identity: two distinct vats can't
have the "same" mutable object. they can have FarRefs to the same
vat-hosted server, which is the cross-vat analogue.

---

## slots are sealed at creation

you cannot add a slot to an object after it's constructed.
`[obj slotAt: 'newThing put: 42]` doesn't exist (intentionally —
the compiler rejects it). this is on purpose:

- **serializability.** objects with fixed shape canonicalize
  cleanly. content-addressing requires stable byte layouts.
- **optimization.** a Cons cell is `{ Cons car: ... cdr: ... }`
  at the object level, but the VM stores it as two Values
  side-by-side. fixed shape enables that.
- **readability.** you can look at an object literal and know
  what slots it has. no hidden state.

to "mutate" a slot, you produce a new object:
```
(def p { Point x: 1 y: 2 })
(def p2 [p with: { x: 99 }])
; p is unchanged; p2 is a new object with x: 99
```

this is how all in-vat state works. mutation-in-place only happens
inside server vats, mediated by Update (see
[effects.md](effects.md)).

---

## handlers are open

handlers are the opposite of slots: you can add or replace handlers
on any prototype at any time. add one to `Integer` and every integer
gains that behavior. add one to `Object` and every object in the
system can respond to it.

```
(defmethod Integer times: (block)
  (if [self <= 0] nil
    (do (block self) [[self - 1] times: block])))
```

`defmethod` is sugar. it installs a handler on a prototype. after
this, every Integer responds to `times:`.

this is "open classes" (smalltalk / ruby) through prototype
delegation. it makes moof extensible at every level; it also means
changes to a prototype affect every instance. be careful what you
edit.

---

## optimized representations

the VM has optimized storage for common objects:

| object | how stored |
|--------|------------|
| Integer (i48) | tagged NaN-box, fits in 8 bytes |
| Float | same |
| Boolean, nil, symbols | tagged immediate |
| Integer (BigInt) | heap object with a `num_bigint` payload |
| Cons | heap object with two Values (car, cdr) |
| Text | heap object with a Rust String payload |
| Bytes | heap object with a Vec<u8> |
| Table | heap object with seq + map payloads |
| general objects | heap object with slot + handler vectors |

semantically these are all Objects. the Cons cell responds to
`[pair car]` via a handler on the Cons prototype, which reads the
optimized storage. `[5 + 4]` is a message to the Integer proto,
which dispatches to a native that does the math.

the optimization is invisible at the moof level. you can always
`slotNames`, `handlerNames`, `describe`, `proto` any object
uniformly.

---

## what "everything is an object" buys us

- uniform introspection: any value tells you its type, slots,
  handlers, parent.
- uniform serialization: any value has a canonical byte form based
  on its shape.
- uniform inspection UI: one inspector works on all values because
  they all have the same shape.
- uniform remote access: sending an object across a vat boundary
  is the same operation regardless of what the object is.
- uniform extension: adding behavior to anything is the same
  gesture.

the uniformity is load-bearing. it's what lets moof have one
inspector, one serializer, one message-send operation — and thereby
one coherent surface.

---

## what you need to know, summarized

- one semantic type: Object.
- objects have prototype, slots, handlers.
- slots are fixed-shape data; handlers are open behavior.
- dispatch walks the prototype chain.
- no classes. `Integer` is an object whose instances use it as
  their prototype.
- mutation happens only through server Updates.
- the VM optimizes common shapes; the semantics are uniform.

---

## next

- [../throughlines.md](../throughlines.md) — the five patterns
  operating on objects
- [messages.md](messages.md) — how sending works, dispatch, DNU
- [protocols.md](protocols.md) — constraints on what objects
  can do
- [vats.md](vats.md) — where objects live, how they're isolated
