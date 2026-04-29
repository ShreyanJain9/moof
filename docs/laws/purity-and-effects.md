# purity and effects

> **a function is pure iff it receives no `$cap` argument and
> performs no operation that requires one. the substrate uses
> `#pure` tagging for safe optimization.**

## P1 — pure means: no cap input, no cap usage

a function `f` is `#pure` iff:

1. its parameter list contains no `$cap`-prefixed names, AND
2. its body never sends to a far-ref (cross-vat communication is a
   capability), AND
3. it does not allocate a vat (`$spawn`-using), AND
4. it does not write to a slot of any Form not allocated within
   this call (no slot-write to a captured-mutable cell, no global
   mutation), AND
5. it only invokes other `#pure` functions (purity is contagious by
   call-graph).

if any of these is violated, `f` is *not* pure. the analyzer must
either:
- mark it `#effectful: <list-of-caps>` if the effects are knowable, or
- mark it `#unknown` if not (e.g., higher-order calls to functions
  whose purity is not yet known).

## P2 — purity ascription is checkable

a user can ascribe `:: #pure` on a function. the analyzer must verify
the body satisfies P1. if not, the analyzer raises a *purity
violation*, naming the specific reason.

ascription failures are reported with source-loc; the user fixes
the function or removes the ascription.

## P3 — substrate may safely transform pure code

if the substrate (or a future optimizer) sees a `#pure` function, it
may:
- memoize results (cache on (function, args)).
- reorder calls relative to other pure calls (e.g., for parallelism).
- evaluate calls speculatively.
- skip evaluation entirely if the result is unused.

these transformations are sound because pure functions have no
observable effect besides their return value.

## P4 — capabilities are received, not constructed

within a function body, the only way to get a `$cap` is to:
1. have it as a parameter.
2. retrieve it from a slot of a Form passed as an argument (which
   itself must have received the cap from somewhere).
3. attenuate an existing cap.

there is no `(make-cap …)` constructor available to user code. the
substrate's only constructors are at boot (root supervisor's
primordial caps) and via attenuation.

## P5 — capability attenuation preserves the protocol

`[cap restricted-to: …]`, `[cap readonly]`, `[cap with-timeout: …]`,
etc. — all return a new cap of the *same protocol* as the original.
client code that takes a `$fs` cap can use either the full cap or
an attenuated version interchangeably (modulo the attenuation's
restrictions taking effect).

## P6 — effect rows are part of type

a function's type signature mentions caps:

```moof
:: ($Console, $Clock, Integer) → Unit
```

the analyzer infers cap-rows from the signature when present, or
from the body when absent. the inferred row appears in
`[m caps-required]` reflection.

## P7 — caps are pure values from outside

receiving a cap as a parameter is *not* itself an impure operation.
calling a method on a cap is. so:

```moof
(def safe-prefix |$out|             ; takes a cap; not impure yet
  (let prefix [Self prefix])         ; pure work
  prefix)                            ; → returns prefix without using $out
```

this is `#effectful: ($Console)` *only because the signature mentions
$out*. the *body* doesn't actually use it. the analyzer can choose
strict (signature is the truth) or strict-bodily (only mark caps the
body actually uses) — we lean *signature-is-the-truth* because
ascription should reflect intent.

## P8 — exceptions are not effects

raising an exception within a vat is *pure-compatible* (no
external effect, no other-vat effect). so:

```moof
(def divide |a b|
  (if [b = 0]
      (raise 'division-by-zero)
      [a / b]))
```

is `#pure`. raising changes control flow but not external state.

## P9 — allocation is not an effect

allocating Forms (creating new objects, blocks, tables) is pure. it
*does* mutate the vat's heap, but the user observes only the
returned value; the mutation isn't observable elsewhere.

(this is the standard erlang/clojure/ml move: allocation in a managed
heap doesn't count as an effect.)

## P10 — IO requires a cap, full stop

every IO operation (file, network, screen, keyboard, clock, random,
spawn) goes through a cap. there is no "ambient global" $out, $clock,
etc. each function that wants to do IO declares it in its signature.

(this is the e / capability discipline. miller PhD thesis 2006.)

## diagnostic: when in doubt

if you think you have a pure function:

1. read its signature. any `$param`? → `#effectful` (at least
   nominally; per P7).
2. read its body. any `[far-ref ...]` send? → `#effectful`.
3. read its body. any slot-write to a Form not allocated here? →
   `#effectful`.
4. read its callees. any non-pure? → not pure (by P1.5).

## inspirations

- e's effect discipline: miller (*robust composition* 2006).
- pony's reference capabilities: clebsch et al.
- haskell's purity discipline (different mechanism, same goal):
  peyton-jones et al.
- newspeak's modules-as-cap-bundles: bracha.

## see also

- `concepts/capabilities.md` — narrative.
- `concepts/types.md` — effect rows in signatures.
- `laws/isolation-laws.md` — vat-level cap rules.
