# capabilities

> **effects in moof are passed as arguments. a function with no
> capability parameter is pure by construction. capabilities are
> unforgeable Forms; the only way to get one is to be handed it.**

this is the **e / mark miller** model (PhD thesis, johns hopkins
2006), with the smaller-surface lessons from pony (clebsch et al.
AGERE! 2015) and newspeak (bracha). monads are not how we get
purity; capability-passing is.

## the convention

a parameter whose name starts with `$` is a capability. the dollar
sigil is the *only* way the substrate marks effect-relevance. user
code can pass other Forms named `$foo` and the substrate doesn't
care; the convention is for human readers and the analyzer.

```moof
(def square |x| [x * x])         ; pure — no $cap

(def log-now |$out $clock|       ; impure — receives caps
  [$out say: [$clock now]])
```

## the rules

1. **no capability is created by user code.** the substrate gives the
   root supervisor the primordial caps at boot. all other caps come
   from those, by attenuation.
2. **caps cross vat boundaries the same way values do.** a far-ref
   to a remote `$out` is itself a `$out`-shaped cap; cross-machine
   caps are handed across the wire as part of message envelopes
   (`concepts/references.md`).
3. **caps are first-class Forms.** they have protos, slots, methods,
   identity. they can be inspected, named, persisted (within their
   originating vat).
4. **a function whose signature contains no `$param` is pure** for
   the purposes of analysis: the analyzer marks it `#pure`. the
   substrate may memoize, reorder, parallelize, or cache pure
   functions safely.

## attenuation

a capability can produce a smaller version of itself by sending it a
message:

```moof
(let safe-fs [$fs restricted-to: #Path "/users/shreyan/notes"])
;; safe-fs is itself a $fs cap, but read/write are confined.
```

attenuation is just method dispatch on the cap. the smaller cap is a
new Form; the original is unchanged. capability attenuation is a
design pattern (e tradition), not a substrate feature.

## the caps that exist by default

a fresh world's root supervisor has access to a small set of
primordial caps, which it then hands out as needed:

| name | does | leaf substrate calls |
|---|---|---|
| `$clock` | wall time, monotonic time, schedule timers | timer leaf |
| `$random` | bytes / numbers from CSPRNG | random leaf |
| `$out`, `$err` | stdout / stderr text | console leaf |
| `$fs` | filesystem read/write | file leaf |
| `$net` | open sockets, listen, connect | network leaf |
| `$keyboard` | input event stream (a data source) | input leaf |
| `$screen` | output drawing surface | screen leaf |
| `$proc` | spawn other processes | process leaf |
| `$registry` | named lookup in the world's namespace | path-resolver |
| `$spawn` | spawn new vats | scheduler |

most user code receives one or two caps per function. caps that take
a lot of state (file handles, sockets) are themselves caps that you
can `:close` and never use again.

## what a "pure" function can do

a `#pure` function:

- read its arguments.
- read constants from its lexical scope.
- send messages to its arguments (which themselves may be impure;
  but invocation transitively passes the caps along — purity is
  *contagious through the cap-flow graph*).
- allocate Forms in its vat's heap (allocation is not a side-effect
  in our model — it's a value-construction).
- invoke other `#pure` functions.

a pure function *cannot*:

- read or write state outside its lexical scope.
- perform i/o.
- spawn vats.
- send to far-refs (cross-vat) — those require the routing scheduler,
  which is a cap.

## the analyzer's role

the type/effect analyzer (which is moof code; see `concepts/types.md`)
walks the Form-graph and tags every method with one of:

- `#pure`
- `#effectful: <ordered-list-of-cap-types>`
- `#unknown` (when the analyzer cannot determine)

these tags are `meta` annotations on the method's Form. the
inspector shows them. the substrate uses them for safe optimizations
(memoization, parallelism). the user can ascribe them explicitly to
double-check.

## why not monads

three reasons:

1. **legibility.** `def f |$out| ...` says "this function does i/o
   via $out" in one line. `IO` monad transformers say it in five
   lines plus a where-clause.
2. **smalltalk-shaped object model.** capabilities *are* objects
   receiving messages. the same primitive (send) handles ordinary
   computation and i/o. nothing special.
3. **effects compose like values, not like wrappers.** if you have
   two caps, you take them as two args. you don't `lift` and
   `bind` and `runEither`. the simpler thing is the right thing.

## inspirations

- e and the capability discipline: mark s. miller, *robust
  composition* (PhD thesis, johns hopkins 2006).
- the practical capability vocabulary: pony (clebsch et al. AGERE! 2015).
- modules-as-cap-bundles: newspeak (bracha).
- the `$name` sigil convention: shell scripting and (loosely) erlang's
  process registry. moof's spin: `$` always means capability.
- the contagious-purity-through-cap-flow framing: pony's reference
  capabilities and rust's borrow-flow analysis (loosely).

## see also

- `concepts/references.md` — how caps cross vat boundaries.
- `concepts/types.md` — effect rows in type signatures.
- `laws/purity-and-effects.md` — formal rules.
- `laws/isolation-laws.md` — vat isolation and cap unforgeability.
