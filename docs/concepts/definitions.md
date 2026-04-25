# definitions

**type:** concept
**status:** in-progress (Wave 11)

> a `.moof` file is an order-independent **bundle of definitions**,
> not a script that runs top-to-bottom. forms declare what they
> bind and what they need; the loader figures out the rest.

---

## why this exists

moof's source files have always looked declarative — sequences of
`def`, `defn`, `defmethod`, `defprotocol`, `defserver` — but they
were evaluated imperatively, so order mattered. that imperative
load order leaks into:

- **the bootstrap dance.** kernel files are arranged in dependency
  order by hand. add a new helper somewhere and the build snaps.
- **source-as-canonical-value awkwardness.** a definer like
  `defmethod` has to fish for its own source text after the form
  is already running. the form is a side-effect; the value is
  reconstructed from runtime crumbs.
- **the inability to talk about a module.** there is no `module`
  value — only the side effects of loading one.

if files are declarative bundles, all three soften:

- **bootstrap** can re-sort the kernel by analyzing each form's
  free variables; you stop fighting load order and start letting
  the system route around you.
- **source** is intrinsic: each definition carries the bytes that
  declared it, so `[v source]` is a lookup, not a reconstruction.
- **modules** become first-class — a bundle is a value with
  inputs (free vars) and outputs (bindings). later moves
  (federation, namespace-as-value) drop into that shape.

this doc covers the smallest viable cut: the **definitions form**.
the file-level rewrite, namespace-as-value, and incremental
recompile are downstream of this; not addressed here.

---

## the definitions form

```moof
(definitions
  (defn quadruple (x) [(double x) * 2])  ; ← appears first, but
  (defn double (x) [x * 2])              ; ← runs first
  (def base 10))
```

semantics: take all enclosed forms (unevaluated). compute, for
each form, the symbols it **produces** (binds globally) and the
symbols it **consumes** (refers to as free variables). build a
dependency graph. topologically sort. evaluate in dep order.

if a consumed symbol is already bound in the enclosing env (e.g.
a kernel function), it's not a dependency on this bundle — just
a normal closure-time lookup.

if a consumed symbol is neither bound globally nor produced by
the bundle, that's a **missing dependency** error — clear at the
top of the load instead of buried in a stack trace.

cycles between definitions are reported as errors with the cycle
spelled out. (mutual recursion uses `fn`-bound captures, not
top-level cycles, so this is a real problem to flag.)

---

## what counts as a definition

forms whose semantic role is "extend the namespace" — either by
binding a new global, or by extending an existing prototype:

| form          | binds (produces)        | extends         | notes                                  |
|---------------|-------------------------|-----------------|----------------------------------------|
| `def`         | the name                | —               | value is any expression                |
| `defn`        | the name                | —               | desugars to `(def name (fn ...))`      |
| `defprotocol` | the name                | —               | clause symbols are not produced        |
| `defserver`   | the name                | —               | constructor closure                    |
| `defmethod`   | —                       | a proto         | requires the proto                     |
| `conform`     | —                       | a proto         | requires both proto and protocol       |
| `alias`       | —                       | a proto         | requires the proto                     |

forms outside this set (bare expressions, `(println ...)`, etc.)
**are rejected by `definitions`** — they're side effects, not
declarations, and their order does matter. side effects belong
in scripts or interfaces, not bundles.

---

## free-variable analysis

`deps-of` walks a form's AST and returns the set of symbols it
references freely — symbols that would be looked up at runtime,
minus those bound by enclosing `fn` / `vau` / `let`, minus
language built-ins.

**binders** that subtract from the free set:

- `(fn (params...) body...)` — params bind in body
- `(vau args $env body...)` — args, $env bind in body
- `(let ((n1 v1) (n2 v2) ...) body...)` — names bind progressively;
  v2 sees n1, body sees all

**non-contributors**:

- inside `(quote x)` — nothing is free; symbols are data
- inside `(quasiquote x)` — nothing is free **unless** wrapped in
  `(unquote ...)` or `(unquote-splicing ...)`, in which case the
  unquoted expression contributes
- selector positions in `[recv sel args]` — `sel` is a symbol but
  it's a handler name, not a binding
- slot names in `obj.field` and `{ field: value }` — names are
  literal, not vars
- form heads like `def`, `defn`, `let`, `if` themselves — these
  are special forms or globally-bound vaus the kernel provides

the analyzer is intentionally conservative: when in doubt about
whether a symbol is free, treat it as free. an extra dependency
edge is a perf concern, not a correctness one. missing one is
a correctness bug.

---

## what's deliberately not here yet

- **file-level conversion.** `definitions` is a form you opt
  into. `.moof` files are still imperative sequences. once
  the form is solid, we'll add a file-level mode that wraps
  the whole file implicitly.
- **namespace-as-value.** Wave 9. a bundle becomes a value
  you can pass around, save to the image, send across vats.
- **incremental refresh.** edit one definition; recompile only
  it; rebind, leaving everything else alone. needs the value
  side to be solid first.
- **mutual recursion across top-level definitions.** today,
  use `fn` captures. cross-definition cycles are an error.

---

## see also

- `concepts/objects.md` — what gets defined
- `concepts/protocols.md` — `defprotocol` / `conform` semantics
- `reference/syntax.md` — the form syntax reference
