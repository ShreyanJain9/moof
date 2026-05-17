//! rust-side safe wrappers over `moof-abi`.
//!
//! mcos written in rust depend on this crate; the substrate also
//! uses it to define native methods on protos that ship in the
//! seed (for instance the rust-backed reflection methods on
//! `Object`).
//!
//! phase A is bare — `MoofValue` and `MoofResult` re-exports plus
//! a placeholder. the proper safe wrappers (typed slot accessors,
//! the `moof_object!` macro for declaring an mco's proto, etc.)
//! land in phase A.9 alongside the mco loader.

pub use moof_abi::*;
