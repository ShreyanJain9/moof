# vaus-pure: working notes

**status:** mid-implementation. partially working, blocking bug not yet
isolated. WIP changes are unstaged on master at the time of writing —
revert with `git checkout -- crates/ lib/tools/definitions.moof` to get
back to clean. design doc is `docs/concepts/vaus-pure.md`.

this is a written-mid-pass note for the next session, not a polished
doc. the goal is to hand off enough context that whoever picks this up
doesn't repeat the dead ends.

---

## the goal, restated

match Kernel to a T:

1. `$e` is a real `Env` value — the caller's actual environment, not
   a slot-snapshot.
2. `(eval form $e)` is a single swap-path in `Op::Eval`. defs land in
   $e. reads walk $e's chain. one semantics.
3. `wrap` exists, takes a vau, returns an applicative.
4. `fn = (wrap (vau …))` — fn is derived, not a separate primitive.

the design is `docs/concepts/vaus-pure.md` (already committed). this
note covers the implementation gap between the design and reality.

---

## why this turned out to be hard

the design is short. the implementation has fought back because moof's
existing model has two features that conflict with Kernel-pure
semantics, and they BOTH fire at the same call sites:

**(a) register-based locals.** moof's compiler binds positional params
and let-locals to register slots, not to env entries. fast, but it
means a runtime-compiled form (the inside of `(eval form $e)`) can't
see those locals via the env chain — they're not in env at all.

**(b) closures-carry-env (`:__scope`).** every closure captures
`heap.lexical_scope` at MakeClosure time. when an applicative is later
invoked, push_closure_frame swaps `heap.env` and `heap.lexical_scope`
to the closure's `:__scope`. this is how `[bundle apply: target]`
achieves real isolation.

these two features compose fine when the only `(eval form X)` call site
is `Bundle apply: target`. they fight the moment a vau body says
`(eval form $e)` with `$e = caller's env` — because:

- if `$e` is the caller's actual env, locals aren't in it, so
  free-var lookup inside the form fails (problem from (a))
- if `$e` is a wrapper that holds locals, then closures created
  during the eval capture the wrapper as their `:__scope`, and the
  wrapper is transient — those closures end up rooted in a dead env
  (problem from (b))

every patch i tried put pressure on one side or the other; none of them
escaped the underlying tension.

---

## what i tried and what i learned

### attempt 1 — wrapped `$e` with locals as overlay bindings

`Op::MakeOpEnv` builds `{ Env parent: heap.env, bindings: locals }`.
the operative call site emits this as `$e`. `Op::Eval` swaps to `$e`
on the existing real-env path.

- **works:** caller-locals visible inside (eval form $e).
- **breaks:** `(defn helper (forms) [forms each: |f| (eval f)])`
  invocation. helper's `:__scope` becomes the transient operative
  `$e` because `Op::Eval` swap sets `heap.lexical_scope = target`,
  and helper's MakeClosure happens inside that swap. when (helper …)
  is later called, heap.env enters that dead env; defs land in it
  and never reach the user's scope.

### attempt 2 — overlay marker on `$e` to skip lexical_scope update

added a `__overlay` slot to MakeOpEnv-built envs. `Op::Eval` reads it;
when set, swaps `heap.env` but leaves `heap.lexical_scope` alone.

- **fixes:** the helper case from attempt 1.
- **breaks:** do-notation. `(do (x <- (Some 5)) [m + x])` —
  the transformed form's `(fn (x) [m + x])` closure must capture an
  env where `m` is reachable. with lexical_scope NOT updating to
  $e, the closure captures the OUTER scope (which doesn't have m),
  so the body fails with `unbound: 'm'`.

i then tried "update lexical_scope only when overlay has bindings,"
which made do-notation work but still left helper-style code broken.
the overall pattern: any heuristic on top of these two fights another
heuristic somewhere else.

### attempt 3 (current) — locals-in-env

the real fix, per the user's "match Kernel to a T" steer. drop the
overlay model entirely. instead:

- on every applicative `push_closure_frame`, allocate a fresh per-call
  Env: `parent = closure.scope`, `bindings = positional params +
  captures`. set `heap.env` and `heap.lexical_scope` to this env.
- the operative call site stops synthesizing $e at all; it emits a
  new `Op::CurrentEnv` that loads `heap.env` directly. `$e` IS the
  caller's actual env (which now has locals, because applicatives
  bind them at frame entry).
