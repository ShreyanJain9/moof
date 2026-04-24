# stdlib vision

a blueprint for moof's standard library and toolkit. this
captures the design philosophy, what to steal from where,
the abstraction hierarchy, and the wave-by-wave plan.

## the three lives of moof

moof is simultaneously three things that nobody has combined:

1. a **language** — dynamic, prototype-based, monadic effects, gradual structural types
2. an **environment** — persistent objectspace, vats as actors, capabilities, live image
3. a **medium** — canvas, agents, direct manipulation, time travel, moldable tooling

every stdlib decision must serve all three. no "this library is
for the language but not the environment." if a type exists, it
must be inspectable, serializable, renderable, queryable, and
editable at runtime. the stdlib IS the environment speaking to
itself.

## philosophical commitments

these constraints shape every file:

1. **nothing is a black box.** every value inspectable. closures
   show source + captures + purity. Acts show target + args.
   servers show history + mailbox. protocols show requirements
   + conformers.

2. **nothing is final.** the inspector IS an object. override
   its render method while looking at it. not a hack — the
   medium working as designed.

3. **errors don't crash the world.** application errors are Err
   values that flow through chains. VM panics fail one handler
   but the vat survives. the workspace continues.

4. **time is recoverable.** image is a checkpoint. event log is
   automatic for servers. rewind any duration. queries "as of T".

5. **computation is value.** Plans, Acts, Closures, Queries —
   all MOVED, SHARED, INSPECTED, and EXECUTED separately.

6. **names are local.** namespaces nest, shadow, alias. no
   global soup. REPL has its own namespace. servers have theirs.

7. **the system documents itself.** every protocol has a
   docstring. every method has examples. `(doc x)` works on
   everything.

8. **one concept exists exactly once.** no three types that
   overlap. the stdlib is a coherent text, not a catalog.

9. **extensibility through protocols, not inheritance.** conform
   and extend. no deep hierarchies. Builder pattern over
   subclassing.

10. **immutable by default, controlled mutation through vats.**
    server state changes via Update. everything else is values.

## what to steal, by project

### language-level inspiration

**Ruby's Enumerable** — implement one method, get fifty. we
already have Iterable; go further. names that read like english.
`each_cons`, `chunk_while`, `tally`, `partition`, `group_by`,
`each_with_object`. aliases everywhere. 80 methods, not 40.

*steal: lavish method richness + english aliases.*

**Clojure's seq** — ANY sequence-shaped thing is a seq. `(seq x)`
coerces. lazy by default. transducers as separate dimension.

*steal: seq as a universal coercion. transducers. lazy default.*

**Elixir's pipe** — `data |> step1 |> step2`. code reads top-to-
bottom, data flows left-to-right.

*steal: pipe as fundamental sugar. everywhere.*

**Haskell's Prelude** — small, surgical, compose-friendly.
data argument usually LAST so functions curry well.

*steal: composition-friendly function signatures.*

**APL/k/J** — every common op is a single token. `+/` for sum.
density + mathematical precision.

*steal: unicode method names for common ops. `∑` for sum, `⇒`
for transform, `↑`/`↓` for take/drop. moof already supports this.*

**SQL** — queries as composable declarative data structures.

*steal: Query as a VALUE you build, inspect, transform, persist,
send across the network.*

**Python's pathlib** — paths as objects. operators where
they make sense. `path / "file.txt"`.

*steal: domain types that feel like built-ins. Path, URL, Color,
Money. each with its own operators and display.*

**Rust's iterator + Result + ?** — explicit errors. short-circuit
via `?`. no panics for expected failures.

*steal: every fallible op returns Result. `then:` is our `?`.*

**Smalltalk's collection hierarchy** — OrderedCollection, Set,
Bag, Dictionary, SortedCollection — each a real type with
correct semantics.

*steal: Set, Bag, SortedSet, OrderedMap. don't treat them as
afterthoughts.*

**Erlang's bit syntax** — declarative binary pattern matching.
`<<Name:8/binary, Age:32/big>>`.

*steal: a binary DSL for formats and protocols.*

