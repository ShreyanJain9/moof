# MOOF Implementation Journal

A record of what was built, why, and what changed along the way.

---

## Day 3 — TCO, mutation, and the honest object model

Three big changes that make MOOF a real system instead of a toy.

### Tail call optimization

Added `OP_TAIL_APPLY` and `OP_TAIL_CALL` opcodes. The compiler threads a `tail` flag through compilation — the last expression in a body, branches of `if`, last expression in `do`, and the final call in `let` all propagate tail position. When a function call is in tail position, the VM replaces the current frame instead of pushing a new one.

Result: `(sum-tc 100000 0)` computes without stack overflow. Tail-recursive list operations are now practical.

### The `<-` operator

Replaced `set!` with `<-` — a vau operative that inspects its target form at macro-expansion time:
- `(<- x 42)` — symbol target → walks the env chain via `[env set: target to: val]`
- `(<- obj.x 42)` — dot-access target → slot mutation via `[obj slotAt: field put: val]`
- `(<- @x 42)` — self-field target → same as dot-access on self

All three forms in one vau, purely in bootstrap.moof. The only VM addition was `set:to:` on environments (walks the parent chain to find and mutate the binding).

### Type prototypes: the honest object model

The old `primitive_send` was a lie — a giant match statement in Rust that pretended to be message dispatch. You couldn't see it, override it, or introspect it.

**Now every primitive type has a real prototype object:** Integer, Boolean, String, Cons, Nil, Symbol, Lambda, Operative, Environment. They're defined in bootstrap.moof, inherit from Object, and have real handlers.

Non-arithmetic handlers (`describe`, `applicative?`, etc.) are fn lambdas written in MOOF. Arithmetic and other performance-critical operations are **native handler lambdas** — real callable Lambda objects whose body is a single `OP_PRIM_SEND` instruction that goes directly to the VM fast path.

```
(def plus [Integer handlerAt: '+])
(type-of plus)    => 'Lambda
[plus params]     => (self a)
(plus 3 4)        => 7
```

Native handlers are real values. You can extract them, pass them around, call them. They show up in `[Integer interface]`. The fast path is an implementation detail, not a semantic difference.

**Dispatch order in `message_send`:**
1. User handlers on GeneralObjects (delegation chain)
2. Type prototype handlers (real lambdas — native or moof-defined)
3. VM fast path fallback (only during bootstrap, before prototypes are registered)
4. `doesNotUnderstand:`

### Killed: Block

Block was a separate HeapObject variant for `{ :x body }` syntax. But `{}` became object literals, and blocks were semantically identical to lambdas. Removed Block from the heap, compiler, VM, and bootstrap. One less concept, zero expressiveness lost.

### Killed: NativeHandler marker

The first attempt at integrating arithmetic used `NativeHandler` — a non-callable marker type that `message_send` would intercept and route to the fast path. It worked for introspection but was a lie: you couldn't call it, compose it, or pass it to `map`. Replaced with real lambdas using `OP_PRIM_SEND`, which bypasses handler lookup to avoid infinite recursion while being fully callable.

---

## Day 1 — The kernel lives

Built the entire runtime from scratch in one session: lexer, parser, compiler, bytecode VM, and a bootstrap library written in MOOF itself.

**Architecture decisions:**
- **Bytecode from day one.** No tree-walking interpreter phase. The cons-cell AST compiles to bytecode, and bytecode is the canonical form. This means introspection, serialization, and persistence all operate on the same representation.
- **Six kernel primitives.** `vau`, `send`, `def`, `quote`, `cons`, `eq`. Everything else is derived. `if`, `lambda`, `let`, `while` — the compiler knows about them for efficiency, but they're semantically expressible in terms of the six.
- **Bootstrap from file.** The standard library (`lib/bootstrap.moof`) is loaded at startup, not hardcoded. The image should be self-modifying — you should be able to change MOOF from within MOOF.

**What works:**
- REPL with multi-line bracket balancing
- Full prototype-delegation object model (slots + handlers)
- Vau operatives with first-class environments (the reflective tower)
- `doesNotUnderstand:` for proxies and dynamic dispatch
- `(source fn)` introspection on any lambda/operative
- `(load "file.moof")` for loading code from files
- ~200 lines of standard library written in MOOF itself

## Day 8 — One path for all native code

### The unification

Killed the three separate native code paths:
- `primitive_send` match arms (hardcoded Rust for arithmetic, strings, etc)
- `make_native_lambda` (fake Lambda bytecode wrappers with OP_PRIM_SEND)
- `NativeRegistry` (Rust closures for FFI only)

