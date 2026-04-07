/// Image persistence: save and load the fabric's heap.
///
/// The image is a bincode serialization of the heap's objects + symbol table.
/// Type prototypes are stored as (name, object_id) pairs.

use crate::heap::Heap;
use serde::{Serialize, Deserialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct Image {
    version: u32,
    heap: Heap,
    protos: Vec<(String, u32)>,
}

/// Save the fabric's heap to a file.
pub fn save(path: &Path, heap: &Heap, protos: &[(String, u32)]) -> Result<(), String> {
    let image = Image {
        version: 2,
        heap: heap.clone(),
        protos: protos.to_vec(),
    };
    let data = bincode::serialize(&image)
        .map_err(|e| format!("serialize: {}", e))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, &data)
        .map_err(|e| format!("write: {}", e))?;
    Ok(())
}

/// Load a fabric's heap from a file.
/// Returns (heap, protos).
pub fn load(path: &Path) -> Result<(Heap, Vec<(String, u32)>), String> {
    let data = std::fs::read(path)
        .map_err(|e| format!("read: {}", e))?;
    let mut image: Image = bincode::deserialize(&data)
        .map_err(|e| format!("deserialize: {}", e))?;
    image.heap.rebuild_symbol_lookup();
    Ok((image.heap, image.protos))
}
