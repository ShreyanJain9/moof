# MOOF Implementation Journal

A canonical record of what was built, why, what went wrong, and what changed along the way. Sequential phases anchored by git commits.

---

## §1 — The kernel (`671b0b5`)

Built the entire runtime from scratch: lexer, parser, compiler, bytecode VM, and a bootstrap library written in MOOF itself.

**Architecture decisions:**

- **Bytecode from day one.** The design doc mandates bytecode as the canonical form (§9.2) — what gets serialized, what introspection operates on. No tree-walking interpreter phase. The cons-cell AST compiles to `BytecodeChunk`s containing a `Vec<u8>` of opcodes and a `Vec<Value>` constant pool.

- **Six kernel primitives.** `vau` (create operative), `send` (message dispatch), `def` (bind), `quote` (literal data), `cons` (pair), `eq` (identity). These get dedicated bytecodes. Everything else — `if`, `lambda`, `let`, `while`, `cond` — is either compiled as a derived form for efficiency or defined as a vau operative in bootstrap.moof.

- **Arena heap with tagged values.** `Value` is a 6-variant enum: `Nil`, `True`, `False`, `Integer(i64)`, `Symbol(u32)`, `Object(u32)`. Heap objects are a `Vec<HeapObject>` indexed by `u32`. Symbols are interned into a separate string table. This design means "serialize the slab" is literally the persistence strategy.

- **Bootstrap from file.** The standard library (`lib/bootstrap.moof`) is loaded at startup. Not hardcoded. The image should be self-modifying — you should be able to change MOOF from within MOOF.

- **Prototype delegation.** Objects have slots (storage, private) and handlers (behavior, public, inherited). `doesNotUnderstand:` enables proxies and dynamic dispatch.

**What works after §1:** REPL with multi-line bracket balancing, full prototype-delegation object model, vau operatives with first-class environments (the reflective tower), `doesNotUnderstand:` for proxies, `(source fn)` introspection on any lambda/operative, `(load "file.moof")` for files, ~200 lines of standard library in MOOF itself.

**Bugs encountered:**
- Frame depth recursion: `self.execute()` called `self.run()` which looped until ALL frames empty, eating parent frames. Fixed by passing `base_depth` to `run()`.
- Lambda vs operative distinction at runtime: solved by convention — `env_param == "$_"` means lambda, otherwise operative.
- `#sourceOf:` parsing: hash symbol parser didn't handle trailing colons. Fixed lexer to consume `#name:` and `#name:more:`.
- `#x` in object construction was being looked up as a variable instead of treated as a literal. Fixed parser to emit `(quote name)`.

---

## §2 — The syntax reform (`0de8374`)

Two design problems were making the language awkward to use day-to-day.

**Problem 1: Symbol syntax was redundant.** Three ways to say "the symbol x": `'x`, `#x`, bare `x`. Two too many. Resolution: kill `#name` entirely. `'sym` is the only symbol literal. Removed `HashSymbol` from the lexer and all references in bootstrap/geometry.

**Problem 2: Objects were ceremony-heavy.** Creating an object required `(object Parent #x 10)` with separate `(handle! obj #method (lambda (self) ...))` calls. Not declarative, too much plumbing.

**Resolution — five coordinated changes:**

1. **Object literals with `{}`** — `{ Parent x: 10 y: 20 }` for thin data objects. Shape-based method detection in the parser: `key: value` is a slot, `key: (params) body...` is a method (auto-injects `self`). Parser emits `%object-literal` AST nodes; compiler handles them by emitting `OP_MAKE_OBJECT` + `OP_HANDLE` chains.

2. **`obj.x` dot access** — Parser desugars tight dots (no preceding whitespace) to `(%dot obj 'field)`, new `OP_SLOT_GET` bytecode reads the slot directly. All slots public. Clearly distinct from `[obj method]` which goes through handler dispatch.

3. **`@x` self-field sugar** — `@x` desugars to `(%dot self 'x)`. Lexer produces `AtField` token. One character to read your own slots inside a method body.

4. **`fn` as a vau operative** — Not compiler sugar. Defined in bootstrap.moof as `(def fn (vau (params . body) $e (eval (cons 'lambda (cons params body)) $e)))`. Lives in MOOF, not the compiler. `fn` could be redefined by the user.

5. **`defmethod`** — Also a vau operative in bootstrap. `(defmethod Point describe () "a Point")` auto-prepends `self` to the param list and desugars to `handle!`.

**The dot ambiguity bug.** The dot-access postfix logic in `parse_expr` was greedily consuming the `.` in dotted pairs like `(a . b)`. Parser saw `a.b` (field access) instead of `a . b` (dotted pair tail). Every vau with rest params was silently broken. Fixed by tracking whitespace in the lexer: tight dots (no preceding whitespace) emit `DotAccess`, loose dots emit `Dot`. Parser uses `DotAccess` for field chains and `Dot` for dotted pairs. Clean separation.

