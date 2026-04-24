// NaN-boxed values: 8 bytes each. the only data type in the runtime.
//
// Layout (IEEE 754 double, 64 bits):
//   float:   any f64 that is NOT a quiet NaN with our prefix
//   tagged:  quiet NaN space (exponent all 1s + quiet bit set)
//     tag bits [50:48] select the type (3 bits, 0-7)
//     payload is bits [47:0] (48 bits)
//
// Tags:
//   0 = nil         1 = true        2 = false
//   3 = integer     4 = symbol      5 = object (store/persistent)
//   6 = object (nursery/ephemeral)

use std::fmt;

const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_SHIFT: u64 = 48;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const SIGN_BIT_48: u64 = 0x0000_8000_0000_0000;
const SIGN_EXT_48: u64 = 0xFFFF_0000_0000_0000;

const TAG_NIL: u64 = 0;
const TAG_TRUE: u64 = 1;
const TAG_FALSE: u64 = 2;
const TAG_INT: u64 = 3;
const TAG_SYM: u64 = 4;
const TAG_OBJ: u64 = 5;
const TAG_NURSERY: u64 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(transparent)]
pub struct Value(u64);

impl Value {
    pub const NIL: Value = Value(QNAN | (TAG_NIL << TAG_SHIFT));
    pub const TRUE: Value = Value(QNAN | (TAG_TRUE << TAG_SHIFT));
    pub const FALSE: Value = Value(QNAN | (TAG_FALSE << TAG_SHIFT));

    pub fn integer(n: i64) -> Value {
        Value(QNAN | (TAG_INT << TAG_SHIFT) | ((n as u64) & PAYLOAD_MASK))
    }

    /// Inclusive bounds of the i48 tagged-integer payload.
    /// `Value::integer` silently truncates past these; callers that
    /// need overflow-checked promotion use `try_integer` + the heap's
    /// `alloc_integer` for the BigInt path.
    pub const INT_MAX: i64 = (1i64 << 47) - 1;
    pub const INT_MIN: i64 = -(1i64 << 47);

    /// Like `integer`, but returns None if `n` would lose bits in
    /// the i48 payload. Callers that need a guaranteed roundtrip
    /// use this to decide between primitive and BigInt storage.
    pub fn try_integer(n: i64) -> Option<Value> {
        if n >= Self::INT_MIN && n <= Self::INT_MAX {
            Some(Self::integer(n))
        } else {
            None
        }
    }

    pub fn float(f: f64) -> Value {
        Value(f.to_bits())
    }

    pub fn symbol(id: u32) -> Value {
        Value(QNAN | (TAG_SYM << TAG_SHIFT) | (id as u64))
    }

    pub fn object(id: u32) -> Value {
        Value(QNAN | (TAG_OBJ << TAG_SHIFT) | (id as u64))
    }

    pub fn nursery(id: u32) -> Value {
        Value(QNAN | (TAG_NURSERY << TAG_SHIFT) | (id as u64))
    }

    pub fn boolean(b: bool) -> Value {
        if b { Self::TRUE } else { Self::FALSE }
    }

    // -- type queries --

    fn is_tagged(self) -> bool {
        (self.0 & QNAN) == QNAN
    }

    fn tag(self) -> u64 {
        ((self.0 & !QNAN) >> TAG_SHIFT) & 0x7
    }

    pub fn is_nil(self) -> bool { self.is_tagged() && self.tag() == TAG_NIL }
    pub fn is_true(self) -> bool { self.is_tagged() && self.tag() == TAG_TRUE }
    pub fn is_false(self) -> bool { self.is_tagged() && self.tag() == TAG_FALSE }
    pub fn is_bool(self) -> bool { self.is_true() || self.is_false() }
    pub fn is_integer(self) -> bool { self.is_tagged() && self.tag() == TAG_INT }
    pub fn is_float(self) -> bool { !self.is_tagged() }
    pub fn is_symbol(self) -> bool { self.is_tagged() && self.tag() == TAG_SYM }
    pub fn is_object(self) -> bool { self.is_tagged() && self.tag() == TAG_OBJ }
    pub fn is_nursery(self) -> bool { self.is_tagged() && self.tag() == TAG_NURSERY }
    pub fn is_any_object(self) -> bool { self.is_object() || self.is_nursery() }

    pub fn is_truthy(self) -> bool { !self.is_nil() && !self.is_false() }

    // -- extractors --

    pub fn as_integer(self) -> Option<i64> {
        if !self.is_integer() { return None; }
        let raw = self.0 & PAYLOAD_MASK;
        Some(if raw & SIGN_BIT_48 != 0 { (raw | SIGN_EXT_48) as i64 } else { raw as i64 })
    }

