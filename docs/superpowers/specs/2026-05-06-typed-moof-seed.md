# Typed moof — design seed

> **status: SEED. not a spec yet — a placeholder so the idea doesn't
> get lost. expand into a full spec when we're ready to design it
> properly. brainstormed in passing during the V0 ship session.**
> **date: 2026-05-06**

## the idea

a haskell-like typed language as a **frontend** that compiles to
moof bytecode and runs in moof vats. types are a *compile-time*
discipline that gates which programs are acceptable; at runtime,
there are no types — it's still ordinary moof bytecode dispatched
through ordinary protos in ordinary vats.

motivation: shreyan wants the haskell-style guarantees (sum types,
pattern matching, classes, total functions, type-checked effects)
for serious moof code, while keeping moof's moldability and
homoiconicity for live REPL / scratchpad work.

## why this is interesting alongside the vat architecture

pure-mode vats (the cap-free, frozen-by-default profile sketched
in `2026-05-04-vats-and-references-protocol-design.md` §4 + §13)
give you the *runtime* for haskell-like execution semantics for
free. typed moof would give the *language* aspect: compile-time
guarantees that complement the runtime's pure-mode discipline.

a typed-moof program checked at compile time + executed in a
pure-mode vat = the closest moof gets to "haskell with capability-
secure federation."

## the four orientations (from the brainstorm)

| option | what | cost | character |
|---|---|---|---|
| **(a) sister lang** | distinct front-end, distinct file ext (`.hsmo`?), haskell-style indented syntax, compiles to moof bytecode | ~12 months | typed lang for "serious" code; moof for moldable / REPL |
| **(b) typed dialect of moof** | stays in sexpr; optional type annotations on `def`/`defmethod`/`fn`; coalton-style; gradual adoption | **~6 months** | "typed moof" — homoiconic, typed, capability-secure, federated |
| **(c) successor** | typed lang becomes canonical "real moof"; current moof becomes the untyped escape hatch | ~18 months | idris-supersedes-haskell move; cultural shift |
| **(d) full replacement** | typed lang has its own everything; moof gets deprecated | ~24+ months | "let's start over" — loses moof's identity |

**lean is (b)** — preserves moldability, doesn't fork the toolchain,
gives the most aligned pitch ("homoiconic, typed, capability-secure,
federated"). but the call is shreyan's when this gets real.

## the design dimensions to settle when this gets real

1. **type system depth.** HM → HM + classes + ADTs → HM + classes
   + ADTs + GADTs → refinement → linear → dependent. lean: HM +
   classes + ADTs + GADTs (modern haskell, pre-dependent).

2. **effects model.** monolithic IO monad (haskell) vs. algebraic
   effects (koka, eff, effekt) vs. capability-as-effect (fits
   moof's cap model perfectly). **lean: capability-as-effect** —
   the cap-bag becomes the effect row, naturally tying back to
   the vat layer. the type of `[$out write: text]` includes the
   `$out` cap in its effect row; pure functions have empty rows.

3. **evaluation.** lazy needs thunks the moof VM doesn't have
   natively; strict-by-default with explicit `lazy` is much more
   aligned. lean: strict.

4. **syntax.** sexpr with annotations (coalton-style, fits (b)
   directly) vs. haskell-style off-side rule (fits (a) better).

5. **compile target.** moof bytecode (cleanest integration, runs
   anywhere moof runs — including federated vats) vs. wasm via
   mco (independent, but splits the runtime). lean: moof bytecode.

6. **interop with untyped moof.** strict separation, `:any` escape
   hatch, or gradual typing (typescript-style). gradual is most
   aligned with (b).

## what this is NOT

- **not substrate work.** the typed lang is a frontend that
  produces moof bytecode. the substrate doesn't even know types
  exist. this can be designed and implemented in parallel with
  the vat phases (V0–V11), not blocking them.
- **not a vat-mode.** vats execute moof bytecode regardless of
  whether the bytecode came from typed moof or plain moof. a
  pure-mode vat is fine running output from either.
- **not a runtime check.** types are gone after type-checking.
  the bytecode that reaches the VM has no type information; the
  type system's job is to prevent bad bytecode from being emitted
  in the first place.

## relationship to the vat phases

independent. typed moof can be designed alongside or after V1–V11.
the only coupling is that pure-mode vats (a refinement of the
spec's `:frozen-by-default` mode + empty cap-bag) give typed-moof
code a particularly clean execution profile. that's a runtime
spec extension worth folding into V8 (supervision + spawn) when
the time comes; the typed-lang frontend is its own workstream.

## what to do next, when we get to this

1. fresh brainstorm session — pin down (a) vs (b), confirm the
   six dimensions above.
2. write a real spec doc in `docs/superpowers/specs/` (parallel
   to the vat spec).
3. plan in phases (parser → type checker → bytecode emitter →
   stdlib in typed moof → tooling).
4. implement in parallel with vat phases, in its own worktree.

## see also

- `docs/superpowers/specs/2026-05-04-vats-and-references-protocol-design.md`
  §4 (vat mode), §13 (capabilities), §22 (phase ordering).
- `docs/concepts/types.md` — early v2 thinking about types in moof
  (probably needs a refresh once this gets real).
- `docs/concepts/capabilities.md` — the cap model that effect-rows
  would attach to.
