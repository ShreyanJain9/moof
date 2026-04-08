# moof: what it is, what it became, what it wants to be

> a synthesis of the entire moof codebase's documentation, written by claude
> as a clear-eyed assessment before a ground-up reimagining.

---

## the soul of moof

moof is not a programming language. it's a **living computational environment** — a persistent, introspectable objectspace where the boundary between "using" and "programming" dissolves. named after clarus the dogcow, it's always half one thing and half another: half lisp, half smalltalk, half database, half IDE, half substrate.

the north star is three ideas:

1. **turtles all the way down.** six kernel primitives (`vau`, `send`, `def`, `quote`, `cons`, `eq`) implemented in rust. everything else — the standard library, the module system, the AI integration, the IDE — is written in moof and lives in the image.

2. **objects and messaging are the only abstraction.** `(f a b c)` is `[f call: a b c]`. there is no separate function type. `send` is the only operation. everything reduces to it.

3. **radical openness.** the image participates in the broader computing ecosystem. FFI, MCP, federation. moof objects speak every protocol through the same message-passing model.

## what exists today

### the kernel
a bytecode VM in rust (~4000 lines across three crates). six primitive forms, compiled to bytecode chunks. arena heap with tagged values (`Nil`, `True`, `False`, `Integer(i64)`, `Float(f64)`, `Symbol(u32)`, `Object(u32)`). prototype delegation with the slots-vs-handlers split: slots are private storage, handlers are public behavior that delegates through parent chains.

### the surface syntax
three bracket species:
- `(f a b c)` — applicative call (desugars to `[f call: a b c]`)
- `[obj selector: arg]` — message send
- `{ Parent x: 10 }` — object literal

plus sugar: `'symbol`, `obj.slot`, `@x` (self slot), `fn` (short lambda), quasiquote.

### the object model
everything is an object. integers, floats, strings, booleans, nil, cons cells, lambdas, environments. type prototypes (Integer, String, etc.) are real objects with real handlers. `doesNotUnderstand:` for dynamic dispatch. protocols for selector namespacing.

### the architecture (current: three crates)
- **moof-fabric** (~900 lines): the substrate kernel. objects, messaging, scheduling, persistence. knows nothing about any language.
- **moof-server** (~300 lines): a running fabric instance. frontends connect as vats. loads language extensions as dylibs.
- **moof-lang** (~2700 lines): the moof language. lexer, parser, compiler, bytecode interpreter. compiles as both rlib and cdylib.

the server is generic. moof-lang is just one extension. the fabric doesn't know about bytecode, ASTs, closures, or s-expressions.

### persistence
the image is the only artifact. `image.bin` — bincode serialization of the entire heap. no source files as a persistence mechanism. source text lives on objects themselves: Definition objects carry the human-authored text, lambdas carry their source AST. `(checkpoint)` saves. auto-checkpoint every 5000 allocations.

### the module system
modules are objects in the heap. `ModuleImage` objects contain `Definition` objects. dependency ordering via topological sort. sandboxed loading environments. workspace autosave for REPL definitions.

### the capability model
vats as security domains. membranes for message interception. facets for restricted views. a reference IS a capability — unforgeable.

### AI integration
MCP server over stdio. the image itself becomes the tool registry — objects with `describe` and `interface` become MCP resources and tools.

## what went right

- **bytecode from day one** avoided the usual tree-walk-then-rewrite trap
- **the six primitives** are genuinely sufficient. `vau` is powerful enough that `if`, `lambda`, `let`, `while` are all library code
- **source text on objects** means introspection gives you real code, not decompiled bytecode
- **the fabric/server/lang split** achieves real language-agnosticism. a python bridge could coexist
- **the journal** is an extraordinary piece of engineering documentation — every decision, every bug, every dead end recorded

## what went wrong (or sideways)

### persistence whiplash
the persistence model was rewritten four times:
1. binary heap snapshot + WAL
2. source files as truth (directory image)
3. heap as truth with source projection
4. heap as truth, no projection

each was a reasonable choice given what came before, but the churn consumed enormous energy and left vestigial code paths.

### the rust/moof boundary is awkward
five VM-level natives (`__save-image`, `__save-source`, `__eval-in`, `__define-global`, `__undef`) are escape hatches with dunder names. the registration bridge between rust and moof objects involves eval'ing string templates. the compiler has special-case handling that should live in the image but can't because of bootstrap ordering.

### the heap is fragile
no compacting GC during runtime. heap grows monotonically. the snapshot GC only runs at save time. no incremental or generational collection. for a system meant to run indefinitely, this is a structural problem.

### the module system is overengineered for what it does
dependency graphs, sandboxed environments, topological sorting — all for what is essentially "load these files in order." the module system is simultaneously too complex (graph theory) and too simple (no versioning, no namespacing beyond exports).

### `vau` is a double-edged sword
operatives are beautiful in theory. in practice, they make optimization nearly impossible — every call site could receive unevaluated arguments and a reified environment. the design doc acknowledges this ("inline caching and operative specialization at the JIT level") but that JIT doesn't exist and may never exist. meanwhile, every `vau` call prevents even basic constant folding.

### the wire protocol is half-built
binary wire protocol defined but the actual server/client communication is mostly in-process. the unix socket path works but the protocol is minimal. MCP is a separate stdio mode, not integrated with the fabric's vat model.

## the aspirations that haven't been reached

- **the reflective tower** — environments as first-class objects that can intercept evaluation. architecturally possible but not implemented.
- **the live environment** — browser, inspector, workspace, transcript, debugger as objects in the image. only TUI inspector and a partial egui browser exist.
- **federation** — CRDT-backed shared objects across images. pure aspiration.
- **the personal objectspace** — replacing files, notes, code editors with a unified object fabric. the vision is compelling but the reality is a REPL.
- **AI as participant** — the MCP server lets agents poke at the image, but agents aren't vat-resident first-class participants yet.
- **"not a programming language"** — this is true architecturally (the fabric is language-agnostic) but in practice, moof IS a programming language because that's the only way to interact with it.

---

*this document was synthesized from README.md, DESIGN.md, JOURNAL.md, ARCHITECTURE.md, PLAN-substrate.md, and SKILL.md in the moof repository. it represents the state of the project as of 2026-04-07.*
