# moof v4 documentation

this folder is the source of truth for moof v4. the implementation is
**docs-driven**: every piece of substrate behavior, syntax, and naming
is specified here *before* it is written in rust or moof.

if the code disagrees with the docs, the docs are authoritative until
amended. if the docs need to change, change them first, then change
the code, in that order.

## reading order

if you are picking this up cold, read in this order:

1. `vision/one-page.md` — what moof v4 is, in two minutes.
2. `vision/manifesto.md` — why moof v4 exists; what makes it different.
3. `vision/lineage.md` — every idea we are building on, attributed.
4. `concepts/forms.md` — the universal substrate primitive.
5. `concepts/vats.md` — the unit of concurrency, persistence, isolation.
6. `concepts/data-sources.md` — the universal i/o primitive.
7. `concepts/references.md` — federation-from-day-one.
8. `concepts/persistence.md` — per-vat database storage.
9. `syntax/overview.md` — the surface, at a glance.
10. `roadmap.md` — what we build, in what order.

after that, browse `concepts/` and `syntax/` as you need them.

## structure

```
docs/
├── README.md               this file
├── glossary.md             quick lookup of every term we use
├── roadmap.md              implementation phases, in order
├── vision/                 the why
│   ├── manifesto.md
│   ├── lineage.md
│   └── one-page.md
├── concepts/               the substrate, conceptually
│   ├── forms.md
│   ├── objects-and-protos.md
│   ├── sends-and-calls.md
│   ├── blocks-and-patterns.md
│   ├── tables.md
│   ├── lists.md
│   ├── strings.md
│   ├── numbers.md
│   ├── types.md
│   ├── capabilities.md
│   ├── references.md
│   ├── vats.md
│   ├── data-sources.md
│   ├── persistence.md
│   ├── queries.md
│   ├── compiled-objects.md
│   ├── reflection.md
│   ├── time-and-journal.md
│   ├── moldability.md
│   └── image-and-world.md
├── syntax/                 the surface
│   ├── overview.md
│   ├── brackets.md
│   ├── literals.md
│   ├── binding-and-defs.md
│   ├── methods-and-handlers.md
│   ├── object-literals.md
│   ├── string-interpolation.md
│   └── sigils.md
├── laws/                   what the substrate guarantees
│   ├── substrate-laws.md
│   ├── reflection-contract.md
│   ├── isolation-laws.md
│   └── purity-and-effects.md
├── process/                how we work
│   ├── docs-driven.md
│   └── open-questions.md
└── reference/              formal specs (filled as we build)
```

## conventions

- **lowercase voice.** moof is a friendly thing; we write about it that way.
- **citations everywhere.** if an idea has prior art, cite it. names, years, paper titles where available. see `vision/lineage.md`.
- **concrete examples.** every concept doc shows real moof code. real, not pseudocode.
- **explicit over implicit.** when there is a choice between magic and verbosity, we pick verbosity.
- **moldable above the rust line.** if a thing can live in moof, it does. `process/docs-driven.md` for the rule.

## status

- **phase 0** (vision + docs): in progress (this folder).
- everything else: see `roadmap.md`.

`>.<` softly. let's build a real one this time.
