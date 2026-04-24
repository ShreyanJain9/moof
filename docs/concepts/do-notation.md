# do-notation

**type:** concept
**specializes:** throughline 1 (contexts)

> `(do ...)` is moof's universal composition syntax. one notation
> sequences cross-vat sends, list comprehensions, stream pipelines,
> fallible chains, optional lookups. this doc is about how it works
> and what the two modes (yield-free / yielded) mean.

---

## why it deserves its own doc

`(do ...)` isn't "part of effects" or "part of streams" — it's
the notation that unifies both (and more). readers who miss
do-notation miss most of moof's ergonomics. it's a genuine primitive.

the moof version is a generalization of haskell's `do`, clojure's
`for`, SQL's `SELECT ... FROM ... WHERE`, python's generator
expressions. same shape in every case: take a source, name each
value, sequence through. `(do ...)` covers them all.

---

## the forms inside a block

every clause in a `(do ...)` block is one of four shapes:

### 1. bind: `(name <- expr)`

`expr` must be a Thenable. the bind calls `[expr then: |name| ...]`
with the rest of the block as the continuation. `name` is the
value the Thenable yields.

```moof
(do (user <- [users get: 'alice])   ; Act<User> bound into user
    (n    <- [table at: 'count])        ; Option<Integer> bound into n
    ...)
```

### 2. let: `(let name expr)`

pure binding. `name` becomes `expr`'s value. no Thenable
involved. equivalent to wrapping the rest of the block in
`(let ((name expr)) ...)`.

```moof
(do (n <- [table at: 'count])
    (let next [n + 1])                 ; pure let
    ...)
```

### 3. bare expression

an expression not wrapped in bind or let. evaluated in order;
its value is the block's value if it's the last form.

```moof
(do (user <- [users get: 'alice])
    [console println: (str "hi, " user.name)])  ; bare — side effect + final value
```

### 4. yield: `(yield expr)`

lift `expr` into the ambient Thenable via class-side `pure:`.
only legal as the last form in a block. see below for what
this means.

---

## two modes: flexible vs comprehension

### flexible mode (no yield)

the block is a **sequencer**. each `(name <- expr)` runs `expr`'s
`then:`, binding `name`. the block's result is the last form's
value, whatever it is.

you can bind from DIFFERENT kinds of Thenables in one block.
each Thenable's `then:` handles sequencing in its own way;
short-circuit semantics propagate (None stops an Option chain,
Err stops a Result chain, etc.).

```moof
; mixing kinds is fine without yield:
(do (n <- [table at: 'count])     ; bind from Option
    (user <- [users get: n])   ; bind from Act
    [console println: user.name]) ; bare; block result is Act<nil>
```

- if `table` has no `count`, the Option short-circuits to `None`;
  the block skips everything after and returns `None`.
- if `users` can't find user `n`, the Act resolves to `Err`;
  the block's result is `Act<Err>`.
- otherwise: the println Act resolves to `nil`, and THAT is the
  block's value.

the "type" of a non-yield block is a union: whatever the
last form returns, OR whatever short-circuit value interrupted
the chain. this is expressive but not always predictable;
use when you want sequencing without forcing a single output
kind.

### comprehension mode (with yield)

the block is a **comprehension**. all bindings must be from the
same kind of Thenable (M). the final `(yield v)` lifts v via
`M.pure:`. the block's result is `M<v>` unambiguously.

```moof
; list comprehension — block returns a list
(do (x <- (list 1 2 3))
    (yield [x * 2]))
; → (2 4 6)  : Cons

; Act comprehension — block returns an Act
(do (user <- [users get: 'alice])
    (addr <- [user getAddress])
    (yield addr.street))
; → Act<String>

; stream comprehension — block returns a stream
(do (click <- canvas-clicks)
    (yield (str "clicked at " click)))
; → Stream<String>
```

the moment you write `yield`, you're committing: this block is
a single-M comprehension. the compiler enforces that every
bind's source is M. mixing triggers an error (or a required
explicit lift).

---

## when to use which

**use yield when**:
- you want the block's type to be predictable `M<v>`.
- you're building a value, not just sequencing effects.
- you're doing a comprehension (for every x in this, produce y).

**omit yield when**:
- the final expression is already in the right context.
- you're sequencing heterogeneous effects and want the last
  expression to be the block's value.
- you genuinely want to mix Thenables in one flow.

most of the time you'll omit yield — it's the common pattern
of "do these things in order, final expression is what it is."
yield is for when you're building a value from a comprehension
and want the type to be crisp.

**Acts almost never need yield.** Act's `then:` always chains
to another Act (that's how async composition works). so if your
do-block's final form is a send-that-returns-an-Act (very
common — every cross-vat send does), the block's value is
already an Act with no lifting required. yield an Act would be
redundant.

this pattern covers most effect code: a sequence of Acts, the
last one being the "real" effect you wanted.

