// Content-addressed blob store + named refs.
//
// Two lmdb databases in one env:
//   blobs — key: 32-byte blake3 hash, value: canonical bytes
//   refs  — key: utf8 name, value: 32-byte hash pointing into blobs
//
// This is the substrate for image persistence (see docs/persistence.md).
// Anything immutable becomes a blob. Mutable heads — the current
// env, the closure-desc vec, capability roots — live as refs pointing
// at the blob that is their current content.
//
// Dedup is automatic: put_blob checks has_blob first, skips if the
// hash is already there. A 10mb image that shares a 1mb sublist
// writes the sublist once.
//
// Atomicity: one lmdb write txn = one committed update. Save walks
// every reachable blob, writes them all, then writes the refs, all
// in one txn. No torn writes.

use heed::types::*;
use heed::{Database, Env, EnvOpenOptions};
use moof_core::canonical::{Hash, hash_hex,
    VTAG_INT, VTAG_FLOAT, VTAG_NIL, VTAG_TRUE, VTAG_FALSE, VTAG_SYMBOL, VTAG_BLOB,
    BTAG_CONS, BTAG_TEXT, BTAG_BYTES, BTAG_TABLE, BTAG_GENERAL, BTAG_FOREIGN};
use moof_core::heap::Heap;
use moof_core::object::HeapObject;
use moof_core::source::ClosureSource;
use moof_core::Value;
use moof_lang::lang::compiler::ClosureDesc;
use moof_lang::opcodes::Chunk;
use std::collections::HashMap;
use std::path::Path;

/// Full image snapshot: the three heads System cares about,
/// produced by `BlobStore::load_snapshot` and consumed by
/// `System::try_load_into`.
pub struct Snapshot {
    pub type_protos: Vec<Value>,
    pub closure_descs: Vec<ClosureDesc>,
    pub env: Value,
}

pub struct BlobStore {
    env: Env,
    blobs: Database<Bytes, Bytes>,  // hash bytes → canonical bytes
    refs: Database<Str, Bytes>,     // name → hash bytes
}

