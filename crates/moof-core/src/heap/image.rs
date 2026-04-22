// Heap persistence — save/load a heap to/from a binary file.
//
// The image captures the object arena, the symbol table, and the
// root environment ID. Well-known symbol IDs (sym_car etc.) are
// re-resolved on load — they're not serialized, since they're
// derivable from the symbol table.
//
// Foreign objects can't go through serde directly (the payload is
// `Arc<dyn Any>`), so we marshal through `HeapObjectImage` — a
// serde-friendly mirror of `HeapObject` where foreign payloads
// are pre-serialized to bytes via the vtable. Load reverses this
// through the registry.

use serde::{Serialize, Deserialize};
use crate::object::HeapObject;
use crate::foreign::{ForeignData, ForeignTypeName};
use crate::value::Value;
use super::Heap;

/// Serde-friendly mirror of `HeapObject`. Foreign payloads become
/// (name, bytes) pairs; load-time resolution through the registry
/// turns them back into live `ForeignData`.
#[derive(Serialize, Deserialize)]
enum HeapObjectImage {
    General {
        proto: Value,
        slot_names: Vec<u32>,
        slot_values: Vec<Value>,
        handlers: Vec<(u32, Value)>,
        foreign: Option<ForeignImage>,
    },
}

#[derive(Serialize, Deserialize)]
struct ForeignImage {
    name: ForeignTypeName,
    bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct HeapImage {
    objects: Vec<HeapObjectImage>,
    symbols: Vec<String>,
    env_id: u32,
}

impl Heap {
    /// Serialize a single `HeapObject` to bytes. Goes through the
    /// image-intermediate so foreign payloads use the registered
    /// vtable's serializer.
    pub fn serialize_object(&self, obj: &HeapObject) -> Result<Vec<u8>, String> {
        let img = self.object_to_image(obj)?;
        bincode::serialize(&img).map_err(|e| format!("serialize: {e}"))
    }

    /// Deserialize bytes back into a `HeapObject`. Foreign payloads
    /// are resolved against this heap's registry at load time.
    pub fn deserialize_object(&self, bytes: &[u8]) -> Result<HeapObject, String> {
        let img: HeapObjectImage = bincode::deserialize(bytes)
            .map_err(|e| format!("deserialize: {e}"))?;
        self.object_from_image(img)
    }

    /// Save the heap to a file (bincode-serialized). Foreign
    /// payloads are serialized through their registered vtable.
    pub fn save_image(&self, path: &str) -> Result<(), String> {
        let objects = self.objects_ref().iter()
            .map(|obj| self.object_to_image(obj))
            .collect::<Result<Vec<_>, _>>()?;

        let image = HeapImage {
            objects,
            symbols: self.symbols_ref().to_vec(),
            env_id: self.env,
        };
        let bytes = bincode::serialize(&image).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, bytes).map_err(|e| format!("write: {e}"))?;
        Ok(())
    }

    /// Load a heap from a file. Foreign payloads are resolved
    /// through the in-memory registry (which must already contain
    /// the same types that were registered when the image was
    /// written). Missing type or schema-hash mismatch = error.
    pub fn load_image(path: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let image: HeapImage = bincode::deserialize(&bytes).ok()?;

        // Need a heap to resolve foreigns against — build a blank
        // one, then populate via the normal restore path. Any
        // foreign type the image contains must have been registered
        // by the time restore runs (callers typically register core
        // plugins before calling load_image).
        let mut h = Heap::new();
        let objects = image.objects.into_iter()
            .map(|img| h.object_from_image(img))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        h.restore_objects(objects, image.symbols, image.env_id);
        Some(h)
    }

    fn object_to_image(&self, obj: &HeapObject) -> Result<HeapObjectImage, String> {
        Ok(match obj {
            HeapObject::General { proto, slot_names, slot_values, handlers, foreign } => {
                let foreign_img = match foreign {
                    Some(fd) => {
                        let vt = self.foreign_registry().vtable(fd.type_id)
                            .ok_or_else(|| format!("unknown foreign type_id {} at save", fd.type_id))?;
                        let bytes = (vt.serialize)(&*fd.payload);
                        Some(ForeignImage { name: vt.id.clone(), bytes })
                    }
                    None => None,
                };
                HeapObjectImage::General {
                    proto: *proto,
                    slot_names: slot_names.clone(),
                    slot_values: slot_values.clone(),
                    handlers: handlers.clone(),
                    foreign: foreign_img,
                }
            }
        })
    }

    fn object_from_image(&self, img: HeapObjectImage) -> Result<HeapObject, String> {
        Ok(match img {
            HeapObjectImage::General { proto, slot_names, slot_values, handlers, foreign } => {
                let foreign_data = match foreign {
                    Some(fimg) => {
                        let type_id = self.foreign_registry().resolve(&fimg.name)?;
                        let vt = self.foreign_registry().vtable(type_id)
                            .ok_or_else(|| format!("vtable missing for '{}' after resolve", fimg.name.name))?;
                        let payload = (vt.deserialize)(&fimg.bytes)?;
                        Some(ForeignData { type_id, payload })
                    }
                    None => None,
                };
                HeapObject::General { proto, slot_names, slot_values, handlers, foreign: foreign_data }
            }
        })
    }
}
