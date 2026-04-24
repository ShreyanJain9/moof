# wave 6 triage board

batch-1 ownership and priority map for `plans/wave-6-core-contract.md`.

## legend

- **priority**
  - `must-fix`: required for wave-6 semantic gates
  - `acceptable-debt`: safe to carry through wave-6 with explicit docs
  - `defer`: intentionally postponed to later wave
- **status**
  - `todo`, `in-progress`, `done`, `defer`

## board

| area | item | priority | batch | owner | status |
|---|---|---|---|---|---|
| runtime/vm | classify opcode set (active/deprecated/dead) | must-fix | 1 | wave-6 | done |
| runtime/vm | ensure deprecated opcodes are un-emittable | must-fix | 2 | wave-6 | done |
| runtime/vm | send arg packing limit contract (doc or generalize) | acceptable-debt | 2 | wave-6 | done |
| runtime/vm | dispatch cache + DNU contract verification | must-fix | 2 | wave-6 | done |
| runtime/vm | `proto` visibility constraints explicitly documented | must-fix | 2 | wave-6 | done |
| runtime/vm | env semantics (`parent` + `bindings`) cleanup/confirm | must-fix | 2 | wave-6 | done |
| runtime/vm | Act resolution + Update merge/apply contract in scheduler | must-fix | 2 | wave-6 | done |
| runtime/vm | local-eval vs cross-vat boundary contract in `vat.rs` | must-fix | 2 | wave-6 | done |
| frontend | compiler legacy-form path cleanup (`while`, `:=`, try/catch) | must-fix | 2 | wave-6 | done |
| frontend | parser dead-branch cleanup (`:=` keyword path) | must-fix | 2 | wave-6 | done |
| frontend | lexer/parser/compiler expectation consistency check | must-fix | 2 | wave-6 | done |
| plugins | capability side-effect surface audit | must-fix | 2 | wave-6 | done |
| plugins | dynload ABI trust boundary docs | must-fix | 1 | wave-6 | done |
| plugins | manifest-first plugin contract explicitness | must-fix | 2 | wave-6 | done |
| persistence | store limits contract (`map_size`, metadata assumptions) | acceptable-debt | 2 | wave-6 | done |
| persistence | repl load/save/gc contract consistency | acceptable-debt | 2 | wave-6 | done |
| persistence | manifest defaults vs contract (`moof.toml`) | acceptable-debt | 2 | wave-6 | done |
| docs/tests | language doc stale-claim cleanup | must-fix | 3 | wave-6 | done |
| docs/tests | errors doc stale-claim cleanup | must-fix | 3 | wave-6 | done |
| docs/tests | protocols/types doc reality sync | must-fix | 3 | wave-6 | done |
| docs/tests | effects-and-vats runtime reconciliation | must-fix | 1 | wave-6 | done |
| docs/tests | moof suite contract tests | must-fix | 3 | wave-6 | done |
| docs/tests | rust contract tests (deprecated path rejection) | must-fix | 3 | wave-6 | done |

## batch-1 kickoff deliverables

- [x] Created triage board with explicit owner/priority/status for every wave-6 checklist item.
- [x] Added core contract feature matrix doc (see `docs/core-contract-matrix.md`).
- [x] Started docs status reconciliation (`docs/README.md`, `docs/effects-and-vats.md`).
- [x] Removed dead parser `:=` keyword branch and added VM tests asserting `TryCatch`/`Throw` rejection.

## batch-2/3 completion status

- [x] All docs stale-claim cleanup done (`language.md`, `errors.md`, `protocols.md`, `types.md`).
- [x] Contract enforcement comments added in `src/vm.rs`.
- [x] Compiler non-emission test added in `src/lang/compiler.rs`.
- [x] All 49 rust tests pass.
