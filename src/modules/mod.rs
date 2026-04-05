/// Module system for MOOF.
///
/// Source files are the canonical representation. Each .moof file declares
/// a module with explicit dependencies and exports. Modules load in
/// topological order within sandboxed environments.
///
/// Directory layout:
///   lib/
///     bootstrap.moof    — kernel module (no deps)
///     collections.moof  — data structures
///     ...
///   .moof/
///     cache/            — compiled bytecode cache (derived, not canonical)

pub mod graph;
pub mod loader;
pub mod sandbox;
pub mod cache;

use std::path::PathBuf;

/// Parsed module header — extracted from the first form in a .moof file.
#[derive(Debug, Clone)]
pub struct ModuleDescriptor {
    /// The module name (e.g., "collections")
    pub name: String,
    /// Names of modules this one depends on
    pub requires: Vec<String>,
    /// Symbols this module exports
    pub provides: Vec<String>,
    /// Path to the source file
    pub path: PathBuf,
    /// SHA-256 hash of the source file contents
    pub source_hash: String,
    /// Byte offset where the module body starts (after the header form)
    pub body_offset: usize,
    /// If true, this module is loaded with unrestricted compiler mode (for IO modules)
    pub unrestricted: bool,
}