impl BlobStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(1024 * 1024 * 1024)  // 1 GB — content-addressed stores grow slowly
                .max_dbs(2)
                .open(path)
                .map_err(|e| format!("lmdb open: {e}"))?
        };
        let mut wtxn = env.write_txn().map_err(|e| format!("txn: {e}"))?;
        let blobs = env.create_database(&mut wtxn, Some("blobs"))
            .map_err(|e| format!("create blobs db: {e}"))?;
        let refs = env.create_database(&mut wtxn, Some("refs"))
            .map_err(|e| format!("create refs db: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(BlobStore { env, blobs, refs })
    }

    // ─────────── low-level blob api ───────────

    /// Read a blob by hash. None if not present.
    pub fn get_blob(&self, hash: &Hash) -> Result<Option<Vec<u8>>, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let out = self.blobs.get(&rtxn, hash.as_slice())
            .map_err(|e| format!("get blob: {e}"))?
            .map(|b| b.to_vec());
        Ok(out)
    }

    pub fn has_blob(&self, hash: &Hash) -> Result<bool, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let exists = self.blobs.get(&rtxn, hash.as_slice())
            .map_err(|e| format!("get blob: {e}"))?
            .is_some();
        Ok(exists)
    }

    // ─────────── low-level ref api ───────────

    pub fn get_ref(&self, name: &str) -> Result<Option<Hash>, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let out = self.refs.get(&rtxn, name)
            .map_err(|e| format!("get ref: {e}"))?
            .map(|bytes| {
                let mut h = [0u8; 32];
                h.copy_from_slice(bytes);
                h
            });
        Ok(out)
    }

    /// List all refs in the store, sorted by name. Useful for
    /// inspection (`[blobstore refs]`-style queries, eventual
    /// snapshots listing).
    pub fn list_refs(&self) -> Result<Vec<(String, Hash)>, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let mut out = Vec::new();
        let iter = self.refs.iter(&rtxn).map_err(|e| format!("iter refs: {e}"))?;
        for item in iter {
            let (name, bytes) = item.map_err(|e| format!("iter refs: {e}"))?;
            let mut h = [0u8; 32];
            h.copy_from_slice(bytes);
            out.push((name.to_string(), h));
        }
        Ok(out)
    }

    // ─────────── high-level save/load for a heap value ───────────

    /// Store every blob reachable from `val` and return its blob hash.
    /// `val` MUST be a heap-allocated value (primitives are never
    /// blob-stored; they're inlined inside their containing blob).
    /// Dedup is automatic: same content → same hash → same key.
    /// One write txn for atomicity.
    ///
    /// Also writes additional name→hash refs in the same txn (use
    /// this for roots like `roots.env`, `roots.closure-descs`).
    pub fn save_value(
        &self,
        heap: &Heap,
        val: Value,
        extra_refs: &[(&str, Hash)],
    ) -> Result<Hash, String> {
        let root_id = val.as_any_object()
            .ok_or("save_value: primitives aren't blob-addressable")?;
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;

        // cycle-placeholder blob for back-edges.
        let placeholder_bytes = moof_core::cycle_placeholder_blob_bytes();
        let placeholder_hash = moof_core::cycle_placeholder();
        if self.blobs.get(&wtxn, placeholder_hash.as_slice())
            .map_err(|e| format!("get placeholder: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, placeholder_hash.as_slice(), &placeholder_bytes)
                .map_err(|e| format!("put placeholder: {e}"))?;
        }

        // collect reachable, then iteratively compute every blob's
        // canonical hash via fixpoint. naive per-object hashing is
        // path-dependent for cyclic graphs — two reachers of the
        // same object encode different parent blobs with different
        // sub-hashes, so the stored sub-blob's hash doesn't match
        // what the parent's encoding references. fixpoint fixes this
        // by using a stable table: every round, each id's encoding
        // emits sub-refs using the table; round-end the table is
        // updated with the new hash per id; iterate till stable.
        let reachable: Vec<u32> = heap.reachable_objects(val).into_iter().collect();
        let hashes = compute_hash_table(heap, &reachable);

        // write every (hash, bytes) — bytes computed using the final
        // table, guaranteed consistent with any sub-ref inside any
        // other blob's bytes.
        for obj_id in &reachable {
            let bytes = canonical_blob_bytes_using_table(heap, *obj_id, &hashes);
            let hash = *hashes.get(obj_id).expect("id not in hash table");
            if self.blobs.get(&wtxn, hash.as_slice())
                .map_err(|e| format!("get blob: {e}"))?.is_none()
            {
                self.blobs.put(&mut wtxn, hash.as_slice(), &bytes)
                    .map_err(|e| format!("put blob: {e}"))?;
            }
        }

        // the root's final blob hash.
        let root = *hashes.get(&root_id).expect("root not in hash table");

        // extra refs — one put each.
        for (name, hash) in extra_refs {
            self.refs.put(&mut wtxn, name, hash.as_slice())
                .map_err(|e| format!("put ref: {e}"))?;
        }

        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(root)
    }

    /// Write a single ref (outside save_value's txn). For incremental
    /// updates where the blobs are already stored.
    pub fn put_ref(&self, name: &str, hash: &Hash) -> Result<(), String> {
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;
        self.refs.put(&mut wtxn, name, hash.as_slice())
            .map_err(|e| format!("put ref: {e}"))?;
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    /// Number of blobs in the store.
    pub fn blob_count(&self) -> Result<u64, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let n = self.blobs.len(&rtxn).map_err(|e| format!("len: {e}"))?;
        Ok(n)
    }

    // ─────────── atomic snapshot: env + descs + type_protos ───────────

    /// Save a full image snapshot in one lmdb txn: env value, closure
    /// descs, type-prototype table. Writes `roots.env`,
    /// `roots.closure-descs`, `roots.type-protos` atomically. On any
    /// error the txn aborts; no partial state is committed.
    ///
    /// This is the preferred save entry. The earlier `save_value` and
    /// `save_closure_descs` methods each use their own txn — callable
    /// but non-atomic if you need to coordinate several roots.
    pub fn save_snapshot(
        &self,
        heap: &Heap,
        env_val: Value,
        descs: &[ClosureDesc],
        type_protos: &[Value],
    ) -> Result<(), String> {
        let env_id = env_val.as_any_object()
            .ok_or("save_snapshot: env must be a heap object")?;

        // ── fixpoint-hash EVERY reachable id across env + descs +
        //    type_protos in one pass, so sub-refs resolve consistently
        //    no matter which root first touched them.
        let mut reach_set: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for id in heap.reachable_objects(env_val) { reach_set.insert(id); }
        for d in descs {
            for bits in &d.chunk.constants {
                let v = Value::from_bits(*bits);
                for id in heap.reachable_objects(v) { reach_set.insert(id); }
            }
            for v in &d.capture_values {
                for id in heap.reachable_objects(*v) { reach_set.insert(id); }
            }
        }
        for v in type_protos {
            for id in heap.reachable_objects(*v) { reach_set.insert(id); }
        }
        let reach: Vec<u32> = reach_set.into_iter().collect();
        let hashes = compute_hash_table(heap, &reach);

        // ── one write txn for everything ──
        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;

        // cycle placeholder blob — needed if any object references a
        // cycle back-edge.
        let placeholder_bytes = moof_core::cycle_placeholder_blob_bytes();
        let placeholder_hash = moof_core::cycle_placeholder();
        if self.blobs.get(&wtxn, placeholder_hash.as_slice())
            .map_err(|e| format!("get placeholder: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, placeholder_hash.as_slice(), &placeholder_bytes)
                .map_err(|e| format!("put placeholder: {e}"))?;
        }

        // write each reachable heap object's blob.
        for id in &reach {
            let bytes = canonical_blob_bytes_using_table(heap, *id, &hashes);
            let h = *hashes.get(id).expect("id not in hash table");
            if self.blobs.get(&wtxn, h.as_slice())
                .map_err(|e| format!("get: {e}"))?.is_none()
            {
                self.blobs.put(&mut wtxn, h.as_slice(), &bytes)
                    .map_err(|e| format!("put: {e}"))?;
            }
        }

        // closure-descs blob.
        let descs_bytes = encode_closure_descs_with(heap, descs, &hashes);
        let descs_hash: Hash = blake3::hash(&descs_bytes).into();
        if self.blobs.get(&wtxn, descs_hash.as_slice())
            .map_err(|e| format!("get descs: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, descs_hash.as_slice(), &descs_bytes)
                .map_err(|e| format!("put descs: {e}"))?;
        }

        // type-protos blob: u32 count + canonical value bytes per entry.
        let type_protos_bytes = encode_type_protos_with(heap, type_protos, &hashes);
        let type_protos_hash: Hash = blake3::hash(&type_protos_bytes).into();
        if self.blobs.get(&wtxn, type_protos_hash.as_slice())
            .map_err(|e| format!("get type-protos: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, type_protos_hash.as_slice(), &type_protos_bytes)
                .map_err(|e| format!("put type-protos: {e}"))?;
        }

        // root refs.
        let env_hash = *hashes.get(&env_id).expect("env root not hashed");
        self.refs.put(&mut wtxn, "roots.env", env_hash.as_slice())
            .map_err(|e| format!("put roots.env: {e}"))?;
        self.refs.put(&mut wtxn, "roots.closure-descs", descs_hash.as_slice())
            .map_err(|e| format!("put roots.closure-descs: {e}"))?;
        self.refs.put(&mut wtxn, "roots.type-protos", type_protos_hash.as_slice())
            .map_err(|e| format!("put roots.type-protos: {e}"))?;

        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    /// Load the type_protos Vec from a stored blob, reconstructing
    /// any referenced heap objects into `heap` along the way.
    pub fn load_type_protos(&self, hash: &Hash, heap: &mut Heap) -> Result<Vec<Value>, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let bytes = self.blobs.get(&rtxn, hash.as_slice())
            .map_err(|e| format!("get: {e}"))?
            .ok_or_else(|| format!("type-protos blob missing: {}", hash_hex(hash)))?
            .to_vec();
        let mut memo: HashMap<Hash, Value> = HashMap::new();
        decode_type_protos(self, &rtxn, &bytes, heap, &mut memo)
    }

    /// Load a full image snapshot: type-protos + closure-descs + env,
    /// sharing one hash→Value memo across all three decodes so the
    /// same blob hash resolves to the same heap id everywhere. This
    /// is crucial: if the three loads each had their own memo, a
    /// prototype reached via both type_protos and env would land as
    /// TWO distinct heap objects, and closure dispatch (which
    /// compares against type_protos[PROTO_CLOSURE]) would fail.
    ///
    /// Returns (type_protos_vec, closure_descs_vec, env_value) on
    /// success; caller installs each where it belongs. Returns None
    /// if any of the roots is missing (no image / partial snapshot).
    pub fn load_snapshot(
        &self,
        heap: &mut Heap,
    ) -> Result<Option<Snapshot>, String> {
        let Some(env_hash) = self.get_ref("roots.env")? else { return Ok(None); };
        let Some(descs_hash) = self.get_ref("roots.closure-descs")? else { return Ok(None); };
        let type_protos_hash = self.get_ref("roots.type-protos")?;

        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let mut memo: HashMap<Hash, Value> = HashMap::new();

        // (1) type-protos first — so anything that references them
        // (handlers, protos in env) gets the right shared instance.
        let type_protos = if let Some(h) = type_protos_hash {
            let bytes = self.blobs.get(&rtxn, h.as_slice())
                .map_err(|e| format!("get type-protos blob: {e}"))?
                .ok_or_else(|| format!("type-protos blob missing: {}", hash_hex(&h)))?
                .to_vec();
            decode_type_protos(self, &rtxn, &bytes, heap, &mut memo)?
        } else {
            Vec::new()
        };

        // (2) closure-descs.
        let descs_bytes = self.blobs.get(&rtxn, descs_hash.as_slice())
            .map_err(|e| format!("get descs blob: {e}"))?
            .ok_or_else(|| format!("closure-descs blob missing: {}", hash_hex(&descs_hash)))?
            .to_vec();
        let descs = decode_closure_descs(self, &rtxn, &descs_bytes, heap, &mut memo)?;

        // (3) env.
        let env = load_blob(self, &rtxn, &env_hash, heap, &mut memo)?;

        Ok(Some(Snapshot { type_protos, closure_descs: descs, env }))
    }

    // ─────────── load path ───────────

    /// Load a heap value by its root hash. Reconstructs heap objects
    /// in `heap`, returns the Value at the root. Shared sub-blobs
    /// restore as shared values (dedup is preserved via a hash→Value
    /// memo built during the load).
    pub fn load_value(&self, hash: &Hash, heap: &mut Heap) -> Result<Value, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let mut memo: HashMap<Hash, Value> = HashMap::new();
        load_blob(self, &rtxn, hash, heap, &mut memo)
    }

    // ─────────── closure-desc persistence ───────────

    /// Encode the whole closure_descs vec as a single blob, store it,
    /// and return the blob's hash. The blob also contains any value
    /// constants inside the descs — those are encoded canonically so
    /// they share with the rest of the heap's blobs where possible.
    pub fn save_closure_descs(
        &self,
        heap: &Heap,
        descs: &[ClosureDesc],
    ) -> Result<Hash, String> {
        // Collect every reachable heap id via desc constants + captures.
        let mut reach_set = std::collections::HashSet::new();
        for d in descs {
            for bits in &d.chunk.constants {
                let v = Value::from_bits(*bits);
                for id in heap.reachable_objects(v) { reach_set.insert(id); }
            }
            for v in &d.capture_values {
                for id in heap.reachable_objects(*v) { reach_set.insert(id); }
            }
        }
        let reach: Vec<u32> = reach_set.into_iter().collect();
        // fixpoint-hash every reachable id — same technique as
        // save_value. needed so any sub-refs inside desc constants
        // use path-independent hashes matching what we store.
        let hashes = compute_hash_table(heap, &reach);

        let mut wtxn = self.env.write_txn().map_err(|e| format!("txn: {e}"))?;

        // placeholder blob.
        let placeholder_bytes = moof_core::cycle_placeholder_blob_bytes();
        let placeholder_hash = moof_core::cycle_placeholder();
        if self.blobs.get(&wtxn, placeholder_hash.as_slice())
            .map_err(|e| format!("get placeholder: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, placeholder_hash.as_slice(), &placeholder_bytes)
                .map_err(|e| format!("put placeholder: {e}"))?;
        }

        // write each reachable blob under its final hash.
        for id in &reach {
            let bytes = canonical_blob_bytes_using_table(heap, *id, &hashes);
            let h = *hashes.get(id).expect("id not in hash table");
            if self.blobs.get(&wtxn, h.as_slice())
                .map_err(|e| format!("get: {e}"))?.is_none()
            {
                self.blobs.put(&mut wtxn, h.as_slice(), &bytes)
                    .map_err(|e| format!("put: {e}"))?;
            }
        }

        // encode the descs themselves; sub-value refs inside go
        // through the same hash table for consistency with stored blobs.
        let bytes = encode_closure_descs_with(heap, descs, &hashes);
        let h: Hash = blake3::hash(&bytes).into();
        if self.blobs.get(&wtxn, h.as_slice())
            .map_err(|e| format!("get: {e}"))?.is_none()
        {
            self.blobs.put(&mut wtxn, h.as_slice(), &bytes)
                .map_err(|e| format!("put: {e}"))?;
        }
        wtxn.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(h)
    }

    /// Rehydrate a closure_descs vec from a stored blob. Value
    /// constants and capture values are restored into `heap`.
    pub fn load_closure_descs(
        &self,
        hash: &Hash,
        heap: &mut Heap,
    ) -> Result<Vec<ClosureDesc>, String> {
        let rtxn = self.env.read_txn().map_err(|e| format!("txn: {e}"))?;
        let bytes = self.blobs.get(&rtxn, hash.as_slice())
            .map_err(|e| format!("get: {e}"))?
            .ok_or_else(|| format!("closure-descs blob missing: {}", hash_hex(hash)))?
            .to_vec();
        let mut memo: HashMap<Hash, Value> = HashMap::new();
        decode_closure_descs(self, &rtxn, &bytes, heap, &mut memo)
    }
}

// ─────────── fixpoint canonical hashing ───────────
//
// Direct canonical hashing has a cycle problem: the hash of X's blob
// depends on whether X's sub-ref Y is encoded normally or as a
// cycle-placeholder (which happens if Y's encoding transitively loops
// back into X). Different walks of the graph treat different edges
// as back-edges, producing different hashes for the same object.
//
// Fixpoint fixes this: every object starts with placeholder hash;
// each round re-encodes using the current table; iterate till nothing
// changes. All objects in the graph converge to canonical hashes
// that are path-independent and stable.
//
// Acyclic graphs converge in `depth` rounds. Cyclic graphs take one
// more round after their components stabilize. For typical moof
// images (~1000 reachable objects), this is millisecond-scale.

type HashTable = std::collections::HashMap<u32, Hash>;

fn compute_hash_table(heap: &Heap, ids: &[u32]) -> HashTable {
    let mut table: HashTable = ids.iter().copied()
        .map(|id| (id, moof_core::cycle_placeholder()))
        .collect();
    // sentinel: how many rounds with zero changes to conclude fixpoint
    let mut rounds_without_change = 0;
    // safety bound — we should converge in a handful of rounds
    for _ in 0..1024 {
        let mut changed = false;
        for id in ids {
            let bytes = canonical_blob_bytes_using_table(heap, *id, &table);
            let new_hash: Hash = blake3::hash(&bytes).into();
            if table.get(id).copied().unwrap() != new_hash {
                table.insert(*id, new_hash);
                changed = true;
            }
        }
        if !changed {
            rounds_without_change += 1;
            if rounds_without_change >= 1 { break; }
        } else {
            rounds_without_change = 0;
        }
    }
    table
}

fn canonical_value_bytes_using_table(heap: &Heap, val: Value, table: &HashTable) -> Vec<u8> {
    if val.is_nil()   { return vec![VTAG_NIL]; }
    if val.is_true()  { return vec![VTAG_TRUE]; }
    if val.is_false() { return vec![VTAG_FALSE]; }

    if let Some(i) = val.as_integer() {
        let mut out = Vec::with_capacity(9);
        out.push(VTAG_INT);
        out.extend_from_slice(&i.to_be_bytes());
        return out;
    }
    if let Some(f) = val.as_float() {
        let mut out = Vec::with_capacity(9);
        out.push(VTAG_FLOAT);
        out.extend_from_slice(&f.to_bits().to_be_bytes());
        return out;
    }
    if let Some(sym_id) = val.as_symbol() {
        let name = heap.symbol_name(sym_id);
        let name_bytes = name.as_bytes();
        let mut out = Vec::with_capacity(5 + name_bytes.len());
        out.push(VTAG_SYMBOL);
        out.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(name_bytes);
        return out;
    }
    if let Some(obj_id) = val.as_any_object() {
        // use the table — may be placeholder in early rounds.
        let hash = table.get(&obj_id).copied()
            .unwrap_or_else(|| moof_core::cycle_placeholder());
        let mut out = Vec::with_capacity(33);
        out.push(VTAG_BLOB);
        out.extend_from_slice(&hash);
        return out;
    }
    vec![VTAG_NIL]
}

fn canonical_blob_bytes_using_table(heap: &Heap, obj_id: u32, table: &HashTable) -> Vec<u8> {
    let val = Value::nursery(obj_id);
    // fast paths for known foreign types — replicate Heap's
    // canonical_blob_bytes structure, but using the table for
    // sub-refs instead of recursive hash_blob.

    if heap.is_pair(val) {
        let (car, cdr) = heap.pair_of(obj_id).unwrap_or((Value::NIL, Value::NIL));
        let mut out = vec![BTAG_CONS];
        out.extend(canonical_value_bytes_using_table(heap, car, table));
        out.extend(canonical_value_bytes_using_table(heap, cdr, table));
        return out;
    }
    if heap.is_text(val) {
        // text has no sub-refs; canonical_blob_bytes is already
        // path-independent, just call it.
        return heap.canonical_blob_bytes(obj_id);
    }
    if heap.is_bytes(val) {
        return heap.canonical_blob_bytes(obj_id);
    }
    if heap.is_table(val) {
        // walk sub-values via table
        let t = heap.foreign_ref::<moof_core::heap::Table>(val);
        if let Some(t) = t {
            let mut out = vec![BTAG_TABLE];
            out.extend_from_slice(&(t.seq.len() as u32).to_be_bytes());
            for v in &t.seq {
                out.extend(canonical_value_bytes_using_table(heap, *v, table));
            }
            let mut entries: Vec<(Vec<u8>, Value)> = t.map.iter()
                .map(|(k, v)| (canonical_value_bytes_using_table(heap, *k, table), *v))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
            for (kb, v) in entries {
                out.extend(kb);
                out.extend(canonical_value_bytes_using_table(heap, v, table));
            }
            return out;
        }
        return vec![BTAG_TABLE, 0, 0, 0, 0, 0, 0, 0, 0];
    }

    // generic foreign (no sub-refs inside the payload beyond what
    // trace visits — serialize deterministically via the vtable).
    if heap.get(obj_id).foreign.is_some() {
        return heap.canonical_blob_bytes(obj_id);
    }

    // general object: proto + sorted slots + sorted handlers.
    let obj = heap.get(obj_id);
    let proto_bytes = canonical_value_bytes_using_table(heap, obj.proto, table);
    let mut slot_entries: Vec<(String, Value)> = obj.slot_names.iter()
        .zip(obj.slot_values.iter())
        .map(|(&sym, &v)| (heap.symbol_name(sym).to_string(), v))
        .collect();
    slot_entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut handler_entries: Vec<(String, Value)> = obj.handlers.iter()
        .map(|&(sym, v)| (heap.symbol_name(sym).to_string(), v))
        .collect();
    handler_entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut out = vec![BTAG_GENERAL];
    out.extend(proto_bytes);
    out.extend_from_slice(&(slot_entries.len() as u32).to_be_bytes());
    for (name, v) in &slot_entries {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_be_bytes());
        out.extend_from_slice(nb);
        out.extend(canonical_value_bytes_using_table(heap, *v, table));
    }
    out.extend_from_slice(&(handler_entries.len() as u32).to_be_bytes());
    for (name, v) in &handler_entries {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_be_bytes());
        out.extend_from_slice(nb);
        out.extend(canonical_value_bytes_using_table(heap, *v, table));
    }
    out
}

// ─────────── canonical blob decoders ───────────

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    if *offset + 4 > bytes.len() {
        return Err("truncated u32".into());
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&bytes[*offset..*offset + 4]);
    *offset += 4;
    Ok(u32::from_be_bytes(arr))
}

