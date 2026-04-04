/// Core value representation for MOOF.
///
/// Every value in MOOF is either an immediate (tagged inline) or a heap reference.
/// The design follows the design doc: every value is an object that can be messaged.
/// Cons cells are the AST. Symbols are interned. The object model has slots + handlers.

use std::fmt;
use serde::{Serialize, Deserialize};

/// A MOOF value. 64 bits. Either an immediate or an index into the heap.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Value {
    /// The nil singleton
    Nil,
    /// Boolean true
    True,
    /// Boolean false
    False,
    /// Small integer (i64)
    Integer(i64),
    /// Interned symbol — index into the symbol table
    Symbol(u32),
    /// Heap-allocated object — index into the heap arena
    Object(u32),
}

impl Value {
    pub fn is_nil(self) -> bool { matches!(self, Value::Nil) }
    pub fn is_truthy(self) -> bool { !matches!(self, Value::Nil | Value::False) }

    pub fn as_integer(self) -> Option<i64> {
        match self { Value::Integer(n) => Some(n), _ => None }
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

}

/// A compiled bytecode chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BytecodeChunk {
    /// The bytecode instructions
    pub code: Vec<u8>,
    /// Constant pool — values referenced by index in the bytecode
    pub constants: Vec<Value>,
}
