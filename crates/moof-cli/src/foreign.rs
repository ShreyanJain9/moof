// User-extensible type registry — Ruby-style wrapping of rust
// values as moof objects. Every moof value that carries a rust
// payload does so through this machinery. Built-in types (Pair,
// Text, Buffer, Table) will eventually be registered the same way
// external plugins register their own; the main binary doesn't
// need to know about any specific foreign type.
//
// Foreign types are IMMUTABLE by construction: the only access
// method is `foreign_ref::<T>() → &T`. If mutable state is
// needed, put it in a capability vat and access via messages.
// The `T: Any + Send + Sync` bound makes interior mutability
// the only route to mutation — and we ban that by convention.
//
// The vtable is fn-pointers, not trait objects, so registries
// are trivially Send + Sync, and dylibs can contribute types
// without the main binary linking them.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Serialize, Deserialize};

use crate::value::Value;

/// Session-local, per-heap registry index. NOT serialized —
/// the on-disk identity is `ForeignTypeName`.
pub type ForeignTypeId = u32;

/// Stable cross-process identity for a foreign type. The `name`
/// is a Ruby-style namespaced string; `schema_hash` catches
/// layout drift across versions. Serialized in images so objects
/// resolve back to the right vtable on load.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ForeignTypeName {
    pub name: String,
    pub schema_hash: u64,
}

impl ForeignTypeName {
    pub fn new(name: impl Into<String>, schema_hash: u64) -> Self {
        ForeignTypeName { name: name.into(), schema_hash }
    }
}

/// Payload stored on `HeapObject::General`. Arc so clones share
/// immutably; nothing exposes a mutable borrow.
#[derive(Clone)]
pub struct ForeignData {
    pub type_id: ForeignTypeId,
    pub payload: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for ForeignData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ForeignData(type_id={})", self.type_id)
    }
}

/// Type-erased vtable. Every entry is a fn pointer so the whole
/// thing is `Copy` (and trivially Send + Sync + dylib-safe).
#[derive(Clone, Copy)]
pub struct ForeignVTable {
    /// Stable identity — written into images.
    pub id: &'static ForeignTypeIdRef,

    /// Callbacks:
    pub trace: fn(&dyn Any, &mut dyn FnMut(Value)),
    pub clone_across: fn(&dyn Any, &mut dyn FnMut(Value) -> Value) -> Arc<dyn Any + Send + Sync>,
    pub serialize: fn(&dyn Any) -> Vec<u8>,
    pub deserialize: fn(&[u8]) -> Result<Arc<dyn Any + Send + Sync>, String>,
    pub equal: fn(&dyn Any, &dyn Any) -> bool,
    pub describe: fn(&dyn Any) -> String,

    /// Virtual-slot read (Pair will use this for car/cdr).
    pub virtual_slot: Option<fn(&dyn Any, sym: u32) -> Option<Value>>,
    pub virtual_slot_names: Option<fn(&dyn Any) -> Vec<u32>>,

    /// Moof-side prototype name — the env binding (`"Cons"`, `"Vec3"`,
    /// `"String"`, …) that carries this type's handlers. Cross-vat
    /// copy uses this to re-link payloads to the right prototype in
    /// the target vat's heap, since prototype values don't match
    /// across independent heaps.
    pub prototype_name: fn() -> &'static str,

    /// Rust TypeId for downcast fast-path checks.
    pub rust_type_id: fn() -> TypeId,
}

/// Wrapper so `&'static ForeignTypeName` works. Names are owned
/// Strings, not &'static — so we indirect through an Arc. Type
/// aliased for vtable cleanliness.
pub type ForeignTypeIdRef = ForeignTypeName;