fn read_len_utf8(bytes: &[u8], offset: &mut usize) -> Result<String, String> {
    let len = read_u32(bytes, offset)? as usize;
    if *offset + len > bytes.len() {
        return Err("truncated utf8".into());
    }
    let s = std::str::from_utf8(&bytes[*offset..*offset + len])
        .map_err(|e| format!("bad utf8: {e}"))?
        .to_string();
    *offset += len;
    Ok(s)
}

fn load_blob(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    hash: &Hash,
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Value, String> {
    if let Some(v) = memo.get(hash) { return Ok(*v); }

    let bytes = store.blobs.get(rtxn, hash.as_slice())
        .map_err(|e| format!("get blob: {e}"))?
        .ok_or_else(|| format!("blob missing: {}", hash_hex(hash)))?
        .to_vec();

    if bytes.is_empty() { return Err("empty blob".into()); }
    let tag = bytes[0];

    // For General objects, pre-allocate an empty placeholder and
    // insert into memo BEFORE recursing — this breaks cycles in the
    // blob DAG (e.g. closure ↔ proto chain). The recursive decode
    // then FILLS IN the pre-allocated id.
    if tag == BTAG_GENERAL {
        let placeholder_id = heap.alloc_val(HeapObject::new_empty(Value::NIL));
        let placeholder_val = Value::nursery(placeholder_id.as_any_object().unwrap());
        memo.insert(*hash, placeholder_val);
        // decode the bytes and POPULATE the placeholder in place
        // rather than allocating a fresh object.
        decode_general_into(placeholder_id.as_any_object().unwrap(), store, rtxn, &bytes, heap, memo)?;
        return Ok(placeholder_val);
    }

    let val = match tag {
        BTAG_CONS    => decode_cons(store, rtxn, &bytes, heap, memo)?,
        BTAG_TEXT    => decode_text(&bytes, heap)?,
        BTAG_BYTES   => decode_bytes(&bytes, heap)?,
        BTAG_TABLE   => decode_table(store, rtxn, &bytes, heap, memo)?,
        BTAG_FOREIGN => decode_foreign(&bytes, heap)?,
        _ => return Err(format!("unknown blob tag: 0x{:02x}", tag)),
    };
    memo.insert(*hash, val);
    Ok(val)
}

/// Decode a Value (inline primitive or blob-ref).
fn decode_value(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    offset: &mut usize,
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Value, String> {
    if *offset >= bytes.len() { return Err("truncated value".into()); }
    let tag = bytes[*offset];
    *offset += 1;
    match tag {
        VTAG_NIL   => Ok(Value::NIL),
        VTAG_TRUE  => Ok(Value::boolean(true)),
        VTAG_FALSE => Ok(Value::boolean(false)),
        VTAG_INT => {
            if *offset + 8 > bytes.len() { return Err("truncated int".into()); }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[*offset..*offset + 8]);
            *offset += 8;
            Ok(Value::integer(i64::from_be_bytes(arr)))
        }
        VTAG_FLOAT => {
            if *offset + 8 > bytes.len() { return Err("truncated float".into()); }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[*offset..*offset + 8]);
            *offset += 8;
            Ok(Value::float(f64::from_bits(u64::from_be_bytes(arr))))
        }
        VTAG_SYMBOL => {
            let name = read_len_utf8(bytes, offset)?;
            Ok(Value::symbol(heap.intern(&name)))
        }
        VTAG_BLOB => {
            if *offset + 32 > bytes.len() { return Err("truncated blob-ref".into()); }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&bytes[*offset..*offset + 32]);
            *offset += 32;
            load_blob(store, rtxn, &hash, heap, memo)
        }
        _ => Err(format!("unknown value tag: 0x{:02x}", tag)),
    }
}

