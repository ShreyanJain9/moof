# the moof language

> **Wave 6 Status Note (Apr 2026):** This document describes the language syntax.
> Some forms documented below are deprecated or removed from the runtime.
> See [core-contract-matrix.md](core-contract-matrix.md) for current feature status.
> Notably: `while`, `:=`, and `try`/`catch`/`throw` are no longer emitted by the compiler
> and are rejected by the VM.

## the computational model

moof has one operation: **send.** everything — arithmetic, slot
access, function calls, control flow — is a message send to an
object.

```moof
[3 + 4]                    ; send + to integer 3 with arg 4
[list map: |x| [x * 2]]   ; send map: to list with a block
obj.x                      ; send slotAt: to obj with symbol 'x
(f x)                      ; send call: to f with arg x
```

moof has six kernel primitives: `vau`, `send`, `def`, `quote`,
`cons`, `eq`. everything else is built from these.

## syntax

### the three bracket species

```moof
(f a b c)            ; applicative call: [f call: (list a b c)]
[obj selector: arg]  ; message send
{ Parent x: 10 }     ; object literal
```

parentheses `()` for function/operative calls.
brackets `[]` for message sends.
braces `{}` for object literals.

### literals

```moof
42                   ; Integer
3.14                 ; Float
"hello"              ; String
'foo                 ; Symbol (interned name)
true false           ; Boolean
nil                  ; Nil (absence, empty list)
```

### collections

```moof
(list 1 2 3)               ; Cons list: (1 2 3)
#[1 2 3]                   ; Table (sequential)
#["x" => 10 "y" => 20]     ; Table (keyed)
#[1 2 "name" => "alice"]   ; Table (both)
```

### blocks (closures)

```moof
|x| [x + 1]                ; one-arg block
|x y| [x + y]              ; two-arg block
|| "thunk"                  ; zero-arg block
```

blocks are objects with a `call:` handler. they close over their
lexical environment. `|x| body` desugars to `(fn (x) body)`.

### object literals

```moof
{ x: 10 y: 20 }                ; object with Object as parent
{ Point x: 3 y: 4 }            ; object with Point as parent
{ Animal name: "rex" legs: 4 } ; slots are fixed at creation
```

slot names are sealed at creation. values are mutable. handlers
are always open (add anytime via `handle:with:`).

### sugar

```moof
'x                   ; (quote x)
obj.x                ; [obj slotAt: 'x]
`(list 1 ,x 3)      ; quasiquote with unquote
```

## special forms

these are handled directly by the compiler and cannot be
overridden (yet — the vision calls for vau-based versions
with compiler optimization via stability analysis).

### def

```moof
(def x 42)
(def greet (fn (name) (str "hello, " name)))
```

binds a name in the global environment.

### if

```moof
(if condition then-expr else-expr)
(if condition then-expr)          ; else is nil
```

`nil` and `false` are falsy. everything else is truthy.

### fn / lambda

```moof
(fn (x) [x + 1])                 ; one-arg function
(fn (x y) [x + y])               ; two-arg
(fn args args)                    ; variadic (rest param)
(fn (head . tail) head)           ; rest param with positional
```

creates a closure. closes over lexical environment.

### let

```moof
(let ((x 1) (y 2))
  [x + y])
```

local bindings. the bindings are visible in the body.

### while

> **DEPRECATED:** The `while` special form is no longer emitted by the compiler.
> Use recursion or `times:` on Integer for iteration instead.

```moof
(while condition body...)
```

loop. evaluates body repeatedly while condition is truthy.
returns nil.

**Status:** Removed from runtime. Kept here for historical reference.

### do

```moof
(do expr1 expr2 expr3)
```

sequence. evaluates each expression, returns the last.

### :=

> **DEPRECATED:** The `:=` mutation form is no longer supported by the compiler.
> Slots are mutable via `slotAt:put:`, but local variable mutation is removed.

```moof
(:= x [x + 1])
```

mutation. rebinds a local variable to a new value.

**Status:** Removed from runtime. Kept here for historical reference.

### quote / quasiquote

```moof
(quote x)        ; => symbol 'x
'x               ; same
`(list 1 ,x 3)  ; quasiquote: x is evaluated, rest is literal
```

### cons / eq

```moof
(cons 1 (cons 2 nil))   ; => (1 2)
(eq x y)                ; identity equality (same bits)
```

kernel primitives. `cons` builds pairs. `eq` checks bit-level
identity (not content equality — use `equal:` for that).

### eval

```moof
(eval '(+ 1 2))          ; evaluate a quoted expression
(eval expr env)           ; evaluate in an environment
```

compiles and executes an AST at runtime. `env` is an object
whose slots become local bindings during evaluation.

### vau

```moof
(vau (params) $env body)
```

creates an operative. unlike `fn`, the arguments are NOT
evaluated — they're passed as raw AST. `$env` binds to the
caller's environment (for `eval`).

```moof
(def my-if (vau (cond then else) $e
  (if (eval cond $e)
    (eval then $e)
    (eval else $e))))
```

vau gives user code compiler-level power. `and`, `or`, `when`,
`unless`, `defn`, `defmethod`, `match` are all vau operatives
defined in `lib/bootstrap.moof`.

### try / error

> **DEPRECATED:** The `try`/`catch` and `error` forms are no longer emitted by the compiler.
> The VM's `TryCatch` and `Throw` opcodes are explicitly rejected at runtime.
> The current error model uses Result/Err values with monadic propagation.
> See [errors.md](errors.md) and [core-contract-matrix.md](core-contract-matrix.md) for details.

```moof
(try body catch: |error| handler)
(error "something went wrong")
```

**Status:** Removed from runtime. Kept here for historical reference.

## bootstrap operatives

defined in `lib/bootstrap.moof` using vau:

### and / or

```moof
(and a b)    ; short-circuit: if a is falsy, return false
(or a b)     ; short-circuit: if a is truthy, return true
```

### when / unless

```moof
(when condition body)     ; if condition, eval body; else nil
(unless condition body)   ; if not condition, eval body; else nil
```

### defn

```moof
(defn name (params) body)
```

sugar for `(def name (fn (params) body))`.

### defmethod

```moof
(defmethod Type selector (params) body)
```

installs a handler on a type prototype. `self` is automatically
bound to the receiver.

```moof
(defmethod Integer double () [self * 2])
[21 double]  ; => 42
```

### list / str

```moof
(list 1 2 3)         ; variadic list construction
(str "x=" x " y=" y) ; variadic string concatenation
```

### match

```moof
(match expr
  (pattern1 result1)
  (pattern2 result2))
```

two-case pattern matching. `_` matches anything.

### other utilities

```moof
(not x)              ; boolean negation
(nil? x)             ; true if x is nil
(some? x)            ; true if x is not nil
(member? x list)     ; true if x is in list
(range n)            ; list of 0..n-1
(apply f args)       ; apply function to argument list
```