/// The trait user types impl. Callbacks here are object-safe and
/// Rust-idiomatic; the `vtable()` helper lowers them into the
/// fn-pointer vtable registered on the Heap.
pub trait ForeignType: Any + Clone + Send + Sync + 'static {
    /// Fully-qualified name: `"mycrate.TypeName"`.
    fn type_name() -> &'static str;

    /// Bump when the serialized layout changes incompatibly.
    fn schema_version() -> u32 { 1 }

    /// Moof-side prototype name this type should be linked to (e.g.
    /// `"Cons"` for `Pair`, `"Vec3"` for the vector type). Used by
    /// cross-vat copy to relocate payloads to the equivalent proto
    /// in the target vat's heap. Default `"Object"` means "no special
    /// prototype" — the generic object proto is fine.
    fn prototype_name() -> &'static str { "Object" }

    /// Enumerate child Values for GC tracing. Default = leaf.
    fn trace(&self, _visit: &mut dyn FnMut(Value)) {}

    /// Deep-copy for cross-vat migration. Default = Clone + remap
    /// via the caller-supplied walker (walker handles any embedded
    /// Values; immutable leaf types don't need to override).
    fn clone_across(&self, _copy: &mut dyn FnMut(Value) -> Value) -> Self {
        self.clone()
    }

    /// Image persistence — must round-trip.
    fn serialize(&self) -> Vec<u8>;
    fn deserialize(bytes: &[u8]) -> Result<Self, String>;

    /// Content equality (default = trait's `PartialEq` if available,
    /// but we can't bound on that here without making the trait less
    /// flexible — so default bumps to identity and users override).
    fn equal(&self, other: &Self) -> bool;

    /// Render for `describe` / REPL printing.
    fn describe(&self) -> String;

    /// Optional: virtual slots not held in the object's real slots vec.
    /// Pair uses this for `car`/`cdr`. Return None if `sym` is unknown.
    fn virtual_slot(&self, _sym: u32) -> Option<Value> { None }
    fn virtual_slot_names(&self) -> Vec<u32> { Vec::new() }
}

/// Per-heap registry. Cross-vat copies translate through
/// `ForeignTypeName` — session-local IDs don't cross boundaries.
pub struct ForeignTypeRegistry {
    vtables: Vec<ForeignVTable>,
    by_name: HashMap<String, ForeignTypeId>,
    names: Vec<Arc<ForeignTypeName>>,
}

impl ForeignTypeRegistry {
    pub fn new() -> Self {
        ForeignTypeRegistry {
            vtables: Vec::new(),
            by_name: HashMap::new(),
            names: Vec::new(),
        }
    }

    /// Register a type, building its vtable from the `ForeignType`
    /// impl. Returns the session-local ID. Idempotent: registering
    /// the same name twice is an error (schema drift check).
    pub fn register<T: ForeignType>(&mut self) -> Result<ForeignTypeId, String> {
        let name = T::type_name();
        let schema_hash = compute_schema_hash(name, T::schema_version());

        if let Some(&existing) = self.by_name.get(name) {
            let existing_hash = self.vtables[existing as usize].id.schema_hash;
            if existing_hash != schema_hash {
                return Err(format!(
                    "foreign type '{name}' re-registered with mismatched schema hash ({existing_hash:016x} vs {schema_hash:016x})"
                ));
            }
            return Ok(existing);
        }

        let id_name = Arc::new(ForeignTypeName::new(name, schema_hash));
        // Leak the name into static storage so vtables can hold &'static.
        // Registries live for the process lifetime anyway.
        let id_ref: &'static ForeignTypeName = Box::leak(Box::new((*id_name).clone()));

        let vt = ForeignVTable {
            id: id_ref,
            trace: trace_wrapper::<T>,
            clone_across: clone_across_wrapper::<T>,
            serialize: serialize_wrapper::<T>,
            deserialize: deserialize_wrapper::<T>,
            equal: equal_wrapper::<T>,
            describe: describe_wrapper::<T>,
            virtual_slot: Some(virtual_slot_wrapper::<T>),
            virtual_slot_names: Some(virtual_slot_names_wrapper::<T>),
            prototype_name: T::prototype_name,
            rust_type_id: || TypeId::of::<T>(),
        };

        let id = self.vtables.len() as ForeignTypeId;
        self.vtables.push(vt);
        self.names.push(id_name);
        self.by_name.insert(name.to_string(), id);
        Ok(id)
    }