Now there's ONE path: everything is a NativeFunction closure in the NativeRegistry. `[3 + 4]` and `(ffi-sin 1.0)` dispatch through the exact same mechanism.

The new `src/vm/natives.rs` (~500 lines) registers all primitive operations — integer arithmetic, float math, string ops, list ops, symbol conversion, introspection — as NativeFunction closures. Each gets added as a handler on the corresponding type prototype. `make_native_lambda` and `OP_PRIM_SEND` are deleted.

Float got its own prototype (was sharing Integer's). `MoofExtension` trait added for external Rust code to hook in cleanly. `primitive_send` shrunk from ~470 lines to ~150 (only bootstrap fallback and VM-dependent ops like eval:/call:).

---

## Day 7 — Native extension interface, compacting GC

### The native extension interface

Replaced the FFI-specific HeapObject::ForeignFunction with a general NativeRegistry. Any Rust code can register a native function:

```rust
vm.register_native("math:sin", Box::new(|heap, args| {
    let x = args[0].as_float().ok_or("expected float")?;
    Ok(Value::Float(x.sin()))
}));
```

The function becomes a callable NativeFunction heap object, dispatched through the same call_value / OP_APPLY paths as lambdas. FFI is rebuilt on top: `ffi-bind` now registers a NativeFn closure that captures the C function pointer. Same interface whether you're binding libm via dlopen or exposing a Rust HTTP client.

### Compacting GC via snapshot

The heap grows freely during a session (no GC pauses). On save, a mark-and-compact pass traces from the root environment, builds a forwarding table, and serializes only the live set with remapped ids.

In practice: 142,646 objects in memory → 1,692 saved to disk (99% garbage). The image file is 42KB. The persistence layer IS the GC — garbage never hits disk.

---

## Day 6 — TUI inspector, floats, and FFI

### TUI Inspector

Built with ratatui + crossterm. `(browse)` opens a heap browser, `(browse obj)` inspects a specific value. Navigate with arrow keys, Enter to drill into references, Backspace to go back, q to quit. Shows objects with their slots/handlers, environments with bindings, lambdas with source/params, cons lists, bytecode chunks, and foreign functions.

### Floats (f64)

Added `Value::Float(f64)` to the value representation. Custom `Eq`/`Hash` using bit representation (so NaN == NaN). Float literals in the lexer: `3.14` parses as Float when a `.` is followed by digits. Full arithmetic (+, -, *, /, %, comparisons), plus math builtins: `sqrt`, `sin`, `cos`, `floor`, `ceil`, `round`. Auto-promotion: `as_float()` accepts both Float and Integer.

### FFI — Dynamic C library binding

Load any C library and call its functions from MOOF:

```
(def libm (ffi-open "m"))
(def sin (ffi-bind libm "sin" '(f64) 'f64))
(sin 1.5707963)  => ~1.0
```

Uses `libloading` for dlopen/dlsym. Supports common calling signatures via unsafe transmute dispatch: `() -> T`, `(T) -> T`, `(T, T) -> T` for T in {i64, f64, string}. No libffi dependency — just pattern-matching on type signatures.

Foreign functions are real heap objects (`HeapObject::ForeignFunction`). They're callable through the normal `(f args)` path — OP_APPLY recognizes them and dispatches to the FFI bridge. Type signatures are stored for serialization, so FFI bindings survive in the image (libraries need re-opening on load).

---

## Day 5 — Persistence: the image lives

The most transformative change yet. MOOF is now a **living environment** — the heap persists between sessions.

### Architecture

The persistence layer has three components:

1. **Snapshot** (`persistence/snapshot.rs`) — serialize the entire heap (`Vec<HeapObject>` + `Vec<String>` symbol table) to disk via serde + bincode. Content-addressed with SHA-256. Stored in `.moof/image.bin`.

2. **WAL** (`persistence/wal.rs`) — every heap mutation (alloc, replace, symbol intern) is appended to `.moof/wal.bin` in real time. On startup: load snapshot, replay WAL, resume. On clean exit: save snapshot, clear WAL. Handles truncated entries gracefully (crash mid-write → partial entry skipped).

3. **Heap mutation refactor** — replaced all 7 `heap.get_mut()` call sites with specific WAL-safe methods: `heap.env_define()`, `heap.add_handler()`, `heap.set_slot()`, `heap.mutate()`. The heap now has a single mutation interface that logs everything.

### Startup flow

```
if .moof/image.bin exists:
  load snapshot → replay WAL → register type prototypes → run REPL
else:
  bootstrap from scratch → register prototypes → run REPL
on exit:
  save snapshot → clear WAL
```

Bootstrap only runs once. After that, the image IS the truth.

### `(checkpoint)` / `(save)`

REPL commands that force a snapshot save without exiting.

---

## Day 4 — String operations and the rest-param fix

### String ops

Full string manipulation suite on the String prototype: `substring:to:`, `at:`, `indexOf:`, `split:`, `trim`, `startsWith:`, `endsWith:`, `contains:`, `toUpper`, `toLower`, `toSymbol`, `toInteger`, `chars`, `replace:with:`. All registered as native handler lambdas, all introspectable via `[String interface]`.

Symbol gets `asString`/`name` for extracting the raw name without the `'` prefix. Integer gets `asString`. Concatenation (`++`) now works with any type, not just strings — falls back to `toString`.

Added `str` helper to bootstrap: `(str "count: " 42 " done")` → `"count: 42 done"`. Variadic, clean.

### The rest-param fix

Found that `call_lambda` was using `list_to_vec` + positional indexing for param binding, which broke rest params. `(fn args body)` (bare symbol = capture all args) was silently only binding the first arg. Fixed all three call paths (call_lambda, OP_TAIL_APPLY, OP_TAIL_CALL) to use `bind_params`, which correctly handles positional, rest, and destructuring patterns.

This also fixed dotted-rest params like `(fn (a b . rest) ...)` in the TCO paths.

---

## Day 2 — The great syntax reform

Tackled two design problems that were making the language awkward to use day-to-day.

### Problem 1: Symbol syntax was redundant

Three ways to say "the symbol x": `'x`, `#x`, bare `x`. Two too many.

**Resolution:** Kill `#name` entirely. `'sym` is the only symbol literal. Removed `HashSymbol` from the lexer and all references in bootstrap/geometry.

### Problem 2: Objects were ceremony-heavy

Creating an object required `(object Parent #x 10)` with separate `(handle! obj #method (lambda (self) ...))` calls. Not declarative, too much plumbing.

**Resolution — a suite of changes:**

1. **Object literals with `{}`** — `{ Parent x: 10 y: 20 }` for thin data objects. Shape-based method detection: `key: value` is a slot, `key: (params) body...` is a method (auto-injects `self`).

2. **`obj.x` dot access** — Parser desugars tight dots (no whitespace) to `(%dot obj 'field)`, new `OP_SLOT_GET` bytecode. All slots are public. Had to disambiguate tight dots (field access) from loose dots (dotted pairs in `(a . b)`) at the lexer level — tight dots emit `DotAccess` token, loose dots emit `Dot`.

3. **`@x` self-field sugar** — `@x` desugars to `(%dot self 'x)`. One character to read your own slots inside a method body.

4. **`fn` as a vau operative** — Not compiler sugar. `fn` is defined in `bootstrap.moof` as a vau that constructs and evals a `lambda` form. Lives in MOOF, not the compiler. This is important: it means `fn` could be redefined by the user.

5. **`defmethod`** — Also a vau operative in bootstrap. `(defmethod Point describe () "a Point")` auto-prepends `self` to the param list and desugars to `handle!`.

### The dot ambiguity bug

The dot-access postfix logic in `parse_expr` was greedily consuming the `.` in dotted pairs like `(a . b)`. The parser saw `a.b` (field access) instead of `a . b` (dotted pair tail). Fixed by tracking whitespace in the lexer: tight dots (no preceding whitespace) emit `DotAccess`, loose dots emit `Dot`. Parser uses `DotAccess` for field chains and `Dot` for dotted pairs. Clean separation.

### Before and after

```lisp
; BEFORE
(def Point (object Object))
(handle! Point #x (lambda (self) [self slotAt: #x]))
(handle! Point #y (lambda (self) [self slotAt: #y]))
(handle! Point #describe (lambda (self) "a Point"))
(handle! Point #distanceTo:
  (lambda (self other)
    (let ((dx [[self x] - [other x]])
          (dy [[self y] - [other y]]))
      [[dx * dx] + [dy * dy]])))
(def make-point
  (lambda (x y) (object Point #x x #y y)))
(def pt (make-point 3 4))

; AFTER
(def Point { Object
  describe: () "a Point"
  distanceTo: (other)
    (let ((dx [@x - other.x])
          (dy [@y - other.y]))
      [[dx * dx] + [dy * dy]])
})
(def pt { Point x: 3 y: 4 })
```

Night and day.
