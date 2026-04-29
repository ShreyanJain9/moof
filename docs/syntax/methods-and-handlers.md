# methods and handlers

> **a method's header is shaped exactly like its call site. you read
> a definition and know exactly how to invoke it. you read a call and
> know exactly what shape of definition it expects.**

this symmetry is one of moof's small-but-load-bearing decisions.

## the basic shape

inside a `defproto` or an object literal:

```moof
[selector arg-pattern …]    body-expression
```

the bracketed header is the call shape. the body is one expression.

## examples by call shape

### unary

```moof
[read]              .count
[length]            [.entries length]
[abs]               (if [.value < 0] [-1 * .value] .value)
```

call-site:

```moof
[counter read]
[xs length]
[(-5) abs]
```

### positional

```moof
[at-put i v]        [.entries at: i put: v]
[swap a b]          (do-swap)
```

call-site:

```moof
[t at-put 0 'first]
[obj swap a b]
```

### keyword (multi-)

```moof
[at: i put: v]      [.entries at: i put: v]
[from: a to: b]     (do-iter)
[draw: rect color: c weight: w]   (do-draw)
```

call-site:

```moof
[t at: 0 put: 'first]
[range from: 1 to: 100]
[canvas draw: r color: 'red weight: 2]
```

### binary

```moof
[+ other]           [.value + [other value]]
[< other]           [.value < [other value]]
```

call-site:

```moof
[a + b]
[x < y]
```

binary methods take exactly one argument; selector is the operator.

## multi-clause methods

stack multiple `[header] body` lines with different patterns:

```moof
[incr-by: 0]                    nil                           ; clause: no-op
[incr-by: n :: Pos]             [self count: [.count + n]]    ; positive arg
[incr-by: n :: Neg]             [self count: [.count + n]]    ; negative arg
[incr-by: n]                    (error 'invalid-step)         ; fallback
```

clauses are tried in order. first match wins. type guards and
predicate guards are part of the pattern (`concepts/blocks-and-
patterns.md`).

## with type ascription

```moof
[area-of-radius: r :: Pos]
  :: Number
  [PI * r * r]
```

return-type ascription appears on its own line between the header
and the body. parameter types are inline in patterns.

## with doc

```moof
[incr]
  ;: increment count by .step. mutates self.
  [self count: [.count + .step]]
```

`;:` doc comment attaches to the method's `meta.doc`.

## inside `defproto`

```moof
(defproto Counter
  (slots count step)
  (handlers
    [incr]              [self count: [.count + .step]]
    [incr-by: n]        [self count: [.count + n]]
    [decr]              [self count: [.count - .step]]
    [read]              .count
    [reset-with: |s :: Pos|]
                        (do
                          [self count: 0]
                          [self step: s])))
```

`(handlers …)` introduces a sequence of method definitions. each
method gets a `:proto-defining` meta-pointer back to `Counter`.

## inside an object literal

```moof
{Counter
  count: 0
  step: 1
  [custom-incr] [self count: [.count + .step + 1]]
  [whoa]        "i am special"}
```

methods inside `{...}` override or augment the proto's methods. the
expression returns one new object.

(`syntax/object-literals.md` for the full object-literal grammar.)

## super sends

inside a method, `super` denotes "the proto above this method's
defining proto":

```moof
(defproto BoundedCounter
  (proto Counter)
  (slots max)
  (handlers
    [incr-by: n]
      (if [[.count + n] > .max]
          (error 'overflow)
          [super incr-by: n])))
```

`[super selector args]` resolves the selector in the proto-chain
*starting above* `BoundedCounter`. found in `Counter`. invokes that
method with `self` bound to this instance.

(this is the smalltalk `super` mechanic, transposed to `[]` send
syntax.)

## the symmetry

the *header is the call site*. compare:

```moof
;; definition
[at: i put: v]      [.entries at: i put: v]

;; call
[t at: 'foo put: 5]
```

both have the same shape. read one, predict the other.

this matters because:
- doc-by-example: the header literally shows you how to call.
- searchability: grep for `at: ` in calls finds matching definitions
  with the same selector.
- inspector clarity: methods are listed in their call-shape.

(an alternative — separate "method declaration" syntax distinct from
call-site shape — was considered. rejected because the symmetry's
worth more than the small ergonomic gain elsewhere.)

## inspirations

- the symmetric header/call shape: smalltalk-80 (kay et al.) — the
  same `at:put:` selector reads at definition and at call. moof
  carries this through into its s-expr-flavored surface.
- multi-clause pattern dispatch: haskell, erlang.
- the placement of return types on a separate line: rust (sort of) /
  haskell signatures.
- super-sends: smalltalk-80.
- doc-attached-to-defs: common lisp.

## see also

- `syntax/object-literals.md` — methods in `{…}`.
- `syntax/binding-and-defs.md` — `def`, `defproto`.
- `concepts/objects-and-protos.md` — proto-chain.
- `concepts/sends-and-calls.md` — dispatch semantics.
