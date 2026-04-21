# core contract matrix

wave-6 runtime truth table: what is active, deprecated, and intentionally dead.

## language/runtime feature status (apr 2026)

| feature | status | notes |
|---|---|---|
| object/message core (`send`, `call:` lowering) | active | compiler + vm hot path |
| `vau` + operative-based forms | active | kernel/library level abstractions rely on this |
| `do` sequencing + Act-aware chaining | active | used in stdlib/tests |
| Result/Err value-flow for failures | active | primary error path |
| cross-vat send -> `Act` | active | scheduler + effects pipeline |
| server state deltas via `Update` | active | scheduler applies delta/reply semantics |
| capability vats (console/clock/file/random) | active | manifest + builtin capability plugins |
| `while` special form | deprecated/removed | compiler comments indicate removal; recursion preferred |
| `:=` mutation special form | deprecated/removed | parser still has legacy symbol branch, but form is removed semantically |
| `try/catch` and VM throw-style exception opcodes | deprecated/removed | opcodes exist for compatibility but VM rejects them |

## opcode status matrix

`src/opcodes.rs` is superset metadata; compiler/vm reality determines active status.

| opcode | compiler emits | VM handles | status |
|---|---:|---:|---|
| `LoadConst` | yes | yes | active |
| `LoadNil` | yes | yes | active |
| `LoadTrue` | yes | yes | active |
| `LoadFalse` | yes | yes | active |
| `Move` | yes | yes | active |
| `Send` | yes | yes | active |
| `TailCall` | yes (peephole) | yes | active |
| `Return` | yes | yes | active |
| `Jump` | yes | yes | active |
| `JumpIfFalse` | yes | yes | active |
| `Cons` | yes | yes | active |
| `Eq` | yes | yes | active |
| `MakeObj` | yes | yes | active |
| `SetHandler` | yes | yes | active |
| `MakeClosure` | yes | yes | active |
| `LoadInt` | yes | yes | active |
| `MakeTable` | yes | yes | active |
| `GetGlobal` | yes | yes | active |
| `DefGlobal` | yes | yes | active |
| `Eval` | yes | yes | active |
| `SetSlot` | no (current compiler path) | yes | compatibility/debt |
| `Call` | no (compiler lowers to `Send ... call:`) | yes | compatibility/debt |
| `JumpIfTrue` | no (current compiler path) | yes | compatibility/debt |
| `GetLocal` | no | no explicit VM arm | dead/legacy |
| `SetLocal` | no | no explicit VM arm | dead/legacy |
| `GetUpval` | no | no explicit VM arm | dead/legacy |
| `TryCatch` | no | explicit rejection | deprecated/removed |
| `Throw` | no | explicit rejection | deprecated/removed |

## immediate wave-6 implications

1. **must-fix:** document and gate deprecated opcodes (`TryCatch`, `Throw`) as non-emittable.
2. **must-fix:** remove stale language docs that still present removed forms as valid.
3. **must-fix:** decide retention policy for compatibility/debt opcodes (`Call`, `SetSlot`, `JumpIfTrue`).
4. **acceptable-debt:** `Send` arg packing limit remains, but must be explicitly documented as contract until changed.

See also:
- [`plans/wave-6-core-contract.md`](../plans/wave-6-core-contract.md)
- [`plans/wave-6-triage.md`](../plans/wave-6-triage.md)
