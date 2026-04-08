//! Object store: LMDB-backed persistent heap.
//!
//! Every object is a key-value entry:
//!   key:   u32 object ID (big-endian for LMDB ordering)
//!   value: bincode-serialized Object
//!
//! The symbol table is a separate LMDB database in the same environment.
//! Strings are objects too (stored as a special object variant).

use std::path::Path;

use heed::types::*;
use heed::{Database, Env, EnvOpenOptions};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::value::Value;

/// A heap object. Four variants — that's it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// General object: parent + slots + handlers.
    Object {
        parent: u64, // Value bits
        slots: SmallVec<[(u32, u64); 4]>,    // (symbol_id, value_bits)
        handlers: SmallVec<[(u32, u64); 4]>,  // (selector_id, handler_value_bits)
    },
    /// Cons cell: car + cdr.
    Cons {
        car: u64,
        cdr: u64,
    },
    /// String data.
    Str(String),
    /// Opaque bytes (bytecode, compiled code, images, whatever).
    Bytes(Vec<u8>),
}

/// The object store. Wraps an LMDB environment with two databases:
/// - `objects`: u32 → HeapObject
/// - `symbols`: u32 → String (and reverse: String → u32)
pub struct Store {
    env: Env,
    objects: Database<U32<heed::byteorder::BE>, Bytes>,
    symbols: Database<U32<heed::byteorder::BE>, Str>,
    sym_reverse: Database<Str, U32<heed::byteorder::BE>>,
    next_id: u32,
    next_sym: u32,
}

impl Store {
    /// Open or create a store at the given directory path.
    pub fn open(path: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(1024 * 1024 * 1024) // 1 GB max
                .max_dbs(3)
                .open(path)
                .map_err(|e| format!("lmdb open: {e}"))?
        };

