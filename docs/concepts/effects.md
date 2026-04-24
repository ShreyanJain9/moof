# effects

**type:** concept

> how things happen in moof. pure values vs effectful operations,
> Acts as first-class effect descriptors, Updates for in-vat state
> changes, do-notation for composing them all.

---

## the rule

**pure values in, pure values out — unless you've crossed a vat.**

inside a vat, all code is pure: no mutation of existing objects,
no IO, no side effects. you produce new values from old ones.
`(+ 3 4) → 7`, `[list map: f] → a new list`, etc.

crossing a vat boundary is the ONLY way to do anything effectful:
print to console, read a file, update server state, wait for time
to pass. and cross-vat sends return **Acts**, not values. the Act
is an effect descriptor — first-class, inspectable, composable.

so the rule: **if you see an Act, you know there's an effect. if
you don't, there isn't.**

this is haskell's IO monad made concrete: the compiler doesn't
enforce it, but the object model does. pure code holds no
cross-vat references, so it can't do effects.

---

## Acts

an Act is an object with:

- a **state**: `pending` (waiting) or `resolved` (has a value).
- a **result**: nil while pending; set when resolved.
- a **chain**: continuations to run when it resolves.

an Act is what you get back from a cross-vat send:

```moof
(def x [console <- println: "hi"])
; x is an Act<nil>. println runs when the scheduler drains.
```

Acts are also what servers return when they want to defer:

```moof
[my-counter incr]
; returns Act<0> (the OLD value) — the increment happens later
```

---

## Updates

inside a server, handlers can return an **Update**:

```
(update { value: [@value + 1] } old-value)
```

which says: "change these slots. reply to the caller with this value."

the scheduler applies the delta atomically, between messages. from
the outside, the server's state appears to transition discretely
and in-order. the caller sees an Act that eventually resolves to
the reply.

updates are the only way to mutate state. even inside a server, you
can't slot-assign. you return an Update and let the scheduler do it.
this keeps every moment of state reachable from the message log.

updates can also carry a **pending reply** — the server asks
"please finish this computation and merge the result into my
state". that lets you do things like:

```
[fetchX] [[[http <- get: "/x"] then: |val|
           (update { x: val } val)]]
```

the http response comes back, the server updates its state AND
replies with the value. all through one coherent message flow.

---

## do-notation

chaining Acts manually is tedious:

```moof
[[console <- println: "first"] then: |_|
  [[console <- println: "second"] then: |_|
    [console <- println: "third"]]]
```

do-notation lifts this into sequential sugar:

```moof
(do
  [console <- println: "first"]
  [console <- println: "second"]
  [console <- println: "third"])
```

and with bindings:

```moof
(do
  (x <- [user <- get: 'name])
  (greeting = (str "hello, " x))
  [console <- println: greeting])
```

`<-` is bind: run the Act, await its value, bind to x.
`=` is let: pure value binding.

do-notation works on anything **Monadic** — Act, Cons, Option,
Result, Update. one syntax, many monads. this is haskell's
`do`-notation applied to moof's object model.

---

## the pure/effectful split

- **pure code**: holds only in-vat references. guaranteed to
  terminate with a value. cacheable, replayable, parallelizable.
- **effectful code**: holds at least one cross-vat reference
  (FarRef). sends to it yield Acts. the moment you bind an Act's
  result you're in effectful territory.

there's no keyword for "this is pure" — purity is a property of
the closure's captures. if a closure captures no FarRef, and it
never creates an Act, it's pure. the VM can prove it; in practice
it's usually obvious from reading.

pragmatically this means: if you're writing a helper function and
it never touches a capability, it's pure. pure functions are cheap
to test and trivially deterministic.

---

## composition patterns

### short-circuit on failure

`Result` (Ok / Err) is Monadic + Fallible. bind on an Err short-
circuits:

```moof
(do
  (user <- (fetch-user id))        ; Act<Result<User>>
  (profile <- (fetch-profile user)) ; if user was Err, this is skipped
  (render profile))
```

### parallel fan-out

spawn multiple Acts, await all, reduce:

```moof
(def urls (list "a" "b" "c"))
(def acts [urls map: |u| [http <- get: u]])
(def results [all: acts])
```

`all:` turns a list-of-Acts into an Act-of-list. when every input
resolves, the result is ready.

### timeout / cancellation

wrap an Act with a deadline:

```moof
(def x [[http <- get: url] withTimeout: 5-seconds])
; resolves to the response, or to Err("timeout") after 5s
```

cancellation propagates: pending Acts can be aborted, freeing the
target vat to ignore the in-flight reply.

---

## why not exceptions?

moof used to have try/catch/throw. it was removed.

reasons:
- exceptions are non-local gotos. they bypass the composition
  story. a function's type signature lies about what it can
  actually do.
- they don't compose across vats. there's no "catch" on the other
  side of an async send.
- error-as-value is more uniform: failures are just values that
  short-circuit in binds. the same syntax handles success and
  failure.

the replacement is Result. `Ok`/`Err` values flow through
do-notation. `[result recover: |err| ...]` handles failure. the
path is explicit.

see [../archive/errors.md](../archive/errors.md) for the historical
deprecation.

---

## the three effect protocols

from the ten-protocol stdlib:

- **Monadic** — `then:`, `pure:`. bind + unit. the shape of a
  sequenceable thing. Act, Cons, Option, Result, Update all
  conform.
- **Fallible** — `ok?`, `recover:`. the shape of a thing that can
  fail. Ok, Err, Some, None, Act (when it resolves to Err)
  conform.
- **Awaitable** — `pending?`, `wait`. the shape of a thing that
  resolves over time. Act, Update conform. Cons and Option do not
  — they're never pending.

a single value can be all three (Act) or just one (Cons is Monadic
only). they're orthogonal.

---

## why Acts are values

Acts aren't "invisible promises you wait on." they're first-class
objects. you can:

- store an Act in a slot ("the pending result goes here").
- pass an Act to another function ("wait for this, then do X").
- inspect an Act ("show me what it's waiting on").
- cancel an Act ("never mind").
- serialize an Act ("i'll resume this computation tomorrow").

the last one matters for persistence. if the image includes
in-flight Acts, closing and reopening moof resumes them. this is
how "reboot is continuity" works (in the planned wave 10+
running-state persistence).

---

## what you need to know

- pure code returns values. effectful code returns Acts.
- cross-vat sends are the only source of effects.
- Updates are how servers change state (atomic between messages).
- do-notation composes Acts, Cons, Option, Result, Update
  uniformly.
- errors are values (Ok/Err), not exceptions.
- Acts are first-class objects, not hidden future-pointers.

---

## next

- [vats.md](vats.md) — the isolation boundary.
- [streams.md](streams.md) — Act chains as temporal flows;
  mailboxes as stream-shaped sources.
- [capabilities.md](capabilities.md) — what "cross-vat reference"
  means in the security model.
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) —
  Monadic / Fallible / Awaitable as protocols.
