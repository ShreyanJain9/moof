# vaus and functions

**type:** concept
**status:** analyzed (kept dual; documented why)

> moof has two callable forms: `(fn ...)` and `(vau ...)`. they
> share 95% of the compiler path and produce the same Closure
> shape with one differentiating flag. is the duality necessary?
> can we collapse to a single primitive plus a wrap derivation?
> short answer: **the duality is real, not redundant.** the
> compiler dedup is straightforward; the runtime dual is
> structural and we keep it.

## the two forms

```moof
(fn (x y) [x + y])              ; applicative
(vau (x y) $env [x + y])        ; operative
```

both produce a Closure: the same heap shape (code_idx, arity,
captures slots, `is_operative` flag). the compiler paths are
nearly identical — same chunk allocation, same captures
analysis, same MakeClosure emit. the differences:

- **vau** has an extra param (`$env`, must start with `$`) for
  the caller's environment.
- **vau**'s `is_operative` is `true`.
- the **call dispatch** at runtime differs (next section).

## what changes at runtime

when the VM dispatches `[v call: args]`:

- if `v` is **applicative** (`is_operative: false`): args are
  evaluated, each result bound to a positional param.
- if `v` is **operative** (`is_operative: true`): the args list
  is passed UNEVALUATED as a single value, plus the caller's
  current scope as `$env`.

a vau body sees `args` as a cons-cell of forms. it can `(eval
each-form $env)` selectively. that's what makes operatives
useful for macro-like behavior — a vau receives forms and
decides what to do with them.

an applicative body sees its params as already-resolved values.
it can't unevaluate them; the forms are gone.

## kernel's collapse — and why it doesn't translate

in shutt's [Kernel](https://web.cs.wpi.edu/~jshutt/kernel.html),
operatives are primitive and applicatives are derived:

```scheme
(wrap operative)  →  applicative
```

`wrap` produces a new applicative that, when called with values
v1..vn, calls the underlying operative with v1..vn as
already-evaluated values. the operative's body sees them as
values, not forms.

**this works in Kernel because operatives there are designed to
be wrappable.** their bodies treat `args` as a list and don't
necessarily call `eval` on each element.

**moof's vaus aren't shaped this way.** typical vau bodies in
moof do things like:

```moof
(def myform (vau args $e
  (let ((name [args car])
        (val (eval [[args cdr] car] $e)))
    ...)))
```

they `eval` selected forms with `$env` because args ARE forms.
if you wrapped this with Kernel-style `wrap`, args would arrive
as values, but the body would still try to `eval` them — which
is wrong; you'd `(eval <value>)` and get either an error or a
double-evaluated mess.

so a moof `wrap` that derives an applicative from an arbitrary
vau **doesn't compose with how vaus are actually written here**.
to make wrap work, vau bodies would need a different convention
(don't call eval; treat args as values). that's a different
design.

## what we keep

1. **two callable surfaces**: `(fn ...)` and `(vau ...)`. their
   semantic difference is real — eval-args-eagerly vs
   pass-forms — and that distinction shows up at the call site,
   not just the storage shape.
2. **one Closure type underneath**: code_idx, arity, captures,
   `is_operative`. nothing changes.
3. **the compiler is duplicated and could be deduplicated.** both
   paths follow the same shape (extract params, sub-compile body,
   emit MakeClosure). pulling out a common helper would shrink
   compiler.rs by ~80 lines without changing semantics. that's a
   straightforward refactor we can do whenever.

## what we don't build

- **`wrap` as a moof primitive**. doesn't compose with how
  moof's vaus are written; would force a different vau-body
  convention.
- **a single-form callable surface** (e.g. drop `fn`, derive
  `(fn ...)` from a vau macro). same reason — can't be done
  cleanly without changing how every vau in the codebase is
  written.

## what we could explore later

- a **type-flag in the closure handler** that lets the call:
  dispatch fast-path either case via the same code (already true
  internally; the surface dual is just two macros). we could
  expose `is-operative` as a slot users could read, which already
  exists.
- a **vau-builder** function in moof that constructs operative
  closures without the compiler's special form, for late-bound
  meta-programming. probably not needed; just write `(vau ...)`.

## related

- `crates/moof-lang/src/lang/compiler.rs:306-385` (vau)
- `crates/moof-lang/src/lang/compiler.rs:387-465` (fn)
- `lib/kernel/bootstrap.moof` — most kernel definers are vaus
  that splice unevaluated args into quasiquoted forms and eval
  the result.

## decision

keep both. docs (this file) describe the distinction. compiler
dedup is queued as a small mechanical refactor for whenever the
two paths feel like they're drifting.
