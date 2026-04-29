# docs-driven implementation

> **the docs are authoritative. the implementation follows. when you
> are about to write code, write or update the relevant doc first.**

## the principle

we are building moof v4 with documentation leading the code. every
substrate decision, syntax choice, and contract is specified here
*before* it lands in rust or moof. the order is:

1. **read** the relevant doc.
2. if it doesn't say what you need: **write** the doc, or update it.
3. then **implement** to match.
4. when implementation forces a doc change: pause, update the doc,
   then continue.

## why

three reasons we keep relearning:

- **shared mind.** when ideas are written down, future-shreyan and
  future-claire can disagree productively. when ideas live only in
  one head, they drift silently.
- **prevent drift.** v3 had docs that were out-of-sync with the
  implementation; the docs lied; nobody trusted them; the docs got
  worse. flipping the order is the only fix.
- **think before code.** writing a sentence about why a primitive
  exists exposes the cases you haven't considered. cheaper to
  notice in prose than in test failures.

## the rule of thumb for rust vs moof

**default: moof.** if the thing can live above the rust line, it
does. write rust only when:

1. the thing touches the OS (file, socket, screen, raw input, clock,
   random).
2. the thing IS the bytecode interpreter or send primitive.
3. the thing is GC, allocation, or scheduler internals.
4. the thing is the bootstrap parser (the smallest possible reader,
   used to load the real parser).

everything else is moof. the parser proper, compiler, analyzer, type
system, query engine, inspector, canvas, editor, package system,
test framework, std library — moof code, in `lib/` (or wherever we
end up putting them), loaded as source on first boot, bytecode-cached
after.

when in doubt about whether a thing should be rust or moof, prefer
moof. you can move it to rust later if perf demands. the moldable
promise gets enforced by this preference.

## the rule of thumb for compiled-objects

`.mco` files are *only* for rust-bound methods on a single object.
do not use `.mco` for image persistence, package distribution,
stdlib delivery. those have their own mechanisms
(`concepts/persistence.md`, `concepts/data-sources.md`).

if you find yourself writing rust code, you are also writing a
`.mco` file with that rust as a native method. otherwise you are not
writing rust.

## the workflow

### for a new feature

1. read existing docs in `concepts/` and `syntax/` for what touches
   this.
2. write a new doc (or update an existing one) describing the new
   feature: surface, semantics, inspirations, examples.
3. raise the doc with collaborators (claire, codex, gemini, etc.)
   for adversarial reading. update.
4. only then implement.

### for a bug fix

1. read the doc for the affected concept.
2. is the bug a doc/code mismatch? if yes, decide which is right.
3. update the doc if needed.
4. fix the code.

### for a refactor

1. read the doc.
2. write the change in the doc first (note: "as of refactor X, the
   shape changes to Y").
3. implement.
4. update any cross-references.

### for a hard question

if the docs don't tell you what to do, *the docs are the bug*. add
the question to `process/open-questions.md` until it's resolved.

## doc style

- **lowercase voice.** matches the project's culture.
- **citations.** when an idea has prior art, attribute it. names,
  years, titles. see `vision/lineage.md`.
- **examples.** every concept doc has runnable-shaped moof code.
- **explicit beats implicit.** when there's a choice, pick the
  more verbose option for documentation.
- **cross-link.** at the end of each doc, link related docs.

## on staleness

a doc that's wrong is worse than a doc that's missing. when you
notice staleness, fix it immediately. we'd rather have fewer docs,
all true, than many docs, half-true.

at every release boundary (whatever that means in our world): a
brief audit pass of the docs. anything that disagrees with reality:
reconciled.

## inspirations

- knuth's *literate programming*: the doc and the code are the same
  artifact, written in the order a human reads.
- the rust RFC process: docs lead implementation; review happens at
  doc level.
- glamorous toolkit's culture of "the inspector is the documentation":
  gîrba. (we extend this to: the substrate is documented in the docs
  *and* introspectable from inside.)

## see also

- `process/open-questions.md` — current unresolved questions.
- `vision/manifesto.md` — why moldability requires this discipline.
- `concepts/moldability.md` — what it produces.
