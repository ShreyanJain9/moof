# sends and calls

> **two bracket shapes, two semantics, one underlying mechanism.
> `(fn args)` is a fn-call. `[recv selector args]` is a message send.
> both reduce to "look up a handler on a proto-chain and invoke it."**

we keep them syntactically distinct because the user reads them
differently, even though they are operationally similar. this is
the *explicit-beats-implicit* call (`vision/manifesto.md`); v3
collapsed them and the result was confusing.

## the surface

```moof
;; fn-call: parens, head is the callable
(+ 1 2)
(map fn coll)
(if cond then else)
(let ((a 1)) body)

;; message send: square brackets, receiver is first
[5 + 3]                        ; binary
[obj read]                     ; unary
[obj method-name arg1 arg2]    ; positional send
[dict at: 'foo put: 5]         ; multi-keyword (smalltalk-style)
```

## the underlying mechanism

both compile to the same primitive:

```
send(receiver, selector, args) :=
    handler := proto-chain-lookup(receiver.proto, selector)
    handler(receiver, *args)
```

a fn-call `(foo x y)` is implemented as a send to the current scope:
`send(scope, :foo, [x, y])`. the scope's proto chain ends at the
language root, where the binding for `foo` resolves.

a message send `[recv msg: x]` is implemented as a send directly:
`send(recv, :msg:, [x])`.

both go through the same dispatch path and benefit from the same
inline caches.

## fn-calls

```moof
(callable arg1 arg2 …)
```

- the head is evaluated; it must be a callable (closure, operative,
  primitive function, anything responding to `:call`).
- the args are evaluated left-to-right (for applicatives) or passed
  unevaluated (for operatives — see below).
- the result is whatever the callable returns.

fn-calls are the lispy rhythm: short, head-first, "do this with these."

## message sends

four sub-forms:

### unary

```moof
[obj selector]
```

the receiver is sent the selector with no args. selector must be
a symbol identifier (alphanumerics + `-` + `?` + `!`).

```moof
[5 abs]
[c read]
[xs length]
```

### positional

```moof
[obj selector arg1 arg2 …]
```

the receiver is sent `selector` with positional args. distinct from
keyword sends because keyword sends require `:` markers.

```moof
[s replace 'old 'new]
[t at-put 0 'first]
[t at 5]
```

### keyword (multi-)

```moof
[obj kw1: arg1 kw2: arg2 …]
```

the selector is the concatenation `kw1:kw2:…`. args are positional in
declaration order. this is smalltalk's signature feature.

```moof
[dict at: 'name put: "ada"]                 ; selector :at:put:
[window draw: rect color: 'red weight: 2]   ; selector :draw:color:weight:
[5 between: 1 and: 10]                      ; selector :between:and:
```

### binary

```moof
[a OP b]
```

OP is a symbol composed of special characters: `+ - * / < > = != == 
<= >= ~= ?? || && | & ^ << >>` etc. selector is the operator.

```moof
[a + b]
[x < y]
[v1 ?? default]
```

binary sends are syntactically distinct so we don't need to write
`[a + 3]` as `[a + 3]`-with-keyword-marker — the operator's character
class identifies it.

## explicit nesting

moof has *no operator precedence* for binary sends. chained binaries
require explicit nesting:

```moof
[[a + b] * c]                  ; correct
[a + b * c]                    ; ERROR — ambiguous; use explicit nesting
```

(this is more strict than smalltalk-80, which uses left-associativity
for binary chains. we prefer the error: it forces the writer to
declare intent.)

similarly for chained unary/keyword sends:

```moof
[[obj a] b]                    ; chain: a, then b on result
[[obj a: x] b: y]              ; chain with keyword sends
[obj a b]                      ; ERROR — looks like positional send
                               ; (for which the selector would be :a)
```

## cascades

a cascade sends multiple messages to the same receiver. introduced by
`;`, returns the receiver:

```moof
[transcript
   show: "hi "
   ; show: "world"
   ; newline]
```

equivalent to:

```moof
(do
  [transcript show: "hi "]
  [transcript show: "world"]
  [transcript newline]
  transcript)                   ; cascade returns the receiver
```

cascades are smalltalk-80 syntax (goldberg & robson 1983). useful for
builder-style sequential mutation.

## self-references

inside a method body, `self` is bound to the receiver. the
`.foo` shorthand is equivalent to `[self foo]`:

```moof
.count                ; ≡ [self count]
[.count x y]          ; ≡ [[self count] x y]  (send :x to value of self.count)
[self count: 5]       ; full setter — no shorthand for writes
```

`.foo` strictly substitutes the value of `self.foo`. it does *not*
mean "send `foo:` to self with following args." for writes, write the
full send.

(`super` is similar but starts the proto-chain lookup at the proto
*above* the one this method was found on.)

## operatives vs applicatives

a callable is one of two kinds:

| kind | args | example | from |
|---|---|---|---|
| **applicative** | evaluated before send | regular fn / closure | most languages |
| **operative** | passed *unevaluated* (as Forms) | `if`, `let`, `quote`, `defproto`, user macros | kernel ($vau) |

operatives let user code define new "special forms." they receive
their args as raw Forms and decide what to do (typically: walk,
transform, eval some, build a new form).

```moof
;; builtin operatives
(if cond then else)
(let ((a 1)) body)
(quote (some-form))
(defproto Counter ...)

;; a user-defined operative
(defop unless [cond then-form]
  `(if (not ,cond) ,then-form))      ; quasiquote/unquote
```

this is shutt's operative/applicative split (PhD thesis, WPI 2010),
adapted into our surface. it eliminates the privileged-special-form
problem: `if` is not magically built into the parser; it is a
proto with an `:eval` handler that does the if-thing. user code can
make new ones.

## inline caches

every send-site has a small inline-cache slot. on first send, the
cache records the resolved (proto, handler). subsequent sends with
the same proto skip the lookup — one pointer compare, one indirect
call. this is the **single biggest speedup in dynamic OO** (self
papers; deutsch & schiffman 1984).

caches are invalidated when a proto's handler table is mutated.
the next send re-resolves. inline caches do not hide source: the
canonical method is still the source-form, accessible via reflection.

## inspirations

- the operative/applicative split: shutt's *kernel* (PhD thesis, WPI 2010).
- the keyword-message rhythm: smalltalk-80 (goldberg & robson 1983).
- the cascade syntax: smalltalk-80.
- inline caches: deutsch & schiffman, *efficient implementation of the
  smalltalk-80 system* (POPL 1984), and self's polymorphic ICs
  (hölzle, chambers, ungar 1991).
- the visual `()` vs `[]` split: original to moof, but in the spirit
  of clojure's bracket-coded vocabulary.

## see also

- `concepts/blocks-and-patterns.md` — `|args| body` syntax and
  pattern-matched method clauses.
- `concepts/forms.md` — what dispatch is operating on.
- `concepts/objects-and-protos.md` — proto-chain mechanics.
- `syntax/brackets.md` — bracket-shape cheat-sheet.
- `syntax/methods-and-handlers.md` — definition syntax.
