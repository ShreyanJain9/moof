# streams

**type:** concept
**specializes:** throughline 1 (contexts), throughline 6 (time)

> a stream is a Thenable context with a **temporal flavor**: a
> value that yields successor values over time. same shape as
> Act, Option, Cons, Result — different meaning of "context."

---

## the one idea

if you've read [throughlines.md](../throughlines.md), you
already know what a stream is.

a stream is a **context**: a value-wrapping-structure satisfying
the `Thenable` protocol. same shape as Option (presence context),
Result (success/failure context), Cons (indexed-sequence
context), Act (cross-vat-pending context).

the flavor: **a stream's context is "more values might arrive
over time."** bind composes; `(do ...)` sequences; you don't
need to learn a new vocabulary for it.

```moof
; same do-syntax as every other Thenable context
(do
  (click <- canvas-clicks)             ; Stream<Click>
  (resolved <- (resolve-target click))  ; → Act<Target> or similar
  (rendered <- (render-target resolved)))
```

what makes a stream a Stream (capital S) rather than some other
Thenable: its `then:` returns future-values one-at-a-time as
they arrive, rather than a single value once. `(do ...)` over a
stream is a true comprehension — the block returns a Stream.

---

## how this specializes the contexts throughline

| context | "the extra structure" | when bind fires |
|---------|----------------------|-----------------|
| Option | presence/absence | now, synchronously |
| Result | success/failure | now, synchronously |
| Cons | each element in sequence | for every element, now |
| Act | pending cross-vat computation | once, when resolved |
| **Stream** | **each value over time** | **every yield, as it arrives** |
| Update | state-change-with-reply | at scheduler tick |

every row here composes via `Thenable.then:` and uses `(do ...)`.
the row-differences are operational: when does the inner
computation fire? with what value? what happens after?

the Thenable protocol is one contract. the rows are six
specializations. **streams are not a separate abstraction family
— they're the sixth row.**

---

## what's stream-shaped in moof

anything that produces values over time. the source is different;
the Thenable semantics are the same:

- **a Cons** — each cell yields its car; the cdr is the "next."
  also Iterable (read eagerly), also Thenable (bound
  per-element), also Stream (pulled incrementally).
- **a Range** — `(range 0 ∞)` is a Stream of integers.
- **a File's lines** — each call to `next:` reads the next line.
- **a Clock's ticks** — every N ms, yield the current time.
- **a vat's mailbox** — every incoming message is a stream
  element.
- **a Canvas's input events** — clicks, keystrokes, touches.
- **a reactive Signal** — every emission yields.
- **an Act's resolution history** — if you want to inspect when
  an Act went through states, that trajectory is a stream.

the last one shows why streams and Acts aren't separate
kingdoms: an Act's *result* is a single Thenable bind; an Act's
*history* is a stream. they're the same object seen along
different axes.

---

## what's NOT stream-shaped (as a primary pattern)

- **a Table's entries** — eager, already there. iterate (Iterable),
  don't stream.
- **a Set's elements** — same.
- **a String's chars** — could be streamed if you wanted, but
  typically you index (Indexable) — they're materialized.

the test: **does the source have an inherent temporal axis?**
if yes, Stream. if no, Iterable or Indexable.

---

## streams through the other throughlines

**contexts (throughline 1).** covered above: Stream is the
temporal flavor of Thenable.

**walks (throughline 3).** a stream's `next:` call is a walk: it
addresses "the next element of this stream" and fetches it. the
walk happens incrementally across time instead of across a
spatial graph.

**additive authoring (throughline 4).** streams are immutable
values — consuming a stream produces a new tail; the old stream
is still valid. forking a stream at some point gives you two
branches that evolve independently. no in-place iteration state.

**canonical form (throughline 5).** finite, deterministic
streams canonicalize the same way lists do: each yielded value
in order, hashed. infinite/nondeterministic streams don't (they
can't — their content isn't a fixed value).

