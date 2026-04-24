# effects

**type:** concept
**specializes:** throughline 1 (contexts), throughline 4 (additive)

> Act and Update are Thenable contexts with specific flavors: Act
> wraps "a computation pending across a vat boundary"; Update
> wraps "a value plus a state-change delta." same composition
> shape as Option, Result, Cons, Stream — different meaning of
> context. this doc says what the flavors are and why they're
> sufficient.

---

## the rule

**pure values in, pure values out — unless you've crossed a vat.**

inside a vat, code is pure: no mutation of existing objects, no
IO, no side effects. you produce new values from old ones.

crossing a vat boundary is the **only** way to do anything
effectful: print, read a file, change server state, wait for
time. and cross-vat sends return Acts, not values.

**if you see an Act, you know there's an effect. if you don't,
there isn't.** pure code simply doesn't produce Acts. there's
nothing else to reason about.

this is haskell's IO monad applied through moof's object model:
purity is enforced not by a type system but by the vat boundary.
holding no FarRef means having no way to do effects.

---

## Act — the cross-vat pending context

an Act is a Thenable context with this flavor: **"a computation
still running across a vat boundary; resolves to a value later."**

structurally:
- **state**: `pending` or `resolved`
- **result**: nil while pending; value or Err when resolved
- **chain**: continuations to run when resolved
- **conforms**: Thenable (with `pending?` overridden, `ok?` and
  `recover:` deferred to resolved value)

```moof
(def x [console <- println: "hi"])
; x is an Act<nil>. println runs when the scheduler drains.
```

composed via `(do ...)`, same as any Thenable:

```moof
(do
  (greeting <- [user <- getName])     ; Act<String>
  (prefixed = (str "hello, " greeting)) ; pure let
  [console <- println: prefixed])     ; Act<nil>
```

this is the "promise" or "future" pattern, but first-class as a
value. you can store an Act in a slot, pass it around, inspect
its state, attach multiple continuations, cancel it, serialize
it (wave 11+).

---

## Update — the state-change context

inside a server vat, a handler can return an **Update**: a
Thenable context whose flavor is **"a reply value plus a delta to
apply to this server's state before the reply ships."**

```moof
(defserver Counter (initial)
  { value: initial
    [incr] (update { value: [@value + 1] } @value) })
```

`(update delta reply)` returns an Update. the scheduler:
1. applies `delta` atomically between messages
2. ships `reply` back to the caller

then-chained on an Update: `[update then: f]` schedules f on the
reply, potentially producing another Update (whose delta merges
with ours).

---

## contexts, not categories

Acts and Updates look like different machinery. they aren't.
they're the same pattern (Thenable context) played in different
keys:

| context | flavor | bind runs |
|---------|--------|-----------|
| Option | maybe-absent | synchronously, iff present |
| Result | maybe-failed | synchronously, iff Ok |
| Cons | indexed sequence | per element, synchronously |
| Act | cross-vat pending | once, when resolved |
| Stream | temporal sequence | every yield, over time |
| Update | state-change + reply | applied at scheduler tick |

all six compose through the same protocol, the same syntax. the
differences are operational (when, with what signal, what
happens after) — not structural.

**the practical consequence:** when you find yourself wanting a
"new kind of effect," check whether it fits one of these first.
almost always it does. don't invent a seventh effect type.

---

## one protocol: Thenable

moof ships ONE Thenable protocol that handles composition for
every context. minimal required methods; the rest are provides
with sensible defaults.

- **required**: `then:` (bind), class-side `pure:` (lift a value).
- **provides with defaults** (override when needed):
  - `recover:` (default: `self` — "i'm already fine"). overridden
    by Err/None to run the recovery continuation.
  - `ok?` (default: `true`). overridden by Err/None to return
    `false`; Act defers to its resolved value when ready.
  - `pending?` (default: `false`). overridden by Act/Update
    while unresolved.

an earlier doctrine proposed splitting this into Monadic +
Fallible + Awaitable. reverted — the split forced every type to
declare three conformances to say what defaults would have said
more cleanly. fused + defaults is the real shape.

**you can always ask** any Thenable `recover:`, `ok?`, or
`pending?` — Cons responds sensibly (`self`, `true`, `false`),
Option responds sensibly, Act responds according to its
resolution state. no "this type doesn't support that." just the
defaults.

---

## do-notation — the universal comprehension

`(do ...)` is not just Act-chaining. it's a comprehension that
infers its output type from what you bind:

```moof
(do
  e1                ; evaluate e1 (in ambient context)
  (x = pure-val)    ; let binding (pure)
  (y <- thenable)   ; bind: sequence through the context
  (yield v))        ; lift v into the same context
```

- bind from an Act → block returns an Act.
- bind from an Option → block returns an Option.
- bind from a Cons → block returns a Cons (comprehension!).
- bind from a Stream → block returns a Stream.
- bind from a Result → block returns a Result; an Err
  short-circuits.

