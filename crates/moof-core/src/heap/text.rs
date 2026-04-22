// Immutable UTF-8 string as a `ForeignType`. Mirrors the old
// `HeapObject::Text(String)` variant behaviorally — content is a
// single `String`, equality is byte-exact, handlers on the String
// prototype provide the real interface (length, ++, reverse, …).

use crate::foreign::ForeignType;
use crate::value::Value;

#[derive(Clone, Debug)]
pub struct Text(pub String);

impl ForeignType for Text {
    fn type_name() -> &'static str { "moof.core.Text" }
    fn prototype_name() -> &'static str { "String" }

    fn serialize(&self) -> Vec<u8> { self.0.as_bytes().to_vec() }
    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        std::str::from_utf8(bytes)
            .map(|s| Text(s.to_string()))
            .map_err(|e| format!("Text: invalid utf-8: {e}"))
    }

    fn equal(&self, other: &Self) -> bool { self.0 == other.0 }

    fn describe(&self) -> String { format!("\"{}\"", self.0.replace('"', "\\\"")) }

    fn virtual_slot(&self, _sym: u32) -> Option<Value> { None }
}
