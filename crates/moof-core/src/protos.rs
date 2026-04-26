// Prototype registry.
//
// Every moof value dispatches through a prototype. Some prototypes
// are per-heap well-known roots (Integer, String, Cons, etc.) —
// they have reserved indices (PROTO_*) so dispatch can look them up
// in O(1) by tag. Others (user-defined, named prototypes) are
// registered by name in the env; those go through `lookup_type`.
//
// ProtoRegistry owns the indexed table. It's a newtype over
// Vec<Value> with a Deref impl so existing call sites that use
// `heap.type_protos[PROTO_INT]` keep working verbatim. The point
// of naming the struct is to have a clear home for PROTO_*
// constants and a place for future methods (e.g. named_slots for
// seed-image serialization in wave 10.1+).

use crate::value::Value;
use std::ops::{Deref, DerefMut};

// Well-known prototype indices. Kept here so plugins have ONE
// import for them. `heap/mod.rs` re-exports these for backward
// compat.

pub const PROTO_NIL: usize = 0;
pub const PROTO_BOOL: usize = 1;
pub const PROTO_INT: usize = 2;
pub const PROTO_FLOAT: usize = 3;
pub const PROTO_SYM: usize = 4;
pub const PROTO_OBJ: usize = 5;
pub const PROTO_CONS: usize = 6;
pub const PROTO_STR: usize = 7;
pub const PROTO_BYTES: usize = 8;
pub const PROTO_TABLE: usize = 9;
pub const PROTO_NUMBER: usize = 10;
pub const PROTO_CLOSURE: usize = 11;
pub const PROTO_ERR: usize = 12;
pub const PROTO_FARREF: usize = 13;
pub const PROTO_ACT: usize = 14;
pub const PROTO_UPDATE: usize = 15;
pub const PROTO_OK: usize = 16;
pub const PROTO_ENV: usize = 17;

/// Total number of indexed prototype slots. Grows as new built-in
/// types land.
pub const NUM_BUILTIN_PROTOS: usize = 18;

/// The prototype registry. A Vec<Value> indexed by PROTO_*
/// constants, with named accessors for eventual
/// serialization-aware operations.
#[derive(Clone)]
pub struct ProtoRegistry {
    entries: Vec<Value>,
}

impl Default for ProtoRegistry {
    fn default() -> Self { Self::new() }
}

impl ProtoRegistry {
    pub fn new() -> Self {
        ProtoRegistry { entries: vec![Value::NIL; NUM_BUILTIN_PROTOS] }
    }

    /// The underlying Vec, for image serialization.
    pub fn as_slice(&self) -> &[Value] { &self.entries }

    /// Total registered slots (always NUM_BUILTIN_PROTOS today;
    /// would grow if we added more).
    pub fn len(&self) -> usize { self.entries.len() }

    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

// Deref impls let `heap.type_protos[PROTO_X]`, `.iter()`,
// `.push()`, `.clear()`, `.get(i)`, etc. all work unchanged on
// the registry. 68+ existing call sites depend on this.
impl Deref for ProtoRegistry {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> { &self.entries }
}

impl DerefMut for ProtoRegistry {
    fn deref_mut(&mut self) -> &mut Vec<Value> { &mut self.entries }
}
