# moof documentation

reference documentation for the moof language and runtime.

## guides

- [language](language.md) — syntax, special forms, evaluation model
- [types](types.md) — every type and its handlers
- [protocols](protocols.md) — the protocol system and standard protocols
- [errors](errors.md) — error handling with try/catch
- [conventions](conventions.md) — how we name things and write docs

## design documents (in project root)

- [VISION.md](../VISION.md) — the full v2 design
- [STDLIB-PLAN.md](../STDLIB-PLAN.md) — stdlib design spec
- [SYNTHESIS.md](../SYNTHESIS.md) — v1 post-mortem

## current status

the core language works: bytecode VM, message dispatch, prototype
delegation, closures, vau operatives, LMDB persistence.

implemented: 5 protocols (Iterable, Comparable, Numeric, Callable,
Indexable), error handling (try/catch), Range type, showable display.

not yet implemented: vats (concurrency), streams, sets, capabilities
(Console/Clock/Random), canvas, agent, federation.
