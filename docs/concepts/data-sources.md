# data sources

> **the universal i/o protocol. anything producing or consuming
> values over time speaks DataSource. lazy by default. composable
> through standard combinators. if in doubt, it's a data source.**

a data source has both an *out* (read side) and an *in* (write
side); most concrete sources lean on one direction. the abstraction
unifies streams, channels, iterators, files, sockets, mailboxes,
journals, query results, timers — *everything*.

## the protocol

```moof
(defprotocol DataSource

  ;; output side — consumer reads
  [next]                ; → next value, or #eof
  [next-or: default]
  [peek]                ; → next without consuming
  [done?]               ; #true when no more values

  ;; input side — producer writes
  [emit: value]         ; push a value
  [close]               ; signal end-of-stream
  [error: exn]          ; signal error

  ;; lifecycle
  [pause]
  [resume]
  [drain])              ; consume to completion → final value
```

every DataSource implementation supports the methods that make sense
for its kind. file-read sources implement `:next` but not `:emit:`;
file-write sinks the reverse. channels implement both.

## combinators (return new DataSources)

```moof
[ds map: f]                      ; lazy transform of values
[ds filter: pred]
[ds take: n]
[ds drop: n]
[ds chunk: n]                    ; group into batches of n
[ds buffer: capacity]            ; pre-load up to N
[ds tee: other-ds]               ; duplicate to a sink
[ds merge: other-ds]             ; interleave two sources
[ds zip: other-ds]               ; pair with another
[ds throttle: 100ms]
[ds debounce: 50ms]
[ds window: 10 by: 1]            ; sliding window
[ds catch: handler]              ; error recovery

;; consuming
[ds reduce: f from: init]        ; consumes; returns final
[ds for-each: blk]               ; consumes; invokes blk
[ds drain-into: sink]            ; consume + emit-on
[ds collect-into: Table]         ; consume + materialize
```

## the `(pipe …)` form

for chains where each stage operates on the result of the previous:

```moof
(pipe (file-at: "/notes.txt")
  [lines]
  [filter: |l| [l contains: "moof"]]
  [map: |l| [l upcase]]
  [take: 10]
  [for-each: |l| [println l]])
```

`pipe` is an operative. each subsequent form takes the previous
result as its receiver. the visual top-to-bottom mirrors
data-flow.

## what speaks DataSource

| thing | as a DS |
|---|---|
| **`$out`, `$err`** | sink-only. canonical "print" path is `[$out say: x]`. no separate console api exists. |
| files | read/write |
| sockets | read/write |
| keyboards | input events |
| screens | output drawing commands |
| timers, intervals | periodic emissions |
| random generators | endless source |
| iterators / sequences | yes |
| query results (datalog) | tuples stream lazily |
| **vat mailboxes** | inbox = sink; outbox = source |
| **journals** | the WAL is a DS — reads replay; writes commit |
| pub-sub topics | many-to-many |
| atom change notifications | source of (new, old) pairs |
| compiled-object loader | source of forms-on-load |
| process stdin / stdout | yes |
| async result handles | yes (one value, then done) |
| **Tables, Lists, Strings** | iteration as a DS |

if it produces or consumes values, it's a data source. this is the
*if in doubt, it's a data source* heuristic.

## laziness

DataSources are *pull-based by default*. consumers ask for the next
value; producers respond. nothing happens until pulled. this makes
chains of combinators cheap (no intermediate materialization).

push-based wrappers (subscriber-style) exist for cases where the
producer drives, but they wrap pull-based primitives:

```moof
[stream subscribe: |v| (handle v)]
;; internally: spawns a fiber that pulls and dispatches.
```

## infinite sources

a DataSource subclass with this contract:

- `:done?` always returns `#false`
- `:close` is a no-op
- `:next` always succeeds (no eof)

two flavors share the same conformance test except in their `:peek`
discipline:

- **polled** (Clock-like): `:next` reads environment state. `:peek`
  is `:next` (idempotent; no internal state to manage). examples:
  Clock, atom-watch, mouse-position.
- **generator** (Random-like): `:next` advances internal state and
  returns. `:peek` stashes one value to return on next `:next` (or
  computes-one-step-ahead — implementation chooses). examples:
  Random, id-mints, fibonacci sequences.

both pass `assert-infinite-source` (in `lib/stdlib/data-source.test.moof`).
combinators (`:take:`, `:for-each:`, `:throttle:`, `:ticks:`) work
on either flavor uniformly:

```moof
[Random take: 10]               ; → Cons of 10 fresh values
[Clock ticks: 1s]               ; → stream that emits clock value once per second
```

protos that conform declare `:infinite-source #true` as a meta-slot;
moof-side default methods in `lib/stdlib/data-source.moof` provide
`:done?` and `:peek` defaults.

## backpressure

backpressure is automatic for pull-based chains (slow consumer ⇒
slow producer). for push-based scenarios, sources can declare a
capacity:

```moof
[stream buffer: 1024]            ; buffer up to 1024 then push back
[stream throttle: 10/s]          ; rate-limit production
```

## errors

errors travel on a separate channel from values. `:error:` sets the
error state; downstream consumers see the error on next `:next`.
`:catch:` recovers:

```moof
(pipe ds
  [map: |x| (parse-int x)]       ; might raise on bad input
  [catch: |err| (default 0)]
  [reduce: + from: 0])
```

## composing with vats

since vat mailboxes are data sources, vats and streams compose
freely:

```moof
;; tap a vat's inbox without modifying it
(let log-source [(vat 'shreyan) inbox])
(pipe log-source
  [tee: log-file-ds]             ; record everything
  [for-each: |msg| (process msg)])
```

```moof
;; route stream into a remote vat
(pipe sensor-readings-ds
  [filter: |r| [r value > 100]]
  [for-each: |r| [remote-monitor alert: r]])
```

## composing with persistence

journals are data sources:

```moof
(let history [(vat 'shreyan) journal])
(pipe history
  [filter: |entry| [entry timestamp > yesterday]]
  [map: 'event]
  [for-each: print-event])
```

time-travel = read-journal-from-position-N. replay = drain-journal-
into-receive. (`concepts/time-and-journal.md`.)

## composing with queries

query results are data sources:

```moof
(let alice-relatives (query (ancestor 'alice ?z)))
(pipe alice-relatives
  [take: 10]
  [for-each: println])
```

datalog queries lazy-evaluate; pulling the next tuple computes only
as needed.

## why one universal protocol

the alternative is a zoo of separate abstractions: streams,
channels, iterators, observables, ports, futures, signals. each
with their own combinators. each with their own conversion mess.

one protocol gives us:
- one mental model.
- one set of combinators.
- one inspector view.
- free composition across all the things.

we accept the cost (a uniform interface won't be optimal for any one
use case) because the benefit (cohesion) is enormous and the cost is
small in practice.

## inspirations

- unix pipes: thompson and ritchie. the original proof that one
  abstraction can serve.
- erlang ports / processes: armstrong et al.
- haskell pipes / conduit / streaming: gabriel gonzález, michael
  snoyman. the "two-channel for values + errors" idea.
- clojure transducers: rich hickey (~2014). composition is the goal.
- rxjava / rxjs: the observable abstraction (less faithful to ours
  because they default push-based).
- io's coroutines.

## see also

- `concepts/vats.md` — vat mailboxes are data sources.
- `concepts/persistence.md` — journals are data sources.
- `concepts/queries.md` — query results are data sources.
- `concepts/tables.md`, `concepts/lists.md`, `concepts/strings.md` —
  collections-as-data-sources.
