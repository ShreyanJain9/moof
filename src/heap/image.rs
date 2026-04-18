// Heap persistence — save/load a heap to/from a binary file.
//
// The image captures the object arena, the symbol table, and the
// root environment ID. Well-known symbol IDs (sym_car etc.) are
// re-resolved on load — they're not serialized, since they're
// derivable from the symbol table.

use serde::{Serialize, Deserialize};
use crate::object::HeapObject;
use super::Heap;

#[derive(Serialize, Deserialize)]
pub struct HeapImage {
    pub objects: Vec<HeapObject>,
    pub symbols: Vec<String>,
    pub env_id: u32,
}

impl Heap {
    /// Save the heap to a file (bincode-serialized).
    pub fn save_image(&self, path: &str) -> Result<(), String> {
        let image = HeapImage {
            objects: self.objects_ref().to_vec(),
            symbols: self.symbols_ref().to_vec(),
            env_id: self.env,
        };
        let bytes = bincode::serialize(&image).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, bytes).map_err(|e| format!("write: {e}"))?;
        Ok(())
    }

    /// Load a heap from a file. Returns None if the file doesn't
    /// exist or is unreadable. Well-known symbols are re-resolved;
    /// missing ones default to 0 (except `message`, which is interned).
    pub fn load_image(path: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let image: HeapImage = bincode::deserialize(&bytes).ok()?;
        Some(Heap::restore(image.objects, image.symbols, image.env_id))
    }
}
