# queries

> **datalog-style relational queries over Forms. rules derive new
> relations. queries pattern-match. `?name` is just a name; `(query)`
> and `(rule)` are the only operatives that treat it as a logic
> variable.**

queries are how you ask the world relational questions. tables are
relations; rules derive new tables from existing ones; queries
extract tuples by pattern. this is **datalog** (ullman, *principles
of database and knowledge-base systems* 1988), embedded in moof.

## facts as Tables

a fact is a tuple. a relation is a Table of tuples.

```moof
(def parents
  #[ #['alice 'bob]
     #['bob 'carol]
     #['carol 'dee] ])
```

equivalently a columnar Table:

```moof
(def parents
  #['parent => #['alice 'bob 'carol]
    'child  => #['bob 'carol 'dee]])
```

rows are interconvertible; queries see both shapes.

## rules

rules derive new relations from existing ones. syntax:

```moof
(rule name (?head-vars …)
  :- (body-relation ?vars …)
     (body-relation ?vars …)
     …)
```

example:

```moof
(rule grandparent (?x ?z)
  :- (parents ?x ?y)
     (parents ?y ?z))
```

reads: *grandparent(x, z) holds whenever parents(x, y) and parents(y, z).*

multiple clauses for one rule head are allowed. they form a
disjunction:

```moof
(rule ancestor (?x ?z)
  :- (parents ?x ?z))
(rule ancestor (?x ?z)
  :- (parents ?x ?y)
     (ancestor ?y ?z))
```

recursion is well-defined under standard datalog semantics: take the
least fixpoint. the substrate's query engine (itself moof code)
implements this with stratified evaluation.

## queries

```moof
(query (grandparent 'alice ?z))
;; → DataSource yielding values of ?z

(query (?p ?q) where (parents ?p 'bob))
;; → DS yielding pairs

(query (ancestor 'alice ?z))
;; → DS — recursive
```

a query returns a data source. tuples stream lazily; you can pipe,
filter, take, etc.:

```moof
(pipe (query (ancestor 'alice ?z))
  [take: 10]
  [for-each: println])
```

## what `?name` means

a name beginning with `?` is *just a regular identifier* — no
special value-kind. `(query)` and `(rule)` operatives treat
`?`-prefixed names as logic variables; outside those contexts,
`?name` evaluates as an ordinary lookup (almost certainly an
unbound-variable error, which is a useful guardrail).

this means: no third name-kind to learn. one symbol type (`'foo`).
one identifier convention. logic-variable behavior is *operative-
local*, not lexer-global.

## negation, aggregation, etc.

stratified negation:

```moof
(rule childless (?p)
  :- (person ?p)
     (not (parents ?p ?_)))
```

aggregation (running sums, counts, averages):

```moof
(rule num-children (?p ?n)
  :- (person ?p)
     (?n := (count (parents ?p ?_))))
```

these are extensions provided as moof libraries on top of the
substrate's basic datalog. user code can add new aggregators.

## queries as a way of life

the inspector is a query consumer:

```moof
(query (?obj proto: Counter where: [?obj count > 100]))
;; → all Counters with count > 100, in this vat
```

the senders/implementors browser:

```moof
(query (?m implements: 'incr in: ?proto))
;; → every method named :incr, with its proto
```

saved searches are saved data-source pipelines.

the world-state itself is queryable:

```moof
(query (?vat status: 'crashed since: ?when))
;; → vats that have crashed and when
```

## scope of a query

by default, a query runs over the current vat's heap.  
explicit scope:

```moof
(query in-vat: 'shreyan-workspace
  (?obj proto: Counter))

(query in-world
  (?vat status: 'running))
```

cross-vat queries fan out via far-refs and are async; results stream
back as the participating vats respond.

## extensibility

the query operative is moof code. user-defined rule heads, custom
matchers, and domain-specific query languages can all be added by
extending the `Query` proto. one substrate hook (`:query`),
unlimited expressiveness.

## why datalog (not SQL or general logic programming)

- **datalog is decidable.** every query terminates; bounded compute
  cost.
- **datalog composes.** rules layer; queries are values.
- **datalog matches our shape.** Tables-as-relations is natural;
  the substrate already has all the facts.
- **datalog has a great descendant family.** datomic, datascript,
  rama, souffle. lots of practical experience to lean on.

prolog's full search is too unbounded for a substrate primitive;
SQL's surface is too tied to a specific stored data model.

## inspirations

- datalog: ullman, *principles of database and knowledge-base
  systems* (1988); the prolog/datalog community.
- prolog: colmerauer & roussel.
- datomic: rich hickey, the datomic talks; pull/datalog syntax.
- datascript: nikita prokopov (modern in-memory datalog).
- glamorous toolkit's spotter: tudor gîrba — proves that "queries
  over the world" is a usable user-facing affordance.

## see also

- `concepts/tables.md` — relations are Tables.
- `concepts/data-sources.md` — query results stream.
- `concepts/forms.md` — what's queryable.
- `concepts/blocks-and-patterns.md` — pattern matching.
