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

---

## §9 — call: invariant, quasiquote, orthogonal persistence

### call: invariant (DESIGN.md §3.1/§4.4)

The design doc's most fundamental invariant: `(f a b c)` is `[f call: a b c]`. Removed Lambda and NativeFunction special cases from OP_APPLY — only Operative keeps its special path (fundamental to vau: operatives receive unevaluated args). Everything else goes through `message_send(callable, sym_call, &evaled)`. A user object with a `call:` handler is now callable with applicative syntax:

```lisp
(def adder (object Object))
(handle! adder 'call: (fn (self a b) [a + b]))
(adder 3 4)        ; => 7 — same as [adder call: 3 4]
```

OP_TAIL_APPLY keeps the Lambda fast path for performance (avoids frame overhead) but semantically it's the same dispatch.

### Quasiquote

`` `(a ,x c) `` where x=42 → `(a 42 c)`. Lexer adds backtick (`` ` ``), comma (`,`), and comma-at (`,@`) tokens. Parser desugars to `(quasiquote ...)` / `(unquote ...)` / `(unquote-splicing ...)` forms. Compiler recursively walks the AST: atoms are quoted, `(unquote x)` compiles x normally (evaluates it), cons cells recursively quasiquote car and cdr then OP_CONS. Unquote-splicing deferred to a follow-up.

