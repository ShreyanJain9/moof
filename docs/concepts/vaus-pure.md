# vaus, pure-kernel-style

**type:** concept (design)
**status:** designed, not built. supersedes most of `vaus.md`.

> moof's vaus today are a hybrid in two ways:
>
> 1. `$e` is a slot-snapshot of caller's locals (not a real Env).
> 2. `fn` and `vau` are independent primitives — fn is not derived
>    from vau via `wrap`.
>
> Kernel proper does it cleaner: `$env` is the caller's actual
> environment, and the only operative-constructor is `vau`;
> applicatives (`fn`-shaped things) are produced by `wrap`-ing a
> vau. moof should match. with closures-carry-env (wave 11.7–11.8)
> the substrate is finally there to do it.

---

## the two problems

### problem 1: `$e` is a slot-snapshot, not a real Env

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

### problem 2: `fn` is a separate primitive, not `wrap (vau …)`

in Kernel there is exactly **one** way to build a callable:
`vau`. it is operative — args arrive unevaluated, plus `$env`.
applicatives (the ones that auto-evaluate args) are made by
applying `wrap` to a vau. `lambda` is just sugar for
`(wrap (vau args $env (eval-each-and-bind args $env body)))`.

moof today has `fn` as its own compiler form, with its own body
shape (positional params, no `$e`), and its own dispatch
(applicative). the call protocol differs between fn and vau:
fn passes evaluated values bound to N positionals; vau passes
the cons of raw forms as one positional plus `$e` as another.
two body shapes. two dispatch paths. two compiler entries.

this isn't *wrong*, but it's missed elegance — and crucially,
without `wrap`, users can't construct an applicative from an
operative. they have to write the pre-evaluation by hand:

```moof
; what users do today:
(def my-op (vau args $e (eval [args car] $e)))
(def my-fn (fn (x) [my-op call: (list x)]))
; what they should be able to do:
(def my-fn (wrap my-op))
```

without `wrap`, vau-style abstraction stops at the operative
boundary; applicatives can't be derived. Kernel has this. moof
should too.

---

## why the obvious fix to problem 1 doesn't quite work

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

## the cleaner design — `$e` is a real env

**make `$e` a real Env that extends the caller's actual scope**.
the operative call site builds:

```
{ Env parent: heap.env bindings: <Table of caller's locals> }
```

i.e. a fresh Env *whose parent is the caller's read scope*, with
the caller's currently-live locals pre-bound. `(eval form $e)`
SWAPs `heap.env` to it (the only path — INJECT goes away).
free-var lookup during the eval walks: `$e`'s bindings → caller's
heap.env → its parent → … all the way up to vat root. defs into
`$e` land in `$e`'s bindings table; defs that target the *caller's*
scope (what `defn` actually wants) eval their final form WITHOUT
swapping, so `(def name val)` lands in `heap.env` directly.

new closures created during the eval get `:__scope =
heap.lexical_scope`, which wave 11.8 already keeps pinned to the
caller's lexical chain — so the transient `$e` doesn't poison
closure capture.

this preserves every property the inject path had:

- caller's local names visible to eval'd code (now via parent
  walk, not slot-injection)
- defs land where the user expects (caller's env, when the body
  evals against `heap.env` directly; into `$e`'s bindings, when
  the body evals against `$e`)
- `$e` is a real walkable, queryable Env value — `[$e at: 'x]`,
  `[$e names]`, `[$e count]` all just work via the moof-side
  Env protocol

and gains:

- `$e` has identity. it can be stashed, returned, walked.
- `(eval form $e)` is one path. INJECT branch deletes.
- compile_operative_call emits `MakeOpEnv` (a small new opcode)
  instead of `MakeObj` + slot fill — same shape, different
  parent-chain semantics.

---

## the cleaner design — `fn = wrap (vau …)`

with `$e` honest, `wrap` becomes implementable as a small
runtime primitive that takes an operative closure and returns
an applicative one. shape:

```moof
; the kernel, conceptually:
(defn wrap (op)
  (fn (& args)
    [op call-with: args env: (current-env)]))
```

i.e. wrap produces an applicative whose body, when invoked with
already-evaluated args, calls the underlying operative passing
those args plus the caller's env. the operative sees them as a
list of values (not forms) — which is fine, because vaus that
intend to be wrapped don't call `(eval x $e)` on their args;
they just use them.

`fn` then desugars at the surface level:

```
(fn (x y) body)
  ≡  (wrap (vau (x y) $_ body))
```

the underlying vau has positional params, `$_` as ignored env,
and the same body. wrap flips dispatch to applicative, so x and
y arrive as values. body just uses them.

