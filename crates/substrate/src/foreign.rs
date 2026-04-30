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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// helper: build a `ForeignHandle` whose destructor increments
    /// a shared counter when called. lets tests verify that
    /// destructors fire exactly when expected.
    fn counter_handle(counter: &Arc<AtomicU32>, tag: u32) -> ForeignHandle {
        // we leak the Arc into a raw pointer; the destructor
        // reconstructs and increments. exact ownership semantics
        // for tests only.
        let ptr = Arc::into_raw(counter.clone()) as *mut c_void;
        unsafe extern "C" fn destructor(p: *mut c_void) {
            let arc: Arc<AtomicU32> = unsafe { Arc::from_raw(p as *const AtomicU32) };
            arc.fetch_add(1, Ordering::SeqCst);
            // arc drops, releasing one strong ref.
        }
        ForeignHandle {
            ptr,
            destructor: Some(destructor),
            tag,
        }
    }

    #[test]
    fn alloc_returns_distinct_ids() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        let a = t.alloc(counter_handle(&counter, 1));
        let b = t.alloc(counter_handle(&counter, 1));
        assert_ne!(a, b);
        assert!(!a.is_none());
        assert!(!b.is_none());
    }

    #[test]
    fn ids_start_at_one() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        let id = t.alloc(counter_handle(&counter, 1));
        assert_eq!(id.0, 1, "id zero is reserved for the sentinel");
    }

    #[test]
    fn release_invokes_destructor() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        let id = t.alloc(counter_handle(&counter, 7));
        t.release(id);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        // the entry is now broken: tag = 0.
        assert_eq!(t.get(id).tag, 0);
    }

    #[test]
    fn release_is_idempotent() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        let id = t.alloc(counter_handle(&counter, 7));
        t.release(id);
        t.release(id);
        // destructor still ran exactly once.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_runs_remaining_destructors() {
        let counter = Arc::new(AtomicU32::new(0));
        {
            let mut t = ForeignTable::new();
            t.alloc(counter_handle(&counter, 1));
            t.alloc(counter_handle(&counter, 1));
            // table goes out of scope here.
        }
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn release_makes_slot_reusable() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        let id1 = t.alloc(counter_handle(&counter, 1));
        t.release(id1);
        // the next alloc reuses the freed slot.
        let id2 = t.alloc(counter_handle(&counter, 1));
        assert_eq!(id1.0, id2.0, "freed slots are reused");
    }

    #[test]
    fn none_sentinel_is_safe_to_release() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        // releasing the sentinel is a no-op; no destructor to fire.
        t.release(ForeignId::NONE);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn len_tracks_live_handles() {
        let mut t = ForeignTable::new();
        let counter = Arc::new(AtomicU32::new(0));
        assert_eq!(t.len(), 0);
        let a = t.alloc(counter_handle(&counter, 1));
        assert_eq!(t.len(), 1);
        let _b = t.alloc(counter_handle(&counter, 1));
        assert_eq!(t.len(), 2);
        t.release(a);
        assert_eq!(t.len(), 1);
    }
}
