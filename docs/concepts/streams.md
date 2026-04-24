# streams

**type:** concept

> unix's real gift wasn't text — it was treating everything as a
> flow of values. moof keeps that commitment but applies it to
> typed values at every scale: mailboxes, event queues, rendered
> frames, log output, sensor data.

---

## the pattern

a **stream** is a sequence you consume incrementally. it might be
finite (a file's contents, a list) or infinite (a clock tick, a
keystroke event source). you compose over it with transducers,
you merge streams, you throttle, you window. you don't
materialize it into a list just to work with it.

this is unix's pipeline taken literally. `cmd1 | cmd2 | cmd3` is
a sequence of streams with transformations between them. moof's
version replaces text-bytes with typed values and makes the
pipeline a composable moof value.

---

## what's a stream in moof

a Stream is an object that responds to `next:`:

```moof
(defprotocol Streamable
  (require (next: k)
    "Produce the next value. k is a continuation:
      [k on-value: v]    — here's the next value
      [k on-end]         — no more values
      [k on-error: e]    — something went wrong
     called exactly once when ready."))
```

this is a pull-based stream: the consumer asks, the producer
yields. push-based streams (with backpressure) are a future
addition.

**what's a stream**:
- a Cons list (each cell is one value; next: pulls the cdr).
- a Range (next: increments).
- a File's lines (each call reads a line; EOF → on-end).
- a Clock's ticks (every N ms, on-value).
- a vat's mailbox (every incoming message).
- a Canvas's input events (every click, keystroke).
- a reactive Signal (every emission).

**what's NOT a stream** in the same sense:
- a Table's entries (it's eager; just iterate).
- a Set's elements (same — eager collection).

the distinction is **temporal vs spatial** — a stream arrives
over time; a collection is already there.

---

## streams compose

the real win. same combinators work on any Streamable:

```moof
(def first-10-clicks
  [canvas-clicks take: 10])

(def warm-prices
  [[prices filter: |p| [p > 0]] map: |p| [p * 1.1]])

(def idle-state
  [[keystrokes throttle: 500ms] debounce: 200ms])

(def merged
  [audio-stream merge: subtitle-stream])
```

the same `map:`, `filter:`, `take:`, `throttle:`, `debounce:`,
`merge:` work across every stream source. the transducer-flavored
story is the backbone.

---

## streams and Iterables

a Cons is both:
- Iterable (you can `fold:with:` it eagerly — materialize).
- Streamable (you can `next:` it incrementally — consume).

same value, two protocols. context picks which to use: a
rendering pipeline wants Streamable (lazy); a sum wants Iterable
(materialize, fold).

most stream combinators delegate to Iterable when the source is
finite. when it's infinite, you MUST use streamable semantics or
you'll never terminate.

---

## streams and effects

cross-vat sends are inherently stream-shaped: messages arrive
over time in a mailbox. moof's mailbox IS a stream, and
server-vat patterns are stream-processors:

```moof
(defserver Counter (initial)
  { value: initial
    [incr] (update { value: [@value + 1] } @value)
    [get] @value })
```

from the outside, the vat processes a stream of incoming
messages, producing a stream of replies. the defserver form
hides this — but the scheduler is implementing it.

future direction: expose this explicitly. `[my-vat messages]`
returns the mailbox as a Streamable. you can inspect the
backlog, apply filters, process in user code. for now it's
handled by the scheduler but the mental model is stream-based.

---

## backpressure

today moof's streams are pull-based. the consumer drives; the
producer yields on demand. this naturally avoids unbounded
buffering.

push-based streams (where the producer pushes at its own rate and
buffers queue up) are a future addition. the right design
introduces explicit backpressure: the consumer signals "slow
down" and the producer complies (or drops).

reactive signals (`lib/flow/reactive.moof`) are approximately
push-based but without formal backpressure yet. this is on the
list to harden when streams-as-first-class gets a wave.

---

## streaming and persistence

a persistent stream is just a stream whose producer's state is in
the image. a log can be a stream. an append-only message history
is a stream. close moof, reopen, the stream picks up where it
left off.

this connects with wave 10+ (running-state persistence): vats'
mailboxes ARE streams, and persisting them is persisting the
stream's state. reboot resumes consumption.

---

## the canvas as stream consumer

the (future) canvas is a huge stream consumer:

- input events (mouse, keyboard) — a stream.
- frame ticks (60 Hz) — a stream.
- animations — functions of time, which is itself a stream.
- updates from subscribed peers — a stream.
- log messages — a stream.

morphic's historical design had an event loop; moof's version
has streams as first-class values you can compose. the visual UI
is a functional composition of these streams, not a callback
soup.

---

## streams and pipelines

moof's `flow/` directory has several stream-adjacent types:

- **Transducer** — composable reducer transformations. apply to
  any Iterable or Streamable.
- **Stream** — lazy sequence with a `next:` producer.
- **Signal** (reactive) — time-varying value, push-based.
- **Atom** (reactive) — cell you can observe changes on.

per the stdlib doctrine, transducer is the primary PIPELINE
primitive. streams are the source/sink. signals and atoms are
the push-based layer.

---

## differences from rxjs / reactive-streams / rust iterators

- **unlike rxjs**: moof streams are values (not subjects); they
  compose by message sends (not method chains on a
  special observable type).
- **unlike reactive-streams**: moof's backpressure is pull-based
  by default; we'll add explicit push-based stream types with
  backpressure as a separate layer, not a refactor.
- **unlike rust iterators**: moof streams are first-class moof
  values, not a zero-cost compile-time abstraction. they're
  slower than rust iterators but universally composable.

---

## status

today's moof has:
- `lib/flow/stream.moof` — Stream type with basic combinators
- `lib/flow/transducer.moof` — the composable primitive
- `lib/flow/reactive.moof` — Atom, Signal
- the stdlib doctrine pins Transducer as the primary pipeline
  primitive

gaps:
- push-based stream semantics with formal backpressure
- stream-as-mailbox (exposing a vat's mailbox as a stream
  value)
- stream persistence (as part of running-state persistence, wave
  11+)
- unified Streamable protocol (today Stream is a type, not a
  protocol — should be abstracted)

---

## what you need to know

- streams are a primitive design pattern: temporal flow of
  values.
- anything that produces values over time is stream-shaped:
  clocks, mailboxes, sensors, log files, canvas events.
- same combinators (map, filter, take, throttle, merge) work on
  every stream source.
- streams compose with transducers (the pipeline primitive).
- moof's design applies unix's stream-centric composition to
  typed values, not just text.
- push-based + backpressure is future work.

---

## next

- [effects.md](effects.md) — Act chains are a stream-shaped
  computation.
- [vats.md](vats.md) — mailboxes as streams.
- [../vision/horizons.md](../vision/horizons.md) — how the
  canvas consumes streams.
