// Canonical serialization + content hashing.
//
// The bytes produced here are deterministic: the same moof value
// always produces the same bytes, regardless of the session's
// symbol-interning order, heap layout, or os. This is the foundation
// of content-addressed persistence — see docs/persistence.md.
//
// The encoding has two forms:
//
//   `value_bytes(v)`  — how a value appears as a sub-element.
//                       primitives inline; heap values as blob-refs
//                       (a 32-byte hash pointing at the blob).
//   `blob_bytes(id)`  — the standalone content of a heap-allocated
//                       value. cons cells, strings, bytes, tables,
//                       general objects each have their own blob
//                       format.
//
// hashing: `hash_value(v)` and `hash_blob(id)` are blake3 of the
// respective canonical bytes.
//
// Key canonicalization rules:
//   - symbols serialize by NAME (utf8), not by intern id.
//   - slot entries sorted by slot-name utf8 bytes.
//   - handler entries sorted by selector-name utf8 bytes.
//   - table map entries sorted by the canonical bytes of the key.
//   - float: raw bits, big-endian. no normalization.
//   - int: i64, big-endian, 8 bytes.
//
// If these rules are ever changed, every hash in every stored
// image becomes invalid — bump a format version.

use crate::heap::Heap;
use crate::value::Value;
use crate::heap::{Pair, Text, Bytes, Table};

// ── tag bytes for value encoding (inline) ──
pub const VTAG_INT:    u8 = 0x01;
pub const VTAG_FLOAT:  u8 = 0x02;
pub const VTAG_NIL:    u8 = 0x03;
pub const VTAG_TRUE:   u8 = 0x04;
pub const VTAG_FALSE:  u8 = 0x05;
pub const VTAG_SYMBOL: u8 = 0x06;
pub const VTAG_BLOB:   u8 = 0x07;

// ── tag bytes for blob encoding ──
pub const BTAG_CONS:    u8 = 0x01;
pub const BTAG_TEXT:    u8 = 0x02;
pub const BTAG_BYTES:   u8 = 0x03;
pub const BTAG_TABLE:   u8 = 0x04;
pub const BTAG_GENERAL: u8 = 0x05;
pub const BTAG_FOREIGN: u8 = 0x06;

/// 32-byte blake3 digest.
pub type Hash = [u8; 32];

