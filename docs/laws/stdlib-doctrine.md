# stdlib doctrine

> the rule book for moof's standard library. decides what goes where,
> what counts as a protocol, when to add vs refuse. read before touching
> anything in `lib/`.

---

## the one rule

**every generic operation lives on a protocol. every protocol is minimal
(1–3 required methods). every type gets generic behavior by conforming.**

that's it. expanded:

1. if an operation makes sense on "anything that can X," it belongs
   inside a protocol as a `(provide ...)` derived from the protocol's
   minimum `(require ...)`.
2. if an operation only makes sense on one type, it's a plain method
   on that type. no protocol.
3. no new protocol unless at least **three** concrete types will
   conform to it at declaration time.
4. no duplication of a protocol's capability by a concrete type —
   if `Cons` is `Iterable`, its `map:` comes from the protocol, not a
   custom defmethod. custom handlers only for demonstrable perf wins,
   with a comment explaining why.

that's the whole governance model. every other rule below derives
from this one.

---

## what the stdlib covers (the surface, mapped)

the stdlib needs to handle these ~18 categories. treat this as the
completeness checklist. anything not categorizable here is probably
not a stdlib concern.

| # | category | what it does | current home |
|---|----------|--------------|--------------|
| 1 | **value identity** | show, hash, equal, compare | `kernel/showable`, `kernel/equatable`, `data/hashable`, `data/comparable` |
| 2 | **numbers** | arithmetic, conversion, ranges | `data/numeric`, `data/range` |
| 3 | **text** | strings, substrings, formatting, escaping | partial in native, no moof protocol |
| 4 | **sequences** | walk, map, filter, reduce, sort | `data/iterable`, `data/indexable` |
| 5 | **associations** | tables as maps, keys, values, lookup | native + Indexable |
| 6 | **sets + bags** | unordered collections, counting, membership | `data/set`, `data/bag` |
| 7 | **option / result** | absence, failure, chaining | `data/option`, `kernel/error` |
| 8 | **time** | clock, duration, timestamps, scheduling | clock cap + TBD |
| 9 | **bytes** | binary buffers, encoding | native |
| 10 | **callables** | compose, partial, flip | `data/callable` |
| 11 | **concurrency** | Acts, Updates, vats, messages | `data/act`, `tools/server`, native scheduler |
| 12 | **patterns** | destructure, test shape | `flow/pattern` (partially broken) |
| 13 | **pipelines** | transducers, lazy streams | `flow/transducer`, `flow/stream` |
| 14 | **reactivity** | atoms, signals, observation | `flow/reactive` |
| 15 | **persistence** | image, snapshots, blobs | rust blobstore + moof save-image |
| 16 | **namespaces** | paths, URLs, tree walks | `kernel/url`, `kernel/namespace` |
| 17 | **system** | services, capabilities, grants | `system/*` |
| 18 | **introspection** | type, prototypes, slots, source | `kernel/identity`, `tools/inspect` |

gaps (explicit):
- **text** has no protocol. should there be a `Textual` protocol? (`as-string`, `length`, maybe) probably yes, so String, Bytes-as-utf8, URL-as-path all conform. punted for now but named.
- **time** has no moof types. `Duration`, `Timestamp`, `Instant` don't exist. every real program needs them. see [the openings](#the-openings) below.

---

## the protocol catalog

these are the protocols moof commits to. every one has ≥3 conformers or
is deleted. every one is minimal. every one has a clear reason to exist.

### `Showable` — renderable for humans
- **require**: `show` (returns a String)
- **provides**: `describe` (defaults to show)
- **conformers**: every nameable type. currently: URL, Option, Some, None, Range, Set, Bag, Atom, Signal, Stream, Inspector, Query, Workspace, Registry, Service, System. target: every concrete type in the stdlib.

