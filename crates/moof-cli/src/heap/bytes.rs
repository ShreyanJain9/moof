// Immutable raw byte buffer as a `ForeignType`. Replaces
// `HeapObject::Buffer(Vec<u8>)`.

use crate::foreign::ForeignType;
use crate::value::Value;

#[derive(Clone, Debug)]
pub struct Bytes(pub Vec<u8>);

impl ForeignType for Bytes {
    fn type_name() -> &'static str { "moof.core.Bytes" }
    fn prototype_name() -> &'static str { "Bytes" }

    fn serialize(&self) -> Vec<u8> { self.0.clone() }
    fn deserialize(bytes: &[u8]) -> Result<Self, String> { Ok(Bytes(bytes.to_vec())) }

    fn equal(&self, other: &Self) -> bool { self.0 == other.0 }

    fn describe(&self) -> String { format!("<bytes:{}>", self.0.len()) }

    fn virtual_slot(&self, _sym: u32) -> Option<Value> { None }
}