fn decode_cons(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Value, String> {
    let mut offset = 1;  // skip tag
    let car = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
    let cdr = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
    Ok(heap.cons(car, cdr))
}

fn decode_text(bytes: &[u8], heap: &mut Heap) -> Result<Value, String> {
    let mut offset = 1;
    let s = read_len_utf8(bytes, &mut offset)?;
    Ok(heap.alloc_string(&s))
}

fn decode_bytes(bytes: &[u8], heap: &mut Heap) -> Result<Value, String> {
    let mut offset = 1;
    let len = read_u32(bytes, &mut offset)? as usize;
    if offset + len > bytes.len() { return Err("truncated bytes payload".into()); }
    let data = bytes[offset..offset + len].to_vec();
    Ok(heap.alloc_bytes(data))
}

fn decode_table(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Value, String> {
    let mut offset = 1;
    let n_seq = read_u32(bytes, &mut offset)? as usize;
    let mut seq = Vec::with_capacity(n_seq);
    for _ in 0..n_seq {
        seq.push(decode_value(store, rtxn, bytes, &mut offset, heap, memo)?);
    }
    let n_map = read_u32(bytes, &mut offset)? as usize;
    let mut map = indexmap::IndexMap::with_capacity(n_map);
    for _ in 0..n_map {
        let k = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
        let v = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
        map.insert(k, v);
    }
    Ok(heap.alloc_table(seq, map))
}

/// Decode a BTAG_GENERAL blob into an already-allocated placeholder
/// object. The placeholder was registered in the memo before this
/// call, so cycles back to the same hash resolve to the same id.
fn decode_general_into(
    target_id: u32,
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<(), String> {
    let mut offset = 1;
    let proto = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;

    let n_slots = read_u32(bytes, &mut offset)? as usize;
    let mut slot_names = Vec::with_capacity(n_slots);
    let mut slot_values = Vec::with_capacity(n_slots);
    for _ in 0..n_slots {
        let name = read_len_utf8(bytes, &mut offset)?;
        let sym = heap.intern(&name);
        let v = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
        slot_names.push(sym);
        slot_values.push(v);
    }

    let n_handlers = read_u32(bytes, &mut offset)? as usize;
    let mut handlers: Vec<(u32, Value)> = Vec::with_capacity(n_handlers);
    for _ in 0..n_handlers {
        let name = read_len_utf8(bytes, &mut offset)?;
        let sym = heap.intern(&name);
        let v = decode_value(store, rtxn, bytes, &mut offset, heap, memo)?;
        handlers.push((sym, v));
    }

    let obj = heap.get_mut(target_id);
    obj.proto = proto;
    obj.slot_names = slot_names;
    obj.slot_values = slot_values;
    obj.handlers = handlers;
    obj.foreign = None;
    Ok(())
}

// ─────────── closure desc codec ───────────
//
// Format (all integers big-endian):
//   u32 n_descs
//   for each desc:
//     u32 code-len + code bytes
//     u32 n-constants + [canonical value bytes per constant]
//     u8 arity
//     u8 num_regs
//     u32 name-len + name utf8
//     u32 n-param-names + [utf8 each]                (symbols by name)
//     u8 _legacy_is_operative (always 0; dispatch is now structural via __underlying)
//     u32 n-cap-names + [utf8 each]
//     u32 n-cap-parent + [u8 each]
//     u32 n-cap-local + [u8 each]
//     u32 n-cap-values + [canonical value bytes]
//     u32 desc_base
//     u8 has_rest_reg + (u8 rest_reg if has)
//     u8 has_source + (if 1: source encoding)
//
// Source encoding (when present):
//   u32 text-len + text utf8
//   u32 label-len + label utf8
//   u64 byte_start
//   u64 byte_end

fn encode_closure_descs_with(heap: &Heap, descs: &[ClosureDesc], table: &HashTable) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(descs.len() as u32).to_be_bytes());
    for d in descs {
        encode_desc_with(heap, d, table, &mut out);
    }
    out
}

fn encode_desc_with(heap: &Heap, d: &ClosureDesc, table: &HashTable, out: &mut Vec<u8>) {
    // code
    out.extend_from_slice(&(d.chunk.code.len() as u32).to_be_bytes());
    out.extend_from_slice(&d.chunk.code);

    // constants — each a canonical Value (using table for heap refs)
    out.extend_from_slice(&(d.chunk.constants.len() as u32).to_be_bytes());
    for bits in &d.chunk.constants {
        let v = Value::from_bits(*bits);
        out.extend(canonical_value_bytes_using_table(heap, v, table));
    }

    out.push(d.chunk.arity);
    out.push(d.chunk.num_regs);

    let name = d.chunk.name.as_bytes();
    out.extend_from_slice(&(name.len() as u32).to_be_bytes());
    out.extend_from_slice(name);

    out.extend_from_slice(&(d.param_names.len() as u32).to_be_bytes());
    for &sym in &d.param_names {
        let n = heap.symbol_name(sym).as_bytes();
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }

    // legacy is_operative byte — always 0 in new images. left in the
    // wire format so older readers don't choke on layout shift; new
    // dispatch is structural via __underlying on the closure object.
    out.push(0u8);

    out.extend_from_slice(&(d.capture_names.len() as u32).to_be_bytes());
    for &sym in &d.capture_names {
        let n = heap.symbol_name(sym).as_bytes();
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }

    out.extend_from_slice(&(d.capture_parent_regs.len() as u32).to_be_bytes());
    out.extend_from_slice(&d.capture_parent_regs);

    out.extend_from_slice(&(d.capture_local_regs.len() as u32).to_be_bytes());
    out.extend_from_slice(&d.capture_local_regs);

    out.extend_from_slice(&(d.capture_values.len() as u32).to_be_bytes());
    for v in &d.capture_values {
        out.extend(canonical_value_bytes_using_table(heap, *v, table));
    }

    out.extend_from_slice(&(d.desc_base as u32).to_be_bytes());

    match d.rest_param_reg {
        Some(r) => { out.push(1); out.push(r); }
        None => { out.push(0); }
    }

    match &d.source {
        Some(s) => {
            out.push(1);
            let text = s.text.as_bytes();
            let label = s.origin.label.as_bytes();
            out.extend_from_slice(&(text.len() as u32).to_be_bytes());
            out.extend_from_slice(text);
            out.extend_from_slice(&(label.len() as u32).to_be_bytes());
            out.extend_from_slice(label);
            out.extend_from_slice(&(s.origin.byte_start as u64).to_be_bytes());
            out.extend_from_slice(&(s.origin.byte_end as u64).to_be_bytes());
        }
        None => out.push(0),
    }
}

fn decode_closure_descs(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Vec<ClosureDesc>, String> {
    let mut offset = 0;
    let n = read_u32(bytes, &mut offset)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(decode_desc(store, rtxn, bytes, &mut offset, heap, memo)?);
    }
    Ok(out)
}

fn decode_desc(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    offset: &mut usize,
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<ClosureDesc, String> {
    let code_len = read_u32(bytes, offset)? as usize;
    if *offset + code_len > bytes.len() { return Err("truncated code".into()); }
    let code = bytes[*offset..*offset + code_len].to_vec();
    *offset += code_len;

    let n_const = read_u32(bytes, offset)? as usize;
    let mut constants = Vec::with_capacity(n_const);
    for _ in 0..n_const {
        let v = decode_value(store, rtxn, bytes, offset, heap, memo)?;
        constants.push(v.to_bits());
    }

    if *offset + 2 > bytes.len() { return Err("truncated arity/regs".into()); }
    let arity = bytes[*offset]; *offset += 1;
    let num_regs = bytes[*offset]; *offset += 1;

    let name = read_len_utf8(bytes, offset)?;

    let n_params = read_u32(bytes, offset)? as usize;
    let mut param_names = Vec::with_capacity(n_params);
    for _ in 0..n_params {
        let n = read_len_utf8(bytes, offset)?;
        param_names.push(heap.intern(&n));
    }

    // legacy is_operative byte — read and discard. dispatch is now
    // structural via __underlying on the heap closure object.
    if *offset >= bytes.len() { return Err("truncated is_operative".into()); }
    let _legacy_is_op = bytes[*offset] != 0; *offset += 1;

    let n_cap = read_u32(bytes, offset)? as usize;
    let mut capture_names = Vec::with_capacity(n_cap);
    for _ in 0..n_cap {
        let n = read_len_utf8(bytes, offset)?;
        capture_names.push(heap.intern(&n));
    }

    let n_cap_parent = read_u32(bytes, offset)? as usize;
    if *offset + n_cap_parent > bytes.len() { return Err("truncated cap-parent".into()); }
    let capture_parent_regs = bytes[*offset..*offset + n_cap_parent].to_vec();
    *offset += n_cap_parent;

    let n_cap_local = read_u32(bytes, offset)? as usize;
    if *offset + n_cap_local > bytes.len() { return Err("truncated cap-local".into()); }
    let capture_local_regs = bytes[*offset..*offset + n_cap_local].to_vec();
    *offset += n_cap_local;

    let n_cap_vals = read_u32(bytes, offset)? as usize;
    let mut capture_values = Vec::with_capacity(n_cap_vals);
    for _ in 0..n_cap_vals {
        capture_values.push(decode_value(store, rtxn, bytes, offset, heap, memo)?);
    }

    let desc_base = read_u32(bytes, offset)? as usize;

    if *offset >= bytes.len() { return Err("truncated rest-reg flag".into()); }
    let has_rest = bytes[*offset] != 0; *offset += 1;
    let rest_param_reg = if has_rest {
        if *offset >= bytes.len() { return Err("truncated rest-reg".into()); }
        let r = bytes[*offset]; *offset += 1;
        Some(r)
    } else { None };

    if *offset >= bytes.len() { return Err("truncated source flag".into()); }
    let has_source = bytes[*offset] != 0; *offset += 1;
    let source = if has_source {
        let text = read_len_utf8(bytes, offset)?;
        let label = read_len_utf8(bytes, offset)?;
        if *offset + 16 > bytes.len() { return Err("truncated source offsets".into()); }
        let mut a = [0u8; 8];
        a.copy_from_slice(&bytes[*offset..*offset + 8]);
        *offset += 8;
        let byte_start = u64::from_be_bytes(a) as usize;
        a.copy_from_slice(&bytes[*offset..*offset + 8]);
        *offset += 8;
        let byte_end = u64::from_be_bytes(a) as usize;
        Some(ClosureSource {
            text,
            origin: moof_core::source::SourceOrigin {
                label, byte_start, byte_end,
            },
        })
    } else { None };

    let mut chunk = Chunk::new(name, arity, num_regs);
    chunk.code = code;
    chunk.constants = constants;

    Ok(ClosureDesc {
        chunk,
        param_names,
        capture_names,
        capture_parent_regs,
        capture_local_regs,
        capture_values,
        desc_base,
        rest_param_reg,
        source,
    })
}

// ─────────── type-protos codec ───────────

fn encode_type_protos_with(heap: &Heap, protos: &[Value], table: &HashTable) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(protos.len() as u32).to_be_bytes());
    for v in protos {
        out.extend(canonical_value_bytes_using_table(heap, *v, table));
    }
    out
}