### `Equatable` — value equality + hash pairing
- **require**: `equal:` (bool)
- **provides**: `!=`, `in:` (membership over Iterable)
- **partners with**: `Hashable`. any `Equatable` should also be `Hashable` so it can be a map/set key.
- **conformers**: Some, None (today). target: every value type — Integer, Float, String, Symbol, Cons, Table, Range, URL, all user types.

### `Hashable` — produce a stable hash Integer
- **require**: `hash` (returns Integer)
- **provides**: nothing derived today; may grow
- **conformers**: Integer, Float, String, Symbol, Boolean, Nil, Cons, Table, Set, Object default. target: matches Equatable's conformers 1:1.

### `Comparable` — total order
- **require**: `<`
- **provides**: `>`, `<=`, `>=`, `between:and:`, `clamp:to:`, `min:`, `max:`
- **conformers**: Number, String (today). target: add Date/Time when they exist, Version, and any orderable user type.

### `Numeric` — arithmetic values
- **require**: `+`, `-`, `*`, `=`, `<`
- **provides**: `abs`, `negate`, `sign`, `zero?`, derived ordering via Comparable
- **conformers**: Number (today, covering Integer + Float + BigInt via the wave-9.3 unification). target: add Rational if it exists, Vector for vec3, complex if it exists.

### `Iterable` — walkable sequence
- **require**: `fold:with:`
- **provides**: `each:`, `map:`, `select:`, `reject:`, `take:`, `drop:`, `count`, `sum`, `product`, `min`, `max`, `reduce:`, `toList`, `toTable`, etc. (big set, documented in the file.)
- **conformers**: Cons, Bag, Set, Stream, Range (should conform, add). target: Table values, String chars when wanted.

### `Indexable` — random-access sequence
- **require**: `at:`, `count`
- **provides**: `first`, `last`, `empty?`, `indexOf:`, plus an `Iterable` promotion via `fold:with:` derived
- **conformers**: String, Table. target: fine as-is; don't force Cons to conform (it's not O(1) access).

### `Callable` — invokable
- **require**: `call:`
- **provides**: `compose:`, `>>`, `partial:`, `flip`
- **conformers**: every `<fn>` value implicitly (native-level). target: add Transducer so `[xform (reducer)]` works, any user-defined callable wrapper.

### `Thenable` — the universal composition contract
- **require**:
  - `then:` (bind)
  - class-side `pure:` (lift a value into this context)
- **provides**:
  - `map:` (fmap; derived as `[self then: |x| [self pure: (f x)]]`)
  - `recover:` (default: `self` — "nothing to recover from").
    Err and None override to run the continuation.
- **conformers**: Cons, Option (via Some/None), Result (via
  Ok/Err), Act, Update, Stream.

**no introspection surface.** Thenable is deliberately opaque.
no `ok?`, no `pending?`, no `resolved?`. the ONLY way to
interact with a Thenable's contents is to compose — via `then:`
or `recover:`. the scheduler handles resolution; users don't
ask "is it done yet?"

acts in particular are meant to stay opaque. probing their
state is a violation of the abstraction. if a specific type
wants to expose a diagnostic like `running?`, that's a
type-specific method, NOT a Thenable method. keep Thenable
minimal.

**why fused.** an earlier doctrine proposed splitting Thenable
into Monadic + Fallible + Awaitable. reverted — the split cost
three conformances per type and bought nothing. Err and None
override `recover:`; that's sufficient to express fallibility
without exposing a `ok?` probe. one protocol, minimal surface.

**total: 9 protocols.** Thenable fused; Reference, Buildable,
Interface deleted. do-notation is universal.

---

## the deletion list

### delete outright

- **`Reference` protocol** (`data/reference.moof`). one conformer (File),
  no generic operation depends on it, URL already provides identity.
  the method `[obj describe]` is enough.
- **`Buildable` protocol** (`data/builder.moof`). zero explicit conformers.
  `Builder` the type is redundant — any Iterable can accumulate via
  `fold:with:` or a transducer. delete the protocol, keep Builder as a
  plain type if any call site uses it, else delete that too.