**Before and after:**
```lisp
; BEFORE (10 lines of ceremony)
(def Point (object Object))
(handle! Point #x (lambda (self) [self slotAt: #x]))
(handle! Point #y (lambda (self) [self slotAt: #y]))
(handle! Point #describe (lambda (self) "a Point"))
(handle! Point #distanceTo:
  (lambda (self other)
    (let ((dx [[self x] - [other x]])
          (dy [[self y] - [other y]]))
      [[dx * dx] + [dy * dy]])))
(def make-point (lambda (x y) (object Point #x x #y y)))

; AFTER (6 lines, declarative)
(def Point { Object
  describe: () "a Point"
  distanceTo: (other)
    (let ((dx [@x - other.x])
          (dy [@y - other.y]))
      [[dx * dx] + [dy * dy]])
})
(def pt { Point x: 3 y: 4 })
```

---

## §3 — TCO, mutation, unified object model (`05877a6`)

Three changes that make MOOF a real system instead of a toy.

### Tail call optimization

Added `OP_TAIL_APPLY` and `OP_TAIL_CALL` opcodes. The compiler threads a `tail: bool` flag through compilation — the last expression in a function body, both branches of `if`, the last expression in `do`, and the final call in `let` all propagate tail position. When a function call is in tail position, the VM replaces the current frame instead of pushing a new one.

Result: `(sum-tc 100000 0)` = 5,000,050,000 without stack overflow.

### The `<-` operator