### two ways to implement, one chosen

**option A: shared closure shape, one flag.**
both `fn` and `vau` compile to a Closure with positional params
plus a final `$e` slot. flag `is_operative` selects dispatch:
operative passes `(cons args nil)` + caller's env to the last two
positionals; applicative pre-evaluates args, binds to first N
positionals, fills `$e` with caller's env. `wrap` is a one-line
native: clone closure, flip flag.

cost: every applicative call now passes an extra arg (`$e`).
small overhead. body shape unifies. `wrap` is trivial.

**option B: wrap is a moof-level adapter.**
`fn` keeps its current applicative-only shape. `wrap` returns a
*new* fn whose body is `[op call-with-env: args env: (current-env)]`.
extra indirection per wrapped call, but no compiler change.
needs one new primitive: `(current-env)` → `Value::nursery(heap.env)`.

**we go with A.** option B leaves fn as a separate compiler
entry, which keeps the duplication problem the user is trying to
remove. option A makes `fn` literally `(wrap (vau …))` at the
shape level — the user's mental model and the implementation
agree. the per-call `$e` overhead is one register slot; we already
pay it for vaus.

### what about vau bodies that DO call `(eval x $e)`?

those are macros. they're written to be operatives, called raw,
with `$e` actually being an env they swap into. wrapping such a
vau gives garbage — args arrive evaluated, then the body tries
to eval them again, double-evaluating. that's a user error, not
a `wrap` problem; same as Kernel.

vaus written in the wrappable style (just-use-args, ignore `$e`)
are fine to wrap. vaus written in the macro style aren't. moof
docs need to say so.

---

## the migration

**phase 1.** add `Op::MakeOpEnv` (real-Env build at operative
call site, parent = caller's heap.env, bindings = locals
snapshot). switch `compile_operative_call` to emit it. update
`Op::Eval` to take only the swap path; drop INJECT.

**phase 2.** verify each kernel vau (defn, defmethod,
defserver, defprotocol, when, unless, and, or, do) still works
under the new shape. each form is its own test surface; bisect
via `cargo test -p moof-lang vau` and the bundles demo.

**phase 3.** unify `fn` and `vau` body shape. compile both to
Closure(positional + `$e`-last). dispatch branches on
`is_operative`. fn's source-level form is preserved; the user
writes `(fn (x y) body)` exactly as today.

**phase 4.** add `wrap` as a native primitive: clone closure,
flip `is_operative=false`. add `unwrap` (Kernel symmetry: extract
the operative from an applicative).

**phase 5.** docs: this file becomes the canonical vau spec;
`vaus.md` is replaced. `scope.md` notes that the inject path is
gone. `roadmap.md` checkbox.

---

## open questions

- **arity check for wrapped vaus.** if the underlying vau is
  `(vau (x y) $e body)`, arity = 3 internally. as an applicative,
  the user calls it with 2 args; dispatch fills `$e` after.
  needs the arity check to subtract 1 when `is_operative=false`.
- **defserver, defprotocol — do they still work?** they're vaus
  whose body builds a class object via `(eval form $e)`. with
  $e as a real env, defs land in $e's bindings. the macro then
  needs to extract bindings from $e and install them on the
  class. that's mechanical but not free.
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
- the syntactic surface — `(vau args $e body)` and `(fn (params)
  body)` — stays. only what `$e` is bound to changes, and only
  fn's *internal* shape unifies with vau's.
- closures keep `:__scope`. the scope chain from wave 11.7–11.8
  is the substrate this whole proposal rides on.

---

## sequencing

phases 1+2 are one coherent landing (real-env $e plus kernel-vau
audit). phase 3+4 are the fn=wrap+vau unification — separate
landing because the dispatch change touches arity logic. phase 5
is docs+roadmap cleanup at the end.

estimate: ~3 sessions. the big risk is phase 2 — kernel vau
patterns that implicitly rely on inject's "caller-locals visible
as globals" behavior. wave 11.8's lexical_scope/heap.env split
is supposed to make this clean, but it needs verifying on the
real kernel forms.

---

## related

- `docs/concepts/vaus.md` — the older doc. now mostly
  superseded; its "we don't build wrap" section is the
  conclusion this doc reverses.
- `docs/concepts/scope.md` — closures-carry-env design.
  describes the substrate this proposal rides on.
- Kernel report ("Revised⁻¹ Report on the Kernel Programming
  Language", Shutt 2005) — primary source on `$vau`,
  `wrap`/`unwrap`, and the operative/applicative split.
