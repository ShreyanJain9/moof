# moof

> *"clarus the dogcow lives again"*

a persistent, concurrent objectspace with capability security
and a lisp-shaped surface syntax.

## everything is an object

integers, strings, cons cells, arrays, hashmaps, lambdas, vats,
the canvas, the agent — all objects. objects have fixed public
slots (data) and open handlers (behavior). the only operation
is `send`.

```
[3 + 4]                           ; message send to an integer
{ Point x: 3 y: 4 }              ; object literal (fixed shape)
[list map: |x| [x * 2]]          ; block passed to a method
[pt <- distanceTo: other]        ; eventual send (returns promise)
[people where: |p| [p.age > 28]] ; query — objects are rows
```

## the big ideas

- **one type:** Object. cons, string, array, hashmap, vat — all objects.
  the VM has fast internal representations, but semantics are uniform.
- **fixed-shape slots:** `{ Point x: 3 y: 4 }` — exactly two slots, forever.
  values mutable, shape sealed. slot access is an array offset, not a hash lookup.
- **open handlers:** add behavior to any object anytime. prototype delegation.
- **vats:** erlang-style concurrent processes. objects. `[Vat spawn: ...]`.
- **capabilities:** a reference IS a capability. no IO without the IO object.
- **LMDB persistence:** crash-safe, concurrent readers, instant startup.
- **the canvas:** zoomable infinite spatial browser. every object renders itself.
- **the agent:** an LLM in a vat with membraned capabilities. lives in the image.
- **vau:** user code has compiler-level power. `if` is a library function.
- **queries:** `where:`, `groupBy:`, `join:on:` — objects-as-rows, sends-as-queries.

## status

design phase. see [VISION.md](VISION.md) for the full design,
[SYNTHESIS.md](SYNTHESIS.md) for the v1 post-mortem. v1 is on
`archive/v1` (tagged `v1-final`).

## debts

erlang (processes, let-it-crash), E (capabilities, eventual sends),
haskell (effects as capabilities), ruby (everything-is-object,
blocks, open classes), self (prototypes, live environment, morphic),
SQL (objects-as-rows, queries-as-sends), kernel (vau).

## license

MIT
