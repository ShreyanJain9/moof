# archive

> these are moof's pre-tear-down docs, kept for historical context.
> do NOT treat them as current. they are not consistent with each
> other; they represent the state of moof's thinking at various
> points from v1 through early v2.

**current documentation lives at [../README.md](../README.md).**

---

## why archived

as of 2026-04-23, we did a complete documentation tear-down and
rebuild. the old docs had real problems:

- **sprawl** — 35 files, ~13,000 lines, no clear reading order.
- **duplication** — persistence described in 3 files, effects in
  4, protocols in 4.
- **contradictions** — some docs still referenced removed forms
  (try/catch, while, :=). others listed protocols that were
  later deleted.
- **wave-apology comments** — "wave 9.4 TBD", "phase 1 deferred",
  embedded in normative docs, rotting.
- **aspirational/current confusion** — readers couldn't tell what
  moof *did* vs what moof *intended*.
- **god-doc syndrome** — VISION.md was 1700 lines and tried to be
  both manifesto and reference.

the rebuild replaced them with clear type labels (vision / concept
/ reference / law / roadmap), one source of truth per concept, a
coherent reading order, and laws + doctrine as the review
constitution.

---

## what's actually valuable to read here

historical context:

- **VISION.md** — the long-form manifesto from v2 alpha. the soul
  of moof is articulated here even when details are stale.
  distilled form: [../vision/manifesto.md](../vision/manifesto.md).
- **authoring-vision.md** — the kay-engelbart-atkinson lineage
  doc. structural parts lifted into the new manifesto.
- **foundations.md** — the "five pillars" doc. material harvested
  into concepts/persistence, concepts/effects, etc.
- **effects-and-vats.md** — deeper treatment of Act composition.
  distilled form: [../concepts/effects.md](../concepts/effects.md).
- **purity.md** — the case for immutability + Updates. now
  covered in concepts/effects and concepts/vats.
- **STDLIB-PLAN.md** — the implementation checklist for the
  stdlib. mostly superseded by stdlib-doctrine.

---

## what NOT to read

explorations, speculation, or early drafts that don't reflect
current moof:

- **morphic-on-vello.md** — renderer experiment; canvas design
  moved on.
- **ui-explorations.md** — early UI sketching; superseded by
  vision/horizons.
- **tell-layer.md** — natural-language command layer experiment;
  not current.
- **type-system.md** — static types proposal; not on the roadmap.
- **prototype-types.md** — structural row types proposal;
  companion to above.
- **infix-sublanguage.md** — alternative infix surface syntax;
  not shipping.
- **sugar-interface.md** — #{...} generic sugar; not shipping.
- **construction.md** — four construction idioms; basics in
  reference/syntax now.

---

## the wave-6 docs

- **wave-6-core-contract.md** and **wave-6-triage.md** were the
  operational artifacts for wave 6's cleanup. done work, not
  current vision. kept for contributors understanding how wave 6
  landed.
- **core-contract-matrix.md** is the truth table they produced.
  current status is tracked in current docs.

---

## the stdlib docs

- **stdlib-doctrine.md** and **stdlib-at-a-glance.md** here are
  pre-archive versions. current copies live in
  [../laws/](../laws/). edit the laws/ copies, NOT these.
- **stdlib-vision.md** was an earlier take on the stdlib.
  superseded by stdlib-doctrine.

---

## cross-references that are now stale

links in these docs point at relative paths that no longer exist,
doc names that moved or merged, phase/wave numbers from old
roadmaps. we're not updating these. the archive is frozen. if a
link 404s, it moved. check the current docs.
