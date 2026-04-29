# lineage

every idea in moof v4 has prior art. this file is the bibliography:
**what we take, from whom, and what they were doing.** if you find
yourself reaching for a design move, check whether one of these
ancestors already thought it through. usually yes.

attribute generously. nobody in this list owes us anything; we owe
them everything.

---

## the smalltalk family

### smalltalk-72 / 76 / 80
- **alan kay**, *the early history of smalltalk* (HOPL-II, 1993).
- **adele goldberg, david robson**, *smalltalk-80: the language and
  its implementation* (addison-wesley, 1983).
- **dan ingalls**, *design principles behind smalltalk* (byte, 1981).

what we take:
- the live image: the world is what you save, not "a file."
- doesNotUnderstand: as the universal extension hook.
- become: as identity-swap at the heap level.
- the inspector and class browser as first-class tools.
- "everything is an object receiving messages" — but see hewitt below
  for the deeper claim.

### self
- **david ungar, randall b. smith**, *self: the power of simplicity*
  (OOPSLA 1987).
- **ungar, smith, et al.**, the self papers on dynamic deoptimization,
  customization, polymorphic inline caches.

what we take:
- pure prototype-based delegation. no class/metaclass split.
- slots and methods unified — `slot` and `method-named-slot` look
  the same to the caller.
- polymorphic inline caches as the basis for fast dispatch without
  losing reflection (`concepts/forms.md`).

### morphic
- **john maloney, randall b. smith**, *directness and liveness in the
  morphic user interface construction environment* (UIST 1995).

what we take:
- ui elements are objects you grab, halo, and edit live.
- no separation of "ui code" and "everything else" — morphs are
  ordinary objects with `:draw` and `:on-event` handlers.

### pharo / glamorous toolkit
- **stéphane ducasse et al.**, pharo papers and books.
- **tudor gîrba et al.**, the glamorous toolkit and *moldable
  development* writings.

what we take:
- per-object custom inspectors.
- moldable development as a design principle: tools are co-developed
  with the domain (`concepts/moldability.md`).
- the spotter and senders/implementors browser as queries.

---

## the actor / message-passing family

### hewitt's actor model
- **carl hewitt, peter bishop, richard steiger**, *a universal modular
  ACTOR formalism for artificial intelligence* (IJCAI 1973).

what we take:
- the foundational claim: a world is many small computations in
  message-passing conversation. influences smalltalk's "objects send
  messages" rhetoric and erlang's processes — both descend from this.

### erlang / OTP
- **joe armstrong**, *making reliable distributed systems in the
  presence of software errors* (PhD thesis, KTH, 2003).
- **armstrong, virding, williams, wikström**, *concurrent programming
  in erlang* (prentice hall, 1996).

what we take:
- cheap isolated processes with mailboxes (`concepts/vats.md`).
- supervision trees and let-it-crash.
- hot code reload as a substrate feature, not a hack.
- ETS / DETS / Mnesia as the model for in-memory + persistent +
  distributed table storage (`concepts/persistence.md`).
- selective receive for actor mailboxes.

### E
- **mark s. miller**, *robust composition: towards a unified
  approach to access control and concurrency control* (PhD thesis,
  johns hopkins, 2006).
- **miller, tribble, shapiro**, *concurrency among strangers*
  (TGC 2005).

what we take:
- vats as the actor abstraction (`concepts/vats.md`).
- promise pipelining for async sends.
- capabilities as unforgeable references (`concepts/capabilities.md`).
- the discipline that cross-vat reference is *the only* reference
  type that crosses isolation boundaries (`concepts/references.md`).

### croquet / TeaTime
- **alan kay, david p. reed, david a. smith, andreas raab,
  julian lombardi, mark s. miller, david ungar**, the croquet
  papers (~2003–2007).

what we take:
- the conviction that distributed live environments are *possible*
  with deterministic-actor synchronization.
