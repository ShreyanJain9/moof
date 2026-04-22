// Manifest-driven plugin dispatch for moof-cli.
//
// The concrete plugin impls now live in sibling crates:
//   - moof-stdlib: type plugins (core, numeric, collections, effects,
//                  block, json, vec3)
//   - moof-caps:   capability plugins (console, clock, file, random)
//
// moof-cli wires the manifest to those catalogs here. External
// type-plugin authors should depend on `moof-core` directly (and,
// for capability-plugin authors, `moof-runtime` + `moof-core`).

use moof_core::{Heap, Plugin};
use moof_runtime::CapabilityPlugin;

pub use moof_core::{native, int_binop, float_binop, float_unary, fnv1a_64};
pub use moof_stdlib::{builtin_type_plugin, register_all};
pub use moof_caps::builtin_capability;

use moof_runtime::dynload;

/// Resolve a manifest's [types] hashmap into a list of boxed plugins.
/// Built-ins map via `moof_stdlib::builtin_type_plugin`; dylib entries
/// are loaded via `DynTypePlugin` (which owns the Library handle so
/// it stays resident as long as the Box does). Plugins are ordered
/// with "core" first so the rest can depend on Object + base types.
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

// Compatibility: legacy callers expected `plugins::CapabilityPlugin`
// at this path. Keep the re-export.
pub use moof_runtime::CapabilityPlugin as _CapabilityPlugin;
