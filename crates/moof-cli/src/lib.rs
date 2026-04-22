// moof — a living objectspace
//
// Library interface for building on the moof runtime. External
// type-plugin authors should depend on `moof-core` instead of
// this crate; `moof` pulls in the full stack (parser, compiler,
// VM, scheduler, stdlib, capabilities) for the binary and for
// capability-plugin authors who need `Vat` access.

pub mod opcodes;
pub mod vm;
pub mod store;
pub mod plugins;
pub mod vat;
pub mod scheduler;
pub mod manifest;
pub mod lang;

// re-export moof-core for convenience: `use moof::{Value, Heap, Plugin};`
pub use moof_core::{Value, Heap, HeapObject, Plugin};
pub use moof_core::foreign;
pub use moof_core::object;
// legacy module aliases — heap/value/dispatch live in moof-core now,
// but existing `moof::heap::X` references keep working via re-export.
pub mod heap { pub use moof_core::heap::*; }
pub mod value { pub use moof_core::value::*; }
pub mod dispatch { pub use moof_core::dispatch::*; }

pub use vat::Vat;
pub use scheduler::Scheduler;
pub use plugins::CapabilityPlugin;
