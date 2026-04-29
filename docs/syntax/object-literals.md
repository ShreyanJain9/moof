# object literals

> **`{Proto slot: val [method] body …}` constructs a fresh object,
> inherits from Proto, sets slots, defines methods, and returns
> itself.**

object literals are the inline cousin of `defproto`. when you want
a one-shot object — an instance, a service singleton, an anonymous
proto-extension — you reach for `{…}`.

## the grammar

```
{ proto-ref?
  ( name : value | [ method-header ] body )*
}
```

- the *first position* (optional) is a proto reference. if omitted,
  defaults to `Object`.
- subsequent positions are either:
  - a slot binding: `name: value`.
  - a method definition: `[header] body`.
- order between slots and methods is irrelevant; the result is the
  same.

## examples

### simple instance

```moof
{Counter count: 0 step: 1}
```

a fresh instance of Counter, with slots filled. uses Counter's
methods unchanged.

### one-shot anonymous

```moof
{x: 5
 y: 10
 [magnitude]
   [(.x sqrt) + (.y sqrt)]}
```

no proto specified ⇒ defaults to `Object`. has two slots and one
method. anonymous; useful as a one-off responder.

### method override

```moof
{Counter
  count: 5
  step: 2
  [custom-incr] [self count: [.count + .step + 1]]
  [whoa]        "i am special"}
```

inherits Counter's `:incr`, `:decr`, `:read`. adds `:custom-incr` and
`:whoa`. instance-level only — Counter itself is unchanged.

### multi-clause methods

```moof
{Counter count: 0
  [incr-by: 0]                       nil
  [incr-by: n :: Pos]                [self count: [.count + n]]
  [incr-by: n]                       (error 'invalid)}
```

multiple clauses for one selector behave identically to multi-clause
methods elsewhere.

## semantics

```moof
(let c {Counter count: 0 step: 1})
```

is roughly equivalent to:

```moof
(let c [Counter new])
[c count: 0]
[c step: 1]
```

with the difference that `{…}` is a single allocation and the slots
are populated atomically before `c` is returned. (no observable
intermediate state; relevant for concurrency.)

method definitions inside `{…}` write to the new object's `handlers`
table directly. they do *not* mutate the proto.

## with respect to identity

each evaluation of an object literal produces a *new* form-id. so
`{Counter count: 0}` evaluated twice yields two distinct objects.

```moof
[{Counter count: 0} is {Counter count: 0}]   ; → #false (different ids)
[{Counter count: 0} = {Counter count: 0}]    ; → #true (structurally equal)
```

(this matches javascript / lua expectation; differs from string-
literal interning where two `"foo"` *might* be the same object.)

## as a proto

an object literal can serve as a proto for *further* objects:

```moof
(let SpecialCounter
  {Counter
    [incr] [self count: [.count + 100]]})    ; one-off subclass

(let c {SpecialCounter count: 0 step: 1})
[c incr]                                     ; → c.count = 100
```

`SpecialCounter` is a Form whose proto is Counter and whose handlers
contain an override of `:incr`. it can be used wherever Counter could.

(this is self's prototype-as-class. classes are objects. one
mechanism, recursing.)

## inside `(handlers …)` of `defproto`

object literals are not used inside `defproto` body — there, you
just write the headers directly:

```moof
(defproto Counter
  (slots count step)
  (handlers                        ; not {…} — bare list
    [incr]   …
    [decr]   …))
```

this is because `(defproto …)` is itself an operative; it consumes
the slot/handler specs in its own grammar. object literals are for
*constructing instances*, not for declaring protos.

## inspirations

- self's clone-and-extend (ungar & smith 1987).
- javascript's object literal `{…}`: braha, eich.
- lua's table literal: ierusalimschy.
- the symmetry between method-header-and-call-site is moof's own,
  carried through here too.

## see also

- `syntax/methods-and-handlers.md` — method header grammar.
- `syntax/binding-and-defs.md` — `defproto`.
- `concepts/objects-and-protos.md` — what protos are.
- `concepts/forms.md` — what objects are.
