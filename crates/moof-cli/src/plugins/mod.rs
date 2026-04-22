// Plugin system: native modules that extend the objectspace.
//
// `Plugin` + `native()` and friends now live in moof-core so that
// external type-plugin authors can depend on just that one crate.
// This module keeps the bits that need the full moof-cli stack:
// the `CapabilityPlugin` trait (which owns a `Vat`), the built-in
// registry (`builtin_type_plugin` / `builtin_capability`), and
// `register_from_manifest` (which wires dylibs through dynload).

pub mod core;
pub mod numeric;
pub mod collections;
pub mod effects;
pub mod block;
pub mod capabilities;
pub mod dynload;
pub mod json;
pub mod vec3;

use moof_core::Heap;
use moof_core::Plugin;
use crate::vat::Vat;

// re-export moof-core plugin API so existing `use crate::plugins::native`
// call sites keep working without churn.
pub use moof_core::{native, int_binop, float_binop, float_unary, fnv1a_64};

/// A native capability that lives in its own vat.
/// Creates a root object with native handlers. Sends to the
/// capability go through FarRef → outbox → scheduler → native
/// handler → Act resolution. All effects are mediated.
pub trait CapabilityPlugin {
    /// The name used to bind the FarRef in the REPL (e.g. "console").
    fn name(&self) -> &str;

    /// Set up native handlers on a root object in the given vat.
    /// Returns the root object ID (for FarRef creation).
    fn setup(&self, vat: &mut Vat) -> u32;
}

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

/// Register type plugins on a heap based on manifest [types].
/// Entries are `builtin:NAME` (compiled in) or a path to a cdylib
/// that exports `moof_create_type_plugin`. Dylibs are held by the
/// scheduler's `loaded_type_plugins` vec so they stay resident for
/// the process lifetime (unload-while-in-use = UB).
pub fn register_from_manifest(
    heap: &mut Heap,
    types: &std::collections::HashMap<String, String>,
    dylib_keepalives: &mut Vec<dynload::DynTypePlugin>,
) {
    // ensure "core" loads first, then alphabetical
    let mut names: Vec<&String> = types.keys().collect();
    names.sort_by(|a, b| {
        if a.as_str() == "core" { std::cmp::Ordering::Less }
        else if b.as_str() == "core" { std::cmp::Ordering::Greater }
        else { a.cmp(b) }
    });
    for name in names {
        let spec = &types[name];
        if let Some(builtin_name) = crate::manifest::Manifest::is_builtin(spec) {
            if let Some(plugin) = builtin_type_plugin(builtin_name) {
                plugin.register(heap);
            } else {
                eprintln!("  ~ unknown builtin type: {builtin_name}");
            }
        } else {
            match dynload::DynTypePlugin::load(std::path::Path::new(spec)) {
                Ok(plugin) => {
                    eprintln!("  loaded type plugin '{}' from {spec}", plugin.name());
                    plugin.register(heap);
                    dylib_keepalives.push(plugin);
                }
                Err(e) => eprintln!("  ~ type plugin '{name}' failed to load: {e}"),
            }
        }
    }
}

/// Register all default type plugins (fallback when no manifest).
pub fn register_all(heap: &mut Heap) {
    for name in ["core", "numeric", "collections", "block", "effects", "json"] {
        if let Some(p) = builtin_type_plugin(name) { p.register(heap); }
    }
}
