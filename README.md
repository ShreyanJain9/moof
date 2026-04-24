# moof

> *"clarus the dogcow lives again"*

a persistent, concurrent objectspace — smalltalk's image, plan 9's
namespaces, erlang's vats, kay-engelbart-atkinson's authoring
vision, in one substrate.

## one sentence

moof is a personal dynamic medium — a living, accretive, shareable
objectspace where the tools and the content are made of the same
material, and everyone is an author.

## everything is an object

```
[3 + 4]                           ; message send to an integer
{ Point x: 3 y: 4 }              ; object literal (fixed shape)
[list map: |x| [x * 2]]          ; block passed to a method
[pt <- distanceTo: other]        ; eventual send (returns Act)
[people where: |p| [p.age > 28]] ; query — objects are rows
```

## the commitments

- **one type: Object.** cons, string, integer, vat, the canvas —
  all objects. the VM optimizes; the semantics don't bend.
- **one operation: send.** `(f x)` is `[f call: x]`. `obj.x` is
  `[obj slotAt: 'x]`. `[3 + 4]` is a send. everything is a send.
- **the image persists.** close moof, reopen, exactly where you
  were. LMDB-backed blob store, content-addressed.
- **vats are isolation.** single-threaded actors with private
  heaps. cross-vat sends return Acts. let it crash.
- **references are capabilities.** if you don't hold the Console,
  you can't print. no ambient authority.
- **protocols are the type system.** implement `fold:with:`, get
  ~40 collection methods free. nominal + structurally queryable.
- **every value has a URL.** content-addressed for immutable;
  path-addressed for live. federation uses the same URLs with
  a peer prefix.

## status

alpha. the substrate works: REPL, image, vats, protocols, capabilities.
the canvas is future work. the agent is future work. federation is
future work. we're making a living thing, not shipping a product.

see [docs/roadmap.md](docs/roadmap.md) for waves and timing.

## read the docs

if you have **five minutes**: [docs/vision/one-page.md](docs/vision/one-page.md)

if you have **an hour**, read in order:
1. [docs/vision/manifesto.md](docs/vision/manifesto.md) — what moof
   is, why
2. [docs/concepts/objects.md](docs/concepts/objects.md) — the material
3. [docs/concepts/messages.md](docs/concepts/messages.md) — the one
   operation
4. [docs/concepts/vats.md](docs/concepts/vats.md) — concurrency
5. [docs/concepts/effects.md](docs/concepts/effects.md) — how things
   happen

contributing?
- [docs/laws/substrate-laws.md](docs/laws/substrate-laws.md) — the
  six substrate invariants
- [docs/laws/stdlib-doctrine.md](docs/laws/stdlib-doctrine.md) — the
  stdlib rulebook
- [docs/laws/review-protocol.md](docs/laws/review-protocol.md) — how
  PRs get reviewed

## debts (acknowledged)

erlang, E (mark miller), haskell, ruby, self, SQL, git, IPLD, unix,
plan 9, smalltalk, alan kay, doug engelbart, bill atkinson. see
[docs/vision/lineage.md](docs/vision/lineage.md).

## license

MIT
