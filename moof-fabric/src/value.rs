/// Core value representation for the moof fabric.
///
/// Every value is either an immediate (tagged inline) or a heap reference.
/// The fabric sees all values as potential message receivers.

use std::fmt;
use serde::{Serialize, Deserialize};

/// A moof value. 64 bits. Either an immediate or an index into the heap.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum Value {
    Nil,
    True,
    False,
    Integer(i64),
    Float(f64),
    Symbol(u32),
    Object(u32),
}

impl Value {
    pub fn is_nil(self) -> bool { matches!(self, Value::Nil) }
    pub fn is_truthy(self) -> bool { !matches!(self, Value::Nil | Value::False) }

    pub fn as_integer(self) -> Option<i64> {
        match self { Value::Integer(n) => Some(n), _ => None }
    }

    pub fn as_float(self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(f),
            Value::Integer(n) => Some(n as f64),
            _ => None,
        }
    }

    pub fn as_symbol(self) -> Option<u32> {
        match self { Value::Symbol(id) => Some(id), _ => None }
    }

    pub fn as_object(self) -> Option<u32> {
        match self { Value::Object(id) => Some(id), _ => None }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::True, Value::True) => true,
            (Value::False, Value::False) => true,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Integer(n) => n.hash(state),
            Value::Float(f) => f.to_bits().hash(state),
            Value::Symbol(id) | Value::Object(id) => id.hash(state),
            _ => {}
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::True => write!(f, "true"),
            Value::False => write!(f, "false"),
            Value::Integer(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Symbol(id) => write!(f, "sym#{id}"),
            Value::Object(id) => write!(f, "obj#{id}"),
        }
    }
}

/// Heap-allocated object. The fabric knows four kinds.
///
/// The first — Object — is the universal kind. An Object has a parent
/// (for delegation), named slots (state), and a handler table (behavior).
/// Everything that "is something" in the fabric is an Object.
///
/// The other three are optimized representations for common data:
/// Cons (pairs/lists), String (text), Bytes (opaque data — bytecode,
/// images, compiled code, whatever a shell needs to store).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// The universal object. Parent + slots + handlers.
    Object {
        parent: Value,
        slots: Vec<(u32, Value)>,
        handlers: Vec<(u32, Value)>,
    },

    /// A cons cell. The pair. Foundation of lists and ASTs.
    Cons { car: Value, cdr: Value },

    /// A string.
    String(String),

    /// Opaque byte data. Bytecode, compiled code, binary blobs.
    /// The fabric doesn't interpret this — shells do.
    Bytes(Vec<u8>),
}
