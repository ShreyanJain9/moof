// moof — a living objectspace
//
// Library interface for building on the moof runtime. External
// type-plugin authors should depend on `moof-core` instead of
// this crate; `moof` pulls in the full stack (parser, compiler,
// VM, scheduler, stdlib, capabilities) for the binary and for
// capability-plugin authors who need `Vat` access.

pub mod plugins;
pub mod boot;

// re-export moof-core / moof-lang / moof-runtime at their historical
// module paths so existing `moof::heap::X`, `moof::vm::X`, etc. keep
// working without churn.
pub use moof_core::{Value, Heap, HeapObject, Plugin};
pub use moof_core::foreign;
pub use moof_core::object;
pub mod heap     { pub use moof_core::heap::*; }
pub mod value    { pub use moof_core::value::*; }
pub mod dispatch { pub use moof_core::dispatch::*; }
pub mod lang     { pub use moof_lang::lang::*; }
pub mod vm       { pub use moof_lang::vm::*; }
pub mod opcodes  { pub use moof_lang::opcodes::*; }
pub mod vat      { pub use moof_runtime::vat::*; }
pub mod scheduler{ pub use moof_runtime::scheduler::*; }
pub mod manifest { pub use moof_runtime::manifest::*; }
pub mod store    { pub use moof_runtime::store::*; }

pub use moof_runtime::{Vat, Scheduler, CapabilityPlugin};