- a sketch of how shared moldable worlds can stay coherent across
  machines without consensus protocols (`concepts/vats.md`).

### ambienttalk
- **tom van cutsem, jessie dedecker, et al.**, ambienttalk papers
  (vrije universiteit brussel).

what we take:
- the actor-OO synthesis with first-class futures.
- fault-tolerance moves for "the network is a partial function."

### pony
- **sylvan clebsch et al.**, *deny capabilities for safe, fast
  actors* (AGERE! 2015).

what we take:
- reference capabilities as a *practical* basis for safe sharing.
  pony does it statically; we do it more loosely, but the spirit
  applies.

---

## the lisp / homoiconic family

### lisp / scheme / common lisp
- **john mccarthy**, *recursive functions of symbolic expressions
  and their computation by machine* (CACM 1960).
- **guy l. steele jr., gerald jay sussman**, the lambda papers /
  scheme reports.
- **guy l. steele jr.**, *common lisp: the language* (digital press,
  2nd ed. 1990).

what we take:
- code is data; data is code. quoted forms are values you can walk.
- the term "form" itself for an evaluable expression.
- `eval` and `read` as first-class.
- macros as functions from forms to forms.

### kernel
- **john n. shutt**, *fexprs as the basis of lisp function
  application; or, $vau: the ultimate abstraction* (PhD thesis,
  WPI, 2010).

what we take:
- the operative/applicative split. operatives receive unevaluated
  arguments; applicatives evaluate before sending. neither is a
  special category — both are first-class objects with the same
  protocol (`concepts/sends-and-calls.md`).
- the conviction that special forms shouldn't be a separate
  category. user-defined operatives are first-class.

### maru
- **ian piumarta, alessandro warth**, *open, extensible object
  models* (VPRI memo, 2007).
- **ian piumarta**, *accessible language-based environments of
  recursive theories: a white paper on the subject of language design*
  (VPRI memo, 2006).

what we take:
- the maru posture: a tiny rust seed; everything else above the
  line, modifiable from within (`process/docs-driven.md`).
- the evaluator-as-object idea: `:eval` is a method on a Form's
  proto.

### io
- **steve dekorte**, the io language (started 2002).

what we take:
- explicit Message objects with name, arguments, next, cached-result.
- prototype-based unification of object and code reification.
- io is the closest existing language to moof v4's spirit; we differ
  by adding actors, persistence, and types.

### clojure
- **rich hickey**, the clojure papers and talks (~2007–onward).

what we take:
- protocols (structural typeclasses).
- transducers (composable lazy stream transformations) — informs
  `concepts/data-sources.md`.
- the discipline of "place-oriented programming is a mistake"
  (informs identity vs value distinctions).
- the rhetorical move of giving constructs names worth saying.

### newspeak
- **gilad bracha**, newspeak papers (~2007–onward).

what we take:
- modules as objects. no global namespace.
- late-bound everything.

---

## the database / time / data family

### datalog / prolog
- **alain colmerauer, philippe roussel**, *the birth of prolog*
  (HOPL-II 1996).
- **jeffrey d. ullman**, *principles of database and knowledge-base
  systems* (computer science press, 1988).
- modern datalog: souffle, datascript, datomic's pull/datalog.

what we take:
- relations + rules + queries as a first-class declarative idiom
  (`concepts/queries.md`).
- recursive queries and stratified negation as the grammar of
  introspection (the senders/implementors browser is a query).

### datomic
- **rich hickey, stuart halloway, et al.**, the datomic papers and
  talks.

what we take:
- time as a first-class axis. "as-of" queries.
- the database is a value; history is free.
- a clear, explicit story for accumulation-only state vs ephemeral
  views (`concepts/time-and-journal.md`).

### LMDB / mmap'd transactional stores
- **howard chu**, lightning memory-mapped database.

