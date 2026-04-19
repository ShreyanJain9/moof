# the objectspace: think in data

moof's vision isn't "a language with a nice stdlib." it's a
**living objectspace** where every object the user cares about —
a file, a git commit, a person, a recipe, an email, a workout,
a chess position, a bookmark, an API response — exists as a
first-class moof value with a moldable view. the user doesn't
open documents. they navigate reality.

this document is the north star. every stdlib and runtime
decision from here on threads through it.

---

## the two commitments

everything in this doc follows from two design commitments:

**1. every object is either *represented* or *stored*.**

- *stored* values are native moof objects, fully owned, their own
  source of truth. a `Recipe` you type into the workspace is
  stored. a `Person` record you're authoring is stored. they
  serialize, migrate, fork, and rewind.
- *represented* references proxy external things. a `File path:`
  represents a filesystem path. a `GitCommit hash:` represents a
  blob in `.git/objects`. an `HTTPResource url:` represents a
  URL. represented objects have a fetch strategy, a cache, and a
  staleness check. the external world is their source of truth.

both kinds are first-class: inspectable, linkable, queryable,
renderable, comparable. **the user rarely cares which is which**
— they inspect a file and see its content; they inspect a
recipe and see its ingredients; the machinery for getting there
is a detail of the type, not a workflow concern.

**2. computation is data flowing through the objectspace.**

no separate "runtime state" hiding in the VM. no global mutable
world. everything the program does is:

- **facts** added to the image (create an object, update a slot)
- **rules** that derive new facts (pure functions, transducers)
- **queries** that ask about the image (relational + datalog)
- **streams** of events threading between vats (effects, messages)
- **dataflow** graphs that react to change (signals, cells)

the stdlib isn't shaped around "collections and their operations."
it's shaped around **values-as-facts, transformations-as-rules,
changes-as-streams, views-as-queries**. *think in data* means
the user describes WHAT they want derived from the data they
have, and the runtime figures out how.

---

## the objectspace side

### identity, hash, content-addressing

every moof value has a **content hash**. two `{ Some value: 42 }`
literals are the same fact — same hash, shared storage, no
duplication. this is the prerequisite for everything downstream.

- `[v hash]` — `Hashable` protocol, required on every type.
  primitives delegate to native, containers fold over children,
  user-defined types get a structural default from `shape`.
- immutable values are *identified by their hash*. two images
  that independently produce the same value recognize each other.
- mutable objects (servers, workspaces) carry a stable **UUID**
  separate from their content. the UUID doesn't change as the
  content mutates; the content has a version-hash that does.

content-addressing is what makes the store queryable,
sharable, distributable, and content-deduplicated. it's load-
bearing. **`Hashable` is the first piece to build.**

### the inspector with aspects

every value answers: *"how do you want to be seen?"* the answer
is not a single rendering — it's a collection of **aspects**.

```
[aFile inspect]
  → Inspector aspects: (
      preview (hex) (tree) (gitBlame) (history) (syntax)
      (linesOverTime) (incomingLinks))
```

each aspect is a named view. the medium (terminal, notebook,
canvas, agent) asks for a specific aspect or enumerates the
available ones. a hex-dump of a file and a syntax-highlighted
view are both aspects of the same underlying `File`. neither
is more "correct."

this is Glamorous Toolkit's moldable-tools pattern. tools mold
to data instead of the other way around. the stdlib ships a
generous default inspector for every type; user-defined types
add their own aspects trivially. inspectors are themselves
values — first-class, composable, overridable at runtime.

- `[v inspect]` → returns an `Inspector`
- `[inspector aspect: 'tree]` → returns a `View` for that aspect
- `[inspector aspects]` → list of available aspects
- `[view render: medium]` → paint into the given medium

there is no `Document` type. a "document" is any value whose
inspector happens to compose child inspectors into a layout. a
notebook is the inspector for a `Workspace`. a spreadsheet is
the inspector for a `Table` under a particular aspect.

