// moof-stdlib — built-in type plugins.
//
// Every plugin here impls `moof_core::Plugin` and runs on a fresh
// Heap to install its type prototypes + handlers. The set is
// curated — "core" (Object + closures), "numeric" (Integer, Float),
// "collections" (Cons, String, Bytes, Table, Error), "effects"
// (Act, Update, Result, FarRef, Vat), "block" (Block), "json"
// (JSON ↔ moof Value), "vec3" (demo Vec3 foreign type).
//
// Depends on moof-core only — no VM, no scheduler, no I/O.

pub mod core;
pub mod numeric;
pub mod collections;
pub mod effects;
pub mod block;
pub mod json;
pub mod vec3;

use moof_core::{Heap, Plugin};

/// Look up a built-in type plugin by name (matches manifest's
/// `builtin:NAME` specs).
pub fn builtin_type_plugin(name: &str) -> Option<Box<dyn Plugin>> {
    match name {
        "core" => Some(Box::new(core::CorePlugin)),
        "numeric" => Some(Box::new(numeric::NumericPlugin)),
        "collections" => Some(Box::new(collections::CollectionsPlugin)),
        "block" => Some(Box::new(block::BlockPlugin)),
        "effects" => Some(Box::new(effects::EffectsPlugin)),
        "json" => Some(Box::new(json::JsonPlugin)),
        "vec3" => Some(Box::new(vec3::Vec3Plugin)),
        _ => None,
    }
}

/// Register all default type plugins (fallback when no manifest).
pub fn register_all(heap: &mut Heap) {
    for name in ["core", "numeric", "collections", "block", "effects", "json"] {
        if let Some(p) = builtin_type_plugin(name) { p.register(heap); }
    }
}
