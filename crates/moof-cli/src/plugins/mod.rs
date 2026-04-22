// Plugin system: native modules that extend the objectspace.
//
// The `Plugin` trait + `native()` helpers live in moof-core; the
// `CapabilityPlugin` trait + dynload (dylib loading) live in
// moof-runtime. This module is moof-cli's glue: it owns the
// concrete built-in plugin impls and the manifest-driven dispatch
// (`builtin_type_plugin` / `builtin_capability` / `register_from_manifest`).

pub mod core;
pub mod numeric;
pub mod collections;
pub mod effects;
pub mod block;
pub mod capabilities;
pub mod json;
pub mod vec3;

use moof_core::Heap;
use moof_core::Plugin;
use moof_runtime::CapabilityPlugin;
use moof_runtime::dynload;

// re-export moof-core plugin API so existing `use crate::plugins::native`
// call sites keep working without churn.
pub use moof_core::{native, int_binop, float_binop, float_unary, fnv1a_64};

/// Look up a built-in type plugin by name.
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

/// Look up a built-in capability plugin by name.
pub fn builtin_capability(name: &str) -> Option<Box<dyn CapabilityPlugin>> {
    match name {
        "console" => Some(Box::new(capabilities::ConsoleCapability)),
        "clock"   => Some(Box::new(capabilities::ClockCapability)),
        "file"    => Some(Box::new(capabilities::FileCapability)),
        "random"  => Some(Box::new(capabilities::RandomCapability)),
        _ => None,
    }
}

/// Resolve a manifest's [types] hashmap into a list of boxed plugins.
/// Built-ins map via `builtin_type_plugin`; dylib entries are loaded
/// via `DynTypePlugin` (which owns the Library handle so it stays
/// resident as long as the Box does). Plugins are ordered with
/// "core" first so the rest can depend on Object + base types.
pub fn resolve_type_plugins(
    types: &std::collections::HashMap<String, String>,
) -> Vec<Box<dyn Plugin>> {
    let mut names: Vec<&String> = types.keys().collect();
    names.sort_by(|a, b| {
        if a.as_str() == "core" { std::cmp::Ordering::Less }
        else if b.as_str() == "core" { std::cmp::Ordering::Greater }
        else { a.cmp(b) }
    });
    let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();
    for name in names {
        let spec = &types[name];
        if let Some(builtin_name) = crate::manifest::Manifest::is_builtin(spec) {
            match builtin_type_plugin(builtin_name) {
                Some(p) => plugins.push(p),
                None => eprintln!("  ~ unknown builtin type: {builtin_name}"),
            }
        } else {
            match dynload::DynTypePlugin::load(std::path::Path::new(spec)) {
                Ok(plugin) => {
                    eprintln!("  loaded type plugin '{}' from {spec}", plugin.name());
                    plugins.push(Box::new(plugin));
                }
                Err(e) => eprintln!("  ~ type plugin '{name}' failed to load: {e}"),
            }
        }
    }
    plugins
}

/// Register all default type plugins (fallback when no manifest).
pub fn register_all(heap: &mut Heap) {
    for name in ["core", "numeric", "collections", "block", "effects", "json"] {
        if let Some(p) = builtin_type_plugin(name) { p.register(heap); }
    }
}
