# moof

> *"clarus the dogcow lives again"*

a persistent, concurrent objectspace with capability security
and a lisp-shaped surface syntax. your personal database.
a federated web of objects.

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
[Image search: "protocol"]        ; full-text search your objectspace
```

## the big ideas

- **one type:** Object. cons, string, array, hashmap, vat — all objects.
- **protocols:** the type system. implement `each:`, get 30 methods free.
- **fixed-shape slots:** data is public and sealed. access is an array offset.
- **open handlers:** add behavior to any object anytime. prototype delegation.
- **vats:** erlang-style concurrent processes, objects, capability-isolated.
- **capabilities:** a reference IS a capability. no IO without the IO object.
- **LMDB persistence:** crash-safe, concurrent readers, instant startup.
- **the canvas:** zoomable infinite spatial browser. every object renders itself.
- **the agent:** an LLM in a vat with membraned capabilities.
- **liveness:** mirrors, fix-and-proceed, Observable, reflective tower.
  source code is objects — read, transform, reinstall from within moof.
- **your database:** full-text search, reactive indexing, collections-as-tables.
  `where:`, `groupBy:`, `join:on:` — queries as message sends.
- **content-addressed:** every object state has a hash. undo, versioning,
  deduplication, snapshots — all free. even locally.
- **federated:** images talk to each other via MCP. far references,
  capability-mediated sharing, CRDT-based merging, offline pinning.
- **your website:** publish objects as web pages. capabilities control
  what's public. your database and your website are the same image.
- **vau:** user code has compiler-level power. `if` is a library function.

## status

design phase. see [VISION.md](VISION.md) for the full design,
[SYNTHESIS.md](SYNTHESIS.md) for the v1 post-mortem. v1 is on
`archive/v1` (tagged `v1-final`).

## debts

erlang (processes, let-it-crash), E (capabilities, eventual sends),
haskell (typeclasses as protocols, effects as capabilities),
ruby (everything-is-object, blocks, Enumerable, open classes),
self (prototypes, live environment, morphic),
SQL (objects-as-rows, queries-as-sends),
git (content-addressed storage, merkle DAG sync),
IPLD (content identifiers, universal linking),
kernel (vau).

## license

MIT
