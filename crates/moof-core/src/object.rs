// The one semantic type: Object.
//
// The VM has optimized internal representations for common shapes,
// but semantically everything is an object that responds to messages.
//
// Prototype delegation is a VM-internal mechanism: every HeapObject
// has a `proto` field used only for message-dispatch chain walking.
// It's NOT a slot. It doesn't appear in slotNames. It can't be read
// via slotAt:. If userland wants a chain-walk concept (e.g.
// Environments pointing at outer scopes for variable lookup), they
// put that on a real slot — `bindings`, `outer`, whatever — and the
// semantics are defined by the type, not by the VM.

use std::cell::Cell;
use crate::value::Value;
use crate::foreign::ForeignData;

/// A prototype pointer (VM-internal, for dispatch) plus named slots,
/// handlers, and an optional foreign payload (Ruby-style rust
/// wrapping). Pair, Text, Bytes, Table, Vec3, and every user-plugin
/// type flow through this — the only distinction is whether `foreign`
/// is None (plain moof object) or Some(…) (rust-backed, with vtable
/// for GC / serialize / cross-vat / virtual slots). Foreign payloads
/// are immutable.
///
/// Merkle cache (Stage A of the merkle-DAG plan):
///   `cached_hash` is the memoized canonical content hash of this
///   object, including its transitive children. `child_fingerprint`
///   is blake3 over the concatenation of children's cached hashes at
///   the time `cached_hash` was computed. At save time, we recompute
///   children's hashes (they're cached too); if the new fingerprint
///   matches the stored one, the parent's cache is still valid even
///   though we never propagated mutation upward. Both fields cleared
///   on every mutation. `Cell` so the hashing pass can write them
///   through `&Heap` (no `&mut` needed).
///
/// Head/Content distinction (Stage B-2.1):
///   `is_head` marks an object as a mutable identity-typed "head" —
///   the legitimate mutation surface (root env, type-proto extension
///   points, mailboxes, etc.). Content objects (is_head=false) are
///   semantically immutable; their canonical hash IS their identity,
///   they're shareable between vats and between save snapshots, and
///   any in-place mutation is a violation of the model (only
///   tolerated during boot / pre-classification cleanup).
///   MOOF_DETECT_HEAD_VIOLATIONS=1 logs each content mutation site so
///   we can convert them to head-mutations or COW patterns.
#[derive(Debug)]
pub struct HeapObject {
    pub proto: Value,                   // VM-internal dispatch pointer (NOT a slot)
    pub slot_names: Vec<u32>,
    pub slot_values: Vec<Value>,
    pub handlers: Vec<(u32, Value)>,
    pub foreign: Option<ForeignData>,
    pub cached_hash: Cell<Option<[u8; 32]>>,
    pub child_fingerprint: Cell<Option<[u8; 32]>>,
    /// Head bit — mutable identity-typed object vs immutable
    /// content. See struct doc.
    pub is_head: bool,
}

impl Clone for HeapObject {
    fn clone(&self) -> Self {
        HeapObject {
            proto: self.proto,
            slot_names: self.slot_names.clone(),
            slot_values: self.slot_values.clone(),
            handlers: self.handlers.clone(),
            foreign: self.foreign.clone(),
            cached_hash: Cell::new(self.cached_hash.get()),
            child_fingerprint: Cell::new(self.child_fingerprint.get()),
            is_head: self.is_head,
        }
    }
}

impl HeapObject {
    pub fn new_general(proto: Value, slot_names: Vec<u32>, slot_values: Vec<Value>) -> Self {
        debug_assert_eq!(slot_names.len(), slot_values.len());
        HeapObject {
            proto,
            slot_names,
            slot_values,
            handlers: Vec::new(),
            foreign: None,
            cached_hash: Cell::new(None),
            child_fingerprint: Cell::new(None),
            is_head: false,
        }
    }

    pub fn new_empty(proto: Value) -> Self {
        HeapObject {
            proto,
            slot_names: Vec::new(),
            slot_values: Vec::new(),
            handlers: Vec::new(),
            foreign: None,
            cached_hash: Cell::new(None),
            child_fingerprint: Cell::new(None),
            is_head: false,
        }
    }

    pub fn new_foreign(proto: Value, foreign: ForeignData) -> Self {
        HeapObject {
            proto,
            slot_names: Vec::new(),
            slot_values: Vec::new(),
            handlers: Vec::new(),
            foreign: Some(foreign),
            cached_hash: Cell::new(None),
            child_fingerprint: Cell::new(None),
            is_head: false,
        }
    }

    /// Promote this object to a head — mutable identity-typed object.
    /// Used at allocation time for known-mutable surfaces (root env,
    /// type protos that accept post-boot handler extension, mailboxes).
    /// Once set, the object is exempt from the "content is immutable"
    /// invariant: get_mut won't warn, the merkle cache won't try to
    /// hash it as content.
    pub fn into_head(mut self) -> Self {
        self.is_head = true;
        self
    }

    /// Mark this object's content hash as needing recomputation. Any
    /// mutation of `proto`, slots, handlers, or foreign payload should
    /// call this. The corresponding `Arena::get_mut` does it
    /// automatically — direct field writes outside that path must call
    /// it themselves.
    #[inline]
    pub fn invalidate_hash(&self) {
        self.cached_hash.set(None);
        self.child_fingerprint.set(None);
    }

    pub fn foreign(&self) -> Option<&ForeignData> {
        self.foreign.as_ref()
    }

    /// The VM-internal prototype used for dispatch's chain walk. Not a
    /// slot — this is the language's delegation machinery.
    pub fn proto(&self) -> Value {
        self.proto
    }

    pub fn set_proto(&mut self, p: Value) {
        self.proto = p;
    }

    /// Look up a slot value by name (symbol ID). Note: this only
    /// walks the real slots vec — foreign virtual slots (e.g. a
    /// Pair's car/cdr) are handled by `Heap::slot_of`.
    pub fn slot_get(&self, name: u32) -> Option<Value> {
        self.slot_names.iter().position(|n| *n == name).map(|i| self.slot_values[i])
    }

    /// Set a slot value by name. Grows the object if the slot doesn't exist.
    pub fn slot_set(&mut self, name: u32, val: Value) -> bool {
        if let Some(i) = self.slot_names.iter().position(|n| *n == name) {
            self.slot_values[i] = val;
        } else {
            self.slot_names.push(name);
            self.slot_values.push(val);
        }
        true
    }

    /// Remove a slot by name. No-op for missing slots.
    pub fn slot_remove(&mut self, name: u32) {
        if let Some(i) = self.slot_names.iter().position(|n| *n == name) {
            self.slot_names.remove(i);
            self.slot_values.remove(i);
        }
    }

    /// Get the explicit slot names (not including the proto — that's
    /// VM-internal, not a slot).
    pub fn slot_names(&self) -> Vec<u32> {
        self.slot_names.clone()
    }

    /// Look up a handler by selector (symbol ID).
    pub fn handler_get(&self, selector: u32) -> Option<Value> {
        self.handlers.iter().find(|(s, _)| *s == selector).map(|(_, v)| *v)
    }

    /// Set (or add) a handler. Handlers are open — always succeeds.
    pub fn handler_set(&mut self, selector: u32, handler: Value) {
        if let Some(entry) = self.handlers.iter_mut().find(|(s, _)| *s == selector) {
            entry.1 = handler;
        } else {
            self.handlers.push((selector, handler));
        }
    }

    /// Get all handler names (for introspection).
    pub fn handler_names(&self) -> Vec<u32> {
        self.handlers.iter().map(|(s, _)| *s).collect()
    }
}
