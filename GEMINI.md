# GEMINI.md - MOOF Project Context

## Project Overview
**MOOF (Moof Open Objectspace Fabric)** is a persistent, introspectable objectspace. It combines a **Lisp-shaped surface syntax** with a **Smalltalk-shaped object model** (everything is an object, message-based dispatch).

- **Runtime:** Custom bytecode VM written in Rust.
- **Persistence (Image v3):** Dual-layer persistence using a binary heap snapshot (`.moof/image.bin`) for fast startup and runtime truth, alongside a directory-based source image (`.moof/modules/`) for human-readability and version control.
- **Core Philosophy:** "The VM's single privileged operation is `send`." Every value, including primitives, behaves as an object that can receive messages.

## Technical Architecture
- `src/runtime/`: Heap management (typed arena) and Value representations (immediates + heap references).
- `src/vm/`: The execution engine, instruction set (opcodes), and native function registry.
- `src/compiler/`: Recursive descent compiler transforming AST into VM bytecode.
- `src/reader/`: Lexer and Parser for the s-expression surface syntax.
- `src/persistence/`: Logic for binary snapshots (`snapshot.rs`) and source module management (`image.rs`).
- `src/modules/`: Dependency-aware module loading, sandboxing, and the first-class `ModuleImage` / `Definition` registry.
- `src/ffi/`: Bridge for calling native C/Rust libraries.
- `src/tui/` & `src/gui/`: Inspection and browsing tools.

## Building and Running
- **Build:** `cargo build`
- **Run REPL:** `cargo run` (Resumes from `.moof/image.bin` if present)
- **GUI Mode:** `cargo run -- --gui`
- **Seed Image:** `cargo run -- --seed` (Forces a fresh load from `lib/` into `.moof/`)
- **Test:** `cargo test`
- **Lint:** `cargo check`

## Development Conventions
1. **Object-Oriented Everything:** Favor message-passing (`[receiver message: args]`) over hardcoded VM primitives where possible.
2. **Persistence Integrity:** Any change to the system's global state should be reflected in both the binary image and the source projection. Use `(checkpoint)` or `(save)` in the REPL.
3. **Definition Objects:** Top-level bindings should be represented as `Definition` objects on the heap to maintain identity across reloads.
4. **Sandboxing:** Non-bootstrap modules are loaded in a restricted environment. Be mindful of which primitives are exposed to the `sandbox`.
5. **Idiomatic Rust:** Follow standard Rust safety and typing conventions in the VM core, but ensure the heap remains a flat, serializable slab.

## Key Files
- `DESIGN.md`: The "North Star" philosophy and architectural roadmap.
- `PLAN-image-v3.md`: Details the transition from source-only to binary-primary persistence.
- `JOURNAL.md`: Implementation history and record of design decisions.
- `.moof/manifest.moof`: Defines the deterministic load order for the source-layer image.