```moof
; comprehension over a list: the result is a list
(do (x <- (list 1 2 3))
    (yield [x * 2]))
; → (2 4 6)

; comprehension over a stream: the result is a stream
(do (click <- canvas-clicks)
    (yield (str "click at " click)))
; → Stream<String>

; Act composition: the result is an Act
(do (user <- [users <- get: 'alice])
    (yield user.name))
; → Act<String>

; short-circuit on failure: the result is an Option
(do (n <- [table at: 'count])    ; Option<Integer>
    (yield [n + 1]))
; → Option<Integer>  — None if count is absent
```

**`yield`** is how you lift a pure value back into the ambient
Thenable. it calls the class-side `pure:` of whatever the block
is returning. bare expressions at the end are the block's value
directly (already in the right context, or lifted automatically
when possible).

**single-kind-per-block is NOT the rule.** effects in the middle
don't care about kind — `(do (x <- list) [console <- println: x])`
runs a println per list element (an effect in the middle of a
list comprehension). the block's kind is whatever you're
ultimately producing.

when kinds genuinely don't lift (binding raw Options mid-Act),
the compiler can ask for explicit promotion. most of the time,
inference handles it.

---

## why this is the universal comprehension

SQL has `SELECT ... FROM ... WHERE`. haskell has list
comprehensions and monad comprehensions. python has generator
expressions. clojure has `for`. each is a syntax for
"iterate-bind-and-produce" over some specific kind of
collection.

moof's `(do ...)` handles all of these with one syntax. the
block's output type follows the input(s). you don't learn a
different syntax for "comprehension over a database result" vs
"comprehension over a stream" vs "sequential async operations"
— they're all the same comprehension over different Thenables.

this is the universal-comprehension dream cashed out. one
notation, infinitely many usable contexts.

---

## why not exceptions?

moof once had `try` / `catch` / `throw`. removed.

- exceptions are non-local gotos. bypass composition.
- they don't cross vat boundaries — no catch on the other side
  of an async send.
- they confuse Thenable composition: "this might short-circuit"
  should be in the type, not in control flow magic.

the replacement is Result. `Ok` / `Err` values flow through
Thenable composition. bind on an `Err` short-circuits (because
Err's `ok?` is false and Thenable's `then:` respects it).
`[result recover: f]` handles failure. the effect path is
explicit.

see [../archive/errors.md](../archive/errors.md) for the
historical deprecation.

---

## effects and additive authoring (throughline 4)

Updates are how mutation works, and they embody the additive
throughline:

- a handler returns `(update delta reply)`.
- the delta lists slot-changes.
- the scheduler applies the delta BETWEEN messages, producing
  a new snapshot of the server's state.
- the old state isn't mutated — it's replaced (referentially)
  by the new one.
- the server's history is a sequence of states-over-time (a
  stream!).

this is why moof can claim "no mutation." mutation LOOKS like
it's happening inside a defserver, but mechanically the server
produces new state values between messages. each state is
immutable while current.

from outside the server, you never observe a half-changed state.
updates are atomic.

---

## acts and streams

an Act is a one-yield stream. a stream that yields once then
ends. this isn't a metaphor; the composition semantics overlap
exactly:

```moof
; Act.then: = run continuation on the resolved value.
; Stream of length 1: next: yields once, then on-end.
; Same shape.
```

in practice they're distinct types because Act carries extra
state (a pending slot, a continuation chain, error-propagation
machinery) that would be wasteful on single-yield streams. but
the family relationship is real.

a stream of Acts is a temporal sequence of pending values. you
compose it with stream combinators; each element binds with Act
combinators. two layers of the same pattern, nested.

---

## cross-vat copy semantics

when an Act crosses a vat boundary (args of an outgoing message
include one), the scheduler copies it. the copy carries the
same URL the original had, so after reload it re-resolves to the
same underlying computation.

this falls out of canonical form (throughline 5): an Act has a
serialized representation, a URL for identity, and a reconstitution
path on load. it's treated like any other value.

---

## what you need to know

- Act is a Thenable context with a "pending cross-vat" flavor.
- Update is a Thenable context with a "reply + state delta"
  flavor.
- both compose via `(do ...)` like any other Thenable value.
- they're specializations of the same pattern — not a separate
  effect system.
- purity is enforced by the vat boundary; no type-system magic
  needed.
- error-as-value (Result / Err) replaces exceptions. same
  composition primitive.
- Act ≈ one-yield Stream; they're family.

---

## next

- [../throughlines.md](../throughlines.md) — the contexts
  pattern this specializes
- [streams.md](streams.md) — Stream, the multi-yield sibling
- [vats.md](vats.md) — the isolation boundary that makes
  effects detectable
- [capabilities.md](capabilities.md) — what "cross-vat
  reference" means for security
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) —
  Thenable — the single composition protocol