- **`Interface` protocol** (`system/system.moof`). zero conformers.
  this is a spec-holder for a rust trait. move the spec to
  `docs/interfaces.md`; delete from the stdlib.

### do NOT split

an earlier version of this doctrine proposed splitting
`Thenable` into `Monadic` + `Fallible` + `Awaitable`. that was a
mistake: it turned one universal protocol into three, required
every context to declare three conformances, and exposed
`ok?` and `pending?` as public probes — violating Acts'
opacity. the fused + opaque shape is correct. `recover:` (as
a composition primitive, not a query) handles fallibility
without breaking abstraction.

### widen (add conformers)

- **`Equatable`**: Integer, Float, String, Symbol, Boolean, Nil, Cons,
  Table, Range, URL. target: every value type gets a `(conform X Equatable)`
  with `equal:` → `[a = b]` or structural equality.
- **`Comparable`**: URL (lex), Boolean (false < true), Nil-as-bottom? punt
  on the last, but URLs at minimum.
- **`Showable`**: any stdlib type that doesn't yet have show. audit.

### pick one, delete the other

- **pipelines**: `Transducer` (lazy, composable) wins. `Query` becomes
  either sugar over Transducer or gets deleted. write the `recipes`
  example using Transducer; if it's not noticeably worse, delete Query.
- **dispatch**: `defmethod` wins. `[Proto handle: sym with: f]` stays
  available as a primitive but isn't used in stdlib code. audit every
  direct `handle:` call; convert to defmethod or document why it can't be.

### rebuild broken things or delete

- **`match-constructor`** in `flow/pattern.moof:97`: references undefined
  `Env`. fix (probably needs a `$env`-capturing vau) or delete constructor
  patterns entirely.
- **`[Query any]`** in `tools/query.moof:52`: inverted. one-line fix.
- **`Thenable.map:`** on Cons: broken (nil parent). fix by
  rewriting `map:`'s default to use `[self pure:]` directly
  (each conformer exposes its own `pure:` class-side) rather
  than walking `[self parent]`.

---

## the rules of addition

before adding to the stdlib, answer:

1. **does it fit an existing category?** if yes, where?
2. **is it a new protocol or a method on an existing one?**
   - "works on any Iterable" → provide in Iterable, not a new protocol
   - "works on any number-like" → provide in Numeric
   - "only works on Cons" → plain defmethod on Cons, no protocol needed
3. **if protocol: ≥3 concrete conformers at declaration?** if no, stop.
4. **does it duplicate an existing operation?** if yes, either replace
   the existing one or abandon the new one. no parallel universes.
5. **is it a "generic operation" (belongs with protocol) or a "specific
   method" (belongs on a type)?** most things are specific. protocols
   are rare by design.

when in doubt: write the specific method. promote to protocol only when
the pattern recurs three times.

---

## the openings (what the stdlib still owes)

things i'd expect a mature stdlib to cover that moof doesn't yet:

1. **time.** `Duration` as a Numeric-ish value with ms/s/min accessors.
   `Timestamp` as a point-in-time with `+ Duration → Timestamp`.
   `Instant` maybe. clock cap gives you raw ms; we need the types to
   make that ergonomic.
2. **text protocol.** `Textual` with `as-string`, `length`, `chars:`.
   so String, URL, Bytes-as-utf8, and user types with a natural text
   form all conform. would replace ad-hoc `describe` → String coercion.
3. **ranges over non-numbers.** Date ranges, Time ranges, arbitrary
   comparables. requires Numeric + Comparable decoupling (ranges need
   order, not arithmetic).
4. **dates / calendars.** downstream of time. punt until time lands.
5. **regex or pattern-matching on text.** String has `contains:`,
   `startsWith:`, `split:`. no proper pattern/regex. decide: is this
   a stdlib thing or a plugin? probably plugin.
