//! the canonical primordial protos.
//!
//! every fresh moof world allocates these proto Forms at boot; their
//! `FormId`s live in [`Protos`] and feed [`World::proto_of`] for
//! tagged-immediate dispatch.
//!
//! protos are *empty* at allocation — their handler tables fill in
//! during phase A.7 (object reflection methods) and beyond. phase A
//! deliberately keeps the proto set small; richer types
//! (`String`, `Float`, `Char`, `Table`) come later.
//!
//! ## the chain
//!
//! ```text
//! Object              proto: Nil
//!  ├── Nil-proto       proto: Object  (proto of nil-the-value)
//!  ├── Bool            proto: Object
//!  ├── Integer         proto: Object
//!  ├── Symbol          proto: Object
//!  ├── List            proto: Object
//!  ├── Method          proto: Object
//!  ├── Chunk           proto: Object
//!  ├── Closure         proto: Method
//!  ├── Env             proto: Object
//!  ├── Frame           proto: Object   (R3 — running computation)
//!  └── ForeignHandle   proto: Object
//! ```
//!
//! note: `Nil-proto` is the *proto* used by the `nil` value. it is
//! distinct from `Value::Nil` itself. the `Nil-proto` Form's own
//! `proto` field is `Object`.

use crate::form::{Form, FormId};
use crate::heap::Heap;
use crate::value::Value;

/// the canonical proto FormIds. populated by [`Protos::bootstrap`].
#[derive(Copy, Clone, Debug)]
pub struct Protos {
    pub object: FormId,
    pub nil: FormId,
    pub bool_: FormId,
    pub integer: FormId,
    pub float: FormId,
    pub symbol: FormId,
    pub char_: FormId,
    pub string: FormId,
    pub bytes: FormId,
    pub cons: FormId,
    pub table: FormId,
    pub method: FormId,
    pub chunk: FormId,
    pub closure: FormId,
    pub env: FormId,
    pub foreign: FormId,
    /// `Frame` proto — the "running computation" face of
    /// reflection. instances are materialized snapshots of the
    /// runtime call stack, populated on demand via
    /// `(currentFrame)` / `(callStack)`. honors
    /// reflection-contract.md R3.
    pub frame: FormId,
}

impl Protos {
    /// allocate empty proto Forms for each canonical kind. handler
    /// tables are filled in by `World::install_object_handlers` and
    /// friends during phase A.7+.
    pub fn bootstrap(heap: &mut Heap) -> Self {
        // Object first; everything else's proto is Object.
        let object = heap.alloc(Form::with_proto(Value::Nil));
        let mk = |heap: &mut Heap| heap.alloc(Form::with_proto(Value::Form(object)));
        let nil = mk(heap);
        let bool_ = mk(heap);
        let integer = mk(heap);
        let float = mk(heap);
        let symbol = mk(heap);
        let char_ = mk(heap);
        let string = mk(heap);
        let bytes = mk(heap);
        let cons = mk(heap);
        let table = mk(heap);
        let method = mk(heap);
        let chunk = mk(heap);
        let env = mk(heap);
        let foreign = mk(heap);
        let frame = mk(heap);
        // Closure has Method as its parent.
        let closure = heap.alloc(Form::with_proto(Value::Form(method)));

        Protos {
            object,
            nil,
            bool_,
            integer,
            float,
            symbol,
            char_,
            string,
            bytes,
            cons,
            table,
            method,
            chunk,
            closure,
            env,
            foreign,
            frame,
        }
    }
}
