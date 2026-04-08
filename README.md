# moof

> *"clarus the dogcow lives again"*

a persistent, concurrent objectspace with capability security
and a lisp-shaped surface syntax.

not a programming language. a runtime.

## the idea

- **three heap types**: Object, Cons, Blob
- **one operation**: send
- **six kernel forms**: vau, send, def, quote, cons, eq
- **vats**: erlang-style concurrent processes with mailboxes
- **capabilities**: E-style vats, membranes, facets
- **persistence**: the image survives restarts
- **vau**: user code has compiler-level power

```
[3 + 4]                          ; message send to an integer
(def Point { Object x: 0 y: 0 }) ; object literal with parent
[pt <- distanceTo: other]        ; eventual send (returns promise)
(spawn (fn () [server listen]))   ; new vat
```

## status

design phase. see [VISION.md](VISION.md) for the full design,
[SYNTHESIS.md](SYNTHESIS.md) for the v1 post-mortem. v1 is
preserved on `archive/v1` (tagged `v1-final`).

## debts

erlang (processes, let-it-crash, distribution), E (capabilities,
eventual sends, promise pipelining), haskell (effects as
capabilities), ruby (everything is an object, blocks, open
classes), self (prototypes, live environment), kernel (vau).

## license

MIT
