# moof documentation

reference documentation for the moof language and runtime.

## guides

- [language](language.md) — syntax, special forms, evaluation model
- [types](types.md) — every type and its handlers
- [protocols](protocols.md) — the protocol system and standard protocols
- [errors](errors.md) — error/result handling model
- [conventions](conventions.md) — how we name things and write docs
- [effects-and-vats](effects-and-vats.md) — effect boundary model and Act/vat architecture
- [core-contract-matrix](core-contract-matrix.md) — wave-6 active/deprecated feature matrix

## design documents (in project root)

- [VISION.md](../VISION.md) — the full v2 design
- [STDLIB-PLAN.md](../STDLIB-PLAN.md) — stdlib design spec
- [SYNTHESIS.md](../SYNTHESIS.md) — v1 post-mortem

## current status

the core language works: bytecode VM, message dispatch, prototype
delegation, closures, vau operatives, LMDB persistence.

implemented (high level):

- runtime core: vm/heap/dispatch, foreign type registry migration path,
  scheduler + vats, capability plugins, manifest bootstrap
- stdlib surface: protocols + collection/query/reactive primitives,
  Result/Act-oriented flows in current codepaths
- test harness: Act-aware moof suite in `lib/tools/test.moof`

known wave-6 contract drift (in progress):

- some docs still describe removed forms (`while`, `:=`, try/catch)
- opcode enum contains compatibility/debt members not emitted by current compiler
- effects docs include long-term aspirations that exceed current runtime guarantees

long-term work still ahead: fully coherent canvas environment,
agent-as-vat workflow, federation/distribution, and deeper core simplification.
