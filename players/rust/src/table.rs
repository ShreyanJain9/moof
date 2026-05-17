//! the Table proto's internal representation.
//!
//! per `docs/concepts/tables.md`, a Table has two simultaneously
//! available content axes:
//!
//! - **positional**: ordered, integer-indexed (0-based).
//! - **keyed**: arbitrary-keyed.
//!
//! we hold them in a single rust struct, owned by a
//! `ForeignHandle` slotted on the Table Form under `:rep`. moof
//! native methods on Table read/write through the handle. all of
//! Table's slots are *substrate-internal* — user code interacts via
//! the Method protocol (`:at:`, `:atPut:`, `:push:`, etc.).
//!
//! this is the ForeignHandle pattern (see
//! `docs/concepts/compiled-objects.md`'s "state stays in moof"
//! rule) — the rust state lives behind a tagged opaque pointer in
//! a moof slot, and reflection still sees the slot.
//!
//! per `docs/laws/determinism-laws.md` D5, iteration is *insertion
//! order* both for the positional vector (trivial — Vec is ordered)
//! and the keyed map (we use `IndexMap`, which preserves insertion
//! order across all operations).

use indexmap::IndexMap;

use crate::value::Value;

/// the rust-side payload owned by a Table form's `:rep`
/// ForeignHandle.
pub struct TableRepr {
    pub positional: Vec<Value>,
    pub keyed: IndexMap<Value, Value>,
}

impl TableRepr {
    pub fn new() -> Self {
        TableRepr {
            positional: Vec::new(),
            keyed: IndexMap::new(),
        }
    }

    /// total entries (positional + keyed).
    pub fn size(&self) -> usize {
        self.positional.len() + self.keyed.len()
    }
}

impl Default for TableRepr {
    fn default() -> Self {
        Self::new()
    }
}

/// destructor for a `Box<TableRepr>` minted by `make_table`. runs
/// when the gc collects the holding Table form (or when the slot
/// is overwritten with another value).
pub unsafe extern "C" fn table_repr_dtor(ptr: *mut std::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was minted by `Box::into_raw(Box<TableRepr>)` in
    // `World::make_table`. consume it back into a Box and let it
    // drop.
    let _ = unsafe { Box::from_raw(ptr as *mut TableRepr) };
}
