//! moof-fabric: the objectspace substrate.
//!
//! knows about five things:
//! 1. values — nil, true, false, integers, floats, symbols, object references
//! 2. objects — parent + slots + handlers, stored in LMDB
//! 3. send — handler lookup, delegation, doesNotUnderstand
//! 4. vats — capability domains with mailboxes
//! 5. persistence — LMDB is the persistence layer
//!
//! does NOT know about: bytecode, ASTs, environments, closures,
//! s-expressions, modules, source code.

pub mod dispatch;
pub mod store;
pub mod value;
pub mod vat;

pub use dispatch::{HandlerInvoker, InvokeContext, NativeInvoker};
pub use store::{HeapObject, Store};
pub use value::Value;
pub use vat::{Message, Scheduler, Vat};
