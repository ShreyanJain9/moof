# numbers

> **one number tower with multiple representations. integer (small-int
> + bigint), float, rational, complex. promotion is automatic. a
> single Number proto-tree is the parent of them all.**

```moof
1                                ; Integer
9999999999999999999999           ; Integer (bigint repr)
1.5                              ; Float
1/3                              ; Rational
3+4i                             ; Complex
0xff, 0b1010, 0o17               ; integer literal in base
1e9                              ; scientific
∞, -∞, NaN                      ; special floats
```

## the tower

```
Number
├── Real
│   ├── Rational
│   │   └── Integer            (special case: denom = 1)
│   └── Float                  (separately, not a sub-Rational)
└── Complex                    (sibling of Real)
```

`Integer ⊆ Rational ⊆ Real ⊆ Number ⊇ Complex`. (Float is a sibling
of Rational under Real; rational and float don't subsume each other
because float is approximate.) protos are real Forms; the chain is
inspectable.

## representation vs identity

`Integer` is one proto. its instances may be backed by:

- a small-int (NaN-boxed in a 48-bit slot, no heap allocation), or
- a bigint (heap-allocated, arbitrary precision).

both are the same proto; the substrate switches representation
transparently on overflow. user code never asks "is this a small-int
or a bigint." it asks "is this an Integer."

(this is the v3 BigInt-unification lesson, kept for v4.)

## promotion

binary ops promote toward the more general:

```moof
[1 + 1.5]                        ; → 2.5  (Integer × Float → Float)
[1/3 + 1/2]                      ; → 5/6  (Rational × Rational → Rational)
[1/3 + 1.0]                      ; → 1.333… (Rational × Float → Float)
[1 + 2i]                         ; → 1+2i (Integer × Complex → Complex)
```

promotion rules are defined as multimethods on the binary-op selector.
user code can extend them for custom numeric types (see
`concepts/types.md`).

## ops

every Number responds to:

- arithmetic: `+`, `-`, `*`, `/`, `mod`, `quotient`, `remainder`
- comparison: `<`, `>`, `<=`, `>=`, `=`, `!=`
- mathematical: `:abs`, `:negate`, `:sqrt`, `:log`, `:exp`, `:sin`, `:cos`
- conversion: `:as: Integer`, `:as: Float`, `:as: Rational`
- predicates: `:zero?`, `:positive?`, `:negative?`, `:integer?`,
  `:rational?`, `:real?`, `:finite?`, `:nan?`

## literal forms

```moof
123                              ; decimal integer
-456                             ; negative
0xff, 0xFF                       ; hex
0b1010                           ; binary
0o17                             ; octal
1_000_000                        ; underscores allowed for readability

1.5, .5, 1.5e3                   ; float (decimal, exponent)
1e9                              ; scientific
∞, -∞, NaN                      ; special floats (also #infinity, #-infinity, #nan)

1/3, -7/2                        ; rational

3+4i, 0+1i, -2-3i                ; complex (i is the imaginary unit)
```

## bigint always available

an Integer is conceptually arbitrary-precision. `[(2 ** 1024)]` is a
valid Integer; the substrate handles overflow by promoting to bigint
representation. division of Integers that would round produces a
Rational by default (use `:quotient` if you want integer division).

```moof
[7 / 2]                          ; → 7/2  (a Rational)
[7 quotient: 2]                  ; → 3    (Integer)
[7 mod: 2]                       ; → 1
```

## protos implemented

`Number` (and its sub-protos) implement:

- `Equatable` (`=`, with promotion semantics)
- `Comparable` (`<`, `>`, `<=`, `>=`)
- `Hashable`
- `Showable`
- `Numeric` — the arithmetic protocol

## inspirations

- the numeric tower: scheme (RnRS, especially R6RS).
- bigint promotion: many lisps and python.
- separating Float from Rational under Real: scheme tradition.
- the broadcasting ops in Tables piggyback on the Number protocol
  (`concepts/tables.md`).

## see also

- `concepts/types.md` — types like `Nat`, `Pos` as refinements.
- `concepts/tables.md` — broadcasting, rank-polymorphism over Numbers.
- `syntax/literals.md` — full numeric literal grammar.