fn decode_type_protos(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Vec<Value>, String> {
    let mut offset = 0;
    let n = read_u32(bytes, &mut offset)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(decode_value(store, rtxn, bytes, &mut offset, heap, memo)?);
    }
    Ok(out)
}

fn decode_foreign(bytes: &[u8], heap: &mut Heap) -> Result<Value, String> {
    let mut offset = 1;
    let type_name = read_len_utf8(bytes, &mut offset)?;
    if offset + 8 > bytes.len() { return Err("truncated foreign schema hash".into()); }
    let mut sh = [0u8; 8];
    sh.copy_from_slice(&bytes[offset..offset + 8]);
    offset += 8;
    let schema_hash = u64::from_be_bytes(sh);
    let payload_len = read_u32(bytes, &mut offset)? as usize;
    if offset + payload_len > bytes.len() {
        return Err("truncated foreign payload".into());
    }
    let payload_bytes = &bytes[offset..offset + payload_len];

    // look up the registered foreign type by name + schema, error if
    // either missing or schema-incompatible.
    let type_id = heap.foreign_registry().resolve(
        &moof_core::foreign::ForeignTypeName { name: type_name.clone(), schema_hash }
    ).map_err(|e| format!("foreign load: {type_name}: {e}"))?;
    let vt = heap.foreign_registry().vtable(type_id)
        .ok_or_else(|| format!("foreign load: no vtable for {type_name}"))?;
    let proto_name: &str = (vt.prototype_name)();
    let proto = heap.lookup_type(proto_name);
    let payload = (vt.deserialize)(payload_bytes)
        .map_err(|e| format!("foreign deserialize {type_name}: {e}"))?;
    let fd = moof_core::foreign::ForeignData { type_id, payload };
    Ok(heap.alloc_val(HeapObject::new_foreign(proto, fd)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("moof-blobstore-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn save_value_dedupes_and_returns_root_hash() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let list = heap.list(&[Value::integer(1), Value::integer(2), Value::integer(3)]);
        let h1 = store.save_value(&heap, list, &[]).unwrap();
        let count1 = store.blob_count().unwrap();
        assert!(count1 > 0, "save should have written at least one blob");

        // save same value again — dedupe should keep count identical
        let h2 = store.save_value(&heap, list, &[]).unwrap();
        assert_eq!(h1, h2, "same value → same root hash");
        let count2 = store.blob_count().unwrap();
        assert_eq!(count1, count2, "dedupe should prevent duplicate blob writes");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn get_blob_round_trip() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let val = heap.alloc_string("hello world");
        let root = store.save_value(&heap, val, &[]).unwrap();

        // fetch the root blob
        let bytes = store.get_blob(&root).unwrap()
            .expect("blob should be present");
        // should match what canonical_blob_bytes would produce
        let obj_id = val.as_any_object().unwrap();
        let expected = heap.canonical_blob_bytes(obj_id);
        assert_eq!(bytes, expected, "blob bytes should match canonical form");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refs_round_trip() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let v = heap.list(&[Value::integer(42)]);
        let root = store.save_value(&heap, v, &[
            ("roots.test", blake3::hash(b"anchor").into()),
        ]).unwrap();

        // the extra ref should be retrievable
        let rref = store.get_ref("roots.test").unwrap();
        assert!(rref.is_some());

        // we can also put+get on our own
        store.put_ref("roots.another", &root).unwrap();
        assert_eq!(store.get_ref("roots.another").unwrap(), Some(root));

        let refs = store.list_refs().unwrap();
        assert!(refs.iter().any(|(n, _)| n == "roots.test"));
        assert!(refs.iter().any(|(n, _)| n == "roots.another"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_cons_list() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let orig = heap.list(&[Value::integer(1), Value::integer(2), Value::integer(3)]);
        let root = store.save_value(&heap, orig, &[]).unwrap();

        // load into a FRESH heap — no symbol id overlap possible
        let mut fresh = Heap::new();
        let restored = store.load_value(&root, &mut fresh).unwrap();

        // check: restored should be a three-element list of integers
        let items = fresh.list_to_vec(restored);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].as_integer(), Some(1));
        assert_eq!(items[1].as_integer(), Some(2));
        assert_eq!(items[2].as_integer(), Some(3));

        // AND: the restored value hashes to the same root hash as
        // the original. content-addressing is idempotent.
        let restored_hash = fresh.hash_blob(restored.as_any_object().unwrap());
        assert_eq!(restored_hash, root);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_general_object() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let a_sym = heap.intern("alpha");
        let b_sym = heap.intern("beta");
        let orig = heap.make_object_with_slots(
            Value::NIL,
            vec![a_sym, b_sym],
            vec![Value::integer(1), Value::integer(2)],
        );
        let root = store.save_value(&heap, orig, &[]).unwrap();

        let mut fresh = Heap::new();
        let restored = store.load_value(&root, &mut fresh).unwrap();
        let rid = restored.as_any_object().unwrap();
        let ra = fresh.intern("alpha");
        let rb = fresh.intern("beta");
        assert_eq!(fresh.get(rid).slot_get(ra), Some(Value::integer(1)));
        assert_eq!(fresh.get(rid).slot_get(rb), Some(Value::integer(2)));

        // hash invariant — restored content-hashes to the same root
        assert_eq!(fresh.hash_blob(rid), root);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_string() {
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let orig = heap.alloc_string("hello from the image");
        let root = store.save_value(&heap, orig, &[]).unwrap();

        let mut fresh = Heap::new();
        let restored = store.load_value(&root, &mut fresh).unwrap();
        assert_eq!(fresh.get_string(restored.as_any_object().unwrap()),
            Some("hello from the image"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closure_descs_round_trip() {
        use moof_lang::opcodes::Chunk;
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        // Construct a minimal closure desc with a few constants and
        // captures — exercises the value codec paths.
        let mut chunk = Chunk::new("test", 2, 4);
        chunk.code = vec![0x01, 0x02, 0x03];
        chunk.constants = vec![
            Value::integer(42).to_bits(),
            Value::NIL.to_bits(),
        ];
        let sym_x = heap.intern("x");
        let sym_y = heap.intern("y");
        let sym_z = heap.intern("z");
        let captured_list = heap.list(&[Value::integer(1), Value::integer(2)]);
        let desc = ClosureDesc {
            chunk,
            param_names: vec![sym_x, sym_y],
            capture_names: vec![sym_z],
            capture_parent_regs: vec![3],
            capture_local_regs: vec![5],
            capture_values: vec![captured_list],
            desc_base: 0,
            rest_param_reg: Some(7),
            source: Some(moof_core::source::ClosureSource {
                text: "(defn foo ...)".to_string(),
                origin: moof_core::source::SourceOrigin {
                    label: "test.moof".to_string(),
                    byte_start: 10, byte_end: 24,
                },
            }),
        };
        let descs = vec![desc];

        let hash = store.save_closure_descs(&heap, &descs).unwrap();

        // load into a fresh heap
        let mut fresh = Heap::new();
        let loaded = store.load_closure_descs(&hash, &mut fresh).unwrap();
        assert_eq!(loaded.len(), 1);
        let d = &loaded[0];
        assert_eq!(d.chunk.code, vec![0x01, 0x02, 0x03]);
        assert_eq!(d.chunk.arity, 2);
        assert_eq!(d.chunk.num_regs, 4);
        assert_eq!(d.chunk.constants.len(), 2);
        assert_eq!(Value::from_bits(d.chunk.constants[0]).as_integer(), Some(42));
        assert!(Value::from_bits(d.chunk.constants[1]).is_nil());
        assert_eq!(d.param_names.len(), 2);
        assert_eq!(fresh.symbol_name(d.param_names[0]), "x");
        assert_eq!(fresh.symbol_name(d.param_names[1]), "y");
        assert_eq!(d.capture_parent_regs, vec![3]);
        assert_eq!(d.capture_local_regs, vec![5]);
        assert_eq!(d.capture_values.len(), 1);
        // captured list is a cons of 1, 2
        let items = fresh.list_to_vec(d.capture_values[0]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_integer(), Some(1));
        assert_eq!(items[1].as_integer(), Some(2));
        assert_eq!(d.rest_param_reg, Some(7));
        assert!(d.source.is_some());
        let src = d.source.as_ref().unwrap();
        assert_eq!(src.text, "(defn foo ...)");
        assert_eq!(src.origin.label, "test.moof");
        assert_eq!(src.origin.byte_start, 10);
        assert_eq!(src.origin.byte_end, 24);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shared_sublist_deduped() {
        // two lists sharing a common tail get stored as distinct
        // roots but the shared tail is only one blob.
        let dir = tempdir();
        let store = BlobStore::open(&dir).unwrap();
        let mut heap = Heap::new();

        let tail = heap.list(&[Value::integer(10), Value::integer(20)]);
        let list_a = heap.cons(Value::integer(1), tail);
        let list_b = heap.cons(Value::integer(2), tail);

        let _ = store.save_value(&heap, list_a, &[]).unwrap();
        let count_after_a = store.blob_count().unwrap();

        let _ = store.save_value(&heap, list_b, &[]).unwrap();
        let count_after_b = store.blob_count().unwrap();

        // adding list_b should only have added its new head cons
        // (one blob), not duplicated the shared tail.
        assert_eq!(count_after_b, count_after_a + 1,
            "shared sublist should not be written twice");

        std::fs::remove_dir_all(&dir).ok();
    }
}