Replaced `set!` (which just re-def'd in local scope) with `<-` — a vau operative that inspects its target form at expansion time:
- `(<- x 42)` — symbol target → walks the env chain via `[$e set: target to: val]`
- `(<- obj.x 42)` — `%dot` form → slot mutation via `[obj slotAt: field put: val]`
- `(<- @x 42)` — same path (parser already desugars `@x` to `(%dot self 'x)`)

All three in one vau, purely in bootstrap.moof. The only VM addition: `set:to:` message on environments that walks the parent chain to find and mutate the binding.

### Type prototypes: making the object model honest

The old `primitive_send` was a lie — a giant match statement in Rust that pretended to be message dispatch. You couldn't see it, override it, or introspect it. `[3 describe]` worked because of a hidden match arm, not because Integer had a handler.

**Now every primitive type has a real prototype object.** Integer, Boolean, String, Cons, Nil, Symbol, Lambda, Operative, Environment — all defined in bootstrap.moof, inheriting from Object. Non-arithmetic handlers (`describe`, `applicative?`, etc.) are fn lambdas in MOOF. Arithmetic was registered as native handler lambdas — real callable Lambda objects whose body used `OP_PRIM_SEND` to route to the fast path. (This intermediate representation was later replaced in §8.)

Dispatch order in `message_send`: (1) user handlers on GeneralObjects, (2) type prototype handlers, (3) VM fast path fallback, (4) `doesNotUnderstand:`.

**Killed Block.** Block was a separate HeapObject variant for `{ :x body }` syntax. But `{}` became object literals in §2, and blocks were semantically identical to lambdas. Removed from heap, compiler, VM, and bootstrap. One less concept, zero expressiveness lost.

---

## §4 — String operations (`4f9002d`)

Full string manipulation suite: `substring:to:`, `at:`, `indexOf:`, `split:`, `trim`, `startsWith:`, `endsWith:`, `contains:`, `toUpper`, `toLower`, `toSymbol`, `toInteger`, `chars`, `replace:with:`. All registered as native handler lambdas on the String prototype, all introspectable via `[String interface]`.

Symbol gets `asString`/`name` for extracting the raw name without the `'` prefix. Integer gets `asString`. Concatenation (`++`) now works with any type — falls back to `toString` on the other operand.

Added `str` variadic helper to bootstrap: `(str "count: " 42 " done")` → `"count: 42 done"`. Uses `fold` over rest args.

**The rest-param bug.** `call_lambda` was using `list_to_vec` + positional indexing for param binding. This broke rest params: `(fn args body)` (bare symbol = capture all args as a list) was silently binding only the first arg. Fixed all three call paths (`call_lambda`, `OP_TAIL_APPLY`, `OP_TAIL_CALL`) to use `bind_params`, which correctly handles positional, rest, and dotted-rest `(a b . rest)` patterns via recursive cons-tree destructuring.

---

## §5 — Persistence (`1cafc43`)

The most transformative change. MOOF is now a living environment — the heap persists between sessions. Bootstrap only runs once, ever.

### Architecture

Three components:

1. **Snapshot** (`persistence/snapshot.rs`). Serialize the entire heap (`Vec<HeapObject>` + `Vec<String>` symbol table) via serde + bincode. Content-addressed with SHA-256. Stored in `.moof/image.bin`.

2. **WAL** (`persistence/wal.rs`). Every heap mutation (alloc, replace, symbol intern) is appended to `.moof/wal.bin` in real time. Each entry is length-prefixed bincode. On startup: load snapshot, replay WAL, resume. Handles truncated entries gracefully (crash mid-write → partial entry skipped on replay).

3. **Heap mutation refactor.** Replaced all 7 `heap.get_mut()` call sites with specific WAL-safe methods: `heap.env_define()`, `heap.add_handler()`, `heap.set_slot()`, `heap.mutate()`. The heap has a single mutation interface that logs everything. `get_mut` kept only as a bootstrap escape hatch.

### Startup flow

```
if .moof/image.bin exists:
    load snapshot → replay WAL → register type prototypes → REPL
else:
    fresh bootstrap → register prototypes → REPL
on clean exit:
    save snapshot → clear WAL
```

`(checkpoint)` / `(save)` REPL commands for manual snapshots mid-session.

---

## §6 — TUI, floats, FFI (`f13cc76`)

### TUI inspector

Built with ratatui + crossterm. `(browse)` opens a heap browser (paginated list of all objects by type), `(browse obj)` inspects a specific value. Enter to drill into references, Backspace to go back, q to quit. Shows objects with their slots/handlers/parent chain, environments with bindings, lambdas with source/params/def_env, cons lists with both pair and list views, bytecode chunks with constant pools.

### Floats

Added `Value::Float(f64)` — the seventh Value variant. Custom `PartialEq`/`Eq`/`Hash` using bit representation (so NaN == NaN and can be used as hash keys). Float literals in the lexer: `3.14` — detected when a decimal point is followed by digits during number reading. Full arithmetic (+, -, *, /, %, comparisons), plus math builtins: `sqrt`, `sin`, `cos`, `floor`, `ceil`, `round`, `toInteger`. Auto-promotion: `as_float()` accepts both Float and Integer.

### FFI — dynamic C library binding

`ffi-open` loads a native library via `libloading` (dlopen/dlsym). `ffi-bind` looks up a symbol and creates a callable function with a type signature:

```lisp
(def libm (ffi-open "m"))
(def sin (ffi-bind libm "sin" '(f64) 'f64))
(sin 1.5707963)  ; => ~1.0
(def pow (ffi-bind libm "pow" '(f64 f64) 'f64))
(pow 2.0 10.0)   ; => 1024.0
```

Supports common calling signatures without libffi — dispatches via unsafe transmute on `(arg_types, ret_type)` patterns: `() → T`, `(T) → T`, `(T,T) → T` for T in {i64, f64, string, void, pointer}. Type signatures stored in the heap object for serialization. Libraries need re-opening on image load (they're runtime resources).

---

## §7 — Compacting GC, native interface (`e0851b7`)

### Compacting snapshot GC

The heap grows freely during a session (no GC pauses, no runtime overhead). On save, a mark-and-compact pass runs:

1. **Mark:** DFS from root environment, tracing all `Value::Object` references through Cons, GeneralObject, Lambda, Operative, Environment, BytecodeChunk fields.
2. **Forward:** Build `HashMap<u32, u32>` mapping old ids to new sequential ids for marked objects only.
3. **Rewrite:** Create a new `Vec<HeapObject>` with only marked objects, all references updated through the forwarding table — including `body`/`def_env` u32 fields in Lambda/Operative and `parent` in Environment.

In practice: 142,646 objects in memory → 1,692 saved to disk (99% garbage). Image file is 42KB. The persistence layer IS the garbage collector.

### NativeRegistry

Replaced `HeapObject::ForeignFunction` (FFI-specific) with general `HeapObject::NativeFunction { name }`. Any Rust closure can register as a native function. FFI rebuilt as a client of this interface. This was the first step toward §8's full unification.

---

## §8 — One path for all native code (`df849f9`)

The architecture had accumulated three separate paths for native Rust code: (1) `primitive_send` — a ~470-line match tree handling messages for every built-in type, (2) `make_native_lambda` — a factory that created fake Lambda heap objects with `OP_PRIM_SEND` bytecode bodies routing back to primitive_send, (3) `NativeRegistry` — Rust closures for FFI only, accessed via NativeFunction heap objects. `[3 + 4]` went through a completely different dispatch path than `(ffi-sin 1.0)`.

**The unification:** all native operations become NativeFunction closures in the NativeRegistry. New `src/vm/natives.rs` (~500 lines) registers every primitive operation — integer arithmetic, float math, string manipulation, list ops, symbol conversion, introspection — as closures on the corresponding type prototypes. `make_native_lambda` and `OP_PRIM_SEND` deleted. `primitive_send` shrunk from ~470 to ~150 lines (bootstrap fallback + VM-dependent ops like `eval:`/`call:`).

Float gets its own prototype (was incorrectly sharing Integer's — `as_integer()` returns None for floats). `MoofExtension` trait added for external Rust code to hook in:

```rust
pub trait MoofExtension {
    fn register(&self, vm: &mut VM, root_env: u32);
}
```

A plugin calls `vm.register_native(name, closure)` to get back a `Value`, then binds it wherever it wants via `vm.heap.env_define()`. No magic, no special-casing. Same mechanism whether you're implementing integer addition or binding a Rust HTTP client.