    pub fn as_float(self) -> Option<f64> {
        if self.is_float() {
            Some(f64::from_bits(self.0))
        } else if self.is_integer() {
            Some(self.as_integer().unwrap() as f64)
        } else {
            None
        }
    }

    pub fn as_symbol(self) -> Option<u32> {
        if self.is_symbol() { Some((self.0 & PAYLOAD_MASK) as u32) } else { None }
    }

    pub fn as_object(self) -> Option<u32> {
        if self.is_object() { Some((self.0 & PAYLOAD_MASK) as u32) } else { None }
    }

    pub fn as_nursery(self) -> Option<u32> {
        if self.is_nursery() { Some((self.0 & PAYLOAD_MASK) as u32) } else { None }
    }

    pub fn as_any_object(self) -> Option<u32> {
        self.as_nursery().or_else(|| self.as_object())
    }

    pub fn as_bool(self) -> Option<bool> {
        if self.is_true() { Some(true) }
        else if self.is_false() { Some(false) }
        else { None }
    }

    pub fn to_bits(self) -> u64 { self.0 }
    pub fn from_bits(bits: u64) -> Value { Value(bits) }

    /// Which type prototype to use for dispatch (index into type_protos array).
    pub fn type_tag(self) -> u8 {
        if self.is_nil() { 0 }
        else if self.is_bool() { 1 }
        else if self.is_integer() { 2 }
        else if self.is_float() { 3 }
        else if self.is_symbol() { 4 }
        else if self.is_any_object() { 5 }
        else { 7 }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nil() { write!(f, "nil") }
        else if self.is_true() { write!(f, "true") }
        else if self.is_false() { write!(f, "false") }
        else if let Some(n) = self.as_integer() { write!(f, "{n}") }
        else if self.is_float() { write!(f, "{}", f64::from_bits(self.0)) }
        else if let Some(id) = self.as_symbol() { write!(f, "sym#{id}") }
        else if let Some(id) = self.as_object() { write!(f, "obj#{id}") }
        else if let Some(id) = self.as_nursery() { write!(f, "~obj#{id}") }
        else { write!(f, "?{:#018x}", self.0) }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singletons() {
        assert!(Value::NIL.is_nil());
        assert!(Value::TRUE.is_true());
        assert!(Value::FALSE.is_false());
        assert!(!Value::NIL.is_truthy());
        assert!(Value::TRUE.is_truthy());
        assert!(!Value::FALSE.is_truthy());
    }

    #[test]
    fn integers() {
        assert_eq!(Value::integer(42).as_integer(), Some(42));
        assert_eq!(Value::integer(-1).as_integer(), Some(-1));
        assert_eq!(Value::integer(0).as_integer(), Some(0));
        assert!(Value::integer(42).is_truthy());
    }

    #[test]
    fn floats() {
        let v = Value::float(3.14);
        assert!(v.is_float());
        assert!((v.as_float().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn integer_promotes_to_float() {
        assert_eq!(Value::integer(42).as_float(), Some(42.0));
    }

    #[test]
    fn symbols_and_objects() {
        let s = Value::symbol(7);
        assert!(s.is_symbol());
        assert_eq!(s.as_symbol(), Some(7));

        let o = Value::object(99);
        assert!(o.is_object());
        assert_eq!(o.as_object(), Some(99));
        assert!(o.is_any_object());
    }

    #[test]
    fn nursery_objects() {
        let n = Value::nursery(42);
        assert!(n.is_nursery());
        assert!(n.is_any_object());
        assert_eq!(n.as_nursery(), Some(42));
        assert_eq!(n.as_any_object(), Some(42));
        assert!(!n.is_object()); // not a store object
    }

    #[test]
    fn type_tags() {
        assert_eq!(Value::NIL.type_tag(), 0);
        assert_eq!(Value::TRUE.type_tag(), 1);
        assert_eq!(Value::integer(5).type_tag(), 2);
        assert_eq!(Value::float(1.0).type_tag(), 3);
        assert_eq!(Value::symbol(0).type_tag(), 4);
        assert_eq!(Value::object(0).type_tag(), 5);
        assert_eq!(Value::nursery(0).type_tag(), 5);
    }

    #[test]
    fn roundtrip_bits() {
        for v in [Value::NIL, Value::TRUE, Value::integer(-99), Value::float(2.718), Value::symbol(42), Value::object(100), Value::nursery(7)] {
            assert_eq!(Value::from_bits(v.to_bits()), v);
        }
    }
}
