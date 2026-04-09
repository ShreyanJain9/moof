# error handling

moof has structured error handling via `try`/`catch` and `error`.

## signaling errors

```moof
(error "something went wrong")
```

signals an error. if uncaught, the error message is printed and
evaluation of the current expression stops. the REPL continues.

## catching errors

```moof
(try body catch: |err| handler)
```

evaluates `body`. if it signals an error (handler not found,
division by zero, explicit `(error ...)`), creates an Error
object and calls the handler block with it.

returns the body's result on success, or the handler's result
on error.

```moof
(try [1 / 0] catch: |e| [e message])
; => "division by zero"

(try (error "boom") catch: |e| (str "caught: " [e message]))
; => "caught: boom"

(try [42 + 1] catch: |e| "nope")
; => 43  (no error, body result returned)
```

## the Error object

when `try` catches an error, it creates an Error object with
a `message` slot containing the error string.

```
[err message]    => String (the error message)
[err describe]   => String ("Error: message")
[err show]       => String ("Error: message")
```

Error objects delegate to the Error prototype. you can add
handlers to Error to enrich error objects globally.

## rescue: (lightweight suppression)

for blocks, `rescue:` provides a one-line error suppression:

```moof
[|| [1 / 0] rescue: -1]        => -1
[|| [2 + 3] rescue: 0]         => 5  (no error)
```

`rescue:` is defined on Object. it calls `[self call: nil]`
(treating self as a block), catches any error, and returns the
default value.

## doesNotUnderstand:

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

## how it works internally

- `try` is a compiler special form that compiles `body` as a
  zero-arg closure and `handler` as a one-arg closure
- the `TryCatch` opcode calls the body closure; on Rust-level
  `Err`, it creates an Error object and calls the handler
- `error` is a compiler special form that emits the `Throw`
  opcode, which returns `Err(msg)` from the VM
- `rescue:` is a moof method on Object (lib/error.moof) that
  wraps `try`/`catch`

## error types

currently, all errors are strings. the Error object's `message`
slot contains the error string regardless of the error source:

- handler not found: `"42 does not understand 'foo'"`
- division by zero: `"division by zero"`
- type mismatch: `"+ : arg not numeric"`
- explicit error: whatever string you pass to `(error ...)`

future: structured error types with selector, receiver, stack
trace.
