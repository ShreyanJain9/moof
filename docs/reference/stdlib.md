# stdlib

**type:** reference

> how to find, use, and extend moof's standard library. for the
> rulebook that governs what belongs in here, see the
> [doctrine](../laws/stdlib-doctrine.md).

---

## layout

```
lib/
├── kernel/       language core — loaded first
├── data/         value types and their protocols
├── flow/         transformation and pipelines
├── tools/        meta-programming, inspection, servers
├── system/       vat-0 substrate (System, Registry, Services)
└── bin/          entry points (-e flag, script runner)
```

load order is declared in `moof.toml` under `[sources].files`.
kernel → data → flow → tools → system → bin.

---

## kernel — language core

| file | purpose |
|------|---------|
| `bootstrap.moof` | fundamental forms (let, if, fn, defn) |
| `protocols.moof` | defprotocol, conform, require, provide |
| `identity.moof` | typeName, prototypes, shape |
| `types.moof` | type registry, is: |
| `error.moof` | Result (Ok/Err), err fn |
| `showable.moof` | Showable protocol |
| `equatable.moof` | Equatable protocol |
| `url.moof` | URL values + parsers |
| `namespace.moof` | walk:, walkTo: for Table-as-namespace |

these define the core language + the first few protocols.
everything above them depends on them loading first.

---

## data — value types + protocols

| file | provides |
|------|----------|
| `comparable.moof` | Comparable protocol |
| `numeric.moof` | Numeric protocol + Integer/Float methods |
| `iterable.moof` | Iterable protocol (~40 derived methods) |
| `indexable.moof` | Indexable protocol (refines Iterable) |
| `callable.moof` | Callable protocol |
| `range.moof` | Range type |
| `act.moof` | Thenable (to be split), Act methods |
| `option.moof` | Option / Some / None |
| `builder.moof` | (pending deletion per doctrine) |
| `set.moof` | Set type |
| `bag.moof` | Bag (multiset) type |
| `hashable.moof` | Hashable protocol |
| `reference.moof` | File type (Reference protocol was deleted) |

the protocols here are the ones users most directly interact
with. learn `Iterable` first — `fold:with:` unlocks 40 methods.

---

## flow — pipelines

| file | purpose |
|------|---------|
| `transducer.moof` | composable, lazy reducers |
| `stream.moof` | lazy sequences |
| `reactive.moof` | Atom, Signal |
| `pattern.moof` | match destructuring |

**transducers** are the primary pipeline primitive. Query (in
tools/) duplicates them; the doctrine may collapse it.

---

## tools — meta and composition

| file | purpose |
|------|---------|
| `server.moof` | `defserver` vau — declare a server vat |
| `workspace.moof` | Workspace type, block composition |
| `inspect.moof` | Inspector + aspects |
| `query.moof` | Query-builder (eager; pending doctrine review) |
| `test.moof` | test harness |

`server.moof` is the big one. `defserver` is how you spawn a
stateful vat with handlers that return Updates.

---

## system — vat-0 substrate

| file | purpose |
|------|---------|
| `system.moof` | System prototype, service registry passthroughs |
| `registry.moof` | Registry prototype (plain object, wave 9.4) |
| `services.moof` | built-in service declarations |
| `source.moof` | source-code introspection helpers |

today these are wave 9.4 state — static registry. wave 9.6 will
reshape them around a live defserver-based System.

---

## finding what you need

**"how do i X a collection?"**
- Iterable has most things: map:, select:, fold:with:, count, sum,
  first, last, any:, all:, take:, drop:, sort, group:, zip, etc.
- Indexable adds: at:, first, last, empty?, indexOf:
- start with `[lib/data/iterable.moof](../lib/data/iterable.moof)`.

**"how do i handle errors?"**
- Use Result (Ok/Err). `[result recover: f]` handles Err.
- `(do ...)` short-circuits on Err automatically.
- see `lib/kernel/error.moof` and `lib/data/act.moof`.

**"how do i do async?"**
- cross-vat sends return Acts. compose with `(do ...)` or `then:`.
- see `lib/data/act.moof`.

**"how do i make a stateful thing?"**
- `(defserver MyThing (init-args) { slots... [handlers] })`.
- returns a FarRef to a vat. handlers return Updates.
- see `lib/tools/server.moof` and `lib/tools/workspace.moof` for
  an example.

**"how do i work with text?"**
- no Textual protocol yet (owed). today: Strings have methods.
  see native bindings on String.

**"how do i time something?"**
- clock capability: `[clock now]` → timestamp.
- no Duration/Timestamp types yet (owed).

---

## the ten protocols

from the doctrine, in one place:

| protocol | required | file |
|----------|----------|------|
| Showable | `show` | `kernel/showable.moof` |
| Equatable | `equal:` | `kernel/equatable.moof` |
| Hashable | `hash` | `data/hashable.moof` |
| Comparable | `<` | `data/comparable.moof` |
| Numeric | `+ - * = <` | `data/numeric.moof` |
| Iterable | `fold:with:` | `data/iterable.moof` |
| Indexable | `at:`, `count` | `data/indexable.moof` |
| Callable | `call:` | `data/callable.moof` |
| Monadic | `then:`, `pure:` | `data/act.moof` (after split) |
| Fallible | `ok?` | `data/act.moof` (after split) |

an eleventh — Awaitable — will split out of Thenable in the wave-9
jubilee. Reference, Buildable, Interface are being deleted (see
the doctrine's deletion list).

---

## adding to the stdlib

read [the doctrine](../laws/stdlib-doctrine.md) first. the short
version:

1. does the thing fit an existing protocol?
   - yes → add as a `(provide ...)` to that protocol.
2. is it specific to one type?
   - yes → plain `defmethod` on that type.
3. is it a new cross-cutting capability?
   - yes, AND 3+ concrete conformers at declaration time → new
     protocol.
   - fewer conformers → stop. write the specific methods; promote
     to protocol when the pattern recurs.

PRs that violate the doctrine get rejected on principle. better
too few protocols than too many.

---

## the stdlib's known gaps

things a full stdlib should have that moof doesn't yet:

1. **Textual protocol** for unifying String / URL / Bytes-as-utf8.
2. **Time types**: Duration, Timestamp, Instant.
3. **Log protocol** for structured diagnostics.
4. **Runnable test dirs** + better test harness.
5. **Regex or text-pattern matching** (or decide: plugin, not
   stdlib).
6. **Push-based streams** with backpressure.

see the doctrine's "owed" section for more.

---

## extending moof via plugins

for things the stdlib shouldn't own (specialized types like Vec3,
JSON, specific capabilities), see [plugins.md](plugins.md). a
plugin is a rust crate compiled as a cdylib, loaded at runtime,
that registers types or capabilities with moof-core.

plugins exist for:
- types: Vec3, Color, JsonValue, GUI widgets
- capabilities: Console, Clock, File, Random, System, Evaluator

user plugins follow the same ABI.

---

## what you need to know

- stdlib is organized by concern: kernel, data, flow, tools,
  system, bin.
- ten canonical protocols cover most of the surface.
- `Iterable` unlocks the most power per line of conformance.
- adding to stdlib follows the doctrine; PRs are reviewed.
- gaps (time, text, logs) are known; they're on the roadmap.

---

## next

- [../laws/stdlib-doctrine.md](../laws/stdlib-doctrine.md) — the
  full rulebook.
- [../laws/stdlib-at-a-glance.md](../laws/stdlib-at-a-glance.md) —
  the one-pager.
- [plugins.md](plugins.md) — extending moof via rust.
