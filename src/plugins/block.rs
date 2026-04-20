use crate::plugins::{Plugin, native};
use crate::heap::*;
use crate::value::Value;

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn name(&self) -> &str { "block" }

    fn register(&self, heap: &mut Heap) {
        let object_proto = heap.type_protos[PROTO_OBJ];

        // -- Block/Closure prototype (type_protos[PROTO_CLOSURE]) --
        let block_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_CLOSURE] = block_proto;
        let block_id = block_proto.as_any_object().unwrap();

        // Block: arity — return the arity of the closure
        native(heap, block_id, "arity", |heap, receiver, _args| {
            heap.closure_arity(receiver)
                .map(|n| Value::integer(n as i64))
                .ok_or_else(|| "arity: not a closure".into())
        });

        // Block: pure? — true if no FarRef captures (safe to memoize/parallelize)
        native(heap, block_id, "pure?", |heap, receiver, _args| {
            if heap.as_closure(receiver).is_none() {
                return Err("pure?: not a closure".into());
            }
            Ok(Value::boolean(heap.closure_is_pure(receiver)))
        });

        // Block: operative? — true if this closure is an operative (fexpr)
        native(heap, block_id, "operative?", |heap, receiver, _args| {
            let (_, is_op) = heap.as_closure(receiver).ok_or("operative?: not a closure")?;
            Ok(Value::boolean(is_op))
        });

        // Block: wrap — convert operative to applicative (Kernel's wrap).
        // [operative wrap] => applicative (same code, args evaluated by caller).
        // Produces a new closure General with is_operative=false.
        native(heap, block_id, "wrap", |heap, receiver, _args| {
            let (code_idx, _) = heap.as_closure(receiver).ok_or("wrap: not a closure")?;
            let arity = heap.closure_arity(receiver).unwrap_or(0);
            let captures = heap.closure_captures(receiver);
            Ok(heap.make_closure(code_idx, arity, false, &captures))
        });

        // Block: describe — human-readable description
        native(heap, block_id, "describe", |heap, receiver, _args| {
            let s = heap.format_value(receiver);
            Ok(heap.alloc_string(&s))
        });

        // register Block as global
        let block_sym = heap.intern("Block");
        heap.env_def(block_sym, block_proto);

        // -- Root environment: expose the actual env object as 'Env' --
        let env_sym = heap.intern("Env");
        heap.env_def(env_sym, Value::nursery(heap.env));
    }
}
