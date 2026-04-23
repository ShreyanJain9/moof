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
use moof_core::Value;
use std::collections::HashMap;
use std::path::Path;

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

        // walk every reachable heap object, put each one's blob.
        // canonical_blob_bytes is pure; we compute bytes + hash + put
        // if absent. identical content skips the write.
        let reachable = heap.reachable_objects(val);
        for obj_id in &reachable {
            let bytes = heap.canonical_blob_bytes(*obj_id);
            let hash: Hash = blake3::hash(&bytes).into();
            if self.blobs.get(&wtxn, hash.as_slice())
                .map_err(|e| format!("get blob: {e}"))?.is_none()
            {
                self.blobs.put(&mut wtxn, hash.as_slice(), &bytes)
                    .map_err(|e| format!("put blob: {e}"))?;
            }
        }

        // the root's blob hash — what the caller uses to retrieve it.
        let root = heap.hash_blob(root_id);

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
    let val = match tag {
        BTAG_CONS    => decode_cons(store, rtxn, &bytes, heap, memo)?,
        BTAG_TEXT    => decode_text(&bytes, heap)?,
        BTAG_BYTES   => decode_bytes(&bytes, heap)?,
        BTAG_TABLE   => decode_table(store, rtxn, &bytes, heap, memo)?,
        BTAG_GENERAL => decode_general(store, rtxn, &bytes, heap, memo)?,
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

fn decode_general(
    store: &BlobStore,
    rtxn: &heed::RoTxn,
    bytes: &[u8],
    heap: &mut Heap,
    memo: &mut HashMap<Hash, Value>,
) -> Result<Value, String> {
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

    let obj = HeapObject {
        proto,
        slot_names,
        slot_values,
        handlers,
        foreign: None,
    };
    Ok(heap.alloc_val(obj))
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
