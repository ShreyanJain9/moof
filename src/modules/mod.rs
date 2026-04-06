/// Module system for MOOF.
///
/// Modules are objects in the image. Source files in lib/ exist only for
/// initial seeding — once the image is built, it IS the program.

pub mod graph;
pub mod loader;
pub mod sandbox;

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
    /// Path to the source file (None when loaded from image)
    pub path: Option<PathBuf>,
    /// SHA-256 hash of the source file contents
    pub source_hash: String,
    /// Byte offset where the module body starts (after the header form)
    pub body_offset: usize,
    /// If true, this module is loaded with unrestricted compiler mode (for IO modules)
    pub unrestricted: bool,
}
