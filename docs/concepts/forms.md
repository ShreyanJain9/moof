# forms

> **the universal substrate primitive. one heap kind. four faces.**

every value in moof is a Form. integers, strings, symbols, lists,
tables, closures, vats, environments, methods, types, and the
running world itself — all Forms. the substrate has exactly one
allocator and exactly one introspection contract. everything else is
how that one thing is used.

## the four faces

a Form simultaneously presents:

| face | fields | the tradition that emphasized it |
|---|---|---|
| **structure** | `head`, `args` | lisp (mccarthy 1958) |
| **identity** | `proto`, `slots`, `handlers` | smalltalk (kay 1972), self (ungar & smith 1987) |
| **liveness** | `mailbox`, `behavior`, `supervisor` (when alive) | actors (hewitt 1973), erlang (armstrong 2003) |
| **history** | `meta` (loc, doc, journal-id, provenance) | datomic (hickey ~2012) |

most values use one or two faces. an integer leans on identity (proto
= Integer) and ignores the others. a parsed code-form leans on
structure (head + args) and history (source-loc). a vat leans on
liveness and identity. a table leans on identity (proto = Table) and
structure (its array+map content).

the four faces are *not* arbitrary. they correspond to the four
traditions the substrate is honoring (`vision/lineage.md`). the
synthesis claim — that one heap cell can carry all four without
losing the character of any — is the core bet of moof v4.

## the conceptual shape

```
Form {
  head     : Value         ; what kind of structure-thing this is
                           ; (a symbol, another Form, or nil)
  args     : Value         ; the children (a List, or nil)

  proto    : FormId        ; delegation parent
  slots    : Table         ; named bindings, mutable
  handlers : Table         ; selector → handler-form (the method dict)

  meta     : Table         ; source-loc, doc, type-info, journal-id, …

  ;; liveness, only present on Forms that are vats:
  mailbox  : DataSource    ; (when this Form is a vat)
  behavior : Form          ; the receive-loop closure
  supervisor : FormId      ; the supervising vat
}
```

(this is the conceptual shape. on disk and in rust, the
representation is more compact — see `reference/canonical-encoding.md`
when written.)

## what specific kinds of value look like

each kind of value is "a Form whose proto is X and whose other faces
are emphasized in particular ways." some examples:

- **integer 5** — `proto: Integer`, `meta: {}`. structure and
  liveness faces unused. (small ints may be NaN-boxed in the
  interpreter for perf; the conceptual model is the same.)
- **symbol 'foo** — `proto: Symbol`, `meta: {name: "foo"}`,
  interned.
- **list (1 2 3)** — `proto: List`, internally a chain of cons-cell
  Forms with `head: 1, args: (rest of list)`. structure-face
  primary.
- **table {1 2 3 'name => "ada"}** — `proto: Table`. its slots are
  the keyed entries; its positional content is in a structure-faced
  internal layout.
- **closure** — `proto: Closure`, `slots: {captured-env, source}`,
  `handlers: {:call ...}`.
- **environment** — `proto: Env`, `slots: {bindings, parent}`.
- **method** — a Closure with provenance: `meta` carries source-loc,
  type-info, doc, the proto it lives on.
- **vat** — a Form with mailbox, behavior, supervisor; otherwise
  ordinary slots and handlers.
- **promise** — `proto: Promise`, `slots: {state: pending|ready|broken,
  value, observers}`.

the universal heap allocator does not care which kind. allocation is
"give me a Form with these fields populated."

## evaluation as message-send to a Form's proto

the entire evaluator can be summarized as:

```
eval(form, env) :=
    handler := lookup(form.proto, :eval)
    handler(form, env)
```

every Form's proto knows how to evaluate that Form. user-defined
protos can supply their own `:eval` — this is how user code defines
new special forms (the `unless` example in
`syntax/binding-and-defs.md`).

similarly, message dispatch:

```
send(receiver, selector, args) :=
    handler := lookup(receiver.proto, selector)
    handler(receiver, *args)
```

both are "look up a handler on the proto-chain and invoke it."
**eval and send are the same primitive** (kernel: shutt 2010, maru:
piumarta & warth 2007). this unification is the lisp/smalltalk
merger's deepest promise.

## reflection

every Form exposes its faces (`laws/reflection-contract.md` for the
formal guarantee):

```moof
[v proto]               ; immediate proto
[v protos]              ; full proto chain
[v slots]               ; map of slot-name → value
[v handlers]            ; map of selector → handler-form
[v meta]                ; source-loc, doc, journal-id, …
[v source]              ; the source-form (for closures/methods)
[v identity]            ; the heap id (within its vat)
```

for live Forms (vats):

```moof
[vat mailbox]           ; the inbox data source
[vat behavior]          ; the receive-loop
[vat supervisor]
```

for closures/methods:

```moof
[m bytecodes]           ; decoded bytecode as a Table
[m caps-required]       ; effect row
[m purity]              ; #pure / #effectful / #unknown
```

nothing is hidden. the substrate guarantees this. (see
`laws/reflection-contract.md`.)

## identity and equality

- `[a is b]` — heap-identity within a vat. true iff same form-id.
- `[a = b]` — value-equality. structural, recursive, default.
- per-proto `:= b` overrides may exist for protos with their own
  notion of equivalence.

across vats, identity does not survive (a far-ref does not have an
identity-relation with the local form-id of its target). value-
equality across vats is well-defined for value-forms (numbers,
strings, immutable tables); for mutable forms it requires
sending a comparison message. (`concepts/references.md`.)

## why one kind, not a discriminated union

the alternative would be: rust enum with variants for Pair, Closure,
Object, Env, etc. (this is what v3 had.) we reject this because:

1. **every special-cased variant becomes a special case in the GC,
   the formatter, the canonical encoder, the dispatcher, the
   plugin code.** v3 had this and it metastasized.
2. **users want to define new "kinds of value" without editing
   rust.** with one heap kind + a proto, every new "kind of value"
   is just a new proto. no rust changes.
3. **uniform reflection is a non-negotiable.** if the substrate has
   privileged variants, those variants escape the moldable promise.

we accept the cost: every value-access pays one indirection through
proto. inline caches absorb most of the cost in practice. the
clarity is worth it.

## inspirations, attributed

- the "Form" name and structure-face from common lisp's *form* (the
  generic term for an evaluable expression).
- the proto/slot/handler unification from self (ungar & smith 1987).
- the eval-via-proto-handler from maru (piumarta & warth 2007) and
  kernel (shutt 2010).
- the "everything is a Message"-shaped value from io (dekorte 2002).
- the journal-as-meta idea from datomic (hickey ~2012).
- the four-faces synthesis is moof's own framing.

## see also

- `concepts/objects-and-protos.md` — how delegation works.
- `concepts/sends-and-calls.md` — how messages dispatch.
- `concepts/types.md` — how types are Forms.
- `concepts/vats.md` — how Forms become alive.
- `laws/reflection-contract.md` — the substrate's introspection promise.
