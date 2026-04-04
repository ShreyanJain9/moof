/// Core value representation for MOOF.
///
/// Every value in MOOF is either an immediate (tagged inline) or a heap reference.
/// The design follows the design doc: every value is an object that can be messaged.
/// Cons cells are the AST. Symbols are interned. The object model has slots + handlers.

use std::fmt;
use serde::{Serialize, Deserialize};

/// A MOOF value. 64 bits. Either an immediate or an index into the heap.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum Value {
    /// The nil singleton
    Nil,
    /// Boolean true
    True,
    /// Boolean false
    False,
    /// Small integer (i64)
    Integer(i64),
    /// IEEE 754 double
    Float(f64),
    /// Interned symbol — index into the symbol table
    Symbol(u32),
    /// Heap-allocated object — index into the heap arena
    Object(u32),
}

// Manual Eq/Hash: float comparison uses bit representation so NaN == NaN
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

impl Value {
    pub fn is_nil(self) -> bool { matches!(self, Value::Nil) }
    pub fn is_truthy(self) -> bool { !matches!(self, Value::Nil | Value::False) }

    pub fn as_integer(self) -> Option<i64> {
        match self { Value::Integer(n) => Some(n), _ => None }
    }

    pub fn as_float(self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(f),
            Value::Integer(n) => Some(n as f64), // auto-promote
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

/// The kind of heap-allocated object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeapObject {
    /// A cons cell — the fundamental building block of lists and ASTs.
    Cons { car: Value, cdr: Value },

    /// A string value.
    MoofString(String),

    /// A general object with slots (storage) and handlers (behavior).
    /// This is the core of the MOOF object model (§4.1 of the design doc).
    GeneralObject {
        /// parent slot for prototype delegation
        parent: Value,
        /// slots: name (symbol) → value. Private storage.
        slots: Vec<(u32, Value)>,
        /// handlers: selector (symbol) → handler value (operative/lambda).
        /// Public behavior. Inherited through delegation.
        handlers: Vec<(u32, Value)>,
    },

    /// A compiled bytecode chunk — the body of a lambda or operative.
    BytecodeChunk(BytecodeChunk),

    /// An operative (vau result). Captures its defining environment.
    Operative {
        params: Value,
        env_param: u32,
        body: u32,
        def_env: u32,
        /// The original source AST (for introspection). Nil if unavailable.
        source: Value,
    },

    /// A wrapped operative (lambda). Evaluates args before passing them.
    Lambda {
        params: Value,
        body: u32,
        def_env: u32,
        /// The original source AST (for introspection). Nil if unavailable.
        source: Value,
    },

    /// A first-class environment (§7.3)
    Environment(super::env::Environment),

    /// A native function registered in the VM's NativeRegistry.
    /// The name is used to look up the closure at call time.
    NativeFunction {
        name: String,
    },
}

/// A compiled bytecode chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BytecodeChunk {
    /// The bytecode instructions
    pub code: Vec<u8>,
    /// Constant pool — values referenced by index in the bytecode
    pub constants: Vec<Value>,
}