### workspaces, not files

the user's session is a **workspace** — a named root value in
the store. workspaces have:

- a title, a creation time, a history
- a pinned collection of objects (shortcuts)
- a canvas / notebook / spatial layout (the default aspect)
- a namespace of user-defined names
- fork/merge semantics (like git branches)

multiple workspaces coexist. they share underlying content-
addressed objects. forking a workspace is O(1): new root, same
graph. diffing two workspaces compares their graphs.

there is no "open a file." the user navigates from their root
workspace into objects, into sub-objects, into referenced
objects, into related objects. the whole image is explorable.

### representing the external world

each represented type is a moof protocol that formalizes:

- **fetch** — how to pull the external truth into the image
- **cache policy** — how stale is acceptable; manual/auto refresh
- **addressing** — the external identity (path, url, hash, row id)
- **staleness check** — cheap way to know if the cache is invalid
- **backlinks** — what other moof objects reference this

example types (all wave-1+ work, but the *shape* is defined now):

- `File` — filesystem paths. cache = content+mtime.
- `HTTPResource` — URLs. cache = body+etag+last-modified.
- `GitCommit` / `GitBlob` — content-addressed already; no
  staleness. perfect fit.
- `MailMessage` — IMAP message ID. fetch via IMAP capability.
- `CalendarEvent` — CalDAV or similar.
- `WebBookmark` — URL + title + scraped summary.

