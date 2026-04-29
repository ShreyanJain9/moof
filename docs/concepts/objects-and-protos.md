# objects and protos

> **moof is prototype-based, not class-based. delegation walks
> protos. methods and slots are unified. there is no distinction
> between "user objects" and "system objects" — only Forms with
> different protos.**

every Form has a `proto` — its immediate parent in the delegation
chain. when a slot is read or a method dispatched, the substrate walks
proto-chain until it finds a match or hits the rooted base
(`Object` / `Form`).

this is the **self model** (ungar & smith 1987), not the smalltalk
class/metaclass model. we keep self's simplicity. we add (from
smalltalk) the live-image culture and the keyword-message rhythm. we
add (from kernel/maru) the evaluator-as-method discipline.

## the chain

```
some-counter                  ; an instance, slots filled
   ↓ proto
Counter                       ; the proto, defining the methods
   ↓ proto
Object                        ; root proto (or whatever we name it)
   ↓ proto
nil                           ; chain terminates
```

`[some-counter incr]` walks up: looks for `:incr` in some-counter's
own handlers, then Counter's, then Object's. first match wins.

## slots vs methods, unified

a slot and a method are looked up by the same protocol. self's
discovery: there is no operational difference between "reading a
slot" and "calling a unary method that returns a value." both look
up a name on the proto-chain and produce a value.

```moof
{Counter
  count: 0                   ; slot — direct value
  step: 1                    ; slot
  [computed-double]          ; method — produces a value
    [.count * 2]}
```

from a caller's standpoint:

```moof
[c count]                    ; slot read or method call — same syntax
[c computed-double]          ; method call, looks identical
.count                       ; (inside Counter's methods) self's count
```

the caller does not need to know whether `count` is "a slot" or "a
method that happens to return the same thing." you can change one
to the other without breaking callers. (this is one of self's
genuinely beautiful properties.)

## defining a proto

most user protos are defined with `defproto`:

```moof
(defproto Counter
  (slots count step)              ; slot names, default to nil
  (handlers
    [incr]            [self count: [.count + .step]]
    [incr-by: n]      [self count: [.count + n]]
    [decr]            [self count: [.count - .step]]
    [read]            .count))
```

`(defproto Counter ...)` produces a Form whose `proto` is `Object`
(the root) and whose `handlers` are populated with the listed methods.
binding it under the name `Counter` makes it available as a parent.

## creating an instance

```moof
(let c [Counter new])          ; → a fresh Form with proto=Counter
[c count: 0]
[c step: 1]
[c incr]                       ; → c.count is now 1
```

`[Counter new]` invokes Counter's `:new` handler, which is provided
by `Object` (the root) by default. user protos can override it for
custom construction.

object literals are an inline shorthand for proto + slot-fill +
optional method-overrides, in a single expression
(`syntax/object-literals.md`).

## inheritance

a proto can have its own proto. this is just chain extension.
single-inheritance only. no multiple inheritance, no traits, no
mixins at the substrate level.

```moof
(defproto BoundedCounter
  (proto Counter)               ; inherits from Counter
  (slots max)
  (handlers
    [incr-by: n]
      (if [[.count + n] > .max]
          (error 'overflow)
          [super incr-by: n])))   ; super-send
```

`super` is bound in a method body to "send to the proto above the
one this method was found on" — standard smalltalk semantics. exists
so an override can delegate to its parent's implementation.

## why prototypes, not classes

three reasons:

1. **uniformity.** in smalltalk, classes are themselves objects, so
   they have classes (metaclasses), so the metaclasses have classes,
   etc. it works but it's a tower of abstractions you have to
   understand. in self, objects have parents which are objects which
   have parents; one mechanism, recursing.
2. **moldability.** to change a method on a class, you change the
   class object's method dictionary. but the class object is itself
   created from a metaclass. in self, you change the proto's handler
   table. one place. one mechanism.
3. **liveness.** in self, you can clone a proto, modify the clone,
   and use it. no compile step. no "the type system needs to be told
   about this." this matches what we want.

## doesNotUnderstand

if a send walks the entire proto-chain without finding a handler,
the substrate sends `:does-not-understand` (with the original
selector and args) to the receiver. user code can override
`:does-not-understand` to provide late-bound or computed dispatch.

```moof
{Logger
  [does-not-understand: selector with: args]
    [println "logger does not understand $selector"]}

[some-logger frobnicate: 5]   ; falls through; logger prints message
```

this is the smalltalk extension hook (kay 1981, goldberg & robson
1983). it makes proxies, smart wrappers, and dispatcher patterns
trivially expressible.

## protos are mutable

a proto's handler table can be mutated at runtime. adding a method to
`Counter` makes that method available on every existing Counter
instance immediately (because they all delegate to the same
proto-form).

removing or replacing a method invalidates inline caches at any
existing send-site that resolved to that method; subsequent sends
re-resolve. this is the "edit a method, watch every caller pick up
the change" property of smalltalk-style live editing.

## become:

`[a become: b]` swaps the heap-cells of `a` and `b`. every existing
reference to `a` now points to `b`'s old contents and vice versa.
this is smalltalk's identity-swap (goldberg & robson 1983), used for
versioned upgrades, schema migrations, and a few rare but powerful
moves.

within a vat, `become:` is cheap because identity is mediated by
form-id. across vats, `become:` is undefined (cross-vat refs are
far-refs; you cannot reach into another vat's heap).

## bootstrapping

the proto-chain has to bottom out. our root proto is named `Object`
(the smalltalk convention). it is created by the substrate at boot
and provides the default `:new`, `:does-not-understand`, `:proto`,
`:slots`, `:handlers`, `:identity`, `:=`, `:is`, `:to-string`,
`:inspect` handlers.

`Object` itself has `proto: nil`. nothing is below it. all other
protos in the world chain up through `Object`.

## inspirations

- self's prototype model (ungar & smith 1987) is the direct ancestor.
- doesNotUnderstand: from smalltalk-80 (goldberg & robson 1983).
- become: from smalltalk-80.
- the slot/method unification is from self.
- the live mutability of protos is from smalltalk's class-edit-and-
  watch culture.

## see also

- `concepts/forms.md` — what a Form is.
- `concepts/sends-and-calls.md` — how dispatch happens.
- `concepts/reflection.md` — what's introspectable about a proto.
- `syntax/object-literals.md` — inline `{Proto …}` syntax.
- `syntax/methods-and-handlers.md` — method-definition syntax.