**time (throughline 6).** the defining flavor. time is the axis
a stream lives on.

---

## composition

every Thenable composer works on streams, because streams are
Thenable:

```moof
[stream map: f]           ; transform each yielded value (Iterable+Stream)
[stream filter: p]        ; drop non-matching yields
[stream take: 10]         ; finite prefix
[stream drop: 5]          ; skip first 5
[stream merge: other]     ; interleave
[stream zip: other]       ; pair elements
[stream throttle: 500ms]  ; temporal combinator (stream-specific)
[stream debounce: 200ms]  ; temporal combinator (stream-specific)
[stream window: 1s]       ; temporal combinator
```

the first six work on anything Iterable/Thenable — Cons, Stream,
Range share them. the last three are temporal specializations:
they need a time axis. streams have one; Cons doesn't.

transducers (`lib/flow/transducer.moof`) are the composable
pipeline primitive. they compose over Iterable OR Streamable:

```moof
(def xf (comp [map: f] [filter: p] [take: 10]))
[stream transduce: xf]      ; Stream<B>
[list transduce: xf]        ; Cons<B>  (materialized)
```

one pipeline, both temporal and spatial sources.

---

## streams and Acts: the deeper connection

an Act is "a computation pending in time — one value, one
resolution."

a Stream is "many values arriving in time — an open series."

both live on the time axis. an Act is a stream that yields
exactly once then ends. a stream of Acts is a computation where
each element is a pending result. merge a stream-of-Acts with a
stream-of-ticks and you get a throttled-async-source.

the mental model: **an Act is a special case of a stream.** in
practice the stdlib keeps them distinct types for performance
and semantic clarity (an Act doesn't have
backpressure/throttle/merge machinery; adding it would be
overhead for a single-value case). but the relationship is
real.

---

## mailboxes are streams

a vat's mailbox is, semantically, a Stream<Message>. the
scheduler drains one element at a time; each message is a yield;
`on-end` happens when the vat exits.

today this is implemented inside the scheduler's event loop,
not exposed as a moof Stream. it SHOULD be exposed — then
reflective scheduler logic ("watch this vat's incoming messages")
becomes a normal stream composition.

this is on the roadmap. when it lands, the scheduler is a moof-
level stream processor; every debugging tool that works on
streams works on the scheduler.

---

## server vats process streams

a defserver is a vat that receives messages (a stream) and
produces replies (a stream of Updates). the scheduler's loop is:

```
forever, for each vat with pending work:
  take the next message from mailbox (stream pull)
  dispatch it to a handler
  handler returns a reply / Update / Act
  send reply on the outbox (stream push)
  apply Update between messages
```

this is a transducer over the message stream. `defserver` hides
the machinery; the reality is a stream processor.

---

## pull-based by default; push with backpressure is future

today's moof streams are **pull-based**: the consumer asks, the
producer yields. consumption naturally caps at the consumer's
rate; no unbounded buffering.

push-based streams — where the producer emits at its own cadence
and buffers queue up — exist as reactive Signals (`lib/flow/
reactive.moof`) but without formal backpressure. formal
push-with-backpressure is on the roadmap; it'll layer on top of
the current pull-based primitive, not replace it.

---

## what you need to know

- a stream is a Thenable context. temporal flavor.
- same composition primitive (Thenable protocol, `(do ...)`,
  transducers) works on streams as on Acts, Options, Results,
  Cons.
- stream-specific operators (`throttle:`, `debounce:`, `merge:`)
  depend on the time axis; they don't apply to Cons.
- mailboxes, input events, sensor readings, signals are all
  stream-shaped.
- an Act is a degenerate one-yield stream. same family.

---

## next

- [../throughlines.md](../throughlines.md) — the contexts
  pattern this specializes
- [effects.md](effects.md) — Acts, Updates — sibling flavors of
  the same pattern
- [vats.md](vats.md) — mailboxes as stream sources
- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) —
  Thenable — the universal composition protocol
