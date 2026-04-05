use std::path::Path;
use std::fs::File;
use std::io::{Read, Write};
use serde::{Serialize, Deserialize};
use crate::runtime::heap::Heap;
use crate::runtime::value::HeapObject;

#[derive(Serialize, Deserialize)]
pub struct Image {
    pub version: u32,
    pub objects: Vec<HeapObject>,
    pub symbol_names: Vec<String>,
    pub root_env_id: u32,
    pub protos: Vec<(String, u32)>,
    // TODO Phase 2: module_registry
}

pub fn save_image(path: &Path, heap: &Heap, root_env_id: u32, protos: Vec<(String, u32)>) -> Result<(), String> {
    let image = Image {
        version: 3,
        objects: heap.objects_clone(),
        symbol_names: heap.symbol_names_clone(),
        root_env_id,
        protos,
    };

    let bytes = bincode::serialize(&image)
        .map_err(|e| format!("cannot serialize image: {}", e))?;

    let mut file = File::create(path)
        .map_err(|e| format!("cannot create {}: {}", path.display(), e))?;
    file.write_all(&bytes)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;

    Ok(())
}

pub fn load_image(path: &Path) -> Result<(Heap, u32, Vec<(String, u32)>), String> {
    let mut file = File::open(path)
        .map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;

    let image: Image = bincode::deserialize(&bytes)
        .map_err(|e| format!("cannot deserialize image: {}", e))?;

    if image.version != 3 {
        return Err(format!("unsupported image version: {}", image.version));
    }

    let heap = Heap::from_image(image.objects, image.symbol_names);
    Ok((heap, image.root_env_id, image.protos))
}
