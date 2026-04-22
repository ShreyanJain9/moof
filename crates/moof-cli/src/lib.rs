// moof — a living objectspace
//
// Library interface for building on the moof runtime. External
// type-plugin authors should depend on `moof-core` instead of
// this crate; `moof` pulls in the full stack (parser, compiler,
// VM, scheduler, stdlib, capabilities) for the binary and for
// capability-plugin authors who need `Vat` access.

pub mod store;
pub mod plugins;
pub mod vat;
pub mod scheduler;
pub mod manifest;

// re-export moof-core + moof-lang for convenience.
pub use moof_core::{Value, Heap, HeapObject, Plugin};
pub use moof_core::foreign;
pub use moof_core::object;
pub mod heap { pub use moof_core::heap::*; }
pub mod value { pub use moof_core::value::*; }
pub mod dispatch { pub use moof_core::dispatch::*; }
pub mod lang { pub use moof_lang::lang::*; }
pub mod vm { pub use moof_lang::vm::*; }
pub mod opcodes { pub use moof_lang::opcodes::*; }

pub use vat::Vat;
pub use scheduler::Scheduler;
pub use plugins::CapabilityPlugin;
