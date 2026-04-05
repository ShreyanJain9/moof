/// Binary cache for compiled modules.
///
/// Stores compiled bytecode per module to avoid re-parsing and re-compiling
/// on startup. The cache is derived from source — never canonical.
///
/// Layout:
///   .moof/cache/manifest.bin   — module list + hashes + load order
///   .moof/cache/<module>.bin   — compiled bytecode for one module

use serde::{Serialize, Deserialize};
use std::path::Path;
use std::fs;
use sha2::{Sha256, Digest};

/// A constant that doesn't depend on runtime heap IDs.
/// Symbols are stored by name, re-interned on load.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PortableConstant {
    Nil,
    True,
    False,
    Integer(i64),
    Float(f64),
    SymbolName(String),
    /// A string literal
    StringLit(String),
}

/// Compiled bytecode with portable constants.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PortableChunk {
    pub code: Vec<u8>,
    pub constants: Vec<PortableConstant>,
}

/// Per-module cache entry.
#[derive(Serialize, Deserialize, Debug)]
pub struct ModuleCache {
    /// SHA-256 of the source file this was compiled from
    pub source_hash: String,
    /// The compiled bytecode chunks for each top-level expression
    pub chunks: Vec<PortableChunk>,
}

/// The manifest tying together all module caches.
#[derive(Serialize, Deserialize, Debug)]
pub struct CacheManifest {
    /// Module name -> source hash
    pub modules: Vec<(String, String)>,
    /// Load order (topo-sorted names)
    pub load_order: Vec<String>,
    /// SHA-256 of the concatenation of all source hashes
    pub global_hash: String,
}

/// Compute SHA-256 hash of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Check if a module's cache is valid (source hash matches).
pub fn load_module_cache(cache_dir: &Path, module_name: &str, expected_hash: &str) -> Option<ModuleCache> {
    let path = cache_dir.join(format!("{}.bin", module_name));
    let data = fs::read(&path).ok()?;
    let cache: ModuleCache = bincode::deserialize(&data).ok()?;
    if cache.source_hash == expected_hash {
        Some(cache)
    } else {
        None
    }
}

/// Save a module's compiled cache.
pub fn save_module_cache(cache_dir: &Path, module_name: &str, cache: &ModuleCache) -> Result<(), String> {
    fs::create_dir_all(cache_dir).map_err(|e| format!("cannot create cache dir: {}", e))?;
    let data = bincode::serialize(cache).map_err(|e| format!("serialize error: {}", e))?;
    let path = cache_dir.join(format!("{}.bin", module_name));
    fs::write(&path, &data).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

/// Save the cache manifest.
pub fn save_manifest(cache_dir: &Path, manifest: &CacheManifest) -> Result<(), String> {
    fs::create_dir_all(cache_dir).map_err(|e| format!("cannot create cache dir: {}", e))?;
    let data = bincode::serialize(manifest).map_err(|e| format!("serialize error: {}", e))?;
    let path = cache_dir.join("manifest.bin");
    fs::write(&path, &data).map_err(|e| format!("cannot write manifest: {}", e))
}

/// Load the cache manifest.
pub fn load_manifest(cache_dir: &Path) -> Option<CacheManifest> {
    let path = cache_dir.join("manifest.bin");
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}