the **pattern** is the contract. any represented type conforming
to `Reference` gets participation in queries ("find all
references that have gone stale"), in the workspace graph
("what's linked to this file?"), in the store ("fetch on
demand; don't persist the cached body").

---

## the dataflow side

### values are facts; programs are rules

thinking in data reframes what a program is:

- **an object is a fact.** `{ Recipe name: "tomato soup" ingredients: (...) }` isn't "an instance being constructed" — it's the assertion that such a recipe exists in this objectspace.
- **a transducer is a rule.** `(compose (map extract-ingredient) (filter vegetarian?))` doesn't execute — it's a *description* of how to derive vegetarian-ingredient-facts from recipe-facts. apply it to a list, a stream, a query result — the description is the same; the substrate decides how.
- **a query is a question.** `[workspace recipes where: |r| [r isVegetarian?]]` isn't imperative filtering. it's: *given these facts and these rules, what satisfies?* the engine decides whether to scan, use an index, or incrementally maintain.
- **a signal is a derivation.** `(def total-time (signal [[cookTime] + [prepTime]]))` declares a relationship. when either input changes, the output re-derives. no explicit update path — dataflow is implicit.

moof leans on three traditions:

- **datalog** (Eve, Datomic, Flix) — relations and rules.
  queries are declarative; indexes and incremental maintenance
  are invisible. the objectspace IS the fact base.
- **stream processing** (core.async, Flink, Rama) — data in
  motion. events flow through operator graphs. transducers ARE
  the operators.
- **dataflow** (Observable, Adapton, Differential Dataflow) —
  reactive DAGs. a change ripples through the graph; only what
  actually depends on the change recomputes.

these aren't three separate sublanguages. they're three views
of the same substrate: **immutable facts + pure rules + typed
streams of events + automatic propagation**.

### streams, transducers, and the collection algebra

the collection hierarchy sits *under* stream-processing, not
beside it:

- **Iterable** — a bounded finite collection; `each:` walks it
  once.
- **Stream** — a potentially-unbounded lazy sequence; `next`
  produces the next value on demand.
- **Signal** — a continuously-updating value; `observe:` gets
  notified on changes.
- **Channel** — a typed pipe between vats; `send:` and `receive`
  cross vat boundaries.

all four support the same operator vocabulary because they all
take **Transducers**: `(map f)`, `(filter g)`, `(take 10)`,
`(distinct)`, `(window-by k)`, `(chunk-every n)`. transducers
are values you compose, name, persist, and ship:

```
(def vegetarian-prep (compose
  (filter vegetarian?)
  (map extract-prep)
  (take 20)))

[recipes-list transduce: vegetarian-prep]   ; runs on a Cons
[recipes-stream transduce: vegetarian-prep] ; runs lazily
[recipes-signal transduce: vegetarian-prep] ; reactive
[recipes-channel transduce: vegetarian-prep] ; cross-vat
```

the per-type `map:`/`select:`/etc. overrides we wrote (`buildLike:`)
are *specializations* of this. they can stay as sugar — `[xs map: f]`
desugars to `[xs transduce: (map f)]` — but they're no longer the
primary path. the substrate is the transducer.

### queries as values

a `Query` is a data structure the user assembles and the engine
interprets:

```
(def recent-vegetarian
  (query
    (from workspace recipes)
    (where (some (filter vegetarian?) ingredients))
    (where (> modified-at (days-ago 30)))
    (order-by modified-at)
    (take 10)))

[recent-vegetarian run]           ; run against the store
[recent-vegetarian explain]       ; show the plan
[recent-vegetarian live]          ; return a Signal, reactive
[peer <- (query send: recent-vegetarian)]  ; ship it across vat
```

queries are inspectable data structures. they compose. they
explain their plans. they run against local Tables, the full
store, or a remote vat. they become Signals for live results.

this is `Queryable` — a protocol every collection-shaped thing
conforms to, with a small required core and a rich provided
surface.

### reactivity is the connective tissue

the moment you have facts + rules + queries, **change
propagation** is the thing that makes it *feel alive*. you edit
a recipe — downstream queries, live inspectors, open canvas
views, running agents — all see the change and re-derive.

the primitives:

- `Atom` — an immutable value with identity. swap one value for
  another; subscribers see the change.
- `Signal` — derived value; when inputs change, output re-derives
  on demand.
- `Observer` — side-effecting subscription; fires on change.
- `History` — automatic event log; every atom-mutation is
  recorded. time-travel queries fall out.

these are listed in the stdlib vision as wave 4. they're the
connective tissue that makes the objectspace LIVE instead of
inert.

---

## the store

the current store (bincode-dump to LMDB on exit) is a *snapshot*.
it doesn't support any of the above. a real store has to handle:

**1. content-addressed blobs.** immutable values keyed by their
hash. deduplicated automatically. shareable across workspaces
and (later) across machines.

**2. object identity for mutable things.** UUIDs for servers and
workspaces. the UUID is stable; the content is a versioned
blob referenced by hash.

**3. indexes.** to make queries fast: type index (all Recipes),
backlink index (what references X), time index (modified after
T), aspect-specific indexes (declared per protocol).

**4. event log.** every mutation appended. queries can say "as
of T." history is automatic. no "did someone save?" — the log
IS the save.

**5. schema evolution.** types change over time. migrations are
moof functions: `(migrate Recipe v1 to v2 |old| ...)`. old
instances load, upgrade on access.

**6. workspaces as named roots.** multiple workspaces, each a
root pointing into the shared object graph. fork/merge/diff as
first-class operations.

**7. represented-reference caches.** represented objects store
their external address + cache metadata; the cached content
itself goes in the content-addressed blob store under a hash.

**8. the store is a capability.** accessed via a capability vat.
the moof runtime doesn't touch persistence directly — it sends
messages to the Store vat. this makes the store swappable:
in-memory for tests, LMDB for local, Postgres or S3 for scale.

---

## implementation path

nothing here is trivial. the order matters because each piece
unblocks the next.

### phase 0: substrate (unblocks everything)

1. **Hashable protocol + implementations.** every primitive
   type, Cons, Table, Set, Closure, General. user-defined types
   get a `shape`-derived default. small, but load-bearing.

2. **Transducers.** `(compose (map f) (filter g))` as a value.
   `[coll transduce: xf]` on every Iterable. `buildLike:` gets
   rewritten in terms of this.

3. **Stream + `next` protocol.** lazy sequences. every Iterable
   can be `[iter lazy]`-ified into a Stream. transducers work
   unchanged.

4. **Sealed + exhaustive match + guards + object patterns.**
   finishes the pattern substrate so rules (match-based) are
   reliable.

### phase 1: the dataflow tier

5. **Signal / Derived / Observer.** the reactive primitives.
   wave 4 of the original plan moves up.
6. **Atom.** mutable cell with change notification.
7. **History.** automatic event log on atoms and servers.
8. **Queryable protocol + Query values.** declarative queries
   against any collection, initially. datalog surface later.

### phase 2: the objectspace and the inspector

9. **Inspector protocol (with aspects).** `inspect`, `aspect:`,
   `aspects`, `render: medium`. terminal/ANSI renderer as the
   first medium. the REPL's display_value moves here.
10. **Reference protocol + first represented types.** `File`,
    `HTTPResource`, `GitBlob`. cache policies. staleness.
11. **Workspace type.** named root. fork/diff/merge.
    persistence redesign starts here.

### phase 3: the real store

12. **Content-addressed blob store** (probably LMDB under the
    hood, but the schema is radically different from today's).
13. **Object-identity + versioning for mutable objects.**
14. **Indexes + incremental maintenance.**
15. **Schema evolution machinery.**
16. **Represented-reference cache integration.**

### phase 4: the medium

17. **Text-mode notebook.** terminal-based. cells render via
    Inspector. keyboard navigation. proves the whole stack.
18. **Canvas** (wave 7+ in the original plan). spatial direct-
    manipulation. egui. the notebook primitives carry over.

### phase 5: the agent

19. **Agent in a membraned vat.** reads the objectspace via
    Inspector. issues queries. proposes changes. the membrane
    logs/filters.

---

## what this means for moof's identity

the one-line pitch shifts from:

> "a persistent concurrent objectspace with prototype-based types"

to:

> "a living objectspace where everything in your digital life is
> a first-class object, queried and transformed as data, with
> moldable views and reactive flow."

the language is still prototype-based. the vat model still
holds. the concurrent image still persists. but the user
doesn't experience it as "a language" — they experience it as a
**medium** that happens to be programmable all the way down.

---

## design rules that emerge

1. **every type is Hashable.** no exceptions for "convenience."
   the store depends on it.
2. **every type has an Inspector with at least one aspect.**
   even if it's just `describe`. tools mold to data.
3. **stored vs represented is a type-level choice**, baked into
   the type's conformances. not a runtime flag.
4. **transducers over per-type overrides.** when tempted to
   override `map:` on a new type, reach for transducers first.
5. **queries, not loops.** if you're writing a for-loop over
   the objectspace, you're asking the wrong question.
6. **reactivity by default for derived values.** if the user
   expects an answer to stay up-to-date, wrap in a Signal.
7. **the store is a capability.** the runtime never touches
   disk directly. all persistence is mediated.
8. **no Document type.** documents are compositions. pages are
   inspectors. notebooks are Workspaces with a particular aspect.

---

## what to build first, concretely

the ordering above is a roadmap. the *first wave* — what we'd
start on right after this doc lands — is:

- **`Hashable`** protocol + implementations for every primitive
  + Cons + Table + Set.
- **`Transducer`** — compose, and the `(map f) (filter g) (take n)`
  vocabulary. `[coll transduce: xf]` on Iterable.
- **`Stream`** — lazy Iterable via `next`. streams compose with
  transducers for free.
- **pattern completion** — guards, object patterns, sealed
  exhaustive match.
- **`Inspector` protocol (shape first)** — the protocol defined;
  a minimal terminal renderer; every existing type declares at
  least a `default` aspect.

that's phase 0 plus the inspector protocol. ~500 lines of moof,
a few hundred lines of rust. enough to *feel* the shift.
