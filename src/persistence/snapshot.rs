/// Snapshot: serialize/deserialize the entire heap + symbol table.
///
/// The image is just the serialized slab — Vec<HeapObject> + Vec<String>.
/// Content-addressed by SHA-256 hash.

use std::path::Path;
use std::fs;
use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};

use crate::runtime::value::HeapObject;

/// The serializable image: everything needed to reconstruct the heap.
#[derive(Serialize, Deserialize)]
pub struct Image {
    pub objects: Vec<HeapObject>,
    pub symbol_names: Vec<String>,
}

/// Save an image to disk. Returns the SHA-256 hash.
pub fn save_image(image: &Image, dir: &Path) -> Result<String, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {}", dir.display(), e))?;

    let data = bincode::serialize(image)
        .map_err(|e| format!("Serialize error: {}", e))?;

    // Content hash
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = format!("{:x}", hasher.finalize());

    // Write image
    let image_path = dir.join("image.bin");
    fs::write(&image_path, &data)
        .map_err(|e| format!("Cannot write {}: {}", image_path.display(), e))?;

    // Write hash
    let hash_path = dir.join("image.sha256");
    fs::write(&hash_path, &hash)
        .map_err(|e| format!("Cannot write {}: {}", hash_path.display(), e))?;

    // Clear WAL on successful snapshot
    let wal_path = dir.join("wal.bin");
    if wal_path.exists() {
        let _ = fs::remove_file(&wal_path);
    }

    Ok(hash)
}

/// Load an image from disk. Verifies SHA-256 hash if present.
pub fn load_image(dir: &Path) -> Result<Image, String> {
    let image_path = dir.join("image.bin");
    let data = fs::read(&image_path)
        .map_err(|e| format!("Cannot read {}: {}", image_path.display(), e))?;

    // Verify hash if available
    let hash_path = dir.join("image.sha256");
    if hash_path.exists() {
        let expected = fs::read_to_string(&hash_path)
            .map_err(|e| format!("Cannot read hash: {}", e))?;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let actual = format!("{:x}", hasher.finalize());
        if actual.trim() != expected.trim() {
            return Err(format!("Image hash mismatch: expected {}, got {}", expected.trim(), actual));
        }
    }

    bincode::deserialize(&data)
        .map_err(|e| format!("Deserialize error: {}", e))
}

/// Check if a saved image exists.
pub fn image_exists(dir: &Path) -> bool {
    dir.join("image.bin").exists()
}
