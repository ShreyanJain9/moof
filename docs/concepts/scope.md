# scope (and why heap.env still exists)

**type:** concept (architecture note)
**status:** current limit, with named follow-ups

> the user-facing surface treats namespaces, envs, and the
> "current scope" as a single Type (Env). but the runtime still
> anchors everything on `heap.env` — a singleton-shaped field
> that DefGlobal writes to and GetGlobal reads from. eliminating
> that anchor — true scope-as-value at the VM level — turns out
> to require a deeper change than just rewriting Op::Eval, and
> this doc records why.

## what's true today

- `Env` is a Type. `[Env new]` constructs fresh first-class envs.
  `(defmethod Env x ...)` works.
- the runtime has `heap.env: u32` — the current scope's id.
  `Op::DefGlobal name reg` writes to `heap.env`'s bindings;
  `Op::GetGlobal name reg` reads from it (walking the parent chain).
- there is **no user-facing name** for the singleton. top-level
  defs land in it implicitly; `Bundle.apply` (no arg) targets it.
- `Bundle.apply: target` populates target via snapshot-and-copy:
  evaluate forms in the current scope, then read the produced
  bindings back and write them into target.
  defs **do** still pollute the current scope along the way.

## what we wanted

`Bundle.apply: target` to be **real isolation**: defs in the
bundle's forms should land in `target.bindings` directly, not in
the current scope. that would make Bundles into actual modules
you can apply without disturbing the surrounding namespace.

## what we tried

modified `Op::Eval` so a real-Env `env_arg` triggered SWAP
semantics: push `heap.env` to a save slot, set `heap.env = env_arg`,
run the form (so its DefGlobals land in target), restore on exit.
also chained `target.parent` to the outer scope during the eval
so READS in the form could still walk back to find globals
(`+`, kernel bindings, etc.).

this worked for top-level `def`s. it didn't work for `defn`,
defmethod, defserver, anything that produces a closure.

## why it broke

closures resolve free globals via `Op::GetGlobal` **at call time,
not creation time**. when `(defn h (x) [x + secret])` runs inside
`Bundle.apply: target`, the closure's body compiles to a body
that looks up `secret` via GetGlobal (because `secret` isn't a
parent local at compile time — defn is at the top level).

with swap-eval, `secret` is bound in `target.bindings` and the
closure value gets stored there. fine.

then later: `[t at: 'h] → h-fn`. user calls `(h-fn 8)` from the
script's top level. at this call site, `heap.env` is the script's
outer scope (not target). h's body runs and does GetGlobal for
`secret`. heap.env now is the outer scope, not target. lookup
fails. `secret` is unfound.

real isolation requires the **closure to remember its lexical
scope and restore it on every call**. that's "closures-carry-env."
not a tweak — it's a real VM addition: every closure stores a
scope value (an Env), and when it's invoked, the VM swaps to
that scope before running its body. the existing capture mechanism
(per-symbol slot copies) doesn't substitute for this because the
captured set is computed at compile time and globals aren't
captured.

## why we reverted

snapshot-and-copy is functional for the main use cases (browse
materialized bundle bindings, look up a value, occasionally call
a closure that only references kernel globals). swap-without-
closure-env-capture is *worse* — it breaks closure-call-from-
outside-target, which is the exact thing users want.

so until closures-carry-env lands, inject-style `Op::Eval` is the
correct semantics for `(eval form $e)`, and `Bundle.apply: target`
copies bindings explicitly. the user-facing model still says "Env
is the namespace type," and most apparent uses behave correctly.

## what would unblock real isolation

the architectural change is:

1. **closures store a `:scope` slot** — a reference to the env
   they were created in. construction sets it; serialization
   preserves it.
2. **Op::Call (or call: dispatch) swaps `heap.env` to the
   closure's `:scope` for the duration of the call**, restoring
   on return. analogous to how it already pushes/pops frames
   for register state.
3. **GetGlobal in closure bodies reads from the closure's
   scope chain**, which walks back to enclosing scopes
   correctly because each level remembers its own scope.

this turns moof's free-variable resolution from "look up in the
current scope at call time" to true lexical scope at the value
level. it's the standard scheme/lisp move; moof's prototype +
late-binding model has been postponing it because most code
worked without it.

estimated: 1–2 sessions of focused VM work. needs careful image-
roundtrip handling (closures must serialize their scope refs).

## what stays the same for users

after closures-carry-env lands:

- `[Env new]` still constructs fresh envs. nothing user-facing
  changes about how you build a namespace.
- `[bundle apply: target]` does what it always promised — defs
  land in target, isolation is real.
- closures defined inside an apply remember target as their
  scope. calling them anywhere works — they always know where
  to look.
- `[bundle apply]` (no target) keeps using the current scope.

so the surface stays; the insides tighten.

## related

- `crates/moof-lang/src/vm.rs:683` — Op::Eval handler. the
  comment block there spells out the inject-not-swap reasoning.
- `crates/moof-core/src/heap/mod.rs:281` — env shape (parent
  slot + bindings Table).
- `docs/concepts/definitions.md` — Env as the namespace type.
- `docs/concepts/vaus.md` — fn-vs-vau.