- `Op::Eval` simplifies to a single swap (no parent re-chaining,
  which was a workaround that also caused infinite loops when
  `target` was an ancestor of `heap.env` — that's a separate bug
  fixed in this attempt).

this is the closest to Kernel proper. locals are env entries; eval
walks the env; closures capture real scopes. no overlay tricks.

---

## what's working under attempt 3

- bootstrap completes (after fixing the parent re-chain cycle in
  `Op::Eval`)
- `(defn foo (x) [x * 2])` then `(foo 5)` returns 10
- `(defn outer (n) (defn inner () n)) (outer 7)` returns 7
- `(defn with-do (m) (do (x <- (Some 5)) [m + x])) (with-do 10)`
  returns 15
- `(defn make-greeter (greeting) (defmethod String greet …) …)` works
- `(bundle-from (list '(defn xyz () 42)))` returns a Bundle
- `(defn wrapper () (bundle …))` then `(wrapper)` returns a Bundle

## the live bug — FIXED

was: `(bundle …)` at top-level failed silently. wrapped in `(defn
wrapper () (bundle …))` worked. bundle-demo broke. no error printed,
script continued returning Ok but all subsequent forms silently
failed (heap.env was corrupted; `console` resolution went sideways
quietly).

**root cause: TailCall didn't update frame.saved_env.** when an
operative tail-called an applicative (bundle's body is `(bundle-from
forms)` — a tail call, applicative-bound), TailCall set
`heap.env`/`heap.lexical_scope` to bundle-from's per-call env but
left the frame's `saved_env: None` (carried over from the operative
push). on Return, `saved_env: None` → no restore → heap.env stayed
pointing at the dead per-call env. subsequent top-level forms
inherited the corruption.

inside a defn wrapper this didn't manifest because the wrapper's
applicative frame had `saved_env: Some(_)`, which the tail-call
chain preserved correctly.

**fix (vm.rs Op::TailCall):** when the replacing call is an
applicative AND the frame's `saved_env` is `None`, capture
`heap.env`/`heap.lexical_scope` into the frame *before* changing
heap.env. that way Return restores correctly. if `saved_env` is
already `Some(_)`, leave it — it points at the outermost env to
restore to, and a tail-call chain shouldn't disturb that.

invariant: a frame's `saved_env` should be the heap.env to restore
to on Return, regardless of how many tail-calls have replaced the
frame's body.

after the fix:

- top-level `(bundle (def x 1))` works, subsequent forms run
- `(def x (bundle …))` at top-level works
- bundle-demo end-to-end clean
- definitions / materialize / bundle-from / bundle-merge all work

## (historical) suspected paths — for the postmortem

i suspected either:

1. **top-level frame setup.** `execute()` pushes a top-level frame
   without my new env-allocation, so when the bundle vau dispatches
   bundle-from (an applicative call), the saved_env mechanic might
   not interact right with operative dispatch from a top-level
   frame. specifically: top-level frame has `saved_env: None`, so
   `Op::Return` from bundle-from skips heap.env restore. that's
   fine in principle (return value is what matters), but if some
   other path expects heap.env to be in a specific state after a
   return, it'd silently misbehave.
2. **a lurking issue in the per-call env's interaction with the
   already-existing `closure_captures` / scope-set logic** that
   only manifests when the operative is called outside a fresh
   per-call env. i didn't bisect this far.

i did NOT instrument this enough to know which one. the right next
move is to add `eprintln!` inside `push_closure_frame` and `Op::Return`
to log the heap.env transitions across the failing call, and compare
to the working call.

(post-fix note: it was neither — see "the live bug — FIXED" above.
the actual cause was a TailCall not updating saved_env. instrumenting
push/return/tail/eval and diffing the working vs failing trace pointed
straight at it. past-me's advice to bisect via instrumentation rather
than reading code was, predictably, correct.)

---

## the SEMANTIC shift this introduces

even when the technical bug is fixed, attempt 3 changes user-visible
semantics in a real way:

- **defs are now LOCAL by default.** `(def x 5)` inside a function
  body lands in the call's per-call env — it disappears when the
  call returns. this matches Kernel.
- moof's old behavior was "defs are global." many existing patterns
  rely on this. the bundle apply mechanism is the canonical example:
  `[b apply]` evaluates each form with `(eval f)` (no env), expecting
  defs to land in vat root. with locals-in-env, they land in apply's
  per-call env and disappear.
- **fix:** anywhere we want defs to land in caller's scope, take an
  explicit env target. `[b apply: target]` already exists; i changed
  `(definitions …)` to use `[b apply: $e]`. there are probably more
  call sites that need similar updates.

a complete pass would audit every `(eval form …)` in lib/ to make sure
the env target is what the user expects. expected breakages:

- pattern-matching `with-bindings` in `lib/flow/pattern.moof` —
  intentionally puts pattern bindings in a temporary env. its eval
  pattern was already correct under attempt 1's design but worth
  re-checking.
- defserver / defprotocol — kernel vaus that bind names. i changed
  defprotocol once (in a discarded attempt) to drop $e from the def
  eval; with attempt 3 this might no longer be needed (since $e IS
  now caller's actual env, eval against $e binds in caller's env).

---

## current code state

modified files (unstaged on master):

- `crates/moof-core/src/heap/mod.rs` — added `make_env(parent, names,
  values)` helper
- `crates/moof-lang/src/opcodes.rs` — added `Op::CurrentEnv` (0x62)
- `crates/moof-lang/src/vm.rs` —
  - added `Op::CurrentEnv` handler
  - rewrote `push_closure_frame` (applicative path allocates per-call
    Env with positional params + captures bound)
  - rewrote `Op::TailCall` (mirrors the new `push_closure_frame`,
    plus the saved_env capture-on-applicative-tail-call fix described
    above)
  - simplified `Op::Eval` real-env path (removed parent re-chain to
    avoid cycles)
- `crates/moof-lang/src/lang/compiler.rs` — `compile_operative_call`
  emits `Op::CurrentEnv` instead of `Op::MakeObj` for `$e`
- `lib/tools/definitions.moof` — `(definitions …)` uses `[b apply: $e]`

next steps for whoever picks this up:
1. **(eval form …) audit.** rg every `(eval ` in lib/ and triage
   each call site for "where should this def land?" — defs are now
   local-by-default (matches Kernel), so anywhere old code expected
   defs to escape into vat root needs an explicit env target. known
   call sites worth checking:
   - `lib/flow/pattern.moof` `with-bindings` — pattern bindings into
     a temporary env, intentionally
   - defserver / defprotocol kernel vaus — with `$e` now being
     caller's actual env, the prior `$e`-drop hacks may be obsolete
2. **wrap + fn unification.** revised plan: don't make Applicative a
   heap variant or `is_operative` a flag. instead, `wrap` is a moof
   vau that returns an operative whose body is "eval my operands,
   then call the wrapped target," i.e. an applicative is "an
   operative that happens to evaluate its args first." `fn` is a
   moof primitive operative whose body invokes wrap+vau. core knows
   only operatives. (this is what `lib/kernel/bootstrap.moof`
   already does for `fn`; the rust side just needs to drop
   `is_operative` from Closure once everything routes through it.)
3. **other latent bugs.** reactive-demo and `moof -e` currently fail
   independently of this fix (verified pre-existing). reactive-demo:
   "0 does not understand 'call:'". worth fishing out next.

if the next session wants a clean slate:
`git checkout -- crates/ lib/tools/definitions.moof`

---

## smaller things i'd do differently in retrospect

- **don't try to keep both old and new $e semantics simultaneously.**
  the overlay marker / "update lexical_scope conditionally" path is
  a tar pit. attempt 3 commits to one model.
- **the parent re-chain in Op::Eval is a footgun.** it was added
  earlier to make outer-name lookup work during isolated evals; it
  creates a cycle when `target` is itself an ancestor of `heap.env`.
  removed in attempt 3 — should stay removed.
- **bisect via instrumentation, not by reading code.** i wasted real
  time tracing call paths in my head when an `eprintln!` would have
  pointed at the bug in 30 seconds.

---

## related

- `docs/concepts/vaus-pure.md` — design doc
- `docs/concepts/scope.md` — closures-carry-env background
- `docs/concepts/vaus.md` — original (pre-pure) vau doc, now mostly
  superseded
