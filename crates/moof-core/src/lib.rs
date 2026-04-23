// moof-core — the plugin-facing core of moof.
//
// If you're writing an external type plugin (a cdylib that
// registers a ForeignType + prototype), this is the ONLY crate
// you need to depend on. It contains:
//
//   - Value (NaN-boxed runtime tag)
//   - HeapObject, Heap (allocation, GC, env bindings, image IO)
//   - ForeignType trait + registry (Ruby-style rust-value wrapping)
//   - Plugin trait + native() handler-registration helper
//   - dispatch (handler chain walk)
//
// It does NOT contain the parser, compiler, VM, scheduler, or
// built-in stdlib/capabilities. Those live in sibling crates
// (moof-lang, moof-runtime, moof-stdlib, moof-caps) and only
// moof-cli pulls the whole stack together into a working binary.

pub mod value;
pub mod object;
pub mod foreign;
pub mod heap;
pub mod dispatch;
pub mod plugin;
pub mod source;
pub mod canonical;

pub use value::Value;
pub use heap::Heap;
pub use object::HeapObject;
pub use foreign::{ForeignType, ForeignData, ForeignTypeId, ForeignTypeName, ForeignVTable, ForeignTypeRegistry};
pub use plugin::{Plugin, native, int_binop, float_binop, float_unary, fnv1a_64, register_foreign_proto};
pub use source::{ClosureSource, SourceOrigin, split_top_level_forms};
pub use canonical::{Hash, hash_hex, cycle_placeholder, cycle_placeholder_blob_bytes};
