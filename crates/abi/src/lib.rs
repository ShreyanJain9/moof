//! raw C ABI for mcos.
//!
//! this crate is the *stable* boundary between the substrate seed
//! and any rust mco that wants to bind native methods. mcos compile
//! against `moof-abi`; the substrate links against `moof-abi`; if
//! the substrate's internal types change, the abi version is bumped
//! and old mcos are recompiled.
//!
//! we are deliberately small here. the seed itself depends on
//! `moof-abi-rust` (the safe wrapper crate) for ergonomic access
//! during phase A; once mcos ship, both substrate and mco talk
//! through this raw layer.
//!
//! see `docs/concepts/compiled-objects.md` for the model.
//! see `docs/reference/native-abi.md` (when written) for the
//! complete signature list.

#![no_std]

/// the abi version this crate represents. mcos record the abi
/// version they were built against; loaders refuse mismatches.
///
/// bumped on any breaking change to the C ABI surface.
pub const MOOF_ABI_VERSION: u32 = 1;

/// every native method's return code.
#[repr(i32)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum MoofResult {
    /// the method completed; `*out` carries the return value.
    Ok = 0,
    /// the method raised an error; the substrate will surface it.
    Raised = 1,
    /// the method's args didn't match its declared shape.
    ArgsMismatch = 2,
    /// a slot lookup failed.
    SlotMissing = 3,
    /// a foreign handle was the wrong type.
    BadForeignTag = 4,
    /// an out-of-memory or substrate-resource error.
    SubstrateError = 5,
}

/// the opaque substrate handle passed to every native method.
///
/// the substrate's actual type is private to `moof` (the seed
/// binary). mco code interacts with `MoofContext *` only through
/// the function pointers in `MoofVTable` (defined in
/// `moof-abi-rust`).
#[repr(C)]
pub struct MoofContext {
    _opaque: [u8; 0],
    _phantom: core::marker::PhantomData<*mut ()>,
}

/// a moof Value, in the C ABI.
///
/// reproduces `substrate::value::Value` discriminator-and-payload
/// shape. NaN-boxing is *not* used in the abi — this is the honest
/// tagged union. the seed may internally use a packed
/// representation.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct MoofValue {
    pub tag: MoofValueTag,
    pub payload: MoofValuePayload,
}

#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum MoofValueTag {
    Nil = 0,
    Bool = 1,
    Int = 2,
    Sym = 3,
    Form = 4,
    Foreign = 5,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union MoofValuePayload {
    /// unused for Nil
    pub nil: u64,
    pub b: bool,
    pub i: i64,
    pub sym: u32,
    pub form: u32,
    pub foreign: u32,
}

/// the signature every native method exposes.
///
/// ```ignore
/// pub type MoofMethod = unsafe extern "C" fn(
///     ctx: *mut MoofContext,
///     self_: MoofValue,
///     args: *const MoofValue,
///     argc: usize,
///     out: *mut MoofValue,
/// ) -> MoofResult;
/// ```
pub type MoofMethod = unsafe extern "C" fn(
    ctx: *mut MoofContext,
    self_: MoofValue,
    args: *const MoofValue,
    argc: usize,
    out: *mut MoofValue,
) -> MoofResult;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_stable() {
        // these sizes are part of the abi contract. if any of them
        // changes, MOOF_ABI_VERSION must be bumped.
        assert_eq!(core::mem::size_of::<MoofValueTag>(), 1);
        // payload is at least 8 bytes (Int variant); whole struct
        // gets aligned/padded by repr(C).
        assert!(core::mem::size_of::<MoofValue>() >= 9);
        assert_eq!(MOOF_ABI_VERSION, 1);
    }
}
