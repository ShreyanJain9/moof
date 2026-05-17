//! foreign handles — opaque rust-side resources held in moof slots.
//!
//! when an mco's native method needs to keep state across calls
//! (an open lmdb env, a wgpu device, a websocket fd), it stores
//! that state as a `ForeignHandle` *value in a slot* on the
//! receiver Form. nothing is hidden: `[obj slots]` lists the
//! handle slot like any other.
//!
//! a `ForeignHandle` carries:
//!
//! - an opaque pointer (the rust-allocated state).
//! - an optional destructor (called by the gc when the holding
//!   form is collected, or when the slot is overwritten).
//! - a `tag` identifying the *kind* of foreign state (a small
//!   safety check: an mco verifies the tag before casting).
//!
//! per `laws/determinism-laws.md` D6, destructors run at *turn
//! boundaries* in replicated vats — never mid-turn, so observable
//! state is unchanged within a turn.
//!
//! per `laws/isolation-laws.md` I3, foreign handles **cannot
//! cross vat boundaries**. the substrate's serialization layer
//! refuses; sending a Form with a `ForeignHandle` slot to another
//! vat raises an error.
//!
//! foreign handles **cannot serialize** to disk either; on
//! snapshot they appear as "broken" placeholders. on load, the
//! owning cap re-opens the resource. this is the standard
//! BEAM NIF resource pattern.

use std::ffi::c_void;

// ─────────────────────────────────────────────────────────────────
// substrate-internal foreign-handle tags.
//
// every foreign handle carries a u32 `tag` identifying its kind.
// mco-supplied tags live in their respective mcos (via the abi
// crate). substrate-supplied tags live here, in one place, so
// world.rs and reader.rs both reference *one* canonical literal
// rather than duplicating it (an earlier `// must agree with
// world.rs's constant` comment in reader.rs admitted the drift).
//
// the four-byte ascii encoding is purely a debugging aid: when
// you `od` a heap dump, the tag reads as text. semantically it's
// just an opaque u32.
// ─────────────────────────────────────────────────────────────────

/// the `:bytes` foreign-handle on a String form.
/// payload: `Box<Vec<u8>>` of the utf-8 bytes.
pub const TAG_STRING_BYTES: u32 = u32::from_be_bytes(*b"WRGU");

/// the `:bytes` foreign-handle on a Bytes form.
/// payload: `Box<Vec<u8>>` — raw byte buffer, no utf-8 invariant.
pub const TAG_BYTES: u32 = u32::from_be_bytes(*b"BYTA");

/// the `:rep` foreign-handle on a Table form.
/// payload: `Box<TableRepr>`.
pub const TAG_TABLE_REPR: u32 = u32::from_be_bytes(*b"TBLE");

/// a vat-local index into the foreign-handle table.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ForeignId(pub u32);

impl ForeignId {
    pub const NONE: ForeignId = ForeignId(0);
    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

/// a single foreign handle entry.
///
/// note: this struct is *only* held inside `ForeignTable`. user
/// code never touches it directly; native methods get the `ptr`
/// via `MoofContext` accessor functions.
pub struct ForeignHandle {
    /// the opaque rust-allocated state.
    ///
    /// `null` indicates a "broken" handle (set when the resource
    /// has been freed or when loading a serialized snapshot).
    pub ptr: *mut c_void,

    /// the destructor, called when the holding slot is overwritten
    /// or when the form is gc'd.
    ///
    /// `None` for handles that don't own their resource (e.g., the
    /// stdout fd, which is owned by the OS).
    pub destructor: Option<unsafe extern "C" fn(*mut c_void)>,

    /// kind tag — small u32 chosen by the mco that minted this
    /// handle. on read, the mco verifies the tag matches what it
    /// expects before casting `ptr` to its concrete type.
    ///
    /// tag = 0 means "broken handle."
    pub tag: u32,
}

// SAFETY: a `ForeignHandle` carries a raw pointer. the substrate
// runs a vat single-threaded; nothing in moof's threading model
// permits a `ForeignHandle` to be observed concurrently. when we
// later add a multi-vat scheduler, foreign handles will be moved,
// not shared, across thread boundaries.
unsafe impl Send for ForeignHandle {}

/// the per-vat table of foreign handles.
///
/// indexed by `ForeignId`. id zero is reserved as the broken-handle
/// sentinel.
pub struct ForeignTable {
    handles: Vec<ForeignHandle>,
    /// indices of slots whose handles have been freed and can be
    /// reused. populated by `release()`. phase-A note: gc is not
    /// implemented yet, so this stays empty unless a slot is
    /// explicitly overwritten.
    free_list: Vec<usize>,
}

impl ForeignTable {
    pub fn new() -> Self {
        // index zero is the broken-handle sentinel.
        ForeignTable {
            handles: vec![ForeignHandle {
                ptr: std::ptr::null_mut(),
                destructor: None,
                tag: 0,
            }],
            free_list: Vec::new(),
        }
    }

    /// allocate a new handle entry.
    pub fn alloc(&mut self, handle: ForeignHandle) -> ForeignId {
        if let Some(idx) = self.free_list.pop() {
            self.handles[idx] = handle;
            ForeignId(idx as u32)
        } else {
            let id = ForeignId(self.handles.len() as u32);
            self.handles.push(handle);
            id
        }
    }

    /// borrow a handle for inspection.
    pub fn get(&self, id: ForeignId) -> &ForeignHandle {
        debug_assert!(!id.is_none(), "get() on ForeignId::NONE");
        &self.handles[id.0 as usize]
    }

    /// release a handle: invokes its destructor (if any), zeros the
    /// entry, and marks the slot for reuse.
    ///
    /// safe to call on the same id twice; the second call is a
    /// no-op because the slot's tag is now zero.
    pub fn release(&mut self, id: ForeignId) {
        if id.is_none() {
            return;
        }
        let entry = &mut self.handles[id.0 as usize];
        if entry.tag == 0 {
            // already broken / released.
            return;
        }
        if let Some(d) = entry.destructor {
            // SAFETY: the destructor was supplied by the mco that
            // minted this handle and was promised to safely consume
            // a `ptr` cast back to the mco's concrete type. that
            // promise is part of the substrate native abi
            // (`docs/concepts/compiled-objects.md`).
            unsafe { d(entry.ptr) };
        }
        entry.ptr = std::ptr::null_mut();
        entry.destructor = None;
        entry.tag = 0;
        self.free_list.push(id.0 as usize);
    }

    /// number of live handles (excluding the broken sentinel).
    pub fn len(&self) -> usize {
        self.handles.len() - 1 - self.free_list.len()
    }
}

impl Default for ForeignTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ForeignTable {
    fn drop(&mut self) {
        // on table teardown, run destructors for any still-live
        // handles. this happens at vat shutdown.
        for id in 1..self.handles.len() {
            self.release(ForeignId(id as u32));
        }
    }
}
