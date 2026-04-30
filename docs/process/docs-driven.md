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

`.mco` files are runtime-loadable native modules: a platform-tagged
dylib + binding metadata, dlopened at runtime, exposing one moof
proto. they are *the* mechanism for bringing rust/c libraries into
moof. blake3, lmdb, ed25519, wgpu, websocket — all mcos.

two rules govern mco use:

- **state stays in moof.** an mco's native methods read and write
  `self`'s slots through the substrate native abi. opaque rust-side
  resources (an open file handle, a wgpu device) live as
  `ForeignHandle` *slot values* on the moof Form, with a destructor
  the gc invokes. nothing about an mco-backed object is hidden from
  reflection.
- **only the primitives are native.** an mco supplies the small set
  of operations that genuinely require rust (the leaf calls into the
  underlying library). everything *derivable* from those primitives
  is moof code, defined in the proto's handler table at mco load
  time. user code can read and modify the derived methods.

if you find yourself writing rust code, you are also writing an mco
with that rust as a *primitive* method. if the rust would be
re-derivable from a smaller set of primitives, those primitives are
the rust; the rest stays moof. otherwise you are not writing rust.

(`concepts/compiled-objects.md` for the full mco model.)

## the rule of thumb for capabilities

**cap discipline is from day one.** there is no "we'll add caps
later" path. the substrate ships with the primordial cap set
(`$out`, `$err`, `$clock`, `$random`, …) constructed at boot by the
root supervisor; user code receives caps as arguments; nothing ever
gets a cap by ambient lookup. this means *even phase A* — the
smallest substrate seed — has caps.

practical consequences:

- **no free-function print, ever.** `print`, `println`, `puts` are
  not in the language. the only path to stdout is `[$out emit:
  bytes]` (low-level) or `[$out say: x]` (the derived
  `:to-string` + emit + newline). a Smalltalk-style `Transcript`
  arrives later as a moof-side Form wrapping `$out`; it is not a
  substrate primitive.
- **caps are first-class Forms** implementing standard protocols.
  `$out` implements `DataSource` (sink); you can attach combinators
  to it (`[$out chunk: 1024]`, `[$out tee: log-file]`).
- **cap calls in phase A are synchronous direct invocations.**
  intent/receipt machinery is added in phase B alongside
  persistence, when the indirection earns its keep. user code
  doesn't notice the change — the syntax `[$out emit: text]` is
  identical at both phases.
- **the substrate's symbol-table check at phase-A acceptance is**:
  no `print`, no `println`, no `puts`, no `simulated_*`. the only
  way to write text from moof is through a cap.

## the rule of thumb for stdlib shape

**prefer methods on protocols. avoid free functions.** define a
small set of primitive methods on a protocol; *derive* a much
larger set from them. this is clojure's protocols + haskell's
typeclasses + rust's traits, in moof's send-shape.

| protocol | primitive methods | derived methods |
|---|---|---|
| Iterable | `:next`, `:done?` | `:map:`, `:filter:`, `:reduce:from:`, `:for-each:`, `:to-list`, `:to-table`, `:take:`, `:drop:`, `:any?:`, `:all?:`, `:contains?:`, `:count`, `:zip:`, `:scan:from:` |
| Equatable | `:=` | `:!=` |
| Comparable | `:<` | `:<=`, `:>`, `:>=`, `:between:and:`, `:min:`, `:max:`, `:clamp:to:` |
| Sized | `:length` | `:empty?`, `:non-empty?` |
| Hashable | `:hash` | (none — qualifies for use as Set/Map keys) |
| Showable | `:to-string` | `:inspect` (default) |
| Ordering | `:compare:` | derives all of Comparable |

a type implements a protocol by providing the primitive methods.
the protocol's *deriving rules* (themselves moof code) install the
derived methods into the type's proto when the protocol is mixed in.

free functions are reserved for: control-flow operatives (`if`,
`let`, `do`), top-level constructors that have no meaningful
receiver (`(make-vat …)`), the rare pure operation that genuinely
takes no preferred subject (`(min a b)` is borderline; `[a min:
b]` is preferred).

**when porting a free function to a protocol method, the change of
viewpoint matters.** `(map f xs)` becomes `[xs map: f]`: the
collection is the subject; mapping is what we ask of it. this also
unlocks polymorphism: any Iterable answers `:map:`; the same code
works for Lists, Tables, Strings, DataSources, Iterables you
haven't written yet.

(do not port the v4-take-1 `bootstrap.moof`'s free-function shape
forward as-is. rewrite it method-shaped from the start in phase
A.10.)

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
