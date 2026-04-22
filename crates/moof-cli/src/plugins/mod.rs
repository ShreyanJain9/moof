// Manifest-driven plugin dispatch for moof-cli.
//
// Every plugin — type or capability — is loaded from a cdylib at
// runtime. The manifest writes either:
//
//   core = "builtin:core"
//     → resolved to `target/<profile>/libmoof_plugin_core.{dylib,so,dll}`
//
//   color = "path/to/libmoof_color_plugin.dylib"
//     → literal path to an external plugin
//
// There are no compiled-in plugins. moof-cli is ~600 LoC of shell +
// wiring; everything else is a dylib.

use moof_core::Plugin;
use moof_runtime::{CapabilityPlugin, dynload};
use std::path::Path;

pub use moof_core::{native, int_binop, float_binop, float_unary, fnv1a_64};

/// Platform-specific prefix + extension for shared libraries.
fn dylib_prefix_ext() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos")   { ("lib", "dylib") }
    else if cfg!(target_os = "windows") { ("", "dll") }
    else                                { ("lib", "so") }
}

/// Resolve a `builtin:NAME` type-plugin spec to an on-disk dylib
/// path. The crate is `moof-plugin-NAME`; its cdylib artifact lands
/// in the workspace's `target/<profile>/` directory.
fn builtin_type_plugin_path(name: &str) -> String {
    let (prefix, ext) = dylib_prefix_ext();
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    // replace hyphens with underscores for the dylib filename, cargo convention
    let fname = format!("moof_plugin_{}", name.replace('-', "_"));
    format!("target/{profile}/{prefix}{fname}.{ext}")
}

/// Same for `builtin:NAME` capability specs — crate `moof-cap-NAME`.
fn builtin_capability_path(name: &str) -> String {
    let (prefix, ext) = dylib_prefix_ext();
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let fname = format!("moof_cap_{}", name.replace('-', "_"));
    format!("target/{profile}/{prefix}{fname}.{ext}")
}

/// Resolve a manifest's [types] hashmap into a list of boxed plugins.
/// All entries go through the dylib loader; `builtin:NAME` is sugar
/// for the in-tree cdylib path.
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
        let raw_spec = &types[name];
        let path_string = match crate::manifest::Manifest::is_builtin(raw_spec) {
            Some(bn) => builtin_type_plugin_path(bn),
            None => raw_spec.clone(),
        };
        match dynload::DynTypePlugin::load(Path::new(&path_string)) {
            Ok(plugin) => {
                eprintln!("  loaded type plugin '{}' from {path_string}", plugin.name());
                plugins.push(Box::new(plugin));
            }
            Err(e) => eprintln!("  ~ type plugin '{name}' failed to load: {e}"),
        }
    }
    plugins
}

/// Resolve a capability spec to a loader. Capabilities are spawned
/// into their own vat by the caller (the scheduler's spawn_capability
/// path), so this returns the boxed CapabilityPlugin rather than
/// spawning directly.
pub fn resolve_capability(spec: &str) -> Result<Box<dyn CapabilityPlugin>, String> {
    let path_string = match crate::manifest::Manifest::is_builtin(spec) {
        Some(bn) => builtin_capability_path(bn),
        None => spec.to_string(),
    };
    dynload::DynCapabilityPlugin::load(Path::new(&path_string))
        .map(|p| Box::new(p) as Box<dyn CapabilityPlugin>)
}
