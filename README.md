# MOOF: Moof Open Objectspace Fabric

> *"clarus the dogcow lives again"*

MOOF is a living computational environment — a persistent, introspectable objectspace where usage and programming are the same activity. It's a lisp-shaped language with Smalltalk's object model, built on a bytecode VM with six kernel primitives.

## Quick start

```
cargo run
```

```
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

## The six kernel primitives

Everything in MOOF is built on exactly six forms, implemented in the VM. Nothing else gets bytecode privilege.

| Primitive | Purpose |
|-----------|---------|
| `vau`     | Create an operative (receives unevaluated args + caller's environment) |
| `send`    | Message dispatch — the VM's single privileged operation |
| `def`     | Bind a name in the current environment |
| `quote`   | Return an expression as literal data |
| `cons`    | Construct a pair |
| `eq`      | Identity comparison |

Everything else — `fn`, `if`, `let`, `cond`, `defmethod`, the entire object model — is derived from these six in `lib/bootstrap.moof`.

## Three bracket species

```lisp
(f a b c)          ; applicative call
[obj selector: a]  ; message send
{ Parent x: 10 }   ; object literal
```

## Syntax sugar

```lisp
'name              ; symbol literal (quote name)
obj.x              ; field access (direct slot read)
@x                 ; self-field access (inside methods)
(fn (x) [x + 1])  ; short lambda (defined as a vau operative)
```

## Object model

Objects have **slots** (public data) and **handlers** (behavior, inherited through prototype delegation).

```lisp
; define a prototype with methods
(def Point { Object
  describe: () "a Point"
  distanceTo: (other)
    (let ((dx [@x - other.x])
          (dy [@y - other.y]))
      [[dx * dx] + [dy * dy]])
})

; create instances with object literals
(def pt { Point x: 3 y: 4 })

pt.x                                 ; => 3
[pt describe]                        ; => "a Point"
[pt distanceTo: { Point x: 0 y: 0 }] ; => 25

; add methods after the fact
(defmethod Point magnitude ()
  [[[@x * @x] + [@y * @y]]])
```

## Operatives (vau)

The real power: `vau` creates operatives that receive their arguments **unevaluated**, plus the caller's environment. This means user code has compiler-level power.

```lisp
; 'when' is defined in bootstrap.moof as a vau operative:
(def when
  (vau (condition . body) $e
    (if (eval condition $e)
      (eval (cons 'do body) $e)
      nil)))

; fn itself is a vau that constructs and evals a lambda:
(def fn
  (vau (params . body) $e
    (eval (cons 'lambda (cons params body)) $e)))
```

## Introspection

```lisp
(source fn)             ; see the AST of any function
[Object interface]      ; list an object's handlers
[pt sourceOf: 'describe] ; see the source of a specific method
(inspect pt)            ; print type, slots, handlers
```

## Project structure

```
src/
  main.rs              REPL, bootstrap loading, eval entrypoint
  reader/
    lexer.rs           Tokenizer (three bracket species + sugar)
    parser.rs          Tokens -> cons-cell AST
  compiler/
    compile.rs         AST -> bytecode
  vm/
    opcodes.rs         Bytecode instruction set
    exec.rs            Stack-based VM, message dispatch
  runtime/
    value.rs           Value representation (immediates + heap refs)
    heap.rs            Arena allocator, symbol interning
    env.rs             First-class environments
lib/
  bootstrap.moof       Standard library, written in MOOF itself
  geometry.moof        Example: Point prototype
DESIGN.md              Full design & philosophy document
```

## What's next

See `JOURNAL.md` for implementation history. Upcoming:

- **Persistence** — WAL + content-addressed image snapshots
- **String operations** — `substring:`, format, symbol conversion
- **TUI inspector** — browse the live object graph
- **FFI** — call into native libraries via libffi
- **MCP server** — expose the objectspace as AI-native tools

## License

MIT
