# Construction in moof

moof has four construction idioms. Each fits a different kind of type;
the convention is to use the shape that matches the type's shape.

## 1. Function-call syntax — `(Type arg ...)`

Works for any type whose prototype has a class-side `call:` handler.
`(Type a b c)` routes to `[Type call: (list a b c)]`; the handler decides
how to turn those args into an instance.

Used for:

- **Value wrappers** — `(Some 5)`, `(Ok 42)`, `(Err "oops")`, `(None)`
- **Defservers** — `(Atom 0)`, `(Signal || [a get] a)`
- **Variadic collections** — `(Set 1 2 3)`, `(Bag "a" "a" "b")`
- **Helper constructors** — `(list 1 2 3)`, `(cons a b)`, `(err "msg")`

## 2. Class-side keyword message — `[Type selector: a more: b ...]`

For types with multiple named fields where positional args would be
ambiguous. The prototype has a keyword-selector handler that builds the
instance. This is the ForeignType convention for foreign-plugin types.

Used for:

- **Multi-field records** — `[Vec3 new: 1.0 y: 2.0 z: 3.0]`,
  `[Color r: 255 g: 0 b: 128]`
- **Ranges** — `[1 to: 10]`, `[1 to: 10 by: 2]` (note: receiver is the
  start value; the keyword form reads naturally as english)

## 3. Object literal — `{ Type field: value ... }`

The most explicit form — lets you name fields at the call site. Works
for any General-backed type (moof-defined protos). Doesn't work for
foreign-payload types (Vec3, Color) because their data isn't stored in
named slots.

Used for:

- **Explicit records** — `{ Ok value: 42 }`, `{ Err message: "x" }`
- **Type literals** — `(def Atom { initial: nil value: nil ... })`
- **Updates** — `(update { blocks: new-list } reply)`

## 4. Literal syntax — `#[...]`, strings, numbers, etc.

Parser-level literals for the most common types.

- **Table seq** — `#[1 2 3]` → `{ Table seq: (list 1 2 3) map: #[] }`
- **Strings** — `"hello"`
- **Symbols** — `'foo`
- **Numbers** — `42`, `3.14`
- **Booleans / nil** — `true`, `false`, `nil`

## The rule of thumb

| Type shape                | Pick this                    |
|---------------------------|------------------------------|
| Single payload            | `(Type x)`                   |
| Many named fields (small) | `[Type kw: a kw: b]`         |
| Record (set or update)    | `{ Type field: v ... }`      |
| Variadic collection       | `(Type a b c d ...)`         |
| Parser-level primitive    | literal (`#[]`, `"..."`, …)  |

You can often get multiple — `(Some 5)` and `{ Some value: 5 }` both
work and produce equal values. The shortest honest form is preferred.
