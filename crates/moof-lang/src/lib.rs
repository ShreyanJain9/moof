// moof-lang — parsing, compilation, and bytecode execution.
//
// Depends only on moof-core (Value, Heap, HeapObject, dispatch).
// No scheduler, no vats, no plugins. Feed it source text and a
// heap, get bytecode back; feed it bytecode and a heap, get a
// Value back.
//
// Layering:
//   moof-core  → types + heap + foreign-type registry
//   moof-lang  ← (this) → parser + compiler + VM
//   moof-runtime → vats, scheduler, manifest, store
//   moof-stdlib  → built-in type plugins (Plugin impls)
//   moof-caps    → built-in capability plugins
//   moof-cli     → binary that wires everything together

pub mod opcodes;
pub mod vm;
pub mod lang;

pub use opcodes::{Chunk, Op};
pub use vm::{VM, RunResult};
pub use lang::compiler::{Compiler, ClosureDesc, CompileResult};
