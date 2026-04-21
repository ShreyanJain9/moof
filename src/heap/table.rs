// Lua-style table as a `ForeignType`: sequential part (indexed
// array) plus keyed part (IndexMap for insertion-order stable
// iteration). Replaces `HeapObject::Table { seq, map }`.
//
// Tables in moof are "immutable from the user's perspective" —
// mutations go through Update deltas like every other stateful
// type. The Rust payload itself is immutable: a new Table is
// allocated for each logical change, same as Pair or Vec3.

use indexmap::IndexMap;
use serde::{Serialize, Deserialize};

use crate::foreign::ForeignType;
use crate::value::Value;

#[derive(Clone, Debug)]
pub struct Table {
    pub seq: Vec<Value>,
    pub map: IndexMap<Value, Value>,
}

#[derive(Serialize, Deserialize)]
struct TableWire {
    seq: Vec<Value>,
    map: IndexMap<Value, Value>,
}

impl ForeignType for Table {
    fn type_name() -> &'static str { "moof.core.Table" }
    fn prototype_name() -> &'static str { "Table" }

    fn trace(&self, visit: &mut dyn FnMut(Value)) {
        for v in &self.seq { visit(*v); }
        for (k, v) in &self.map { visit(*k); visit(*v); }
    }

    fn clone_across(&self, copy: &mut dyn FnMut(Value) -> Value) -> Self {
        let seq = self.seq.iter().map(|v| copy(*v)).collect();
        let map = self.map.iter().map(|(k, v)| (copy(*k), copy(*v))).collect();
        Table { seq, map }
    }

    fn serialize(&self) -> Vec<u8> {
        let wire = TableWire { seq: self.seq.clone(), map: self.map.clone() };
        bincode::serialize(&wire).unwrap_or_default()
    }

    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        let wire: TableWire = bincode::deserialize(bytes)
            .map_err(|e| format!("Table: {e}"))?;
        Ok(Table { seq: wire.seq, map: wire.map })
    }

    fn equal(&self, other: &Self) -> bool {
        self.seq == other.seq && self.map.len() == other.map.len()
            && self.map.iter().all(|(k, v)| other.map.get(k) == Some(v))
    }

    fn describe(&self) -> String {
        format!("#[... {} seq, {} map]", self.seq.len(), self.map.len())
    }
}