what we take:
- the implementation strategy for per-vat persistence: mmap'd
  B-tree, multi-version concurrency control, single-writer ACID
  (`concepts/persistence.md`).

### ETS / DETS / Mnesia
- **erlang/OTP** documentation; klacke and joe armstrong's writings.

what we take:
- the table abstraction as ergonomic in-memory + persistent +
  distributed storage. *declare what you want; runtime makes it real.*

---

## the array / data-structure family

### APL / J / K
- **kenneth e. iverson**, *a programming language* (wiley, 1962),
  and *notation as a tool of thought* (turing award lecture 1979).
- **roger hui, et al.**, the J language.
- **arthur whitney**, K and Q.

what we take:
- rank-polymorphic operations.
- broadcasting, reductions, scans, outer products.
- the conviction that uniform tables/arrays are a fundamental
  primitive worth syntactic sugar (`concepts/tables.md`).

### lua
- **roberto ierusalimschy, et al.**, *the implementation of lua 5.0*
  (jucs 2005), and *programming in lua* (book).

what we take:
- the table as a universal collection: array + map in one type.
  `concepts/tables.md` is essentially "lua's table, with APL
  vocabulary, as a Form."

---

## the friendly-syntax family

### ruby
- **yukihiro matsumoto**, the ruby language (~1995–onward).

what we take:
- string interpolation `"#{expr}"`.
- predicate-method `?` and mutating-method `!` suffixes.
- the cultural conviction that programming languages should
  be friendly to humans and apologize for nothing.

### haskell
- **simon peyton-jones, paul hudak, philip wadler, et al.**, the
  haskell reports.

what we take:
- pattern-matched multi-clause definitions as a *rhythm* of code.
- type ascription `::` syntax.
- the discipline that effects should be explicit (we do this with
  capabilities, not monads — `concepts/capabilities.md`).

---

## the environment family

### hyperCard
- **bill atkinson, dan winkler**, hyperCard and hypertalk (apple, 1987).

what we take:
- accessibility-without-compromise. direct manipulation. cards and
  scripts on the same screen as buttons and fields.

### genera (lisp machine)
- **symbolics inc.**, the genera environment (~1980s).

what we take:
- presentation-based UI: every thing on screen is the actual object,
  available for clicking and inspecting.
- the editor-and-runtime-are-one-thing posture.

### lively kernel
- **dan ingalls et al.**, the lively kernel papers (~2007).

what we take:
- morphic in the browser: the *running world is the page*; you sculpt
  it in place.

---

## what we deliberately do not take

- **JVM/CLR-style bytecode portability.** moof bytecode is per-image,
  invalidated by source edits, never a distribution target.
- **monad-shaped effect systems.** capabilities are clearer for our
  audience and our use case.
- **static-by-default typing.** types are optional and gradual; the
  haskell typeclass *vocabulary* is welcome, the type-erasure machinery
  is not.
- **multi-language polyglot platforms.** moof is one language, by
  intent. multi-language is solved by mounting external systems as
  data sources and far-refs, not by sharing a heap.
- **JIT/native compilation as a substrate concern.** if it ever
  matters, it can be added in moof above the line.

---

## reading order, if you have a weekend

if you read three things to understand where moof v4 is coming from,
make them:

1. shutt's *kernel* (~80 pages) — for the operative/applicative split
   and the conviction that special forms shouldn't be special.
2. piumarta & warth, *open extensible object models* (~30 pages) — for
   the substrate-as-tiny-seed posture.
3. miller's *robust composition* (~300 pages, but worth it) — for the
   actor / vat / capability synthesis that makes federation tractable.

if you have one weekend more, add:

4. armstrong's PhD thesis — the supervision philosophy.
5. ungar & smith's *self: the power of simplicity* — the prototype
   philosophy.
6. one of the recent gîrba talks on glamorous toolkit — the moldable
   development culture.

we are standing on a *lot* of shoulders. the synthesis is ours; the
ideas are not. cite generously.
