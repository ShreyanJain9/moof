// moof — a living objectspace
//
// Library interface for building on the moof runtime.
// External code can:
//   - implement Plugin to add type prototypes and handlers
//   - implement CapabilityPlugin to add native capability vats
//   - create a Scheduler and run moof programs
//
// Example:
//   use moof::{CapabilityPlugin, Scheduler, Vat, Value};
//
//   struct MyDatabase;
//   impl CapabilityPlugin for MyDatabase {
//       fn name(&self) -> &str { "db" }
//       fn setup(&self, vat: &mut Vat) -> u32 { ... }
//   }

pub mod value;
pub mod object;
pub mod heap;
pub mod dispatch;
pub mod opcodes;
pub mod vm;
pub mod store;
pub mod plugins;
pub mod vat;
pub mod scheduler;
pub mod manifest;
pub mod lang;

// re-export the most-used types at crate root
pub use value::Value;
pub use heap::Heap;
pub use vat::Vat;
pub use scheduler::Scheduler;
pub use plugins::{Plugin, CapabilityPlugin};