Enables real metaprogramming:
```lisp
(def make-def (fn (name val) `(def ,name ,val)))
(make-def 'y 99)  ; => (def y 99)
```

### "You never save"

Auto-checkpoint every 5000 heap allocations. The WAL catches every mutation between checkpoints. The image persists continuously — users never need to think about saving. Moves toward the design doc's §6.1 vision of true orthogonal persistence.

---

## §10 — OO standard library (`acb41e5`, `3aadae7`)

### OO rewrite

Rewrote the standard library as proper moof objects:

- **Assoc** — wraps a key-value pair list. `[m get: key]`, `[m set: key to: val]`, `[m has: key]`, `[m keys]`, `[m values]`, `[Assoc from: k1 v1 k2 v2]`. Multi-keyword methods via `handle!` with `["set:to:" toSymbol]` pattern (the `{}` literal parser can't concatenate multiple keywords into one selector).

- **JSON** — singleton object. `[JSON parse: str]` returns Assocs for objects, lists for arrays. `[JSON serialize: val]` produces JSON strings. Full round-trip including nested structures. Parser is recursive descent in pure moof using string `=` for content comparison (not `eq` which is identity).

- **Membrane/Facet/LoggingMembrane** — capability wrappers as objects. `[Membrane wrap: target on-send: handler]`, `[Facet wrap: target allowing: selectors]`.

- **List methods on Cons** — `any:`, `every:`, `find:`, `flatMap:`, `sortBy:`, `sort`, `join:`. All nil-safe (check cdr before recursing).

### Unquote-splicing

Added `OP_APPEND` opcode. Compiler detects `,@` in quasiquoted lists, builds with segments + append instead of simple cons. Bootstrap rewritten to use quasiquote everywhere — `defmethod` went from a 3-line cons/list nightmare to a readable one-liner.

### Bugs found

- `eq` is identity, not content equality. `(eq "a" "a")` is false for separately allocated strings. Added `String.=` for content comparison. All string-comparing code (Assoc keys, JSON parser) must use `[a = b]`, not `(eq a b)`.
- Cons methods that recurse via `[(cdr self) method: arg]` crash on nil (nil isn't a Cons). Fixed by checking `(null? (cdr self))` before recursing.
- `{}` object literals can't handle multi-keyword selectors (`set:to:` becomes two separate keywords). Multi-keyword methods must use `handle!` with `["sel:name:" toSymbol]`.

---

## §12 — The module system and the death of image.bin (`8cb3db2` → `54c43db`)

Five commits in one session. The biggest architectural change since §1. Moof got a module system, lost its binary image, and the persistence model was rewritten three times before settling.

### The standard class library (`8cb3db2`)

Added `lib/classes.moof` with eight reusable prototypes: Stack, Queue, Set, Counter, Range, Pair, Box, EventEmitter. These exercise the prototype-delegation object model and provide genuine utility. Added to the stdlib load list, checkpointed into the binary image.

### Attempt 1: source files as truth (`b7f2d99`)

The original image.bin serialized the heap as a bincode blob with SHA-256 hashing and a WAL for crash recovery. The problem: serialization destroyed comments, sugar, and documentation. The image could never replace raw source — it was lossy.

**Solution: a source-level module system.** Each `.moof` file declares a module header:

```scheme
(module collections
  (requires bootstrap)
  (provides Assoc assoc-equal? flat-map flatten find any? every? sort sort-by join))
```

Modules load in topological order (Kahn's algorithm with BTreeSet for deterministic tie-breaking). Each module evaluates in a sandboxed environment that has only its declared dependencies. The compiler gained `CompilerMode::Sandboxed` which rejects `print`, `load`, `eval-string`, `ffi-open`, `ffi-bind` at compile time.

**Architecture:**
- `src/modules/mod.rs` — `ModuleDescriptor` struct
- `src/modules/graph.rs` — `ModuleGraph` with topo sort, cycle detection (DFS), removal safety checks, transitive dependent queries
- `src/modules/loader.rs` — `ModuleLoader` with discover, load, reload, merge
- `src/modules/sandbox.rs` — isolated env construction
- `src/compiler/compile.rs` — `CompilerMode` enum gates restricted forms

**Bootstrap ordering problem:** after bootstrap loads, native handlers (toSymbol, +, -, etc.) must be registered before other modules can run. Fixed by splitting `load_all` into per-module `load_one`, with `register_type_prototypes` called between bootstrap and everything else.

**lib/ was declared canonical.** Binary image removed from git. `.moof/` gitignored. All REPL commands were string-parsed: `(modules)`, `(module-source name)`, `(module-reload name)`, etc.

### Attempt 2: directory-based image (`f6309d3`)

The user pointed out that the serialized image should canonically replace lib/, not the other way around. But the image must preserve source text.

**Key insight: the "image" isn't a binary blob — it's a directory.**

```
.moof/
  manifest.moof          ; load order, per-module SHA-256 hashes, global hash
  modules/
    bootstrap.moof       ; full source, comments and all
    collections.moof
    ...
```

The manifest is the integrity anchor. Global hash = SHA-256 of all source hashes concatenated in order. Verified on load. Each module file IS the source code — comments, sugar, whitespace preserved perfectly.

**Gutted:** `snapshot.rs` (binary serialization), `wal.rs` (write-ahead log), `source_project.rs` (one-way projection), WAL from `heap.rs`. The heap became a clean arena — no more WAL logging in `alloc`, `mutate`, `intern`.

**Gutted:** `lib/` directory. Everything lives in `.moof/modules/`. The `--seed` flag reads from `lib/` as a one-time bootstrap. `(export-modules lib)` writes back for git.

Startup is robust: always re-discovers from `.moof/modules/` regardless of manifest state. Stale manifests, missing manifests, external edits — all handled. Manifest rebuilt on every save.

**Bug found and fixed (`1441bac`):** `module-remove` wasn't saving the manifest, leaving stale entries that prevented startup. Fixed by having `remove()` delete the .moof file, clean the graph, and re-save.

### Attempt 3: in-image development (`c521181`)

Added `(define-in module (def name ...))` for defining into modules from the REPL. Source-level append + dedup: redefining a symbol removes the old `(def name ...)` form, keeps the last. Provides header auto-updated.

Also: `(module-create name (requires ...))`, `(which-module symbol)`.

### The moof philosophy turn (`54c43db`)

String-parsed REPL commands aren't moof philosophy. Everything should be objects and messages.

Added two general-purpose I/O natives: `read-file` and `write-file`. With those, the entire module API can be written in moof itself.

**`modules.moof`** defines `Module` and `Modules` prototypes:

```scheme
[Modules list]                          ; => ("bootstrap" "collections" ...)
[Modules which: "Stack"]                ; => "classes"
[Modules named: "geometry"]             ; => <Module geometry>
[[Modules named: "geometry"] source]    ; => full source with comments
[[Modules named: "geometry"] exports]   ; => ("Point")
[Modules create: "my-lib" requires: (list "bootstrap")]
```

**Workspace autosave:** every `(def ...)` at the REPL is automatically appended to `workspace.moof`. Survives restart. `[Modules which: "my-thing"]` → `"workspace"`.

**Registration bridge:** after all modules load, Rust populates `Modules._modules` by eval'ing registration calls, then sets source text directly on each Module object's slot (too large for inline eval). This is the only Rust→moof bridge; everything else is pure moof.

### What went wrong

- **Symlink self-copy disaster:** during testing, created a symlink `lib → .moof/modules` and ran `--seed`, which copied each file over itself. All module files zeroed. Recovered from git. Added robustness: startup always re-discovers rather than trusting the manifest.

- **Sandbox env mismatch:** early version loaded all modules, then registered type prototypes. But `collections.moof` uses `["set:to:" toSymbol]` which sends `toSymbol` to a String — a native handler. Fixed by registering prototypes between bootstrap and subsequent modules.

- **Rust→moof dispatch mismatch:** initial attempt called `vm.message_send` directly from Rust to register modules with the Modules object. The arguments didn't dispatch correctly (types showed as Object instead of String). Fixed by using `eval_source` to eval moof expressions, then setting large source texts via direct heap slot manipulation.

- **Workspace creation timing:** workspace.moof was initially created before the `modules` module existed, so its requires list didn't include `modules`. REPL defs that referenced `Modules` failed with "Unbound symbol" when autosaved to workspace. Fixed by regenerating workspace after `modules` loads.

### Current architecture

```
Startup:
  VM::new() → bootstrap_env(nil/true/false)
  → discover .moof/modules/*.moof
  → parse headers, build dependency graph
  → topo sort → load each in sandboxed env
  → register natives after bootstrap
  → merge exports into root
  → populate Modules registry
  → create workspace (if missing)
  → REPL

Persistence:
  .moof/manifest.moof — integrity hash + load order
  .moof/modules/*.moof — source files (canonical)
  autosave on define-in, module-create, module-remove
  workspace autosave on every (def ...) at REPL

No binary image. No WAL. No compaction.
Source files are the image. The directory IS the objectspace.
```

---

## §11 — Source projection, MCP server, eval-string (`4761f23`)

### Source projection (design doc §6)

On every checkpoint, walks the root environment and dumps one `.moof` file per named definition into `.moof/source/`. Objects reconstructed as `{ Parent slot: val ... }` with handler source ASTs. Lambdas use their stored source. Simple values get literals.

The source directory is committable — `git diff` shows exactly which objects changed and how. The binary `image.bin` is a fast-load cache. Source is the diffable truth. `.gitignore` updated to only exclude `.moof/wal.bin`.

### MCP server (design doc §8)

`cargo run -- --mcp` launches a JSON-RPC 2.0 server over stdio. Built on the JSON and Assoc libraries in pure moof (`lib/mcp.moof`). Handles:
- `initialize` → protocol version, capabilities, server info
- `tools/list` → tool registry (extensible)
- `tools/call` → evaluates moof expressions via `eval-string`, returns results

### eval-string

New compiler form + `OP_EVAL_STRING` opcode. Parses and evaluates a moof expression from a runtime string. Needed for MCP's tools/call (receives expression as JSON string, needs to eval it in the live image).

### let-seq

Sequential let bindings (Scheme's `let*`). Named `let-seq` because `*` is an operator character in the lexer. Each binding visible to the next. Implemented as a quasiquote-powered vau that nests `let` forms.

---

## §13 — Image v3: the heap becomes truth (`92b0ff8`..`HEAD`)

The largest architectural change since the module system. Three sessions, three contributors (codex wrote the plan revision, gemini did a first implementation pass, claude did the heap-walker rewrite and moof migration). The result: moof is now a self-modifiable living image.

### The problem

The system had a source/image duality. Rust-side `ModuleLoader` owned HashMaps of module state (source texts, exports, environments, dependency graph) that duplicated what should have lived as moof objects. The binary image serialized the heap but couldn't actually restore module state — every startup re-parsed, re-compiled, and re-evaluated everything from source files. The design doc says "things just survive." We had the opposite.

### What changed

**Heap-walker infrastructure.** The ModuleLoader stopped owning module state. New methods on the VM (`read_slot`, `all_module_ids`, `definitions_list`, `definition_source`) walk the heap to read ModuleImage and Definition objects directly. The Rust struct went from five fields to one (`image_dir: PathBuf`). All consumers — `merge_into_root`, `save_image`, `reload`, `remove`, the `(modules)` command — read from the heap now.

**Definition objects.** Every top-level form in every module becomes a `Definition` object on the heap during loading: name, source text, kind, owning module. The `split_into_definitions` parser walks the module body and creates one Definition per form. On `define_in`, existing Definitions are updated (not appended) by name. Definitions are the canonical source — `.moof` files are projections.

**Source projection.** `save_image` now projects source files from Definition objects: builds the module header from ModuleImage slots (name, requires, provides, unrestricted), concatenates each Definition's source slot. Round-trips verified: delete image.bin → load from source → checkpoint → load from projected source → identical behavior.

**Bootstrap stub and migration.** ModuleImage and Definition prototypes are defined in bootstrap.moof, but the Modules registry is defined in modules.moof (which loads later). Solution: create a stub Modules object with list-backed handlers before loading begins, then migrate all registered ModuleImage objects to the real Assoc-backed Modules after modules.moof loads. The stub is garbage after migration.

**Image resume actually works.** Loading from `image.bin` now deserializes the heap, re-registers native functions, and goes straight to REPL. No parsing, no compiling, no evaluating. All ModuleImage objects, Definition objects, the Modules registry, all module environments with bindings — everything is in the heap. The 25-line "compromise" comment block that said "re-eval everything anyway" is gone.

**Unrestricted module environments.** Modules marked `(unrestricted)` now get `root_env` as their environment parent, so they can access VM-level natives like `__save-image` and `__eval-in`. Sandboxed modules still get isolated environments.

### Shrinking Rust, growing moof

**system.moof.** New module providing moof-level functions for what used to be Rust REPL command dispatch. `(modules)`, `(module-source bootstrap)`, `(module-exports system)`, `(which-module Assoc)`, `(checkpoint)`, `(save)`, `(undef foo)`, `(define-in system hello "(def hello ...)")` — all moof functions now. Most are vaus that accept bare symbols: you write `(module-exports bootstrap)` not `(module-exports "bootstrap")`.

**VM-level natives.** Five new natives intercepted in `call_native` with full VM access: `__save-image` (bincode serialization), `__save-source` (source projection + manifest), `__eval-in` (eval string in specific env), `__define-global` (bind in root), `__undef` (remove from root). These are the escape hatches — everything else is moof.

**Compiler diet.** Removed compiler special-case handling for `while`, `set!`, `%do`, `list`. Bootstrap.moof now provides `(def list (fn args args))` (variadic rest param) and `(def while (vau (test . body) ...))` (recursive operative). 67 lines gone from compile.rs.

**Native handler migration.** `startsWith:`, `endsWith:`, `contains:` moved from Rust natives to moof defmethod definitions in system.moof, using the existing `indexOf:` and `substring:to:` primitives.

**`populate_modules_registry` deleted.** This was 52 lines of Rust that built moof objects by eval-ing string templates. Replaced entirely by `register_module_on_heap` which creates objects directly during module loading.

**REPL interceptors removed.** The Rust REPL loop no longer string-matches against `(modules)`, `(module-source ...)`, `(module-exports ...)`, `(which-module ...)`, `(define-in ...)`, `(module-create ...)`, `(checkpoint)`. These all fall through to eval and are handled by moof functions.

### ModuleImage methods (in moof)

```
[module define: "name" source: "(def name ...)"]   ; create/update Definition
[module remove-def: "name"]                         ; remove Definition + provides
[module project-source]                             ; build .moof text from Definitions
```

The `define:source:` multi-keyword handler is installed in system.moof (not bootstrap) because it needs `toSymbol` which requires type prototypes to be registered.

### In-image editing works

The litmus test: you can `(define-in system greet "(def greet (fn (x) x))")` from the REPL, and it evals in the module's env, creates a Definition, updates provides, saves the binary image AND projects the source file, all without touching any .moof file directly. Workspace autosave uses the same path — `(def foo 42)` at the REPL auto-saves via `define-in`.

### What the design doc says vs where we are

The design doc (§6) says "source files are the canonical representation." We've moved past that: the heap is canonical for runtime state, Definition objects are canonical for source meaning, and source files are deterministic projections. This is the plan's "three layers" (Definition, State, Projection) made real. The design doc will need updating.

### What's still missing

- Compacting GC before serialization (heap grows monotonically)
- Identity-preserving reload (re-eval still creates new objects rather than patching existing ones)
- The `ModuleLoader` still has HashMap fields used during source-load mode (graph, loaded_envs, exports, source_texts) — they're not the source of truth anymore but they're not removed yet
- Many REPL commands still have Rust interceptors (module-edit, module-reload, module-remove, export/import-modules, browse)
- No `form->string` serializer for building source text from AST (blocks fully vau-based define-in)

### Bugs fixed along the way

- `modules.moof` used `.keys` dot access (reads a slot) instead of `[... keys]` message send on Assoc — broke `[Modules list]`
- `bootstrap.moof` didn't export `Definition` or `ModuleImage` — they were defined but not in the provides list
- `all_module_ids()` looked for an `entries` slot on Assoc but the real Assoc uses `data` — fixed to try both
- `let` vs `let*` scoping: `define-in` vau had a let binding that referenced a sibling binding (parallel scope), fixed by nesting lets
- `[sym toString]` returns `"'hello"` with quote prefix — fixed `as-name` helper to use `[sym name]` instead

---

## §14 — Death of source projection: the image IS the program (`53fd98f`..`HEAD`)

### The shift

Source projection was always a compromise. The idea: project .moof files from heap-resident Definition objects for git diffing, human reading, and source-load fallback. But this created a maintenance burden (two projection paths, dual save logic, projection filtering by value type) and a philosophical contradiction: if the heap is truth, why are we maintaining a parallel text representation?

The answer: we shouldn't be. The image is the only artifact.

### What this means

**No more .moof source files as a persistence mechanism.** The binary image (`image.bin`) is the canonical and only representation. Source text lives on the objects themselves:
- Lambdas and operatives already carry their source AST in a `source` slot
- Definitions carry human-authored source text (with comments, sugar, docs)
- Objects carry their handlers (which are lambdas with source)

**You don't edit source files. You evolve the image.** `(define-in system greet "(def greet (fn (x) x))")` modifies the heap, creates/updates a Definition, saves the image. There's no .moof file to write. The source text is on the Definition object in the heap.

**Reading code = inspecting objects.** `(module-source bootstrap)` walks Definition objects and prints their source slots. `(source greet)` returns the lambda's source AST. `[def slotAt: 'doc]` returns documentation. All from the living image.

**The MCP server becomes the programming interface.** An AI agent inspects the image via MCP tools, modifies definitions by sending messages, and sees changes reflected immediately. No file I/O. No text editor. The objectspace is the IDE.

### What was removed

- `__save-source` native (source projection to .moof files)
- `native_save_source` in exec.rs (the projection + manifest writer)
- `project_module_source` in exec.rs (the code/value filtering logic)
- `project_source` in loader.rs (delegated to VM, then removed)
- `save_image` in loader.rs (the manifest/source writer)
- Source file writing on checkpoint — `(checkpoint)` now just saves `image.bin`
- The manifest system (`manifest.moof`, per-module hashes)
- The `--seed` path and `lib/` directory concept

### What was kept

- `read` native — still needed for parsing strings into ASTs at the REPL
- `image.bin` serialization via `__save-image` — the only persistence
- Definition objects on the heap — they carry source text as metadata
- ModuleImage objects — they organize definitions into modules
- The module loader for initial bootstrap (first boot from lib/ if no image exists)

### The bootstrap problem

If there's no image and no source files, how do you start? Answer: a seed image. Ship a pre-built `image.bin` that contains the full bootstrap. First run loads it. After that, the image evolves in place. The `lib/` directory becomes a development artifact for building the seed, not a runtime dependency.

### What this enables

- **The MCP server is the IDE.** The agent reads/writes the image directly. No file system needed.
- **Versioning becomes object-level.** Instead of `git diff` on text files, version individual objects. `[obj history]` returns a stream of past states. This is future work but the architecture now supports it.
- **The image is portable.** One file. Copy it, run it. No directory structure, no manifest, no source files to keep in sync.
- **Simpler codebase.** Removed ~200 lines of projection, manifest, and dual-save logic.

### The `read` native and vau

`read` parses a string into an AST without evaluating. Combined with `eval` and `$env` from vau, this gives full in-image code manipulation:

```moof
; parse → eval → bind, all through moof primitives
(eval (read "(def foo (fn (x) [x + 1]))") root-env)

; define-in: vau captures symbols unevaluated, reads the source string,
; evals in the module's env, updates the Definition
(define-in system greet "(def greet (fn (x) x))")
```

No Rust natives beyond `read` (parsing) and `__save-image` (serialization). Everything else is vau + environment manipulation.
