# open questions

> **things we have not decided. each is recorded with its constraint
> set so future-us can resolve it without re-discovering the
> tensions.**

resolving a question: edit the question to `## (resolved) …`, write
the resolution, link the doc that now reflects it.

## syntax

### Q1 — operator precedence on binary sends

current: explicit nesting required. `[a + b * c]` is an error.

tension:
- math precedence is friendlier (`*` binds tighter than `+`).
- smalltalk-flat is more uniform.
- explicit nesting is most honest, but verbose for math-heavy code.

leaning: explicit nesting wins until we have user feedback otherwise.
revisit after first 10kloc of moof code is written.

### Q2 — name shorthand for symbol keys in `#[...]`

current: `#['name => "ada"]`. explicit.

tension:
- `#[name: "ada"]` is friendlier but visually conflicts with
  send-keyword syntax `[obj name: x]`.
- consistent grammar is worth a few more characters.

leaning: stay with `=>` until painful.

### Q3 — `is` vs `==` vs `eq?` for identity check

current: `[a is b]` for identity, `[a = b]` for value-equality.

tension:
- `is` reads natural ("a is b") but conflicts with potential type
  ascription (`x is Integer`).
- `eq?` is lispy.

leaning: `is` for now; switch if `is` is needed elsewhere.

## semantics

### Q4 — vat granularity boundaries

current: vats for "non-trivial persistent identities" — workspaces,
windows, services. ordinary objects are not vats.

unresolved:
- should every long-running computation be a vat (game-of-life,
  training-run)?
- is "a function call that runs for >1s" a vat?
- what's the cost of vat creation?

leaning: leave it pragmatic until we have running code to measure.

### Q5 — synchronous vs async sends within a vat

current: synchronous within a vat.

unresolved:
- can a within-vat send "yield" to allow the inbox to be processed
  in the middle of a long synchronous chain?
- if not, long computations starve the inbox until they complete.
  is that ok? (smalltalk-80: yes. erlang: usually not.)

leaning: yes, sync within a turn. user code that wants async behavior
splits into multiple turns explicitly.

### Q6 — caching pure-function results

current: substrate may memoize pure functions safely.

unresolved:
- what's the cache key? (function-id, args)
- when is the cache invalidated? (proto-edit invalidates anything
  closing over that proto's methods)
- is this opt-in per function or substrate-default?

leaning: substrate-managed, opt-in via `#memoize` annotation;
opt-out via `#no-cache`. default neither.

## persistence

### Q7 — checkpoint / compaction frequency

current: configurable per-vat. unspecified default.

unresolved:
- "every N turns" vs "every X seconds" vs "on idle."
- size threshold ("when journal exceeds K entries").

leaning: start with "every 1000 turns or 5 minutes idle, whichever
first." tune from real workloads.

### Q8 — content-addressing inside a vat

current: forms have a vat-local id. canonical encoding produces
deterministic bytes.

unresolved:
- do we deduplicate proto-forms by content-hash? (a proto referenced
  by 1000 instances need only be stored once.)
- if so, what's the GC story for retired proto-versions?

leaning: yes dedup; reference-counted protos with periodic sweep.
defer detailed design.

### Q9 — cross-vat replicated tables

current: not implemented. each vat has its own state.

unresolved:
- how do we mirror a Table across vats for read-only access?
- consistency model — strongly consistent (slow), eventually
  consistent (fast)?
- erlang's mnesia is the model; how much of mnesia do we want?

leaning: defer to "phase 4 — distribution." don't pre-build.

## reflection

### Q10 — frame edit-and-continue semantics

current: `[frame edit-method!]` allows live editing during debug.

unresolved:
- if the new method has different signature, what happens to
  args already on the stack?
- can the user roll back to a previous source version of the method?

leaning: same-signature requirement for in-place edit; otherwise
restart from a higher frame. signal an error if can't.

### Q11 — bytecode visibility

current: `[m bytecodes]` returns decoded bytecode.

unresolved:
- do we expose the *raw* bytecode bytes? or only a Table-of-opcodes
  decoded view?
- can users edit bytecode directly? (probably no — it's derived;
  edit source.)

leaning: decoded view only; raw bytes hidden because they're a
representation detail.

## ergonomics

### Q12 — slot setter syntax

current: `[self count: 5]` to write a slot. no shorter form.

tension:
- ruby/python use `@count = 5` / `self.count = 5` — concise.
- `[self count: 5]` is consistent with general send syntax.
- a special `(.count := 5)` form would shorten but adds syntax.

leaning: keep verbose form. revisit if we hate it.

### Q13 — implicit return vs explicit `(return …)`

current: every block/method returns its body's value (implicit).

unresolved:
- early return from a multi-clause method?
- `(return ...)` form to break out of a chain?

leaning: implicit return only; use `(if … …)` or restructure for
early-return-flavored control flow. revisit if painful.

### Q14 — let-vs-let* vs let-rec defaults

current: three flavors, all available.

tension: do we need three? scheme has them all; clojure collapses.

leaning: keep all three. each has clear use case.

## federation

### Q15 — vat-id format

current: UUIDv7 timestamp-prefixed, with friendly path aliases.

unresolved:
- exact format of the alias path (`/`-separated? hierarchical?
  canonical mapping?).
- migration when a vat moves to a new alias.

leaning: filesystem-style paths; aliases are mutable; ids are not.

### Q16 — discovery of remote vats

current: through the world's path-table.

unresolved:
- how does a federation announce its vats to peers? gossip? central
  registry? bonjour-style?
- defer until we actually federate.

leaning: defer.

## tooling

### Q17 — package format for libraries

current: a library is a directory of `.moof` files plus optional
`.mco` for rust bindings. `package.moof` manifest.

unresolved:
- versioning (semver? content-hash?).
- dependency resolution.
- registry vs decentralized.

leaning: defer until we have multiple users.

### Q18 — testing framework

current: not designed.

unresolved:
- inline `(test "name" body)` vs separate test files.
- assertion library design.
- how does the testing framework express in-vat vs cross-vat tests?

leaning: build it in moof, atop queries and data sources, when we
need it.

---

(this list grows. each entry that resolves moves into a doc and is
deleted from here.)