**Lua** — tiny, simple, one collection (table) does much.
OOP emerges from metatables.

*steal: simplicity. don't have OrderedDict + DefaultDict +
Counter — have Table with methods.*

**Go's fmt** — `fmt.Printf("%v", x)` works on anything. extensible
format verbs.

*steal: a powerful format mini-language with extensible verbs.*

**Racket's `#lang`** — every file declares its language.
entire programs in custom DSLs.

*steal: namespaces as languages. `(namespace my-dsl ...)`
exports a vau-based DSL. files can declare which DSL they use.*

**OCaml's modules** — first-class, parameterizable, functors.

*steal: namespaces as first-class values. alias, compose,
parameterize.*

**Scala's collections** — Builder pattern, traits everywhere.

*steal: the Builder protocol. how does a collection know what
"same kind" means? Builder gives the answer.*

### environment-level inspiration

**Smalltalk (1980)** — the image. no "program file" — the
objectspace IS the program, persistent across sessions.

*steal: make the image a first-class concept the user interacts
with. save, load, branch, inspect.*

**Lisp Machines (1984)** — every error opens a debugger you can
fix the bug in and continue. the system pauses, not crashes.

*steal: continuations and restart machinery. `(debug)` pauses
execution with the full environment available.*

**HyperCard (1987)** — direct manipulation of objects with
handlers. layout IS the program. ordinary people made software.

*steal: the script-on-object pattern. every UI element has
handlers for `mouseUp`, `keyDown`. events ARE the dispatch.*

**Self (1995)** — prototype objects + morphic UI. grab any
object on screen, inspect and modify live.

*steal: morphic-style direct manipulation. every rendered
object is real, not a picture of an object.*

**Croquet/Squeak (2003)** — collaborative real-time objectspaces.
multiple people in the same image.

*steal: the multiplayer foundation — vats over network. a
shared workspace is just a vat with far refs for multiple
REPL connections.*

**Glamorous Toolkit (2020)** — moldable inspectors. every
object has a custom view. JSON becomes a tree, SQL becomes
a grid. tools mold to data.

*steal: THIS IS THE BIG ONE. type-specific inspectors. a
Table inspector is a grid. a Server inspector is a dashboard.
each type ships its own inspection surface.*

**Observable / Jupyter** — the notebook. cells + results.
dependencies tracked. change a cell, downstream re-runs.

*steal: every cell is a vat. computation IS the medium.
reactive dependencies via Signals.*

**Datomic** — time as a query dimension. immutability + log =
perfect history.

*steal: `[image at: 'yesterday]`. query any past state. server
history available by default.*

**Git** — content-addressed storage. immutable values addressed
by hash. branching/merging as first-class.

*steal: moof values are immutable — hash them. caching is
automatic. distributing computation = sending hashes.*

**Plan 9** — everything is a file. one mechanism.

*for moof: everything is an object. we're already there.
make it visible.*

### medium-level inspiration

**Self / Morphic** — direct manipulation. the inspector IS the
thing. *teaches: every UI element is a real, manipulable object.*

**Genera / Lisp Machines** — presentations. values on screen
retain identity. click to send messages. *teaches: output isn't
text, it's projections of live objects.*

**Squeak / Etoys** — kids make programs by drawing tiles.
abstraction is tangible. *teaches: visual flow as real syntax.*

**HyperCard's stacks** — "documents" were programs. no
data/code line. *teaches: workspaces are first-class artifacts.*

**Eve (the failed one)** — datalog as substrate. records and
bindings. *teaches: queries are how you ASK, not just retrieve.*

**Subtext** — copy-by-reference live structure. *teaches:
liveness through derivation.*

**Nile / Gezira** — graphics as functional pipelines. 10k lines
of C become 100 lines of Nile. *teaches: domain-specific density
produces 100x compression.*

**Aurora / LightTable / Bret Victor** — immediate connection
code↔result. *teaches: feedback latency is design.*

**Datasette** — sqlite + web UI. standard interfaces unlock
compounding. *teaches: ship one inspection interface that
works for all data.*

