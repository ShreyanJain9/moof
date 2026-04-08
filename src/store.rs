// LMDB-backed persistent object store.
//
// Objects stored as bincode-serialized HeapObject entries.
// Key: u32 object ID (big-endian). Value: serialized bytes.
// Symbol table in a separate LMDB database.
// Globals + operatives in a third database.
//
// The nursery (heap.rs) is the fast in-memory arena.
// Promotion: when an object needs to persist, it moves here.
// For now: save_all dumps the entire nursery to LMDB on exit.
// load_all restores on startup.

use std::path::Path;
use heed::types::*;
use heed::{Database, Env, EnvOpenOptions};
use crate::object::HeapObject;
use crate::value::Value;

pub struct Store {
    env: Env,
    objects: Database<U32<heed::byteorder::BE>, Bytes>,
    meta: Database<Str, Bytes>, // "symbols", "globals", "operatives", "closure_descs"
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

    /// Save all nursery objects + metadata to LMDB.
    pub fn save_all(
        &self,
        objects: &[HeapObject],
        symbols: &[String],
        globals: &std::collections::HashMap<u32, Value>,
        operatives: &std::collections::HashSet<u32>,
        closure_chunks: &[crate::lang::compiler::ClosureDesc],
    ) -> Result<(), String> {
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;

        // clear old data
        self.objects.clear(&mut wtxn).map_err(|e| format!("clear: {e}"))?;
        self.meta.clear(&mut wtxn).map_err(|e| format!("clear meta: {e}"))?;

        // write objects
        for (i, obj) in objects.iter().enumerate() {
            let bytes = bincode::serialize(obj).map_err(|e| format!("serialize obj: {e}"))?;
            self.objects.put(&mut wtxn, &(i as u32), &bytes)
                .map_err(|e| format!("put obj: {e}"))?;
        }

        // write symbols
        let sym_bytes = bincode::serialize(symbols).map_err(|e| format!("serialize syms: {e}"))?;
        self.meta.put(&mut wtxn, "symbols", &sym_bytes)
            .map_err(|e| format!("put syms: {e}"))?;

        // write globals as Vec<(u32, u64)>
        let globals_vec: Vec<(u32, u64)> = globals.iter()
            .map(|(&k, &v)| (k, v.to_bits()))
            .collect();
        let glob_bytes = bincode::serialize(&globals_vec).map_err(|e| format!("serialize globals: {e}"))?;
        self.meta.put(&mut wtxn, "globals", &glob_bytes)
            .map_err(|e| format!("put globals: {e}"))?;

        // write operatives as Vec<u32>
        let ops_vec: Vec<u32> = operatives.iter().copied().collect();
        let ops_bytes = bincode::serialize(&ops_vec).map_err(|e| format!("serialize ops: {e}"))?;
        self.meta.put(&mut wtxn, "operatives", &ops_bytes)
            .map_err(|e| format!("put ops: {e}"))?;

        // write closure descs (bytecode chunks + metadata)
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
            }
        }).collect();
        let desc_bytes = bincode::serialize(&desc_data).map_err(|e| format!("serialize descs: {e}"))?;
        self.meta.put(&mut wtxn, "closure_descs", &desc_bytes)
            .map_err(|e| format!("put descs: {e}"))?;

        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    /// Load everything from LMDB. Returns None if empty.
    pub fn load_all(&self) -> Option<LoadedImage> {
        let rtxn = self.env.read_txn().ok()?;

        // read symbols first (needed for everything else)
        let sym_bytes = self.meta.get(&rtxn, "symbols").ok()??;
        let symbols: Vec<String> = bincode::deserialize(sym_bytes).ok()?;

        // read objects
        let mut objects = Vec::new();
        let iter = self.objects.iter(&rtxn).ok()?;
        for item in iter {
            let (_, bytes) = item.ok()?;
            let obj: HeapObject = bincode::deserialize(bytes).ok()?;
            objects.push(obj);
        }
        if objects.is_empty() { return None; }

        // read globals
        let glob_bytes = self.meta.get(&rtxn, "globals").ok()??;
        let globals_vec: Vec<(u32, u64)> = bincode::deserialize(glob_bytes).ok()?;
        let globals: std::collections::HashMap<u32, Value> = globals_vec.into_iter()
            .map(|(k, v)| (k, Value::from_bits(v)))
            .collect();

        // read operatives
        let ops_bytes = self.meta.get(&rtxn, "operatives").ok()??;
        let operatives_vec: Vec<u32> = bincode::deserialize(ops_bytes).ok()?;
        let operatives: std::collections::HashSet<u32> = operatives_vec.into_iter().collect();

        // read closure descs
        let desc_bytes = self.meta.get(&rtxn, "closure_descs").ok()??;
        let descs: Vec<SerializableClosureDesc> = bincode::deserialize(desc_bytes).ok()?;

        rtxn.commit().ok()?;

        Some(LoadedImage { objects, symbols, globals, operatives, closure_descs: descs })
    }
}

pub struct LoadedImage {
    pub objects: Vec<HeapObject>,
    pub symbols: Vec<String>,
    pub globals: std::collections::HashMap<u32, Value>,
    pub operatives: std::collections::HashSet<u32>,
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
}
