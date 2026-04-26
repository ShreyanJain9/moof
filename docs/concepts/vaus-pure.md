# vaus, pure-kernel-style

**type:** concept (design)
**status:** designed, not built. supersedes parts of `vaus.md`.

> moof's vaus today are a hybrid. they receive a slot-snapshot
> `$e` of the caller's locals, and they call `(eval form $e)`
> which INJECTS those slots into `heap.env` so the eval'd code
> can see them. that's a workaround — Kernel proper just makes
> `$e` the caller's real environment, and locals naturally live
> there. the hybrid was the practical move while moof's
> infrastructure was being built; with closures-carry-env in
> wave 11.7–11.8, it's now possible to retire it.

---

## what's wrong with the current shape

a vau like `(do body)` runs:

```moof
(def do (vau body $e
  (eval (%do-transform body) $e)))
```

at runtime, `$e` is a synthesized `Object` — a plain heap object
whose **slots** are the caller's local registers, snapshotted at
the call site. when `(eval form $e)` runs, the VM iterates `$e`'s
slots and `env_def`s each into `heap.env` (the inject path),
runs the form, then removes the injected slots. the form's free
vars resolve correctly because, during the eval, the caller's
locals are visible as globals.

this is a real wart for several reasons:

1. **`$e` lies about what it is.** users see "the caller's env"
   in the docstring but get a slot-snapshot that doesn't have
   the same identity, can't be queried as an env, doesn't walk
   a parent chain, and gets discarded after the call.
2. **defs into `$e` go to the wrong place.** `(def x 5)` inside
   a vau-eval lands in `heap.env`, not in `$e`. the user can't
   ask `$e` "what was just defined" — `$e` has no bindings of
   its own.
3. **the inject path is parallel to the swap path** in
   `Op::Eval`. real-env eval (e.g. `[bundle apply: target]`)
   takes the swap branch. operative `$e` eval takes inject.
   two distinct semantics riding the same opcode.
4. **mutability asymmetry.** swap-eval can mutate the target
   (defs land in target.bindings); inject-eval can't (defs land
   in caller's heap.env, restored after).

Kernel sidesteps all of this by making `$env` the caller's
*actual* environment — a real, walkable, mutable Env value.
`(eval form $env)` swaps `heap.env` to it, defs land in it,
reads walk its chain. one path. one semantics.

---

## why the obvious fix doesn't quite work

the surface fix: make `$e` a real Env. construct it at the
operative call site as `{ Env parent: <caller's heap.env>
bindings: <table-of-locals> }`.

i tried this. it broke `(defn outer () (defn inner () 7))`:

- `outer` is compiled. `(defn outer ...)` is an operative call.
  the compiler builds `$e_outer` (a real Env with locals=∅,
  parent=vat root) and dispatches defn.
- defn's body: `(eval (fn (defn inner () 7)) $e_outer)`. real-env
  swap. `heap.env = $e_outer`. compile + run `(fn ...)`.
- `MakeClosure` produces `outer-closure` with `:__scope =
  heap.env = $e_outer`. WRONG — outer is lexically in vat root,
  not in the transient `$e_outer`.
- when `(outer)` is later called, `heap.env` swaps to `$e_outer`.
  `(defn inner ...)` runs with `heap.env = $e_outer`. defn's
  final `(eval (def inner …))` writes to `heap.env = $e_outer`.
  `inner` lands in a transient env that the user can't reach.

wave 11.8 fixed this by **separating `heap.env` (read scope)
from `heap.lexical_scope` (where new closures capture)**. the
real-env swap path updates both; the inject path leaves
lexical_scope alone. closures created during inject-style eval
get scope = caller's heap.env, not the transient operative env.

with that, the obvious fix becomes possible. but it requires
more than just changing `compile_operative_call` — see below.

---

## the cleaner design

**make `$e` the caller's actual `heap.env`**, passed by
reference. operative call sites stop synthesizing a slot-snapshot
or a transient real env; they just hand over `Value::nursery(heap.env)`.

what changes:

1. `compile_operative_call` emits `[heap.env-as-value]` as the
   `$e` arg, not a `MakeObj` of locals.
2. `(eval form $e)` always SWAPs `heap.env` to `$e`. but if `$e`
   is already `heap.env`, the swap is a no-op.
3. caller's locals are NOT visible during vau-eval. references
   in the form to outer locals would fail at runtime compile
   unless they're in `heap.env`.
4. vau bodies that need locals must declare it explicitly —
   probably by force-capturing the names ahead of time, or
   by an explicit env-extension primitive.

**this last constraint changes how do-notation is written**.
`%do-transform` produces forms that reference bind-names like
`forms`, `result`, etc. — all introduced INSIDE the do, so
they're not "caller's locals" in the problem sense. the
transform should compose without needing inject.

