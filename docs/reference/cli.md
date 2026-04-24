# cli

**type:** reference

> the `moof` command and its flags. today's surface is small —
> the REPL, `-e`, and a few flags.

---

## invocations

```
moof                 ; start the REPL; load image from .moof/store
moof -e "expr"       ; evaluate expr and print the result
moof --help          ; show flags (minimal today)
moof --version       ; print version
```

all invocations consult `moof.toml` in the current directory.

---

## the manifest — `moof.toml`

at project root:

```toml
[image]
name = "moof"
path = ".moof/store"          # where the image lives on disk

[types]
# type plugins loaded on every vat
core = "builtin:core"
numeric = "builtin:numeric"
collections = "builtin:collections"
# ... more ...
color = "examples/type-plugin/target/release/libmoof_color_plugin.dylib"

[capabilities]
# capability plugins, each gets its own vat
console = "builtin:console"
clock = "builtin:clock"
file = "builtin:file"
random = "builtin:random"
system = "builtin:system"
evaluator = "builtin:eval"

[sources]
# moof files eval'd into each new vat during bootstrap
files = [
  "lib/kernel/bootstrap.moof",
  "lib/kernel/protocols.moof",
  # ... in dependency order ...
]

[grants]
# which capabilities each interface can access
repl = ["console", "clock", "file", "random", "system", "evaluator"]
script = ["console", "clock", "file", "random", "system", "evaluator"]
eval = ["console", "clock", "file", "random", "system", "evaluator"]
```

- **`builtin:X`** — resolves to `target/<profile>/libmoof_plugin_X.{dylib,so,dll}`.
- **literal path** — loads the dylib at that path.

---

## the REPL

```
$ moof

  .  *  .        m o o f        .  *  .
       ~ a living objectspace ~
    clarus the dogcow lives again
  manifest: moof.toml
  loaded type plugin 'core' ...
  loaded capability 'console' ...
  ...
  image loaded into vat 7

✨ [3 + 4]
  7  : Integer
✨ (def greet |name| (str "hi, " name))
  <fn arity:1>  : Fn
✨ (greet "clarus")
  "hi, clarus"  : String
✨ ^D

  the circle closes. moof.
```

- `✨` is the prompt.
- multi-line input auto-continues until parens balance.
- `^D` exits; the image saves on the way out.
- `^C` interrupts the current eval (returns to prompt).

the REPL is a moof `Interface` — the REPL code lives in rust today
but it will move into moof (wave-10+ goal).

---

## `-e` — one-shot evaluation

```
$ moof -e '(+ 1 2 3)'
6
```

loads the image, evaluates the expression in an eval vat, prints
the result, exits.

`-e` is implemented as a moof-side script (`lib/bin/eval.moof`)
rather than a rust special-case — the rust side just boots moof
with the `eval` interface instead of `repl`.

---

## image location

the image lives at `.moof/store` in the current directory by
default. the path is configurable via `[image].path` in
`moof.toml`.

first-run creates the store. subsequent runs open it.

**DO NOT** manually edit the store. it's LMDB. the format will
change. use moof-side introspection (`(save-image)`,
`(load-image)`) if you need to script around it.

---

## common flags (current + planned)

| flag | today | planned |
|------|-------|---------|
| `-e EXPR` | ✓ evaluate and print | |
| `--help` | ✓ (minimal) | richer help |
| `--version` | ✓ | |
| `--fresh` | planned | start without loading saved image |
| `--rescue` | planned | boot into bare repl without user image (recovery) |
| `-f FILE` | planned | load moof source file on startup |
| `--headless` | planned | run a service mode, no repl |
| `--store PATH` | planned | override manifest's image path |

---

## the REPL's implicit drain

every eval drains the scheduler before printing. any cross-vat
Acts created during your expression resolve before you see the
result. this is why synchronous-looking code "just works" — the
REPL does the waiting.

in a non-REPL interface (script, service) drain semantics are
explicit. scripts drain at end-of-script; services drain in their
message loop.

---

## exit behavior

- normal exit (^D, `(quit)` in the planned API): save image, close
  store, exit 0.
- crash (unhandled panic): depending on point, may or may not save.
  the scheduler's Drop ordering is explicit to avoid segfaults
  from dylib unload order.
- signal (^C twice, SIGTERM): abort without save. LMDB's crash
  safety means the last committed state is intact; in-flight
  changes since last commit are lost.

---

## what you need to know

- `moof` starts the REPL with the image at `.moof/store`.
- `moof -e EXPR` is one-shot.
- `moof.toml` configures plugins, capabilities, boot sources,
  grants.
- image save happens on clean exit; crashes preserve the last
  committed state.
- flags are minimal today; many are planned.

---

## next

- [../concepts/persistence.md](../concepts/persistence.md) — what
  the image stores and why.
- [../concepts/capabilities.md](../concepts/capabilities.md) —
  what the grants matrix controls.
- [plugins.md](plugins.md) — writing your own plugin.
