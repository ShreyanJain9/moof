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
6. `concepts/world-and-space.md` — the 3D zoomable world; what users
   inhabit.
7. `concepts/replication.md` — croquet-style multi-replica vats.
8. `concepts/effect-intents.md` — how cap effects fit into determinism.
9. `concepts/compiled-objects.md` — mco-as-dylib; how the substrate
   stays small.
10. `concepts/data-sources.md` — the universal i/o primitive.
11. `concepts/references.md` — federation-from-day-one.
12. `concepts/persistence.md` — per-vat database storage.
13. `concepts/pixmap.md` — one inhabitant proto, demonstrative.
14. `syntax/overview.md` — the surface, at a glance.
15. `roadmap.md` — what we build, in what order.
16. `process/audit-2026-04-29.md` — why the roadmap is shaped the way
    it is (post-stress-test).
17. `process/impl-plan-v4.md` — the day-by-day next steps.

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
│   ├── replication.md         ;; replicated vats, croquet-style
│   ├── effect-intents.md      ;; intent/receipt model for caps
│   ├── transport.md           ;; reflector ↔ replica wire
│   ├── world-and-space.md     ;; 3D zoomable world; Frames, Placements
│   ├── data-sources.md
│   ├── persistence.md
│   ├── queries.md
│   ├── compiled-objects.md    ;; mco-as-dylib; substrate stays small
│   ├── reflection.md
│   ├── time-and-journal.md
│   ├── moldability.md
│   ├── canvas-and-input.md    ;; $canvas, $pointer caps (mco-delivered)
│   ├── pixmap.md              ;; one inhabitant proto (was: moofpaint)
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
│   ├── purity-and-effects.md
│   └── determinism-laws.md   ;; what replicated vats observe and refuse
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
