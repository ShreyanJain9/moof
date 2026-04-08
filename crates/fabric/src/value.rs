//! NaN-boxed values: the only data type in the fabric.
//!
//! Layout (IEEE 754 double, 64 bits):
//!   float:   any valid f64 that is NOT a NaN with our quiet-NaN prefix
//!   tagged:  0x7FF8_xxxx_xxxx_xxxx  (quiet NaN space)
//!     tag bits [51:48] select the type, payload is bits [47:0]
//!
//! Tags:
//!   0 = nil
//!   1 = true
//!   2 = false
//!   3 = integer (48-bit signed, i48 range: ±140 trillion)
//!   4 = symbol (48-bit index into symbol table)
//!   5 = object (48-bit index into object store)

use std::fmt;

const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_SHIFT: u64 = 48;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

const TAG_NIL: u64 = 0;
const TAG_TRUE: u64 = 1;
const TAG_FALSE: u64 = 2;
const TAG_INT: u64 = 3;
const TAG_SYM: u64 = 4;
const TAG_OBJ: u64 = 5;

// sign-extension masks for i48
const SIGN_BIT_48: u64 = 0x0000_8000_0000_0000;
const SIGN_EXT_48: u64 = 0xFFFF_0000_0000_0000;

/// A NaN-boxed value. 8 bytes. Copy. The only data type in the fabric.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Value(u64);

impl Value {
    // ── constructors ──

    pub const NIL: Value = Value(QNAN | (TAG_NIL << TAG_SHIFT));
    pub const TRUE: Value = Value(QNAN | (TAG_TRUE << TAG_SHIFT));
    pub const FALSE: Value = Value(QNAN | (TAG_FALSE << TAG_SHIFT));

    pub fn integer(n: i64) -> Value {
        // truncate to 48 bits
        let payload = (n as u64) & PAYLOAD_MASK;
        Value(QNAN | (TAG_INT << TAG_SHIFT) | payload)
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

    pub fn boolean(b: bool) -> Value {
        if b { Self::TRUE } else { Self::FALSE }
    }

    // ── type queries ──

    /// Is this a tagged (non-float) value? All tagged values live in the
    /// quiet NaN space: exponent all 1s + quiet bit set.
    fn is_tagged(self) -> bool {
        (self.0 & QNAN) == QNAN
    }

    /// Extract the tag bits (bits 50:48), stripping the QNAN prefix first.
    fn tag(self) -> u64 {
        ((self.0 & !QNAN) >> TAG_SHIFT) & 0x7
    }

    pub fn is_nil(self) -> bool {
        self.is_tagged() && self.tag() == TAG_NIL
    }
    pub fn is_true(self) -> bool {
        self.is_tagged() && self.tag() == TAG_TRUE
    }
    pub fn is_false(self) -> bool {
        self.is_tagged() && self.tag() == TAG_FALSE
    }
    pub fn is_bool(self) -> bool {
        self.is_true() || self.is_false()
    }
    pub fn is_integer(self) -> bool {
        self.is_tagged() && self.tag() == TAG_INT
    }
    pub fn is_float(self) -> bool {
        !self.is_tagged()
    }
    pub fn is_symbol(self) -> bool {
        self.is_tagged() && self.tag() == TAG_SYM
    }
    pub fn is_object(self) -> bool {
        self.is_tagged() && self.tag() == TAG_OBJ
    }

    /// Returns true if this value is "truthy" (everything except nil and false).
    pub fn is_truthy(self) -> bool {
        !self.is_nil() && !self.is_false()
    }

    // ── extractors ──

    pub fn as_integer(self) -> Option<i64> {
        if !self.is_integer() {
            return None;
        }
        let raw = self.0 & PAYLOAD_MASK;
        // sign-extend from 48 bits
        let val = if raw & SIGN_BIT_48 != 0 {
            (raw | SIGN_EXT_48) as i64
        } else {
            raw as i64
        };
        Some(val)
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
        if self.is_symbol() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    pub fn as_object(self) -> Option<u32> {
        if self.is_object() {
            Some((self.0 & PAYLOAD_MASK) as u32)
        } else {
            None
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        if self.is_true() {
            Some(true)
        } else if self.is_false() {
            Some(false)
        } else {
            None
        }
    }

    /// Raw bits, for serialization.
    pub fn to_bits(self) -> u64 {
        self.0
    }

    /// From raw bits, for deserialization.
    pub fn from_bits(bits: u64) -> Value {
        Value(bits)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nil() {
            write!(f, "nil")
        } else if self.is_true() {
            write!(f, "true")
        } else if self.is_false() {
            write!(f, "false")
        } else if let Some(n) = self.as_integer() {
            write!(f, "{n}")
        } else if let Some(fl) = self.as_float() {
            if self.is_float() {
                write!(f, "{fl}")
            } else {
                write!(f, "{fl}")
            }
        } else if let Some(id) = self.as_symbol() {
            write!(f, "sym#{id}")
        } else if let Some(id) = self.as_object() {
            write!(f, "obj#{id}")
        } else {
            write!(f, "?({:#018x})", self.0)
        }
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
    fn nil_true_false() {
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
        assert!(Value::integer(42).is_integer());
        assert!(Value::integer(42).is_truthy());
    }

    #[test]
    fn floats() {
        let v = Value::float(3.14);
        assert!(v.is_float());
        assert!((v.as_float().unwrap() - 3.14).abs() < f64::EPSILON);
        assert!(v.is_truthy());
    }

    #[test]
    fn symbols_and_objects() {
        let s = Value::symbol(7);
        assert!(s.is_symbol());
        assert_eq!(s.as_symbol(), Some(7));

        let o = Value::object(99);
        assert!(o.is_object());
        assert_eq!(o.as_object(), Some(99));
    }

    #[test]
    fn integer_promotes_to_float() {
        let v = Value::integer(42);
        assert_eq!(v.as_float(), Some(42.0));
    }
}
