# MOOF Implementation Journal

A record of what was built, why, and what changed along the way. Entries are sequential phases anchored by git commits.

---

## §1 — The kernel (`671b0b5`)

Built the entire runtime from scratch: lexer, parser, compiler, bytecode VM, and a bootstrap library written in MOOF itself.

**Architecture decisions:**
- **Bytecode from day one.** No tree-walking interpreter. Cons-cell AST compiles to bytecode; bytecode is the canonical form.
- **Six kernel primitives.** `vau`, `send`, `def`, `quote`, `cons`, `eq`. Everything else derived.
- **Bootstrap from file.** `lib/bootstrap.moof` loaded at startup, not hardcoded.

What works: REPL with multi-line bracket balancing, prototype-delegation object model (slots + handlers), vau operatives with first-class environments, `doesNotUnderstand:`, source introspection, `(load)`, ~200 lines of stdlib in MOOF.

---

## §2 — The syntax reform (`0de8374`)

Two design problems: redundant symbol syntax (`'x`, `#x`, bare `x`) and ceremony-heavy object creation.

**Changes:** Kill `#name` — `'sym` is the only symbol literal. Add `{ Parent key: value }` object literals with shape-based method detection. Add `obj.x` dot access (tight dots = field access, loose dots = dotted pairs — disambiguated at the lexer level). Add `@x` self-field sugar. Add `fn` and `defmethod` as vau operatives in bootstrap.

Before: 10 lines of boilerplate for a Point with two fields and a method. After: 6 lines.

---

## §3 — TCO, mutation, honest object model (`05877a6`)

**Tail call optimization.** `OP_TAIL_APPLY` / `OP_TAIL_CALL` with tail-position tracking through the compiler. 100k+ recursion without overflow.

**The `<-` operator.** Unified mutation — a vau that inspects its target: `(<- x 42)` for env rebinding, `(<- obj.x 42)` for slot mutation, `(<- @x 42)` for self-field mutation. Pure moof, backed by `set:to:` on environments.

**Type prototypes.** Every primitive type (Integer, Boolean, String, Cons, Nil, Symbol, Lambda, Operative, Environment) gets a real prototype object defined in bootstrap.moof. Handlers are real callable values — native operations registered on the prototypes, visible to introspection. Killed Block (was identical to Lambda). Killed NativeHandler markers (replaced with real callable lambdas).

---

## §4 — String operations (`4f9002d`)

Full string suite: `substring:to:`, `at:`, `indexOf:`, `split:`, `trim`, `startsWith:`, `endsWith:`, `contains:`, `toUpper`, `toLower`, `toSymbol`, `toInteger`, `chars`, `replace:with:`. Symbol gets `asString`/`name`. `++` works with any type via `toString` fallback. Added `str` variadic helper.

Also fixed rest-param binding — `call_lambda` and both TCO paths now use `bind_params` for proper destructuring of positional, rest, and dotted-rest parameter patterns.

---

## §5 — Persistence (`1cafc43`)

The heap persists between sessions. Bootstrap only runs once.

**Snapshot:** serialize the heap slab (`Vec<HeapObject>` + symbol table) via serde + bincode. Content-addressed with SHA-256.

**WAL:** every mutation (alloc, replace, symbol intern) appended to `wal.bin`. Startup replays WAL over last snapshot. Handles truncated entries.

**Heap mutation refactor:** all 7 `get_mut()` call sites replaced with WAL-safe methods (`env_define`, `add_handler`, `set_slot`, `mutate`).

---

## §6 — TUI, floats, FFI (`f13cc76`)

**TUI inspector:** ratatui + crossterm. `(browse)` opens heap browser, `(browse obj)` inspects a value. Navigate objects, environments, lambdas, bytecode.

**Floats:** `Value::Float(f64)` with custom `Eq`/`Hash` (bit-level). Float literals, full arithmetic, math builtins (`sqrt`, `sin`, `cos`, `floor`, `ceil`, `round`).

**FFI:** dynamic C library binding via `libloading`. `ffi-open` loads a library, `ffi-bind` creates callable functions with type signatures.

---

## §7 — Compacting GC, native interface (`e0851b7`)

**Compacting snapshot GC.** On save: mark reachable objects from roots, build forwarding table, serialize only the live set with remapped ids. 142k objects in memory → 1.7k on disk (99% garbage). 42KB images. The persistence layer IS the garbage collector.

**NativeRegistry.** Replaced `HeapObject::ForeignFunction` with general `NativeFunction { name }`. FFI rebuilt as a client of this interface.

---

## §8 — One path for all native code (`df849f9`)

Killed three separate native code paths (hardcoded `primitive_send` match arms, `make_native_lambda` bytecode wrappers, `NativeRegistry` closures). Now ONE mechanism: all native operations are NativeFunction closures in the NativeRegistry. `[3 + 4]` dispatches the same way as `(ffi-sin 1.0)`.

New `src/vm/natives.rs` registers all primitive operations as closures on type prototypes. `make_native_lambda` and `OP_PRIM_SEND` deleted. Float gets its own prototype. `MoofExtension` trait for external Rust code to hook in.