6. **JSON / structured text.** already a plugin (moof-plugin-json).
   confirm: data interchange is a plugin, not stdlib.
7. **logging / diagnostics.** console cap is raw IO. a `Log` protocol
   + default file sink would be useful. small wave.
8. **testing.** `tools/test.moof` exists, but is moof-side-only and
   sparse. wave: turn it into a harness that can run a test dir and
   produce JUnit-style output.
9. **streams with backpressure.** `flow/stream.moof` is pull-based;
   push-based with backpressure is a reactive-systems concern. later.

these are the OWED items. not urgent, but acknowledged.

---

## protocol file conventions

every protocol file follows this template:

```moof
; protocol-name.moof — one-line summary
;
; 2-3 sentences of what the protocol is for, why it matters,
; and who conforms to it at a glance. cite real conformers.

(defprotocol Name
  "Docstring: what conformers can do."
  (require (method: args) "what this must return.")
  (provide (derived: args) [body using self + requires]))

; conformances AT THE BOTTOM, sorted alphabetically:
(conform Type1 Name)
(conform Type2 Name)
(conform Type3 Name)
```

no mixing of protocol declaration with unrelated method installations.
no ad-hoc `[Proto handle: sym with: f]` in protocol files. if a
protocol-file touches something other than the protocol, it's in the
wrong file.

---

## the jubilee verdict, condensed

applying this doctrine to current lib/:

| file | verdict |
|------|---------|
| `kernel/bootstrap.moof` | keep |
| `kernel/protocols.moof` | keep — the defprotocol/conform/provide vaus |
| `kernel/identity.moof` | keep — typeName, prototypes |
| `kernel/types.moof` | keep |
| `kernel/error.moof` | consolidate with `data/option.moof` and result flow |
| `kernel/showable.moof` | keep |
| `kernel/equatable.moof` | keep, widen conformances |
| `kernel/url.moof` | keep |
| `kernel/namespace.moof` | **move to `system/`** — not kernel-foundational |
| `data/comparable.moof` | keep, widen |
| `data/numeric.moof` | keep |
| `data/iterable.moof` | keep |
| `data/indexable.moof` | keep |
| `data/callable.moof` | keep |
| `data/range.moof` | keep, ensure conforms Iterable |
| `data/act.moof` | **move to `kernel/` (or new `effects/`)**, keep Thenable fused, fix Cons's `map:` default |
| `data/option.moof` | keep |
| `data/builder.moof` | **delete the protocol**; Builder type probably deletable too |
| `data/set.moof` | keep |
| `data/bag.moof` | keep |
| `data/hashable.moof` | keep |
| `data/reference.moof` | **delete** |
| `flow/transducer.moof` | keep as the pipeline primitive |
| `flow/stream.moof` | keep |
| `flow/reactive.moof` | keep |
| `flow/pattern.moof` | **fix match-constructor or delete constructor-patterns** |
| `tools/inspect.moof` | keep, drop "later wave wires to canvas" comment |
| `tools/query.moof` | **decide: delete or rewrite as transducer sugar**. fix `any` |
| `tools/workspace.moof` | keep |
| `tools/server.moof` | keep (the defserver vau) |
| `tools/test.moof` | keep, expand later |
| `system/system.moof` | rewrite after wave 10; delete Interface protocol |
| `system/registry.moof` | keep as-is for wave 9.4; replace with defserver when wave 10 lands |
| `system/services.moof` | keep; wave 9.5 wires spawners |
| `system/source.moof` | check, may be dead |
| `bin/eval.moof` | rename dir: `lib/entry/` or inline into system |

---

## the contract

this doctrine IS the stdlib's constitution. PRs that add to lib/ get
reviewed against it. PRs that violate (new protocol with <3 conformers,
new operation duplicating an existing one, comment apologizing for wave
debt) are rejected on principle, not opinion.

the cost of saying no is cheap. the cost of saying yes to every idea is
how we got here.
