// Vat — the spawn/serve entrypoints. calling [Vat spawn: block]
// queues a SpawnRequest that the scheduler picks up between turns.

use crate::plugins::native;
use crate::heap::*;
use crate::object::HeapObject;
use crate::value::Value;

pub fn register(heap: &mut Heap) {
    let object_proto = heap.type_protos[PROTO_OBJ];

    let vat_proto  = heap.make_object(object_proto);
    let vat_id_obj = vat_proto.as_any_object().unwrap();

    native(heap, vat_id_obj, "spawn:", |heap, _receiver, args| {
        spawn_request(heap, args, false, false)
    });
    native(heap, vat_id_obj, "spawn:with:", |heap, _receiver, args| {
        spawn_request(heap, args, true, false)
    });
    native(heap, vat_id_obj, "serve:", |heap, _receiver, args| {
        spawn_request(heap, args, false, true)
    });
    native(heap, vat_id_obj, "serve:with:", |heap, _receiver, args| {
        spawn_request(heap, args, true, true)
    });

    native(heap, vat_id_obj, "describe", |heap, _receiver, _args| {
        Ok(heap.alloc_string("<Vat>"))
    });

    let vat_sym = heap.intern("Vat");
    heap.env_def(vat_sym, vat_proto);
}

/// Build a SpawnRequest and queue it. `with_args` decides whether
/// arg[1] is a list of spawn-time arguments. `serve` distinguishes
/// [Vat spawn:] (act resolves to closure result) from [Vat serve:]
/// (act resolves to a FarRef; vat stays alive).
fn spawn_request(heap: &mut Heap, args: &[Value], with_args: bool, serve: bool) -> Result<Value, String> {
    let first = args.first().copied().ok_or("spawn: needs a block or source string")?;

    let payload = if with_args {
        let args_val = args.get(1).copied().ok_or("spawn:with: needs args")?;
        if heap.as_closure(first).is_none() {
            return Err("spawn:with: first arg must be a closure".into());
        }
        SpawnPayload::ClosureWithArgs(first, heap.list_to_vec(args_val))
    } else {
        match first.as_any_object().map(|id| heap.get(id)) {
            Some(HeapObject::Text(s))     => SpawnPayload::Source(s.clone()),
            Some(HeapObject::Closure { .. }) => SpawnPayload::Closure(first),
            _ => return Err("spawn: argument must be a block or source string".into()),
        }
    };

    let act = heap.make_pending_act();
    let act_obj_id = act.as_any_object().unwrap();
    heap.spawn_queue.push(SpawnRequest {
        payload,
        act_id: act_obj_id,
        serve,
    });
    Ok(act)
}
