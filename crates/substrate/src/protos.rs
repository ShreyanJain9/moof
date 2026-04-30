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
    pub symbol: FormId,
    pub string: FormId,
    pub list: FormId,
    pub method: FormId,
    pub chunk: FormId,
    pub closure: FormId,
    pub env: FormId,
    pub foreign: FormId,
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
        let symbol = mk(heap);
        let string = mk(heap);
        let list = mk(heap);
        let method = mk(heap);
        let chunk = mk(heap);
        let env = mk(heap);
        let foreign = mk(heap);
        // Closure has Method as its parent.
        let closure = heap.alloc(Form::with_proto(Value::Form(method)));

        Protos {
            object,
            nil,
            bool_,
            integer,
            symbol,
            string,
            list,
            method,
            chunk,
            closure,
            env,
            foreign,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_assigns_distinct_ids() {
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        let ids = [
            p.object, p.nil, p.bool_, p.integer, p.symbol, p.list,
            p.method, p.chunk, p.closure, p.env, p.foreign,
        ];
        // each proto-Form is distinct.
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j], "duplicate proto-Form id");
            }
        }
    }

    #[test]
    fn object_has_nil_proto() {
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        assert_eq!(h.get(p.object).proto, Value::Nil);
    }

    #[test]
    fn integer_inherits_object() {
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        assert_eq!(h.get(p.integer).proto, Value::Form(p.object));
    }

    #[test]
    fn closure_inherits_method() {
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        assert_eq!(h.get(p.closure).proto, Value::Form(p.method));
    }

    #[test]
    fn list_inherits_object() {
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        // List is direct child of Object until phase A.10
        // introduces Iterable/Sized/etc. mixins.
        assert_eq!(h.get(p.list).proto, Value::Form(p.object));
    }

    #[test]
    fn proto_chain_is_acyclic() {
        // L2: every Form's transitive proto chain reaches Object,
        // whose proto is `nil`. there are no cycles.
        let mut h = Heap::new();
        let p = Protos::bootstrap(&mut h);
        for proto_id in [p.integer, p.bool_, p.symbol, p.list, p.method, p.chunk, p.closure, p.env, p.foreign] {
            let mut current = Value::Form(proto_id);
            let mut visited = std::collections::HashSet::new();
            loop {
                match current {
                    Value::Nil => break,
                    Value::Form(id) => {
                        assert!(visited.insert(id), "proto chain cycle: {:?}", id);
                        current = h.get(id).proto;
                    }
                    _ => panic!("non-Form in proto chain"),
                }
            }
            assert!(visited.contains(&p.object), "proto chain didn't reach Object");
        }
    }
}
