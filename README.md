# MOOF: Moof Open Objectspace Fabric

> *"clarus the dogcow lives again"*

MOOF is a persistent, introspectable objectspace with a Lisp-shaped surface syntax and a Smalltalk-shaped object model. It runs on a bytecode VM in Rust, with most higher-level behavior defined in MOOF itself.

Current persistence is module-oriented: the live source image lives under `.moof/`, and startup reloads modules from `.moof/modules/*.moof` in dependency order.

## Quick start

```bash
cargo run
```

On first boot, MOOF seeds `.moof/modules/` from `lib/` if present or uses the checked-in `.moof/` image if it already exists. After that, it reloads modules from `.moof/modules/` and rebuilds `.moof/manifest.moof`.

```text
MOOF — Moof Open Objectspace Fabric
clarus the dogcow lives again
Type expressions to evaluate. Ctrl-D to exit.

moof> [2 + 3]
=> 5
moof> (def greet (fn (name) [name ++ " says moof"]))
=> <lambda>
moof> (greet "clarus")
=> "clarus says moof"
```

## Kernel

The VM has six privileged primitives:

| Primitive | Purpose |
|---|---|
| `vau` | Create an operative that receives unevaluated args plus caller env |
| `send` | Message dispatch |
| `def` | Bind a name in the current environment |
| `quote` | Return literal data |
| `cons` | Construct a pair |
| `eq` | Identity comparison |

Everything else is derived in MOOF code, primarily in `.moof/modules/bootstrap.moof`.

## Surface syntax

```lisp
(f a b c)          ; applicative call
[obj selector: a]  ; message send
{ Parent x: 10 }   ; object literal
```

Additional sugar:

```lisp
'name              ; symbol literal
obj.x              ; slot access
@x                 ; self slot access inside methods
(fn (x) [x + 1])   ; short lambda
```

## Object model

Objects have slots and handlers, with prototype delegation through `parent`.

```lisp
(def Point { Object
  describe: () "a Point"
  distanceTo: (other)
    (let ((dx [@x - other.x])
          (dy [@y - other.y]))
      [[dx * dx] + [dy * dy]])
})

(def pt { Point x: 3 y: 4 })

pt.x
[pt describe]
[pt distanceTo: { Point x: 0 y: 0 }]
```

You can also attach methods after the fact:

```lisp
(defmethod Point magnitude ()
  [[@x * @x] + [@y * @y]])
```

## Persistence and modules

Today, MOOF persists source modules, not a binary heap image.

The live image directory is:

```text
.moof/
  manifest.moof
  modules/
    bootstrap.moof
    collections.moof
    classes.moof
    geometry.moof
    json.moof
    mcp.moof
    membrane.moof
    modules.moof
    workspace.moof
```

What persists now:

- Module source files in `.moof/modules/`
- REPL `def` and `defmethod` forms, autosaved into `workspace.moof`
- Manifest metadata and per-module source hashes in `.moof/manifest.moof`

What does not persist yet:

- Arbitrary heap mutation that is not reflected back into module source
- A binary object image with instant heap restore
- GC/compaction at checkpoint time

## REPL and module commands

Built-in commands exposed by `main.rs` include:

- `(save)` or `(checkpoint)` to write the current source image
- `(browse)` or `(browse expr)` to open the TUI inspector
- `(modules)` to list loaded modules
- `(module-source name)` to print a module's source
- `(module-edit name)` to edit a module in `$EDITOR`, reload, and save
- `(module-reload name)` to reread a module from disk and reload it
- `(module-remove name)` to delete a module
- `(module-exports name)` to list exported symbols
- `(module-create name (requires dep1 dep2))` to create a new module
- `(define-in module-name (def ...))` to define directly into a module
- `(which-module symbol-name)` to find the owning module
- `(export-modules)` to copy `.moof/modules/` out to `lib/`
- `(import-modules)` to seed `.moof/modules/` from `lib/`

## Other runtime modes

```bash
cargo run -- --gui
cargo run -- --mcp
cargo run -- --seed
```

- `--gui` launches the egui system browser
- `--mcp` starts the MCP stdio server defined in `.moof/modules/mcp.moof`
- `--seed` re-seeds `.moof/modules/` from `lib/`

## Project structure

```text
src/
  main.rs              startup, REPL, module commands
  reader/              lexer and parser
  compiler/            AST to bytecode
  vm/                  VM, opcodes, native registrations
  runtime/             values, heap, environments
  modules/             discovery, dependency graph, sandboxing
  persistence/         directory-image manifest and module storage
  tui/                 terminal inspector
  gui/                 egui system browser
  ffi/                 native library binding
.moof/modules/         current live source image
DESIGN.md              design goals and longer-term architecture
JOURNAL.md             implementation history by phase
PLAN-image-v3.md       binary image persistence plan
```

## Current status

Implemented now:

- Bytecode VM with operatives, lambdas, tail calls, objects, strings, floats, and FFI
- Source-backed module system with dependency ordering and sandboxed loading
- TUI inspector and egui browser
- MCP server module
- Workspace autosave into `.moof/modules/workspace.moof`

Planned next major shift:

- Real heap image persistence as described in `PLAN-image-v3.md`

## License

MIT