```moof
; yield-free; final is already Act<nil>
(do (user <- [users get: id])
    (let greeting (str "hi, " user.name))
    [console println: greeting])
; → Act<nil>

; yield-free; final is an Act<Profile>
(do (user <- [users get: id])
    [user getProfile])
; → Act<Profile>
```

yield kicks in when:
- you're in a Cons/Option/Result/Stream comprehension and
  producing a pure result: `(yield [x * 2])` — pure value
  needs lifting into the list/option/etc.
- you specifically want the block's type to be `M<v>` when the
  final form would otherwise be bare.

---

## short-circuit semantics

every Thenable's `then:` can short-circuit. bind from a None
and the continuation never runs — the block returns None. bind
from an Err and the continuation never runs — the block returns
Err. bind from a terminated stream and the continuation never
runs.

```moof
(do (n <- none-value)    ; None
    (y <- some-other)     ; never runs
    (yield [n + y]))
; → None
```

this replaces exception handling: a failure is a value that
propagates through the chain via the Thenable's own `then:`
semantics. nothing special-cased. see
[effects.md](effects.md) for the rationale.

to HANDLE a failure — not just propagate it — use `recover:`:

```moof
(do (user <- [[users get: id] recover: |err| (Err "user missing")])
    ...)
```

`recover:` runs its continuation only when the Thenable
represents failure. for types that don't fail (Cons, a
successful Act), `recover:` returns self.

---

## what do-notation DOESN'T give you

- **no introspection.** you don't `[act pending?]` or
  `[result ok?]`. Acts are opaque by design. you `then:` or
  `recover:`, and the scheduler delivers. probing is a
  violation of the abstraction.
- **no synchronous waits.** you never block the current vat
  waiting for another's reply. every cross-vat bind yields an
  Act, composed through do. the scheduler runs everyone fairly.
- **no exception catching.** failures are values, not panics.
  `recover:` handles them inline.

---

## the compilation sketch

for intuition, here's how `(do ...)` desugars. this isn't the
actual compiler — the real one handles yield-inference and
optimization — but it's close enough to read.

```
(do expr)
;; → expr

(do expr1 expr2 ...)
;; → (do-seq expr1 (do expr2 ...))
;; where do-seq runs expr1 (for effect), then the rest

(do (let x v) body ...)
;; → (let ((x v)) (do body ...))

(do (x <- src) body ...)
;; → [src then: |x| (do body ...)]

(do ... (yield v))
;; → enforce single-M; lift v via M.pure:
```

the yield case checks all the binds' sources are the same M
(via protocol inspection at compile time, or runtime if
necessary), then generates `[... then: |lastVar| (M pure: v)]`.

---

## examples: the unification in practice

### sequential cross-vat calls

```moof
(do (user <- [users get: id])
    (profile <- [user getProfile])
    (yield profile))
; → Act<Profile>
```

### list comprehension

```moof
(do (x <- (list 1 2 3))
    (y <- (list 'a 'b))
    (yield (list x y)))
; → ((1 a) (1 b) (2 a) (2 b) (3 a) (3 b))
;   — yes, this is cartesian product — that's how list-monad bind works
```

### option chain

```moof
(do (user  <- [users at: id])       ; Option<User>
    (email <- user.email)            ; Option<String> (could be absent)
    (yield (normalize-email email)))
; → Option<String>  — None if any link is absent
```

### result chain with recovery

```moof
(do (cfg <- (parse-config source))              ; Result<Config>
    (db  <- [[connect: cfg] recover: use-default]) ; recover if connect fails
    (yield db.status))
; → Result<Status>  — recovery keeps it in Ok-land
```

### stream pipeline with effect

```moof
(do (click <- canvas-clicks)
    [console println: (str "got " click)]     ; per-click effect
    (let target (hit-test click))
    (target-act <- [renderer render: target])  ; Act per click
    (yield target-act))
; → Stream<Act<Rendered>>
```

this is actually subtle — we've bound from a Stream and an Act
in the same block, with yield. because the outer binding is
Stream, every inner Act is treated as a per-step subroutine;
the block produces a Stream of Acts. if you tried to yield
BOTH stream-dependent AND act-dependent values, the compiler
would require explicit lifting.

### effect-only (no yield)

```moof
(do (x <- (range 0 10))
    [console println: x])
; bare expression at end; block runs 10 println Acts; its value
; is the last Act<nil>. no yield means no comprehension;
; the block is just "do these things in sequence."
```

---

## what you need to know

- `(do ...)` has four clause shapes: bind, let, bare, yield.
- no yield → flexible sequencer, can mix Thenables, block's
  value is the last form.
- with yield → single-M comprehension, block's value is `M<v>`.
- short-circuit is automatic via the bind's Thenable.
- never probe — compose. no `pending?`, no `ok?`.
- do replaces haskell's do, clojure's for, SQL SELECT/FROM/WHERE,
  python genexps, JS promise chains — all at once.

---

## next

- [../throughlines.md](../throughlines.md) — the contexts
  pattern `do` animates
- [effects.md](effects.md) — Act and Update flavors
- [streams.md](streams.md) — Stream flavor
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) —
  Thenable's contract