**Notion / Roam** — bidirectional links, blocks as first-class.
*teaches: structure emerges from how you USE things.*

**Excel** — cells + formulas. accidentally the most-used
programming language. *teaches: simple substrate + composition
beats complex features.*

**NeXT/Cocoa's KVO+KVC+bindings** — data binding as first-class.
*teaches: connect any value to any other, automatically.*

**Genera command line** — typed args with completion, history,
undo on commands. *teaches: even text input can be rich.*

**Tcl/Tk/Wish** — entire UI scriptable from one language.
*teaches: the GUI is just programs.*

**Emacs Org-mode** — outlines + tables + links + code + exports.
one text format = knowledge OS. *teaches: a shared structured
format unlocks compounding.*

**Wolfram Notebooks** — cells with rich output. *teaches:
writing IS the runtime.*

**Plan9's acme + sam** — text as universal interface, mouse
chord operations. *teaches: small uniform mechanisms beat
featureful ones.*

**HyperCard + Mathematica + Smalltalk together** — if you took
HyperCard's directness, Mathematica's symbolic power, and
Smalltalk's live objectspace, you'd have moof's medium target.

## the abstraction hierarchy

### tier 0: substrate

foundations that shape everything above.

- **Reactive** — Signal, Derived, Observer (dependency tracking)
- **Builder** — `[type builder]` + `[builder add:]` + `[builder result]`
  (round-trip protocol — map: on Cons returns Cons, on Table
  returns Table)
- **Pattern** — destructuring, guards, exhaustiveness
- **Transducer** — composable transformations independent of collection
- **Namespace** — scoping with import/export/alias
- **Membrane** — capability filtering around FarRefs

### tier 1: data

representation and shape.

- **Collections** — Sequence, Set, Map, Bag (proper hierarchy)
  - Cons (linked list), Vector (table seq), Stream (lazy), Range
  - HashSet, SortedSet
  - Table (map), SortedMap, OrderedMap
  - Multiset
- **Lazy** — Stream, Conduit, Generator (all via Iterable)
- **Persistent** — HAMT for sets/maps, RRB-trees for vectors
- **Serialization** — JSON, CBOR, EDN, CSV, Markdown, HTML, XML

### tier 2: domain types

- Path, URL, Email, IP, Domain, UUID, Version
- Color (RGB/HSL/OKLCH), Pixel, Image
- Money, Currency
- Instant, Duration, Calendar, Timezone
- Vector, Matrix, Tensor, Complex
- Geo (Point, Polygon), Distance, Bearing
- Range, Interval
- Hash, Signature, Cipher

### tier 3: text and language

- Format (fmt + extensible verbs)
- Template (first-class interpolation)
- Regex, Glob, Wildcard
- Diff, Patch
- Parse combinators, Pratt parser, Grammar
- Lex, Tokenize

### tier 4: systems (all via capability vats)

- File, Network, Process, Env, OS signals
- Random, Clock
- Storage (KV, doc, SQL)

### tier 5: concurrency

- Pool, Queue, Channel, Conduit
- Semaphore, Mutex, RWLock
- Supervisor, Cluster, Discovery, Migration

### tier 6: tooling

- Profile, Trace, Debug, Bench
- Test, Property, Fuzz
- Lint, Format (code), Doc

### tier 7: semi-magical (moof-specific)

- Atom (immutable values with identity)
- History (automatic event sourcing)
- Replay, Snapshot, Migration, Branching
- Memo (automatic memoization for pure fns)
- Cache (content-addressed result storage)

### tier 8: UI / canvas

- Renderable protocol
- Shape (primitives)
- Layout algorithms
- Style (colors, fonts, themes)
- Animation (interpolation, easing, timelines)
- Input, Window, Viewport
- Inspector (type-specific introspection)
- Workspace (your session as data)

### tier 9: agentic

- Persona, Tool, Memory, Context
- Membrane (capability filtering and logging)
- Deliberation, Critic, Plan

### tier 10: world-class extras

- Web (HTTP server/client, routing, auth, sessions)
- Notebook (cell-based env)
- Chart, Map, Graph viz
- Audio, MIDI
- PDF, Crypto, Compress
- 3D

