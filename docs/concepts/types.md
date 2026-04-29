# types

> **types are Forms with a `:satisfies?` handler. the system is
> optional, gradual, and infinitely composable. everything from
> nominal to refinement to dependent types is built from one
> substrate hook.**

types in moof are values. you can pass them, store them, compute
with them, save them, query them. the type system is moldable code,
not a privileged compile-time mechanism.

## the substrate hook

a Type is any Form whose `proto` (transitively) is `Type`. the Type
proto requires one method:

```moof
(defproto Type
  (handlers
    [satisfies?: value]
      ;; → #true if value belongs to this type
      …))
```

everything else (intersection, union, refinement, dependent
parameterization, function types, structural records) is built on
this single hook. compositions are *moof code*, not substrate
features.

## the basic kinds

### nominal

every Form already has a `proto`. nominal type-checking is just
"is `proto` in this proto-chain?":

```moof
[5 :: Integer]                   ; type ascription / check
([Integer satisfies?: 5])        ; explicit
([Integer satisfies?: "x"])      ; #false
```

### refinement

a refinement is a base type plus a predicate:

```moof
(def Nat (Type refine: Integer where: |n| [n >= 0]))
(def Pos (Type refine: Integer where: |n| [n > 0]))
(def NonEmpty (Type refine: Iterable where: |xs| [[xs length] > 0]))

([Nat satisfies?: 5])            ; #true
([Nat satisfies?: -3])           ; #false
```

### structural

an open record shape:

```moof
(def HasPosition
  (Type structural: '(x: Number y: Number)))

([HasPosition satisfies?:
  #[x: 0 y: 0 color: 'red]])     ; #true — has x and y of right types
```

### function

```moof
(def IntToString
  (Type fn: '(Integer) → String))

([IntToString satisfies?: |n| [n to-string]])
```

### dependent

types that depend on values:

```moof
(def (Vec n T)
  (Type with: |v|
    [[v has-proto?: Table]
     and: [[v length] = n]
     and: [[v values] all?: |x| [T satisfies?: x]]]))

([(Vec 3 Integer) satisfies?: #[1 2 3]])   ; #true
([(Vec 3 Integer) satisfies?: #[1 2]])     ; #false
```

`Vec n T` is itself a function returning a Type. use it like any
other type.

### intersection / union

binary message operators:

```moof
(def NonEmptyList [List ∩ NonEmpty])
(def StrOrInt    [String ∪ Integer])
```

(unicode `∩` and `∪` are syntactic sugar; ascii `:and:` and `:or:`
also work.)

## type ascription

types are optional everywhere. you can ascribe:

```moof
;; in a def signature
(def fact :: (Nat) → Nat
  |0| 1
  |n| [n * (fact [n - 1])])

;; on a binding
(let total :: Number ← 0)

;; on a parameter
|n :: Pos| [n + 1]

;; inline
[v :: Comparable]                ; assertion / runtime check
```

unannotated code runs unchecked. the type system *does not* require
annotations to function — it gives more help when you provide them,
nothing more.

## effect rows

types can mention required capabilities:

```moof
(def-typed log-now :: ($Console, $Clock) → Unit
  |$out $clock|
  [$out say: [$clock now]])
```

the substrate distinguishes:
- pure functions (no `$cap` parameters) — the analyzer infers and
  tags them as `#pure`.
- effectful functions — tag is `#effectful: <list-of-caps>`.
- unanalyzed — `#unknown` (default).

(`concepts/capabilities.md` for the cap-passing model.)

## the analyzer

the type analyzer is **moof code**, not substrate code. it walks
`Form`-graphs of definitions, infers types and effects, and checks
ascriptions. you can:

- run it eagerly on save.
- run it on-demand when inspecting.
- run a *different* analyzer (e.g., one that's more liberal).
- write a domain-specific analyzer for a sub-language.

the analyzer is moldable, like everything else above the rust line.

## type-aware reflection

every type-form is browsable:

```moof
[Integer protocols]              ; protocols Integer implements
[Comparable implementors]        ; types that implement Comparable
[Integer method-source: '+]      ; the source of Integer's +
[Counter infer-protocols]        ; analyzer asks "you implement what?"
```

the inspector shows types as first-class artifacts.

## why not haskell-style static types

a few reasons:

1. **moldability requires runtime mutability.** smalltalk's
   class-edit-and-watch only works because the type system isn't a
   gauntlet you submit through. we keep the live-edit posture.
2. **gradual is what most users want.** unannotated code "just runs."
   annotations are an opt-in tool.
3. **effects via capabilities, not monads.** simpler to read,
   simpler to reason about, more consistent with our object model.

we keep the haskell *vocabulary* — pattern-matched clauses, type
ascription, structural protocols — and the *spirit* of "the shape
tells you what to do." we drop the type-erasure machinery and the
"compile or it doesn't run" discipline.

## inspirations

- typeclasses / structural protocols: haskell (peyton-jones et al.),
  clojure protocols (hickey).
- gradual typing: typed racket (tobin-hochstadt, felleisen),
  python's type hints (van rossum), sorbet.
- refinement types: liquid haskell (jhala et al.).
- dependent types: idris (brady), agda (norell), cayenne (augustsson).
- types-as-values: cayenne and modern dependently-typed languages.
- effect-rows-via-capabilities: e (miller), pony (clebsch), and
  newspeak (bracha).
- the *types are values, the system is moldable* posture: moof's own,
  but structurally similar to how clojure protocols are values you
  can manipulate.

## see also

- `concepts/forms.md` — Type is a Form with `:satisfies?`.
- `concepts/capabilities.md` — effect rows.
- `concepts/blocks-and-patterns.md` — type-guarded patterns.
- `laws/purity-and-effects.md` — formal effect rules.
