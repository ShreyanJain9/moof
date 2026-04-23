// LMDB-backed persistent object store.
//
// Objects stored as bincode-serialized HeapObject entries.
// Key: u32 object ID (big-endian). Value: serialized bytes.
// Symbol table in a separate LMDB database.
// Environment is a heap object — its ID is stored as metadata.
//
// The nursery (heap.rs) is the fast in-memory arena.
// Promotion: when an object needs to persist, it moves here.
// For now: save_all dumps the entire nursery to LMDB on exit.
// load_all restores on startup.

use std::path::Path;
use heed::types::*;
use heed::{Database, Env, EnvOpenOptions};
use moof_core::object::HeapObject;
use moof_core::heap::Heap;

pub struct Store {
    env: Env,
    objects: Database<U32<heed::byteorder::BE>, Bytes>,
    meta: Database<Str, Bytes>, // "symbols", "env_id", "closure_descs"
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(256 * 1024 * 1024) // 256 MB
                .max_dbs(2)
                .open(path)
                .map_err(|e| format!("lmdb open: {e}"))?
        };
        let mut wtxn = env.write_txn().map_err(|e| format!("txn: {e}"))?;
        let objects = env.create_database(&mut wtxn, Some("objects"))
            .map_err(|e| format!("create db: {e}"))?;
        let meta = env.create_database(&mut wtxn, Some("meta"))
            .map_err(|e| format!("create db: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(Store { env, objects, meta })
    }

    /// Save all nursery objects + metadata to LMDB. Object bytes
    /// go through `Heap::serialize_object` so foreign payloads
    /// round-trip via the registered vtable.
    pub fn save_all(
        &self,
        heap: &Heap,
        closure_chunks: &[moof_lang::lang::compiler::ClosureDesc],
    ) -> Result<(), String> {
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;

        // clear old data
        self.objects.clear(&mut wtxn).map_err(|e| format!("clear: {e}"))?;
        self.meta.clear(&mut wtxn).map_err(|e| format!("clear meta: {e}"))?;

        // write objects
        for (i, obj) in heap.objects_ref().iter().enumerate() {
            let bytes = heap.serialize_object(obj)?;
            self.objects.put(&mut wtxn, &(i as u32), &bytes)
                .map_err(|e| format!("put obj: {e}"))?;
        }

        // write symbols
        let sym_bytes = bincode::serialize(heap.symbols_ref()).map_err(|e| format!("serialize syms: {e}"))?;
        self.meta.put(&mut wtxn, "symbols", &sym_bytes)
            .map_err(|e| format!("put syms: {e}"))?;

        // write env_id
        let env_bytes = bincode::serialize(&heap.env).map_err(|e| format!("serialize env: {e}"))?;
        self.meta.put(&mut wtxn, "env_id", &env_bytes)
            .map_err(|e| format!("put env: {e}"))?;

        // write closure descs (bytecode chunks + metadata). source
        // is included so live inspectors still see handler source
        // after save/restore.
        let desc_data: Vec<SerializableClosureDesc> = closure_chunks.iter().map(|d| {
            SerializableClosureDesc {
                code: d.chunk.code.clone(),
                constants: d.chunk.constants.clone(),
                arity: d.chunk.arity,
                num_regs: d.chunk.num_regs,
                param_names: d.param_names.clone(),
                is_operative: d.is_operative,
                capture_names: d.capture_names.clone(),
                capture_parent_regs: d.capture_parent_regs.clone(),
                capture_local_regs: d.capture_local_regs.clone(),
                desc_base: d.desc_base,
                rest_param_reg: d.rest_param_reg,
                source: d.source.clone(),
            }
        }).collect();
        let desc_bytes = bincode::serialize(&desc_data).map_err(|e| format!("serialize descs: {e}"))?;
        self.meta.put(&mut wtxn, "closure_descs", &desc_bytes)
            .map_err(|e| format!("put descs: {e}"))?;

        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    /// Load everything from LMDB. Returns None if empty. `heap`
    /// provides the foreign-type registry used to rehydrate
    /// persisted foreign payloads.
    pub fn load_all(&self, heap: &Heap) -> Option<LoadedImage> {
        let rtxn = self.env.read_txn().ok()?;

        // read symbols first (needed for everything else)
        let sym_bytes = self.meta.get(&rtxn, "symbols").ok()??;
        let symbols: Vec<String> = bincode::deserialize(sym_bytes).ok()?;

        // read objects
        let mut objects = Vec::new();
        let iter = self.objects.iter(&rtxn).ok()?;
        for item in iter {
            let (_, bytes) = item.ok()?;
            let obj: HeapObject = heap.deserialize_object(bytes).ok()?;
            objects.push(obj);
        }
        if objects.is_empty() { return None; }

        // read env_id
        let env_bytes = self.meta.get(&rtxn, "env_id").ok()??;
        let env_id: u32 = bincode::deserialize(env_bytes).ok()?;

        // read closure descs
        let desc_bytes = self.meta.get(&rtxn, "closure_descs").ok()??;
        let descs: Vec<SerializableClosureDesc> = bincode::deserialize(desc_bytes).ok()?;

        rtxn.commit().ok()?;

        Some(LoadedImage { objects, symbols, env_id, closure_descs: descs })
    }
}

pub struct LoadedImage {
    pub objects: Vec<HeapObject>,
    pub symbols: Vec<String>,
    pub env_id: u32,
    pub closure_descs: Vec<SerializableClosureDesc>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SerializableClosureDesc {
    pub code: Vec<u8>,
    pub constants: Vec<u64>,
    pub arity: u8,
    pub num_regs: u8,
    pub param_names: Vec<u32>,
    pub is_operative: bool,
    pub capture_names: Vec<u32>,
    pub capture_parent_regs: Vec<u8>,
    pub capture_local_regs: Vec<u8>,
    pub desc_base: usize,
    pub rest_param_reg: Option<u8>,
    #[serde(default)]
    pub source: Option<moof_core::source::ClosureSource>,
}