    pub fn lookup(&self, name: &str) -> Option<ForeignTypeId> {
        self.by_name.get(name).copied()
    }

    pub fn vtable(&self, id: ForeignTypeId) -> Option<&ForeignVTable> {
        self.vtables.get(id as usize)
    }

    pub fn name(&self, id: ForeignTypeId) -> Option<&ForeignTypeName> {
        self.vtables.get(id as usize).map(|v| v.id)
    }

    /// Resolve an image-serialized `ForeignTypeName` to a live ID.
    /// Errors if missing or schema hash mismatches.
    pub fn resolve(&self, stored: &ForeignTypeName) -> Result<ForeignTypeId, String> {
        let id = self.by_name.get(&stored.name)
            .copied()
            .ok_or_else(|| format!("unknown foreign type: '{}'", stored.name))?;
        let live = self.vtables[id as usize].id;
        if live.schema_hash != stored.schema_hash {
            return Err(format!(
                "foreign type '{}': schema hash mismatch (image {:016x}, live {:016x})",
                stored.name, stored.schema_hash, live.schema_hash
            ));
        }
        Ok(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ForeignTypeId, &ForeignVTable)> {
        self.vtables.iter().enumerate().map(|(i, v)| (i as ForeignTypeId, v))
    }
}

impl Default for ForeignTypeRegistry {
    fn default() -> Self { Self::new() }
}

// ============================================================
// Vtable wrapper fns — bridge the trait (&self sigs) to the
// type-erased fn-pointer vtable (&dyn Any sigs).
// ============================================================

fn trace_wrapper<T: ForeignType>(payload: &dyn Any, visit: &mut dyn FnMut(Value)) {
    if let Some(v) = payload.downcast_ref::<T>() { v.trace(visit); }
}

fn clone_across_wrapper<T: ForeignType>(
    payload: &dyn Any, copy: &mut dyn FnMut(Value) -> Value,
) -> Arc<dyn Any + Send + Sync> {
    let v = payload.downcast_ref::<T>().expect("clone_across: type mismatch");
    Arc::new(v.clone_across(copy))
}

fn serialize_wrapper<T: ForeignType>(payload: &dyn Any) -> Vec<u8> {
    payload.downcast_ref::<T>().map(|v| v.serialize()).unwrap_or_default()
}

fn deserialize_wrapper<T: ForeignType>(bytes: &[u8]) -> Result<Arc<dyn Any + Send + Sync>, String> {
    T::deserialize(bytes).map(|v| Arc::new(v) as Arc<dyn Any + Send + Sync>)
}

fn equal_wrapper<T: ForeignType>(a: &dyn Any, b: &dyn Any) -> bool {
    match (a.downcast_ref::<T>(), b.downcast_ref::<T>()) {
        (Some(x), Some(y)) => x.equal(y),
        _ => false,
    }
}

fn describe_wrapper<T: ForeignType>(payload: &dyn Any) -> String {
    payload.downcast_ref::<T>()
        .map(|v| v.describe())
        .unwrap_or_else(|| "<?foreign>".into())
}

fn virtual_slot_wrapper<T: ForeignType>(payload: &dyn Any, sym: u32) -> Option<Value> {
    payload.downcast_ref::<T>().and_then(|v| v.virtual_slot(sym))
}

fn virtual_slot_names_wrapper<T: ForeignType>(payload: &dyn Any) -> Vec<u32> {
    payload.downcast_ref::<T>().map(|v| v.virtual_slot_names()).unwrap_or_default()
}

/// Stable-ish hash of (name, schema_version). FNV-1a 64 — no deps,
/// deterministic, collision-resistant enough for type identity.
fn compute_schema_hash(name: &str, version: u32) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for byte in name.as_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    for byte in &version.to_le_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}