## the design rules

rules for every stdlib file:

1. **conform-and-multiply.** implement 1, get 50. no duplication.
2. **uniform names.** `take:`, `drop:`, `first`, `last`, `reverse`,
   `map:`, `filter:` — same names across ALL collection-like types.
3. **immutable everywhere.** every op returns a new value.
   structural sharing for performance.
4. **lazy for streams, eager for collections.** type chooses.
5. **errors as values.** Result for fallible ops. No exceptions.
6. **introspection at every level.** describe, doc, conforms?,
   handlerNames on everything.
7. **DSL-friendly.** vau variants of features that make sense.
8. **small files, clear protocols.** no tangled webs.
9. **namespace is the unit of organization.** not files.
10. **everything is a value.** types, namespaces, protocols,
    modules, queries, plans — first-class.

## safety rules (learned from mistakes)

rules for writing stdlib code:

1. **no infinite construction at load time.** if iteration
   could be infinite, do NOT consume it during file loading.
2. **bounded tests.** every example has explicit `(take: N)`
   or similar bound.
3. **test files are tiny.** load fast, fail fast.
4. **no complex nested closures in object literals during
   parse_all load.** use defmethod for complex handler bodies.
5. **commit after every working state.** small, reversible changes.
6. **load incrementally.** add one file at a time, verify,
   then add the next.
7. **structural equality must be opt-in.** don't override
   Object#equal: — use deep-equal helpers.
8. **never have a file that can't load standalone.** each
   file is self-sufficient given its declared dependencies.

## the wave plan

each wave is a coherent deliverable. ~1-2k lines per wave.

### wave 1: data substrate
- builder.moof — round-trip protocol
- collections.moof — Sequence/Set/Map hierarchy
- pattern.moof — match form with destructuring
- transducer.moof — composable transformations
- format.moof — fmt + Template
- regex.moof — pattern strings
- json.moof — JSON serialization
- path.moof — Path domain type
- time.moof — Instant/Duration/Calendar

### wave 2: namespaces + tooling
- namespace.moof — proper scoping (redo carefully)
- inspect.moof — Inspector protocol + 5 type-specific impls
- doc.moof — extract docstrings, search docs
- profile.moof — time + memory measurement
- trace.moof — execution log
- bench.moof — measurement framework

### wave 3: real capabilities (moof + plugins)
- random.moof + plugin — RNG
- file.moof + plugin — filesystem
- network.moof + plugin — HTTP
- process.moof + plugin — spawn
- storage.moof + plugin — KV store

### wave 4: reactive substrate
- signal.moof — Signal/Derived/Observer
- atom.moof — immutable values with identity
- history.moof — automatic event sourcing
- conduit.moof — typed pipes between vats

### wave 5: web
- http.moof — client + server DSL
- routes.moof — routing DSL
- session.moof — auth/sessions
- websocket.moof — real-time

### wave 6: agent
- persona.moof — LLM vat configuration
- tool.moof — capability description
- membrane.moof — capability filtering
- memory.moof — conversation persistence
- context.moof — context assembly

### wave 7+: canvas and beyond
- rendering primitives
- layout algorithms
- notebook framework
- visualization

## the first brick: Builder + Pattern + Collections

start with three files that unlock everything above:

1. **builder.moof** — solves the duplication problem. every
   Iterable method that constructs a new collection uses Builder.
   map: on Cons returns Cons, on Table returns Table. one
   implementation, all types.

2. **pattern.moof** — structural matching. `(match val
   (Some x) [x + 1] None 0)`. unlocks ergonomic code everywhere.
   pattern IS the structural type system applied at runtime.

3. **collections.moof** — proper Set, Bag, SortedMap on top
   of existing Table/Cons. doesn't duplicate — extends.

each file is small. each demonstrates a design principle.
each compounds the power of what comes after.

## the vibe

the stdlib should feel like an invitation, not a reference.
a new user explores it. each type comes with examples. each
example is runnable. running puts values in the workspace.
you can mess with them.

the stdlib isn't documentation you read — it's a place you
visit.
