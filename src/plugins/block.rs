use crate::plugins::{Plugin, native};
use crate::heap::*;
use crate::object::HeapObject;
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
            let id = receiver.as_any_object().ok_or("arity: not a closure")?;
            match heap.get(id) {
                HeapObject::Closure { arity, .. } => Ok(Value::integer(*arity as i64)),
                _ => Err("arity: not a closure".into()),
            }
        });

        // Block: operative? — true if this closure is an operative (fexpr)
        native(heap, block_id, "operative?", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("operative?: not a closure")?;
            match heap.get(id) {
                HeapObject::Closure { is_operative, .. } => Ok(Value::boolean(*is_operative)),
                _ => Err("operative?: not a closure".into()),
            }
        });

        // Block: wrap — convert operative to applicative (Kernel's wrap)
        // [operative wrap] => applicative (same code, args evaluated by caller)
        native(heap, block_id, "wrap", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("wrap: not a closure")?;
            match heap.get(id) {
                HeapObject::Closure { code_idx, arity, captures, parent, .. } => {
                    let code_idx = *code_idx;
                    let arity = *arity;
                    let parent = *parent;
                    let captures = captures.clone();
                    let new_id = heap.alloc(HeapObject::Closure {
                        parent,
                        code_idx,
                        arity,
                        is_operative: false,
                        captures,
                        handlers: Vec::new(),
                    });
                    let val = Value::nursery(new_id);
                    let call_sym = heap.sym_call;
                    heap.get_mut(new_id).handler_set(call_sym, val);
                    Ok(val)
                }
                _ => Err("wrap: not a closure".into()),
            }
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
