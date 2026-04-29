# reflection contract

> **what the substrate guarantees you can introspect, on every Form,
> at all times. this is the *moldable promise*.**

## R1 — every Form responds to the basic reflection protocol

```moof
[v proto]                ; immediate parent (a Form, or nil for Object)
[v protos]               ; List of all protos in delegation chain
[v slots]                ; Table of slot-name → value
[v handlers]             ; Table of selector → handler-form
[v meta]                 ; Table of metadata
[v source]               ; source-form, if any (closures/methods/defs)
[v identity]             ; heap-id (Integer, vat-local)
```

these methods are inherited from `Object` and cannot be removed.
user code can override `:inspector-view` for custom rendering, but
`:slots` etc. always return the actual underlying state.

## R2 — closures and methods expose their compilation

```moof
[m bytecodes]            ; Table of decoded opcodes
[m disassemble]          ; pretty string of bytecode
[m caps-required]        ; List of cap protos this method receives
[m purity]               ; #pure | #effectful: <list> | #unknown
[m arity]                ; Integer or :variadic
[m parameters]           ; List of param Forms (with patterns/types)
[m proto-defining]       ; the proto this method belongs to
```

bytecode is *derived from source*. editing source invalidates and
regenerates. the source-form is canonical (substrate-laws/L5).

## R3 — running computation exposes its frame

a frame is a Form (proto: `Frame`). when a vat is paused (debugger
attached, etc.), the frame stack is reachable:

```moof
[frame method]           ; the method being executed
[frame self]             ; the receiver
[frame locals]           ; Table of name → current value
[frame caps]             ; Table of cap-name → cap (in scope)
[frame stack]            ; List of frames (this and all callers)
[frame pc]               ; bytecode offset (or source-loc)
[frame send-site]        ; the call expression that produced this frame
[frame returnable?]      ; can the substrate force a return?
```

actions on a frame:

```moof
[frame resume!]          ; continue execution
[frame retry!]           ; re-attempt the current send
[frame edit-method!]     ; live-edit + re-attempt (debugger move)
[frame return: value]    ; force a return with this value
[frame raise: exn]       ; force a raise from this frame
```

## R4 — vats expose their full state

```moof
[vat id]
[vat name]
[vat proto]
[vat heap]               ; live Form-graph, queryable
[vat inbox]              ; data source (read-only here)
[vat outbox]
[vat behavior]           ; the receive-loop closure
[vat supervisor]         ; far-ref to supervisor vat
[vat caps]               ; Table of cap-name → cap
[vat journal]            ; data source over WAL
[vat status]             ; #running | #paused | #crashed | #shutdown
[vat snapshot-time]      ; when state was last checkpointed
[vat seq-id]             ; current journal seq
```

vat introspection happens via far-refs. you query the introspection
methods like any other; the target vat answers via its mailbox.
self-introspection (a vat looking at itself) is sync.

## R5 — types expose their structure

```moof
[T satisfies?: v]        ; the predicate
[T describe]             ; human-readable
[T protocols]            ; List of protocols implemented
[T implementors]         ; List of types implementing (for protocols)
[T parameters]           ; List of type parameters (for parameterized types)
[T components]           ; List of constituent types (for ∩ ∪ refinements)
[T base]                 ; the base type for refinements
```

types are Forms. R1 applies to types as well.

## R6 — nothing the substrate knows is hidden

if the rust line stores a piece of state about a Form, it must be
exposed through the reflection protocol. this is the load-bearing
promise. example: an inline cache at a send-site is *substrate-level
state*, but it's exposed as `[send-site cache-stats]` for inspection.

if a piece of state cannot be reflected, *the rust line should not
store it.* prefer a moof-level representation that is reflected.

## R7 — meta annotations are extensible

`[v meta]` returns a Table that can be extended with arbitrary keys:

```moof
[v meta at: 'doc put: "this is a thing"]
[v meta at: 'tags put: #['load-bearing 'temporary]]
[v meta at: 'author put: "shreyan"]
```

meta keys reserved by the substrate:
- `'source` — source-loc record (file, line, col, span).
- `'doc` — doc-comment string (set by `;:` comments).
- `'journal-id` — seq-id of the journal entry that last mutated this.
- `'type` — type ascription, if any.
- `'caps-required` — cap effect row.
- `'purity` — `#pure` / `#effectful: …` / `#unknown`.

user code can use any other key freely.

## R8 — query the world

queries (`concepts/queries.md`) are the relational frontend to
reflection. every reflectable property is queryable:

```moof
(query (?obj proto: Counter where: [?obj count > 100]))
(query (?vat status: 'crashed))
(query (?m implements: 'incr in: ?proto))
(query (?frame in-vat: 'shreyan halted-at: 'error))
```

the inspector, debugger, browser, and profiler are all queries
under the hood.

## what user code may NOT block

user code can override `:inspector-view` to control how an object
*renders* in a UI. user code can *not* override `:slots`, `:handlers`,
`:source`, etc. to lie about underlying state. those are substrate
guarantees.

if a domain object wants to hide certain slots from a casual user,
it does so by:
- providing a custom `:inspector-view` that shows a curated subset.
- relying on cap-attenuation (`concepts/capabilities.md`) to limit
  who has access to invoke `:slots`.

but the underlying `:slots` access *exists* and *is honest*.

## inspirations

- the metaobject protocol's discipline of reified-everything: kiczales
  et al., *the art of the metaobject protocol* (1991).
- smalltalk-80 reflection: kay, goldberg, robson.
- self's "everything is an object you can inspect": ungar & smith.
- glamorous toolkit's per-object views: gîrba — adopted as the
  *presentation* layer atop honest substrate reflection.

## see also

- `laws/substrate-laws.md` — broader substrate guarantees.
- `concepts/reflection.md` — narrative version of this contract.
- `concepts/moldability.md` — what reflection enables.
