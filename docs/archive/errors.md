# error handling

> **Wave 6 Status Note (Apr 2026):** The `try`/`catch` and `error` forms
> documented below are **deprecated** and no longer emitted by the compiler.
> The VM's `TryCatch` and `Throw` opcodes are explicitly rejected at runtime.
> See [core-contract-matrix.md](core-contract-matrix.md) for current feature status.

## current error model

moof uses **Result/Err value-flow** for error handling. Failures produce
`Err` values that propagate monadically through the computation pipeline.

The `try`/`catch` special forms and `error` form described below were part
of an earlier try/catch-based model that has been removed from the runtime.

---

## legacy documentation (deprecated)

The following describes the removed try/catch model, kept for historical reference.

### signaling errors (deprecated)

```moof
(error "something went wrong")
```

~signals an error. if uncaught, the error message is printed and~
evaluation of the current expression stops. the REPL continues.

**Status:** The `error` form is no longer emitted by the compiler.
The `Throw` opcode is rejected by the VM.

### catching errors (deprecated)

```moof
(try body catch: |err| handler)
```

~evaluates `body`. if it signals an error (handler not found,~
division by zero, explicit `(error ...)`), creates an Error
object and calls the handler block with it.

**Status:** The `try` form is no longer emitted by the compiler.
The `TryCatch` opcode is rejected by the VM.

### the Error object

Error objects still exist for values produced by other failure paths:

```
[err message]    => String (the error message)
[err describe]   => String ("Error: message")
[err show]       => String ("Error: message")
```

Error objects delegate to the Error prototype. you can add
handlers to Error to enrich error objects globally.

### rescue: (lightweight suppression)

The `rescue:` handler on blocks provides fallback behavior:

```moof
[|| [1 / 0] rescue: -1]        => -1
[|| [2 + 3] rescue: 0]         => 5  (no error)
```

`rescue:` is defined on Object. it calls `[self call: nil]`
(treating self as a block), catches any error, and returns the
default value.

### doesNotUnderstand:

when a message send fails (no handler found), the VM tries
sending `doesNotUnderstand:` to the receiver before signaling
an error. if the receiver handles it, that result is used.

```moof
(defmethod MyProxy doesNotUnderstand: (sel args)
  (str "you sent " [sel name] " but i don't know that"))

[{ MyProxy } foo]
; => "you sent foo but i don't know that"
```

this enables: proxies, delegation, DSLs, method-missing-style
metaprogramming. the default behavior (if no DNU handler) is
to signal the error normally.

## error types

currently, errors are strings. the Error object's `message`
slot contains the error string regardless of the error source:

- handler not found: `"42 does not understand 'foo'"`
- division by zero: `"division by zero"`
- type mismatch: `"+ : arg not numeric"`

future: structured error types with selector, receiver, stack
trace.

---

## see also

- [core-contract-matrix.md](core-contract-matrix.md) — opcode and feature status
- [language.md](language.md) — language syntax reference
