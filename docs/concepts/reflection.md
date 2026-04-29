# reflection

> **everything is introspectable. the substrate guarantees this.
> if a piece of state cannot be inspected from inside moof, that
> piece of state is in the wrong place.**

reflection is not a "nice to have." it is the *moldable promise*.
without it, the inspector lies, the debugger fails, and the user
loses the world's transparency.

## what every Form exposes

```moof
[v proto]                ; immediate proto
[v protos]               ; the full proto chain
[v slots]                ; map of slot-name → value
[v handlers]             ; map of selector → handler-form
[v meta]                 ; meta-table (loc, doc, journal-id, …)
[v source]               ; source form (for closures/methods/defs)
[v identity]             ; heap-id within its vat
[v inspector-view]       ; the per-proto custom UI view, if any
```

these methods are provided by the root proto (`Object`) and are
overridable per-proto for custom presentation, but their *contract*
is fixed: they always return real, faithful information about the
underlying state. a proto cannot "lie" about its slots.

## what closures and methods expose

```moof
[m source]               ; source-form (parsed)
[m bytecodes]            ; decoded bytecode as a Table
[m disassemble]          ; pretty-printed bytecode listing
[m caps-required]        ; effect row — which $caps it expects
[m purity]               ; #pure | #effectful: <caps> | #unknown
[m arity]                ; argument arity (or :variadic)
[m parameters]           ; parameter forms (with patterns/types)
[m proto-defining]       ; which proto this method lives on
```

## what running computations expose

a frame is a Form. the call stack is a List of frames.

```moof
[frame locals]           ; map of name → current value
[frame stack]            ; list of frames (from this up to root)
[frame method]           ; the method being executed
[frame pc]               ; bytecode position (or source-loc)
[frame self]             ; the receiver
[frame caps]             ; caps in scope
[frame resume!]          ; (debugger) resume execution
[frame retry!]           ; (debugger) re-attempt the current send
[frame edit-method!]     ; live-edit and re-attempt
```

debuggers use these to halt, inspect, edit, and resume — the
smalltalk debugger experience (kay et al., goldberg & robson 1983).

## what vats expose

```moof
[vat id]
[vat name]
[vat proto]
[vat heap]               ; live form-graph (queryable!)
[vat inbox]              ; data source
[vat outbox]
[vat behavior]
[vat supervisor]
[vat journal]            ; data source over WAL
[vat caps]               ; capabilities held
[vat status]             ; #running | #paused | #crashed | #shutdown
```

(see `concepts/vats.md`.)

## what types expose

```moof
[T satisfies?: v]        ; check membership
[T describe]             ; human-readable description
[T protocols]            ; protocols this type implements
[T implementors]         ; types that implement (for protocols)
[T parameters]           ; type parameters (for parameterized types)
```

types are Forms; reflection works the same way.

## what the world exposes

```moof
[$registry vats]         ; all live vats
[$registry paths]        ; the world's path-table
[$registry queries-supported]   ; query operatives available
```

## moldable inspector views

each proto can register a custom `:inspector-view` that produces a
domain-specific UI for instances. the inspector consults this when
rendering. if absent, a default (slot/handler list) is used.

```moof
{Counter
  …
  [inspector-view]
    (Morph
      label: "$.count / $.step"
      buttons: #[(Button "tick" |_| [self incr])])}
```

this is the **glamorous toolkit** culture (gîrba et al.): tools
co-developed with the domain. the inspector is *not a fixed
component*; it is a renderer that respects per-proto contracts.

## the substrate's promise

four invariants (`laws/reflection-contract.md`):

1. **no hidden state in the rust line.** every piece of substrate
   state is reachable through the reflection methods above.
2. **proto methods cannot lie.** `[v slots]` returns the actual
   slot table. user code can refuse to expose certain slots in
   custom inspector views, but `[v slots]` itself is a substrate
   guarantee.
3. **source is canonical.** every closure's `:source` is the
   actual source from which it was compiled. bytecode is derived;
   source is not.
4. **time is visible.** every form's `:meta` includes a journal-id
   pointing to the change-record that produced its current state
   (`concepts/time-and-journal.md`).

## reflection has a cost

we accept it. inline caches make reflection's overhead small in hot
paths. the alternative (opaque substrate that performs better) is
not the moof we are trying to build.

## inspirations

- smalltalk-80's reflection: kay et al.; classes and methods are
  themselves objects.
- self's universal slot-access: ungar & smith 1987.
- common lisp's debugger and `:reify`: clos / amop (kiczales et al.,
  *the art of the metaobject protocol* 1991).
- glamorous toolkit's per-object views: gîrba et al.
- erlang's tracing facilities (process-level introspection):
  armstrong et al.

## see also

- `laws/reflection-contract.md` — formal substrate guarantees.
- `concepts/moldability.md` — what reflection enables.
- `concepts/forms.md` — the Form's faces.
