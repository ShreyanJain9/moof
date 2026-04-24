# messages

**type:** concept

> sending a message is the one operation moof has. function call,
> arithmetic, slot access, control flow, IO — all message sends.

---

## the shape of a send

`[obj selector: arg1 other-keyword: arg2]`

three parts:

- **receiver** — the object receiving the message. here, `obj`.
- **selector** — the symbol identifying the message. here,
  `'selector:other-keyword:'` (the full keyword string with colons).
- **args** — zero or more argument values. here, `(arg1, arg2)`.

the syntax mirrors smalltalk's. keywords end in colons. unary sends
omit them: `[obj describe]`. binary selectors have no colon:
`[a + b]`.

```
[3 + 4]                 ; binary: selector '+', args (4)
[obj describe]          ; unary: selector 'describe', args ()
[list at: 3 put: 99]    ; keyword: selector 'at:put:', args (3, 99)
```

---

## dispatch

sending a message means finding a handler and calling it.

1. **start at the receiver.** does `obj` have the selector installed
   as a handler directly?
2. **walk the prototype chain.** no? check `obj`'s prototype, then
   its prototype, up to `Object` → nil.
3. **found a handler.** call it with `(receiver, args)`.
4. **not found.** send `[obj doesNotUnderstand: 'selector args]` to
   the receiver. if *that* fails too, raise a dispatch error.

the walk is `MAX_DELEGATION_DEPTH` = 256 steps. longer chains get a
"delegation chain too deep" error. in practice chains are short —
type → Object, usually ≤ 4.

## doesNotUnderstand:

when dispatch fails, the VM tries one more send:

```
[receiver doesNotUnderstand: 'selector args]
```

this is a feature, not an error recovery. it's how you write:

- **proxies** — an object that forwards every message to another
  (local or remote).
- **dynamic DSLs** — a "schema" object that responds to any selector
  by treating it as a column name.
- **lazy loading** — an object that loads its handlers on first
  access and reinstalls them.
- **far refs** — cross-vat references use `doesNotUnderstand:` to
  queue the message as an outgoing send.

every object has a default `doesNotUnderstand:` that signals an
error. override it on a prototype to make anything work.

---

## the universality claim

**every operation in moof is a send.** this is load-bearing. let's
walk through:

### arithmetic

`[3 + 4]` is a send to integer 3 with selector `'+'` and arg `(4)`.
Integer's `+` handler is a native that adds two integers.

### function calls

`(f x y)` is sugar for `[f call: x y]`. a function is an object with
a `call:` handler. this is how you can make ANY object callable by
giving it a `call:`:

```
(def greet { [call: args] (str "hello, " [[args car] describe]) })
(greet "world")      ; "hello, world"
```

### slot access

`obj.x` is sugar for `[obj slotAt: 'x]`. the Object prototype has a
`slotAt:` native that reads the slot by symbol.

### control flow

`(if c a b)` is `[c ifTrue:ifFalse: a b]` — a message to the boolean.
Boolean-True's handler runs `a`; Boolean-False's runs `b`. this
means:

- you can override `if` for your types (true means something custom).
- a non-boolean value that responds to `ifTrue:ifFalse:` is
  "truthy" or "falsy" by your definition.

### introspection

`[obj type]`, `[obj slotNames]`, `[obj handlerAt: sym]` — all
messages. no side channel into "compiler tables."

### IO

`[console println: "hi"]` is a send to the Console vat. crossing a
vat boundary makes it eventual (returns an Act). the semantics are
the same as any send; the Act is explicit because the effect is
explicit.

### concurrency

spawning a vat is a send to a Scheduler object. queueing a message
is a send to a vat reference. there's no "spawn primitive" separate
from sends.

---

## selectors are symbols

selectors are Symbols — interned strings. `'+'`, `'describe'`,
`'at:put:'`. the VM uses integer symbol IDs for dispatch speed;
moof code sees symbols.

because selectors are symbols, you can:

- construct them at runtime: `[obj perform: sym withArgs: args]`
- introspect them: `(handler-names obj)` returns a list of symbols.
- install them programmatically via `handle:with:` (a rare but
  available move for meta-programming).

---

## synchronous vs eventual sends

two send forms:

- **synchronous.** `[obj sel: arg]` — when receiver and sender are
  in the same vat. the handler runs; the result is the send's
  value.
- **eventual.** `[obj <- sel: arg]` — across vats. the send is
  enqueued; the sender gets an Act immediately; the handler runs
  later in the receiver's vat.

most of the time you don't write `<-`. moof infers it:

- if `obj` is a local value, sends are synchronous.
- if `obj` is a FarRef (cross-vat), sends are eventual
  automatically.

you can write `<-` explicitly when you want to defer a local call to
a future tick (rare but legal).

eventual sends return Acts — first-class effect descriptors. see
[effects.md](effects.md) for how Acts compose.

---

## calling vs sending: the surface

moof has three bracket species:

```
(f x y)              ; applicative — desugars to [f call: x y]
[obj sel: x y]       ; message send — the primitive
{ Parent slot: val } ; object literal — creates a new object
```

they all reduce to sends. `(f x y)` is syntactic sugar for
`[f call: x y]`. `{ Parent ... }` is syntactic sugar for constructing
an object literal (which is a special form at the parser level).

you'll see parentheses in applicative lisp code, brackets for
OO-style message sends, braces for object literals. the choice is
stylistic — use whatever reads best.

---

## promise pipelining (eventual only)

if `a` is a FarRef:

```
[[a <- foo] <- bar]
```

pipelines. the first send queues, returns an Act. the second send
is queued on the ACT, not the result — so it's sent as soon as `a`
receives `foo`, not after the reply has traveled all the way back.

this is E language's promise pipelining. it makes chain-of-calls
cheap across the network: one round trip per chain, not one per
send.

---

## methods, not functions

moof has no notion of "function that isn't a method." every
function is a closure object with a `call:` handler. every named
top-level value is an object. there's no free function namespace,
no method-vs-function distinction.

```
(defn add (a b) [a + b])
```

desugars to defining a closure and binding it to `add`. `(add 3 4)`
is `[add call: 3 4]`.

this uniformity means:
- function arguments are passed as lists to `call:`.
- closures can be introspected: `[f arity]`, `[f source]`, etc.
- you can replace a function's behavior by reinstalling `call:`.

---

## what you need to know, summarized

- send is the primitive. `[obj sel: arg]`.
- dispatch walks the prototype chain.
- missing handlers become `doesNotUnderstand:` messages.
- selectors are symbols.
- every operation is a send — including arithmetic, control flow,
  slot access.
- synchronous sends are in-vat; eventual (`<-`) cross vats and
  return Acts.
- `(f x)` is sugar for `[f call: x]`. no functions-vs-methods
  distinction.

---

## next

- [protocols.md](protocols.md) — how we reason about what a
  receiver can respond to.
- [vats.md](vats.md) — what happens when a send crosses a vat.
- [effects.md](effects.md) — how Acts work, cross-vat chains,
  do-notation.
