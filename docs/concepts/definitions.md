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

## Bundle — a definition bundle as a value

`(bundle ...)` is the same machinery as `(definitions ...)` but
it stops one step earlier — instead of applying the forms it
returns a **Bundle value** you can pass around, introspect, save
to the image, or apply later.

```moof
(def my-mod (bundle
  (defn quad (x) [(double x) * 2])
  (defn double (x) [x * base])
  (def base 10)))

[my-mod typeName]    ; → 'Bundle
[my-mod count]       ; → 3
[my-mod provides]    ; → (base double quad)  — declared exports
[my-mod requires]    ; → ()                   — declared imports
[my-mod apply: Env]  ; → binds base/double/quad into Env, returns nil
```

shape:

| slot       | meaning                                           |
|------------|---------------------------------------------------|
| `forms`    | the list of definition forms in dependency order  |
| `provides` | global names the bundle binds                     |
| `requires` | free symbols the bundle expects from outside      |

a Bundle is the seed for namespace-as-value: a value that knows
its own interface (what it gives, what it needs) and can be
applied into any compatible env. once we have first-class
namespaces (a fresh env you can target), `[bundle apply: ns]`
materializes a module without polluting the global env.

cycles surface as `{ Err message: "cycle in definitions ..." }`
instead of a Bundle, so callers can branch on `[result is: Err]`.

### composition

bundles compose:

```moof
(def math (bundle
  (defn square (x) [x * x])
  (defn cube (x) [(square x) * x])))

(def stats (bundle
  (defn variance (xs)
    (let ((m [[xs sum] / [xs count]]))
      [[xs map: |x| (square [x - m])] sum]))))

[stats requires]               ; → (square)        — needs help
(bundle-satisfies? math stats) ; → true            — math has it

(def all (bundle-merge math stats))
[all provides]                 ; → (square cube variance)
[all requires]                 ; → ()              — internally closed
```

three combinators:

| op                              | meaning                                      |
|---------------------------------|----------------------------------------------|
| `(bundle-merge b1 b2 ...)`      | concatenate forms, re-analyze; cycles → Err  |
| `(bundle-satisfies? prov cons)` | true iff prov.provides ⊇ cons.requires       |
| `[b1 equal: b2]`                | structural equality (forms + provides + requires) |
| `[b content-hash]`              | content-addressed hash; same-content dedupes |

### identity through the image

bundles round-trip through `.moof/store`. save one in run 1:

```moof
(def my-mod (bundle (defn double (x) [x * 2]) (def base 10)))
```

restart moof, run 2:

```moof
[my-mod provides]      ; → (base double)
[my-mod apply: Env]
(double 5)             ; → 10
```

forms, provides, requires — all preserved across image boundaries
via the existing object serialization (cons cells already
serialize). bundles are values, and the image holds them.

## Env — the namespace value

moof's namespace type is **Env**. there isn't a separate
"Namespace" type — namespace, environment, the global scope
that VM lookups walk, and the value `[bundle materialize]`
produces are all the same shape: an Env. one type, many roles.

an Env is an Object with two slots: `parent` (the outer scope,
or nil at the root) and `bindings` (a Table mapping symbol →
value). `at:` walks the parent chain on miss. `bind:to:`
mutates the bindings table. plus the usual namespace ops:

```moof
(def fresh [Env new])
[fresh bind: 'k to: 42]
[fresh at: 'k]                  ; → 42
[fresh has?: 'k]                ; → true
[fresh names]                   ; → (k)
[fresh count]                   ; → 1
[fresh walk: "/k"]              ; → 42
[fresh union: other-env]        ; merge other's bindings in
[Env new: fresh]                ; child env, falls through to fresh
[Env]                           ; the global env (singleton)
```

`Env` is the **prototype** — the Type, like Object/Cons/Set/etc.
`[Env new]` and `[Env new: parent]` are class-side constructors;
`(defmethod Env x ...)` adds methods to the type. there is **no
user-facing name for "the global env"** — top-level defs land in
the runtime's current scope implicitly, and `[bundle apply]` (no
arg) targets that scope. eliminating the singleton's name makes
"what scope is this binding going into?" a real question to
think about, instead of a silent default.

**`[bundle materialize]`** returns an Env populated with the
bundle's bindings:

```moof
(def math (bundle
  (defn square (x) [x * x])
  (defn cube (x) [(square x) * x])))

(def math-env [math materialize])
[math-env typeName]              ; → 'Env
[math-env names]                 ; → (square cube)
((math-env at: 'cube) 4)         ; → 64
[math-env walk: "/cube"]         ; → the cube fn
```

today's caveat: **materialize is not isolated**. applying a
bundle still rebinds in the global env first; the fresh env is
populated by reading those globals back. real isolation needs
a VM-level `eval-into-target` op so `DefGlobal` lands in a
specific env instead of global. when that lands, materialize's
contract stays the same; only the implementation tightens.

an Env is:
- inspectable: name → value bindings, queryable
- composable: `[a union: b]` merges
- addressable: `at:` and `walk:` (plan-9 shaped)
- context-linked: `parent` chains for lexical fallback
- serializable: it's an Object with a Table; the image holds it

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

## file-level conversion (the MVP path)

today, a `.moof` file opts in by wrapping its body in a single
`(definitions ...)` form:

```moof
; lib/tools/inspect.moof — definition bundle

(definitions

  (def Inspector { ... })
  (defn aspects args (aspects-build #[] args))
  (defn aspects-build (acc pairs) ...)
  (defmethod Object inspect () ...)
  ...
  (conform Inspector Showable))
```

constraints:

- `(definitions ...)` is provided by `lib/tools/definitions.moof`.
  files that load BEFORE that one (the kernel, lib/data, the
  imperative parts of lib/tools) cannot use bundle mode yet.
- the bundle is one form, so the file's whole content lives at
  one indentation level. comments, whitespace, and section
  banners still work inline as expected.

bundles confirmed working in tree (all stage 11.0):

- `lib/tools/inspect.moof`
- `lib/tools/query.moof`
- `lib/tools/test.moof`

## what's deliberately not here yet

- **per-file pragma**, e.g. a leading `;; #bundle` comment that
  the loader recognizes and wraps automatically. saves one
  level of indentation. the current `(definitions ...)` wrap is
  a workable substitute until we have enough bundles to justify
  the loader change.
- **bundle-mode for early-load files** (kernel + early lib/data).
  needs a Rust-side analyzer in the loader so bootstrapping
  doesn't depend on moof-side definitions.moof being loaded
  first.
- **namespace-as-value.** Wave 9. a bundle becomes a value you
  can pass around, save to the image, send across vats.
- **incremental refresh.** edit one definition; recompile only
  it; rebind, leaving everything else alone. needs the value
  side to be solid first.
- **mutual recursion across top-level definitions.** today, use
  `fn` captures. top-level cross-definition cycles are an error.
  self-recursion in a single defn IS allowed (filtered out of
  deps automatically).

---

## see also

- `concepts/objects.md` — what gets defined
- `concepts/protocols.md` — `defprotocol` / `conform` semantics
- `reference/syntax.md` — the form syntax reference
