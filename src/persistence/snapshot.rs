/// Snapshot: serialize/deserialize the entire heap + symbol table.
///
/// The image is just the serialized slab — Vec<HeapObject> + Vec<String>.
/// Content-addressed by SHA-256 hash.

use std::path::Path;
use std::fs;
use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};

use crate::runtime::value::{Value, HeapObject};
use std::collections::{HashMap, HashSet};

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

/// Compact an image by mark-and-compact: only reachable objects are kept,
/// with ids renumbered sequentially. Returns (compacted_image, new_root_env_id).
///
/// The in-memory heap is NOT modified — compaction only affects the saved image.
pub fn compact_image(image: &Image, root_env: u32) -> (Image, u32) {
    let objects = &image.objects;

    // ── Mark phase: DFS from root_env ──
    let mut marked = HashSet::new();
    let mut worklist = vec![root_env];

    while let Some(id) = worklist.pop() {
        if (id as usize) >= objects.len() || !marked.insert(id) {
            continue;
        }
        // Trace references from this object
        match &objects[id as usize] {
            HeapObject::Cons { car, cdr } => {
                trace_value(*car, &mut worklist);
                trace_value(*cdr, &mut worklist);
            }
            HeapObject::MoofString(_) => {}
            HeapObject::NativeFunction { .. } => {}
            HeapObject::GeneralObject { parent, slots, handlers } => {
                trace_value(*parent, &mut worklist);
                for &(_, v) in slots {
                    trace_value(v, &mut worklist);
                }
                for &(_, v) in handlers {
                    trace_value(v, &mut worklist);
                }
            }
            HeapObject::BytecodeChunk(chunk) => {
                for &c in &chunk.constants {
                    trace_value(c, &mut worklist);
                }
            }
            HeapObject::Operative { params, env_param: _, body, def_env, source } => {
                trace_value(*params, &mut worklist);
                worklist.push(*body);
                worklist.push(*def_env);
                trace_value(*source, &mut worklist);
            }
            HeapObject::Lambda { params, body, def_env, source } => {
                trace_value(*params, &mut worklist);
                worklist.push(*body);
                worklist.push(*def_env);
                trace_value(*source, &mut worklist);
            }
            HeapObject::Environment(env) => {
                if let Some(parent) = env.parent {
                    worklist.push(parent);
                }
                for &v in env.bindings.values() {
                    trace_value(v, &mut worklist);
                }
            }
        }
    }

    // ── Build forwarding table: old id -> new sequential id ──
    let mut sorted_marked: Vec<u32> = marked.into_iter().collect();
    sorted_marked.sort();

    let mut forwarding: HashMap<u32, u32> = HashMap::new();
    for (new_id, &old_id) in sorted_marked.iter().enumerate() {
        forwarding.insert(old_id, new_id as u32);
    }

    // ── Rewrite: create compacted object vec with updated references ──
    let mut new_objects = Vec::with_capacity(sorted_marked.len());
    for &old_id in &sorted_marked {
        let obj = rewrite_object(&objects[old_id as usize], &forwarding);
        new_objects.push(obj);
    }

    let new_root = *forwarding.get(&root_env).unwrap_or(&0);

    let compacted = Image {
        objects: new_objects,
        symbol_names: image.symbol_names.clone(),
    };

    (compacted, new_root)
}

/// Push a heap object id onto the worklist if the value is a heap reference.
fn trace_value(val: Value, worklist: &mut Vec<u32>) {
    if let Value::Object(id) = val {
        worklist.push(id);
    }
}

/// Rewrite a heap object, updating all Value::Object references through the forwarding table.
fn rewrite_object(obj: &HeapObject, fwd: &HashMap<u32, u32>) -> HeapObject {
    match obj {
        HeapObject::Cons { car, cdr } => HeapObject::Cons {
            car: rewrite_value(*car, fwd),
            cdr: rewrite_value(*cdr, fwd),
        },
        HeapObject::MoofString(s) => HeapObject::MoofString(s.clone()),
        HeapObject::NativeFunction { name } => HeapObject::NativeFunction { name: name.clone() },
        HeapObject::GeneralObject { parent, slots, handlers } => HeapObject::GeneralObject {
            parent: rewrite_value(*parent, fwd),
            slots: slots.iter().map(|&(k, v)| (k, rewrite_value(v, fwd))).collect(),
            handlers: handlers.iter().map(|&(k, v)| (k, rewrite_value(v, fwd))).collect(),
        },
        HeapObject::BytecodeChunk(chunk) => {
            use crate::runtime::value::BytecodeChunk;
            HeapObject::BytecodeChunk(BytecodeChunk {
                code: chunk.code.clone(),
                constants: chunk.constants.iter().map(|&v| rewrite_value(v, fwd)).collect(),
            })
        }
        HeapObject::Operative { params, env_param, body, def_env, source } => HeapObject::Operative {
            params: rewrite_value(*params, fwd),
            env_param: *env_param,
            body: *fwd.get(body).unwrap_or(body),
            def_env: *fwd.get(def_env).unwrap_or(def_env),
            source: rewrite_value(*source, fwd),
        },
        HeapObject::Lambda { params, body, def_env, source } => HeapObject::Lambda {
            params: rewrite_value(*params, fwd),
            body: *fwd.get(body).unwrap_or(body),
            def_env: *fwd.get(def_env).unwrap_or(def_env),
            source: rewrite_value(*source, fwd),
        },
        HeapObject::Environment(env) => {
            use crate::runtime::env::Environment;
            let new_parent = env.parent.map(|p| *fwd.get(&p).unwrap_or(&p));
            let new_bindings = env.bindings.iter()
                .map(|(&k, &v)| (k, rewrite_value(v, fwd)))
                .collect();
            HeapObject::Environment(Environment {
                parent: new_parent,
                bindings: new_bindings,
            })
        }
    }
}

/// Rewrite a Value, updating Object references through the forwarding table.
fn rewrite_value(val: Value, fwd: &HashMap<u32, u32>) -> Value {
    match val {
        Value::Object(id) => Value::Object(*fwd.get(&id).unwrap_or(&id)),
        other => other,
    }
}
