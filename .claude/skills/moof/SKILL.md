---
name: moof
description: Evaluate expressions in the live moof image, inspect objects, read module sources, and modify the objectspace. Use when working with moof code, inspecting the image, or when the user asks about moof objects/modules.
---

# Moof Image Interaction

Evaluate expressions in the live moof image. Use `./scripts/moof-eval.sh` from the project root.

## How to evaluate

```bash
./scripts/moof-eval.sh 'expression1' 'expression2' ...
```

Each argument is one expression. Results come back as `=> value` or `!! error`.

## Translating requests to moof

| Request | Moof expression |
|---|---|
| list modules | `[Modules list]` |
| definitions in module X | `(map (fn (d) [d slotAt: (quote name)]) [[Modules named: "X"] slotAt: (quote definitions)])` |
| source of module X | `(module-source X)` |
| type of value | `(type-of VALUE)` |
| inspect object | `[OBJ describe]` then `[OBJ interface]` then `[OBJ slotNames]` |
| define a name | `(def name value)` — errors if already bound, use `<-` to update |
| remove a name | `(undef name)` |
| save image | `(checkpoint)` |
| list handler names | `[OBJ handlerNames]` or `[42 handlerNames]` (works on primitives too) |
| get parent/proto | `[OBJ parent]` or `[42 parent]` |
| read a slot | `[OBJ slotAt: (quote slotname)]` |
| write a slot | `[OBJ slotAt: (quote slotname) put: value]` |
| eventual send | `[OBJ <- selector: arg]` (returns Promise) |
| spawn a vat | `(spawn (fn () body))` |

## Moof syntax cheat sheet

- `[obj selector: arg]` — message send (brackets)
- `(f arg1 arg2)` — function call (parens)
- `{ Parent key: value }` — object literal (braces)
- `@slot` — read slot on self (inside handlers only)
- `'symbol` or `(quote name)` — quoted symbol
- `<-` — update existing binding: `(<- name newval)`
- `[obj <- sel: arg]` — eventual send, returns Promise

## Important rules

- `def` errors on existing bindings. Use `<-` to update, or `(undef name)` then `(def name val)`.
- `(checkpoint)` saves the image to disk. Always save after modifications.
- The image IS the program. There are no source files. Everything lives in the heap.
- `[1 + 2]` not `(+ 1 2)` — arithmetic is message sends.

## Multi-step introspection example

```bash
# Get full picture of an object
./scripts/moof-eval.sh \
  '[Modules list]' \
  '(map (fn (d) [d slotAt: (quote name)]) [[Modules named: "bootstrap"] slotAt: (quote definitions)])' \
  '[Object interface]'
```

$ARGUMENTS