a clean Kernel-style do:

```moof
(def do (vau body $e
  ; transform produces self-contained forms; outer locals
  ; that the user references are force-captured into the
  ; closure WRAPPING this do call at compile time.
  (eval (%do-transform body) $e)))
```

`(eval form $e)` swaps `heap.env` to caller's actual env.
defs would land in caller's env (which is what makes `defn`
work). reads walk caller's chain.

for the rare case where a vau wants to *read* a caller's local
name: that's still possible if the name was force-captured into
the surrounding closure, because the closure already has a slot
for it that resolves at compile time of the do call. the
runtime-eval'd transform just needs to reference the name and
the surrounding closure's capture provides it via heap.env's
chain (after closures-carry-env, the closure's `:__scope`
already includes the relevant chain).

i need to verify this last claim by inspecting concrete cases
before implementing — see "open questions" below.

---

## the migration

**phase 1.** add a way for `compile_operative_call` to emit
`heap.env-as-value` as `$e`. doesn't replace the existing
behavior yet — adds an alternate path. one operative form
(probably `do`) opts in as a test.

**phase 2.** switch each kernel vau (defn, defmethod, defserver,
defprotocol, when, unless, and, or, do) to use the real-env
`$e`. each switch should be a separate, testable change.

**phase 3.** drop the slot-snapshot mode from
`compile_operative_call`. drop the INJECT branch from `Op::Eval`.
single path: real-env swap.

**phase 4.** docs: this file becomes the canonical vau spec;
`vaus.md` gets updated or replaced. `scope.md` notes the
unification finally landed.

---

## open questions

- **does force-capture cover all the locals-from-vau-body
  patterns?** examples: do-notation referencing outer
  locals (yes, captured), defmethod whose body references
  outer locals (rare but real), pattern-matching vaus that
  query outer locals.
- **what happens when a vau body does `(eval form (env))` with
  a fresh env, not `$e`?** that's a Kernel idiom — eval in a
  clean env. with our design it'd just work — `(env)` returns
  some real env, eval swaps to it.
- **`(def x 5)` inside a vau body, no eval — does it mean
  "bind x in caller's env" or "bind x in the global root"?**
  Kernel: caller's env. moof currently: heap.env (which under
  closures-carry-env is the calling closure's scope, which IS
  the caller). consistent — should just work.
- **performance.** a per-call slot-snapshot allocation goes
  away. that's a small win. but force-capture pre-captures
  may need to be more aggressive — slight compile-time cost.
  not load-bearing either way.
- **image roundtrip.** vaus saved before this change have
  bytecode that emits a slot-snapshot for `$e`. on load,
  they'd still produce the old shape, which would fail when
  `Op::Eval` no longer has the inject branch. need to either
  keep both branches or migrate the bytecode. the cleanest
  move is migrating bytecode at load time, or keeping inject
  as a deprecated alternate path until images age out.

---

## why not just do compile-time macros?

a deeper alternative: **make vaus run at compile time, not
runtime**. this is what classic lisp does. the compiler
recognizes vau calls, expands them to forms, and recursively
compiles the result. no `$e` at runtime, no inject path, no
restoration dance.

cost: a major architectural shift. moof's vaus are full vau
values today — they're stored in env, redefinable at runtime,
serialized in images. compile-time macros are special-cased
at the compiler level. retrofitting is a lot of work, and it
takes away the runtime-redefine property which moof relies on
for live editing.

so: keep vaus as runtime values; just make `$e` the real
env. that's the move proposed here.

compile-time macros remain a longer-term option if the
runtime model proves insufficient. for now, the change is
"make `$e` honest" — small in spec, real in implementation.

---

## what's NOT being changed

- vaus stay first-class runtime values. you can `(def x (vau
  ...))` and rebind freely.
- the syntactic surface — `(vau args $e body)` — stays. only
  what `$e` is bound to changes.
- closures keep `:__scope`. the scope chain from wave 11.7–11.8
  is the substrate this design rides on; without it, the move
  would be impossible.

---

## sequencing

i'd estimate the migration at ~2 sessions of focused work,
with the bulk being:

- phase 1+2 as one coherent commit
- phase 3 (deletion + docs) as another
- testing across the kernel + bundle code in between

there's a real risk that some kernel vau patterns rely
implicitly on inject — i'd want to find them first. the audit
is part of phase 1.

---

## related

- `docs/concepts/vaus.md` — the older "we keep both fn and vau,
  Kernel's wrap doesn't apply to moof" doc. parts of it are
  superseded by this one. specifically: the section on "what
  we don't build" (no `wrap`) stays correct; the section on the
  current `$e` shape is the wart this doc proposes to fix.
- `docs/concepts/scope.md` — closures-carry-env design.
  describes the substrate this proposal rides on.
