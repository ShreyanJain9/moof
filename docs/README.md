# moof documentation

welcome. moof is a persistent, concurrent objectspace — smalltalk's
image, plan-9's namespaces, erlang's vats, kay-engelbart-atkinson's
authoring vision, in one substrate.

this is the documentation tree. the structure:

## start here

if you have **five minutes**, read [vision/one-page.md](vision/one-page.md).
it's the pitch in a page.

if you have **twenty minutes**, read
[throughlines.md](throughlines.md) — five deep patterns that unify
every concept in moof. read this BEFORE the concept docs and most
of them will feel obvious.

if you have **an hour**, read these in order:
1. [vision/manifesto.md](vision/manifesto.md) — what moof is, why
2. [throughlines.md](throughlines.md) — the five unifying patterns
3. [concepts/objects.md](concepts/objects.md) — the material
4. [concepts/messages.md](concepts/messages.md) — the one operation
5. [concepts/vats.md](concepts/vats.md) — concurrency and isolation
6. [concepts/effects.md](concepts/effects.md) — how things happen
7. [concepts/do-notation.md](concepts/do-notation.md) — the
   composition syntax everything uses

if you're **contributing**, add after the hour:
8. [concepts/persistence.md](concepts/persistence.md) — the image
9. [concepts/capabilities.md](concepts/capabilities.md) — security
10. [concepts/addressing.md](concepts/addressing.md) — URLs, namespaces
11. [concepts/protocols.md](concepts/protocols.md) — the type system
12. [laws/](laws/) — what moof commits to, what PRs get reviewed against

## structure

```
docs/
├── throughlines.md  the five patterns that unify every concept
│
├── vision/          why moof exists; what it's trying to be
│   ├── one-page.md       the pitch in one page
│   ├── manifesto.md      the long vision, in depth
│   ├── lineage.md        debts to smalltalk, plan-9, erlang, etc.
│   └── horizons.md       the far future: canvas, agent, federation
│
├── concepts/        what moof is — the computational model
│   ├── objects.md        the object model, slots, handlers, prototypes
│   ├── messages.md       send, dispatch, doesNotUnderstand
│   ├── protocols.md      the type system: contracts + conformance (handlers)
│   ├── schemas.md        explicit slot-type contracts (data shapes, optional)
│   ├── vats.md           concurrency, message boxes, scheduler
│   ├── effects.md        Acts, Updates, purity
│   ├── do-notation.md    the universal composition syntax
│   ├── streams.md        temporal flows: unix streaming for typed values
│   ├── persistence.md    content-addressing, the image, blobstore
│   ├── addressing.md     URLs, paths, namespaces
│   ├── capabilities.md   security via reference
│   └── authoring.md      the canvas, liveness, the ladder
│
├── reference/       how to use moof — concrete, spec-like
│   ├── syntax.md         s-expressions, sugar, reader rules
│   ├── stdlib.md         the stdlib surface, linked to doctrine
│   ├── vm.md             bytecode, dispatch, scheduler internals
│   ├── plugins.md        writing type plugins and capabilities
│   └── cli.md            moof command, flags, manifest
│
├── laws/            the constitution — what we commit to
│   ├── substrate-laws.md       6 laws the runtime satisfies
│   ├── stdlib-doctrine.md      the stdlib rulebook (long form)
│   ├── stdlib-at-a-glance.md   the stdlib rulebook (one-pager)
│   └── review-protocol.md      how PRs get reviewed
│
├── roadmap.md       where we are, where we're going
├── glossary.md      every term in moof, defined
└── archive/         older docs kept for reference, not canon
```

## about these docs

every doc declares its **type** at the top:

- **vision** — aspirational, the direction
- **concept** — what we believe about how moof works (can be
  designed-not-implemented, but the design IS the commitment)
- **reference** — how it actually works today (commits to implementation)
- **law** — invariants we refuse to violate
- **roadmap** — what we're working on and in what order

when vision and reference disagree, vision points at where we're
going and reference says what's there today. neither is allowed to
lie. if reference says something is working, it works. if vision
says something is planned, there's a roadmap item for it.

## what's not here

- **tutorials.** not written yet. the language is alpha; tutorials
  would rot. start with the concept docs and the repl.
- **API docs per type.** the moof inspector and `[Type describe]`
  are the authoritative API browser. when docs get out of date,
  the inspector doesn't.
- **marketing.** this isn't a product, it's a substrate. come back
  when the canvas lands.
