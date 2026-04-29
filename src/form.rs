//! Forms — the universal substrate primitive.
//!
//! see docs/concepts/forms.md. every Form has four faces:
//! structure (head + args), identity (proto + slots + handlers),
//! liveness (mailbox + behavior, when alive), and history (meta).
//!
//! phase 1 populates structure (for parsed s-exprs) and identity
//! (for callables and protos). liveness lands when vats arrive
//! in phase 2; history lands when journaling arrives.

use std::collections::HashMap;

use crate::sym::SymId;
use crate::value::Value;

/// stable identity for a heap-allocated Form within its vat.
/// (phase 1 has a single global heap; ids are world-wide.)
///
/// `0` is reserved as a sentinel "no form" — nothing useful is
/// allocated there.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct FormId(pub u32);

impl FormId {
    pub const NONE: FormId = FormId(0);

    pub fn is_none(self) -> bool {
        self == FormId::NONE
    }
}

/// a method implementation. handlers in a Form's handler-table map
/// selectors to one of these.
#[derive(Clone)]
pub enum MethodImpl {
    /// rust-implemented method. receives the world (for heap access),
    /// the receiver value, and the evaluated args.
    Native(NativeFn),
    /// bytecode-implemented method. carries everything the VM needs
    /// to invoke it: the chunk, the captured lexical env (a Form),
    /// and the parameter list. when this is stored on a *proto's*
    /// handler table (i.e. a method on a type), invoking it sets
    /// `self` to the *receiver* — not the closure.
    Bytecode {
        chunk: crate::opcodes::ChunkId,
        captured_env: FormId,
        params: Value,
    },
}

/// signature for a rust-implemented method.
///
/// receives a borrow of the World, the receiver, and the args.
/// returns either a Value or an error string. (phase 1's error
/// model is "string"; the proper exception/condition system lands
/// in a later phase.)
pub type NativeFn = fn(
    world: &mut crate::world::World,
    recv: Value,
    args: &[Value],
) -> Result<Value, String>;

/// a Form. the heap stores these.
///
/// fields named for the four-faces model in docs/concepts/forms.md.
/// phase 1 uses `head` and `args` for parsed s-expr lists, and
/// `proto` + `handlers` for callables and type-protos. `slots`,
/// `meta`, and the liveness fields are present and populated as
/// needed but mostly empty in phase 1.
#[derive(Default)]
pub struct Form {
    /// delegation parent. `FormId::NONE` for the root `Object`.
    /// substrate-laws.md L2: every Form's chain bottoms out at Object.
    pub proto: FormId,

    /// structure-face: head of a code/list form.
    /// `Value::Nil` for "data-only" forms.
    pub head: Value,

    /// structure-face: rest of a code/list form. either `Value::Nil`
    /// (terminator) or `Value::Form` (next cons-cell). together with
    /// `head`, gives the lisp cons-shape.
    pub args: Value,

    /// identity-face: named slots.
    pub slots: HashMap<SymId, Value>,

    /// identity-face: method dispatch table.
    pub handlers: HashMap<SymId, MethodImpl>,

    /// history-face: source-loc, doc, journal-id, etc.
    /// (phase 1 puts source-loc here when the reader supports it;
    /// later phases extend.)
    pub meta: HashMap<SymId, Value>,

    /// optional UTF-8 byte payload. used by String Forms
    /// (concepts/strings.md: "internally optimized to UTF-8 bytes;
    /// semantically a Table-of-Chars"). other Forms leave this `None`.
    /// later phases may grow more typed payloads (Bytes, BigInt, etc.).
    pub bytes: Option<Box<str>>,
}

impl Form {
    /// fresh empty Form with the given proto.
    pub fn with_proto(proto: FormId) -> Self {
        Form {
            proto,
            ..Form::default()
        }
    }

    /// fresh cons-cell-shaped Form (structure-face populated).
    pub fn cons(proto: FormId, head: Value, args: Value) -> Self {
        Form {
            proto,
            head,
            args,
            ..Form::default()
        }
    }
}