/// Hex-encode a hash for display. Lowercase, 64 chars.
pub fn hash_hex(h: &Hash) -> String {
    let mut s = String::with_capacity(64);
    for b in h {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

impl Heap {
    /// Collect every heap object id reachable from `val` (itself +
    /// transitive closure over proto, slot_values, handler values,
    /// and foreign sub-values via the vtable's trace fn). Used by
    /// the blob store's save path to enumerate blobs to persist.
    pub fn reachable_objects(&self, val: Value) -> std::collections::HashSet<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut worklist: Vec<u32> = Vec::new();
        if let Some(id) = val.as_any_object() {
            if !crate::heap::is_virtual_env_id(id) { worklist.push(id); }
            else {
                // walk through the cell's outgoing refs WITHOUT
                // adding the virtual id itself (it's not an arena
                // object and the blobstore can't serialize it).
                let idx = crate::heap::frame_env_idx(id);
                if let Some(cell) = self.envs.get(idx) {
                    if let Some(pid) = cell.parent.as_any_object() { worklist.push(pid); }
                    for v in &cell.values {
                        if let Some(sid) = v.as_any_object() { worklist.push(sid); }
                    }
                }
            }
        }
        while let Some(id) = worklist.pop() {
            if !seen.insert(id) { continue; }
            // Virtual env cells aren't in the arena. Trace through
            // their bindings + parent so anything reachable via env
            // chain stays alive — but DON'T add the virtual id to
            // `seen` (the blobstore can't serialize it).
            if crate::heap::is_virtual_env_id(id) {
                seen.remove(&id);
                let idx = crate::heap::frame_env_idx(id);
                if let Some(cell) = self.envs.get(idx) {
                    if let Some(pid) = cell.parent.as_any_object() { worklist.push(pid); }
                    for v in &cell.values {
                        if let Some(sid) = v.as_any_object() { worklist.push(sid); }
                    }
                }
                continue;
            }
            let obj = self.get(id);
            if let Some(pid) = obj.proto.as_any_object() { worklist.push(pid); }
            for v in &obj.slot_values {
                if let Some(sid) = v.as_any_object() { worklist.push(sid); }
            }
            for (_, hv) in &obj.handlers {
                if let Some(hid) = hv.as_any_object() { worklist.push(hid); }
            }
            if let Some(fd) = &obj.foreign {
                if let Some(vt) = self.foreign_registry().vtable(fd.type_id) {
                    (vt.trace)(&*fd.payload, &mut |v| {
                        if let Some(sid) = v.as_any_object() { worklist.push(sid); }
                    });
                }
            }
        }
        seen
    }
}

/// Canonical bytes for the cycle-placeholder blob: an empty General
/// object (proto=nil, no slots, no handlers). Produces a fixed
/// 10-byte sequence, hence a fixed hash. The blob store writes this
/// blob unconditionally during save so any cycle-reference inside a
/// real blob can be resolved on load to an empty object (rather than
/// an "unknown blob" error). Lossy for cyclic data — cycles become
/// empty objects — but cycles aren't expressible as pure content
/// anyway.
pub fn cycle_placeholder_blob_bytes() -> Vec<u8> {
    vec![BTAG_GENERAL, VTAG_NIL, 0, 0, 0, 0, 0, 0, 0, 0]
}

/// Hash of the cycle-placeholder blob. Stable across runs, processes,
/// machines. Returned whenever canonicalization revisits an in-flight
/// blob (prototype chain cycles).
pub fn cycle_placeholder() -> Hash {
    blake3::hash(&cycle_placeholder_blob_bytes()).into()
}

impl Heap {
    // ─────────── value encoding ───────────

    /// Canonical bytes for a Value as a sub-element. Primitives are
    /// inlined; heap values appear as a blob-ref (tag + 32 bytes).
    /// Pure function: no side effects, no allocation apart from the
    /// returned Vec.
    pub fn canonical_value_bytes(&self, val: Value) -> Vec<u8> {
        let mut visited = Vec::new();
        self.canonical_value_bytes_in(val, &mut visited)
    }

    fn canonical_value_bytes_in(&self, val: Value, visited: &mut Vec<u32>) -> Vec<u8> {
        if val.is_nil()   { return vec![VTAG_NIL];   }
        if val.is_true()  { return vec![VTAG_TRUE];  }
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
            let name = self.symbol_name(sym_id);
            return encode_symbol(name);
        }
        if let Some(obj_id) = val.as_any_object() {
            let hash = self.hash_blob_in(obj_id, visited);
            let mut out = Vec::with_capacity(33);
            out.push(VTAG_BLOB);
            out.extend_from_slice(&hash);
            return out;
        }

        // unrepresentable (e.g. closure captures without a heap id).
        // we emit nil rather than panic; deserialization produces nil
        // at that position, which is honest about having lost data.
        vec![VTAG_NIL]
    }

    /// Blake3 of the canonical value encoding. Primitives have a
    /// well-defined hash too; heap values get their blob's hash.
    pub fn hash_value(&self, val: Value) -> Hash {
        let bytes = self.canonical_value_bytes(val);
        blake3::hash(&bytes).into()
    }

    // ─────────── blob encoding ───────────

    /// Canonical bytes for the BLOB of a heap-allocated value.
    /// Recursive; uses a visited stack to break prototype-graph
    /// cycles (closures ↔ protos) with a placeholder hash.
    pub fn canonical_blob_bytes(&self, obj_id: u32) -> Vec<u8> {
        let mut visited = Vec::new();
        self.canonical_blob_bytes_in(obj_id, &mut visited)
    }

    fn canonical_blob_bytes_in(&self, obj_id: u32, visited: &mut Vec<u32>) -> Vec<u8> {
        // fast paths for known foreign types
        let val = Value::nursery(obj_id);
        if self.is_pair(val)  { return self.cons_blob(obj_id, visited); }
        if self.is_text(val)  { return self.text_blob(obj_id); }
        if self.is_bytes(val) { return self.bytes_blob(obj_id); }
        if self.is_table(val) { return self.table_blob(obj_id, visited); }

        // other foreign types get the generic foreign blob form
        if self.get(obj_id).foreign.is_some() {
            return self.foreign_blob(obj_id);
        }

        // plain General object (or closure, or env, ...)
        self.general_blob(obj_id, visited)
    }

    /// Blake3 of a blob's canonical bytes. Used externally to get
    /// a heap object's identity.
    pub fn hash_blob(&self, obj_id: u32) -> Hash {
        let mut visited = Vec::new();
        self.hash_blob_in(obj_id, &mut visited)
    }

    fn hash_blob_in(&self, obj_id: u32, visited: &mut Vec<u32>) -> Hash {
        if visited.contains(&obj_id) {
            return cycle_placeholder();
        }
        visited.push(obj_id);
        let bytes = self.canonical_blob_bytes_in(obj_id, visited);
        visited.pop();
        blake3::hash(&bytes).into()
    }

    // ─────────── blob forms by type ───────────

    fn cons_blob(&self, obj_id: u32, visited: &mut Vec<u32>) -> Vec<u8> {
        let val = Value::nursery(obj_id);
        let pair = self.foreign_ref::<Pair>(val);
        let (car, cdr) = match pair {
            Some(p) => (p.car, p.cdr),
            None => (Value::NIL, Value::NIL),
        };
        let mut out = vec![BTAG_CONS];
        out.extend(self.canonical_value_bytes_in(car, visited));
        out.extend(self.canonical_value_bytes_in(cdr, visited));
        out
    }

    fn text_blob(&self, obj_id: u32) -> Vec<u8> {
        let val = Value::nursery(obj_id);
        let text = self.foreign_ref::<Text>(val).map(|t| t.0.as_str()).unwrap_or("");
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(5 + bytes.len());
        out.push(BTAG_TEXT);
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
        out
    }

    fn bytes_blob(&self, obj_id: u32) -> Vec<u8> {
        let val = Value::nursery(obj_id);
        let bytes_ref = self.foreign_ref::<Bytes>(val).map(|b| b.0.as_slice()).unwrap_or(&[]);
        let mut out = Vec::with_capacity(5 + bytes_ref.len());
        out.push(BTAG_BYTES);
        out.extend_from_slice(&(bytes_ref.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes_ref);
        out
    }

    fn table_blob(&self, obj_id: u32, visited: &mut Vec<u32>) -> Vec<u8> {
        let val = Value::nursery(obj_id);
        let table = match self.foreign_ref::<Table>(val) {
            Some(t) => t,
            None => {
                return vec![BTAG_TABLE, 0, 0, 0, 0, 0, 0, 0, 0];
            }
        };
        let mut out = vec![BTAG_TABLE];
        // seq
        out.extend_from_slice(&(table.seq.len() as u32).to_be_bytes());
        for v in &table.seq {
            out.extend(self.canonical_value_bytes_in(*v, visited));
        }
        // map — sorted by key's canonical bytes for determinism
        let mut entries: Vec<(Vec<u8>, Value, Value)> = table.map.iter()
            .map(|(k, v)| (self.canonical_value_bytes_in(*k, visited), *k, *v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (key_bytes, _k, v) in entries {
            out.extend(key_bytes);
            out.extend(self.canonical_value_bytes_in(v, visited));
        }
        out
    }

    fn foreign_blob(&self, obj_id: u32) -> Vec<u8> {
        let obj = self.get(obj_id);
        let fd = obj.foreign.as_ref().expect("foreign_blob on non-foreign");
        let vt = self.foreign_registry().vtable(fd.type_id)
            .expect("foreign_blob: no vtable");
        let type_name = vt.id.name.as_str();
        let schema_hash = vt.id.schema_hash;
        let payload = (vt.serialize)(&*fd.payload);
        let name_bytes = type_name.as_bytes();
        let mut out = Vec::with_capacity(1 + 4 + name_bytes.len() + 8 + 4 + payload.len());
        out.push(BTAG_FOREIGN);
        out.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&schema_hash.to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    fn general_blob(&self, obj_id: u32, visited: &mut Vec<u32>) -> Vec<u8> {
        let obj = self.get(obj_id);
        let proto_bytes = self.canonical_value_bytes_in(obj.proto, visited);

        // slots: sort by symbol name utf8. SKIP `__`-prefixed names —
        // the moof convention for "internal, not part of public
        // identity." closures' `:__scope` is the load-bearing example:
        // it back-references the lexical env the closure was created
        // in, which transitively pulls in vat-wide state. excluding
        // these from canonical bytes keeps content-hash/equal: focused
        // on the actual structural content.
        let mut slot_entries: Vec<(String, Value)> = obj.slot_names.iter()
            .zip(obj.slot_values.iter())
            .map(|(&sym, &v)| (self.symbol_name(sym).to_string(), v))
            .filter(|(name, _)| !name.starts_with("__"))
            .collect();
        slot_entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        // handlers: sort by selector name utf8
        let mut handler_entries: Vec<(String, Value)> = obj.handlers.iter()
            .map(|&(sym, v)| (self.symbol_name(sym).to_string(), v))
            .collect();
        handler_entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        let mut out = vec![BTAG_GENERAL];
        out.extend(proto_bytes);
        // slots
        out.extend_from_slice(&(slot_entries.len() as u32).to_be_bytes());
        for (name, v) in &slot_entries {
            let name_bytes = name.as_bytes();
            out.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
            out.extend_from_slice(name_bytes);
            out.extend(self.canonical_value_bytes_in(*v, visited));
        }
        // handlers
        out.extend_from_slice(&(handler_entries.len() as u32).to_be_bytes());
        for (name, v) in &handler_entries {
            let name_bytes = name.as_bytes();
            out.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
            out.extend_from_slice(name_bytes);
            out.extend(self.canonical_value_bytes_in(*v, visited));
        }
        out
    }
}

fn encode_symbol(name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut out = Vec::with_capacity(5 + name_bytes.len());
    out.push(VTAG_SYMBOL);
    out.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(name_bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::Heap;

    #[test]
    fn integer_hash_stable() {
        let heap = Heap::new();
        let a = heap.hash_value(Value::integer(42));
        let b = heap.hash_value(Value::integer(42));
        assert_eq!(a, b);
        let c = heap.hash_value(Value::integer(43));
        assert_ne!(a, c);
    }

    #[test]
    fn nil_true_false_distinct() {
        let heap = Heap::new();
        let n = heap.hash_value(Value::NIL);
        let t = heap.hash_value(Value::boolean(true));
        let f = heap.hash_value(Value::boolean(false));
        assert_ne!(n, t);
        assert_ne!(n, f);
        assert_ne!(t, f);
    }

    #[test]
    fn symbols_hash_by_name_not_id() {
        // Intern a symbol in two different orders and check that
        // the SAME name hashes to the SAME digest.
        let mut h1 = Heap::new();
        let _unused = h1.intern("zz_other_symbol_first");
        let s1 = h1.intern("target");
        let hash1 = h1.hash_value(Value::symbol(s1));

        let mut h2 = Heap::new();
        let s2 = h2.intern("target");  // different id in this heap
        let hash2 = h2.hash_value(Value::symbol(s2));

        // Different intern ids (s1 != s2 most likely), but canonical
        // hash depends only on the name.
        assert_eq!(hash1, hash2,
            "symbol hashes must be name-based, not id-based");
    }

    #[test]
    fn cons_list_hash_stable() {
        let mut h1 = Heap::new();
        let l1 = h1.list(&[Value::integer(1), Value::integer(2), Value::integer(3)]);
        let hash1 = h1.hash_value(l1);

        let mut h2 = Heap::new();
        let l2 = h2.list(&[Value::integer(1), Value::integer(2), Value::integer(3)]);
        let hash2 = h2.hash_value(l2);

        assert_eq!(hash1, hash2, "identical cons content → identical hash");
    }

    #[test]
    fn slot_order_does_not_affect_hash() {
        // two objects with same content but slots constructed in
        // different orders should hash identically. the canonical
        // form sorts by slot name.
        let mut h = Heap::new();
        let a_sym = h.intern("a");
        let b_sym = h.intern("b");
        let obj1 = h.make_object_with_slots(
            Value::NIL,
            vec![a_sym, b_sym],
            vec![Value::integer(1), Value::integer(2)],
        );
        let obj2 = h.make_object_with_slots(
            Value::NIL,
            vec![b_sym, a_sym],
            vec![Value::integer(2), Value::integer(1)],
        );
        let h1 = h.hash_value(obj1);
        let h2 = h.hash_value(obj2);
        assert_eq!(h1, h2, "slot order shouldn't affect canonical hash");
    }

    #[test]
    fn different_int_values_differ() {
        let heap = Heap::new();
        let a = heap.hash_value(Value::integer(1));
        let b = heap.hash_value(Value::integer(2));
        assert_ne!(a, b);
    }

    #[test]
    fn float_bit_exact() {
        let heap = Heap::new();
        let a = heap.hash_value(Value::float(1.5));
        let b = heap.hash_value(Value::float(1.5));
        let c = heap.hash_value(Value::float(1.5 + f64::EPSILON));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn string_content_addressable() {
        let mut h1 = Heap::new();
        let s1 = h1.alloc_string("hello world");
        let hash1 = h1.hash_value(s1);

        let mut h2 = Heap::new();
        let s2 = h2.alloc_string("hello world");
        let hash2 = h2.hash_value(s2);

        assert_eq!(hash1, hash2);

        let s3 = h2.alloc_string("hello worlx");
        assert_ne!(hash1, h2.hash_value(s3));
    }
}