        let mut wtxn = env.write_txn().map_err(|e| format!("txn: {e}"))?;
        let objects = env
            .create_database(&mut wtxn, Some("objects"))
            .map_err(|e| format!("create db: {e}"))?;
        let symbols = env
            .create_database(&mut wtxn, Some("symbols"))
            .map_err(|e| format!("create db: {e}"))?;
        let sym_reverse = env
            .create_database(&mut wtxn, Some("sym_reverse"))
            .map_err(|e| format!("create db: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;

        // find the next free IDs
        let rtxn = env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let next_id = objects
            .last(&rtxn)
            .map_err(|e| format!("last: {e}"))?
            .map(|(k, _)| k + 1)
            .unwrap_or(0);
        let next_sym = symbols
            .last(&rtxn)
            .map_err(|e| format!("last: {e}"))?
            .map(|(k, _)| k + 1)
            .unwrap_or(0);
        rtxn.commit().map_err(|e| format!("commit: {e}"))?;

        Ok(Store {
            env,
            objects,
            symbols,
            sym_reverse,
            next_id,
            next_sym,
        })
    }

    /// Create a fresh in-memory store (for tests).
    pub fn in_memory() -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("tmpdir: {e}"))?;
        Self::open(dir.path())
    }

    // ── object operations ──

    /// Allocate a new object, return its ID.
    pub fn alloc(&mut self, obj: HeapObject) -> Result<u32, String> {
        let id = self.next_id;
        self.next_id += 1;
        let bytes = bincode::serialize(&obj).map_err(|e| format!("serialize: {e}"))?;
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;
        self.objects
            .put(&mut wtxn, &id, &bytes)
            .map_err(|e| format!("put: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(id)
    }

    /// Read an object by ID.
    pub fn get(&self, id: u32) -> Result<HeapObject, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let bytes = self
            .objects
            .get(&rtxn, &id)
            .map_err(|e| format!("get: {e}"))?
            .ok_or_else(|| format!("no object with id {id}"))?;
        bincode::deserialize(bytes).map_err(|e| format!("deserialize: {e}"))
    }

    /// Update an object in place.
    pub fn put(&mut self, id: u32, obj: HeapObject) -> Result<(), String> {
        let bytes = bincode::serialize(&obj).map_err(|e| format!("serialize: {e}"))?;
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;
        self.objects
            .put(&mut wtxn, &id, &bytes)
            .map_err(|e| format!("put: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    // ── convenience object operations ──

    /// Get a slot value from an object.
    pub fn slot_get(&self, id: u32, slot: u32) -> Result<Value, String> {
        match self.get(id)? {
            HeapObject::Object { slots, .. } => {
                for &(k, v) in &slots {
                    if k == slot {
                        return Ok(Value::from_bits(v));
                    }
                }
                Ok(Value::NIL)
            }
            _ => Err("not a general object".into()),
        }
    }

    /// Set a slot value on an object.
    pub fn slot_set(&mut self, id: u32, slot: u32, val: Value) -> Result<(), String> {
        let mut obj = self.get(id)?;
        match &mut obj {
            HeapObject::Object { slots, .. } => {
                for entry in slots.iter_mut() {
                    if entry.0 == slot {
                        entry.1 = val.to_bits();
                        self.put(id, obj)?;
                        return Ok(());
                    }
                }
                slots.push((slot, val.to_bits()));
                self.put(id, obj)?;
                Ok(())
            }
            _ => Err("not a general object".into()),
        }
    }

    /// Get a handler value from an object.
    pub fn handler_get(&self, id: u32, selector: u32) -> Result<Option<Value>, String> {
        match self.get(id)? {
            HeapObject::Object { handlers, .. } => {
                for &(k, v) in &handlers {
                    if k == selector {
                        return Ok(Some(Value::from_bits(v)));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Set a handler on an object.
    pub fn handler_set(
        &mut self,
        id: u32,
        selector: u32,
        handler: Value,
    ) -> Result<(), String> {
        let mut obj = self.get(id)?;
        match &mut obj {
            HeapObject::Object { handlers, .. } => {
                for entry in handlers.iter_mut() {
                    if entry.0 == selector {
                        entry.1 = handler.to_bits();
                        self.put(id, obj)?;
                        return Ok(());
                    }
                }
                handlers.push((selector, handler.to_bits()));
                self.put(id, obj)?;
                Ok(())
            }
            _ => Err("not a general object: cannot set handler".into()),
        }
    }

    /// Get the parent of an object.
    pub fn parent(&self, id: u32) -> Result<Value, String> {
        match self.get(id)? {
            HeapObject::Object { parent, .. } => Ok(Value::from_bits(parent)),
            _ => Ok(Value::NIL),
        }
    }

    // ── cons operations ──

    pub fn cons(&mut self, car: Value, cdr: Value) -> Result<u32, String> {
        self.alloc(HeapObject::Cons {
            car: car.to_bits(),
            cdr: cdr.to_bits(),
        })
    }

    pub fn car(&self, id: u32) -> Result<Value, String> {
        match self.get(id)? {
            HeapObject::Cons { car, .. } => Ok(Value::from_bits(car)),
            _ => Err("not a cons cell".into()),
        }
    }

    pub fn cdr(&self, id: u32) -> Result<Value, String> {
        match self.get(id)? {
            HeapObject::Cons { cdr, .. } => Ok(Value::from_bits(cdr)),
            _ => Err("not a cons cell".into()),
        }
    }

    // ── string operations ──

    pub fn alloc_string(&mut self, s: &str) -> Result<u32, String> {
        self.alloc(HeapObject::Str(s.to_string()))
    }

    pub fn get_string_owned(&self, id: u32) -> Result<String, String> {
        match self.get(id)? {
            HeapObject::Str(s) => Ok(s),
            _ => Err("not a string".into()),
        }
    }

    // ── bytes operations ──

    pub fn alloc_bytes(&mut self, data: Vec<u8>) -> Result<u32, String> {
        self.alloc(HeapObject::Bytes(data))
    }

    pub fn get_bytes(&self, id: u32) -> Result<Vec<u8>, String> {
        match self.get(id)? {
            HeapObject::Bytes(b) => Ok(b),
            _ => Err("not bytes".into()),
        }
    }

    // ── symbol table ──

    /// Intern a symbol, returning its ID. Idempotent.
    pub fn intern(&mut self, name: &str) -> Result<u32, String> {
        // check reverse lookup first
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        if let Some(id) = self
            .sym_reverse
            .get(&rtxn, name)
            .map_err(|e| format!("get: {e}"))?
        {
            rtxn.commit().map_err(|e| format!("commit: {e}"))?;
            return Ok(id);
        }
        rtxn.commit().map_err(|e| format!("commit: {e}"))?;

        let id = self.next_sym;
        self.next_sym += 1;
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;
        self.symbols
            .put(&mut wtxn, &id, name)
            .map_err(|e| format!("put: {e}"))?;
        self.sym_reverse
            .put(&mut wtxn, name, &id)
            .map_err(|e| format!("put: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(id)
    }

    /// Look up a symbol name by ID.
    pub fn symbol_name(&self, id: u32) -> Result<String, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let name = self
            .symbols
            .get(&rtxn, &id)
            .map_err(|e| format!("get: {e}"))?
            .ok_or_else(|| format!("no symbol with id {id}"))?
            .to_string();
        rtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(name)
    }

    /// Make a new empty general object with the given parent.
    pub fn make_object(&mut self, parent: Value) -> Result<u32, String> {
        self.alloc(HeapObject::Object {
            parent: parent.to_bits(),
            slots: SmallVec::new(),
            handlers: SmallVec::new(),
        })
    }
}
