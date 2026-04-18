// Update — a state transition from a server handler.
//   (update { count: [@count + 1] } @count)
// describes "change count, reply with old count." Update's then:
// applies f to the reply and merges deltas if f returns another
// Update.

use crate::plugins::native;
use crate::heap::*;
use crate::value::Value;

pub fn register(heap: &mut Heap) {
    let object_proto = heap.type_protos[PROTO_OBJ];

    let update_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_UPDATE] = update_proto;
    let update_id = update_proto.as_any_object().unwrap();

    native(heap, update_id, "describe", |heap, receiver, _args| {
        Ok(heap.alloc_string(&format_update(heap, receiver)?))
    });
    native(heap, update_id, "show", |heap, receiver, _args| {
        Ok(heap.alloc_string(&format!("{}  : Update", format_update(heap, receiver)?)))
    });

    native(heap, update_id, "delta", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("delta: not an update")?;
        let delta_sym = heap.intern("__delta");
        Ok(heap.get(id).slot_get(delta_sym).unwrap_or(Value::NIL))
    });

    native(heap, update_id, "reply", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("reply: not an update")?;
        let reply_sym = heap.intern("__reply");
        Ok(heap.get(id).slot_get(reply_sym).unwrap_or(Value::NIL))
    });

    // then: — create a pending Act that runs f(reply). the scheduler
    // notices __merge_delta and merges our delta after f's result.
    native(heap, update_id, "then:", |heap, receiver, args| {
        let f  = args.first().copied().ok_or("then: needs a function")?;
        let id = receiver.as_any_object().ok_or("then: not an update")?;
        let reply_sym = heap.intern("__reply");
        let delta_sym = heap.intern("__delta");
        let reply     = heap.get(id).slot_get(reply_sym).unwrap_or(Value::NIL);
        let our_delta = heap.get(id).slot_get(delta_sym).unwrap_or(Value::NIL);

        let new_act = heap.make_pending_act();
        let new_act_id = new_act.as_any_object().unwrap();
        let cont_fn_sym     = heap.intern("__cont_fn");
        let cont_val_sym    = heap.intern("__cont_val");
        let merge_delta_sym = heap.intern("__merge_delta");
        heap.get_mut(new_act_id).slot_set(cont_fn_sym, f);
        heap.get_mut(new_act_id).slot_set(cont_val_sym, reply);
        heap.get_mut(new_act_id).handler_set(merge_delta_sym, our_delta);
        heap.ready_acts.push(new_act_id);
        Ok(new_act)
    });

    // map: is an alias for then: (same as on Act)
    let then_sym = heap.intern("then:");
    let map_sym  = heap.intern("map:");
    let then_handler = heap.get(update_id).handler_get(then_sym).unwrap();
    heap.get_mut(update_id).handler_set(map_sym, then_handler);

    let update_sym = heap.intern("Update");
    heap.env_def(update_sym, update_proto);

    // `update` global — (update delta) or (update delta reply).
    // call: handler because users write it as a function call.
    let update_fn = heap.register_native("update", |heap, _receiver, args| {
        // args[0] is the cons list of actual call args
        let arg_list = args.first().copied().unwrap_or(Value::NIL);
        let unpacked = heap.list_to_vec(arg_list);
        let delta = unpacked.first().copied().ok_or("update: needs a delta object")?;
        let reply = unpacked.get(1).copied().unwrap_or(Value::NIL);
        let update_proto = heap.type_protos[PROTO_UPDATE];
        let delta_sym = heap.intern("__delta");
        let reply_sym = heap.intern("__reply");
        Ok(heap.make_object_with_slots(
            update_proto,
            vec![delta_sym, reply_sym],
            vec![delta, reply],
        ))
    });
    let update_obj    = heap.make_object(object_proto);
    let update_obj_id = update_obj.as_any_object().unwrap();
    let call_sym      = heap.sym_call;
    heap.get_mut(update_obj_id).handler_set(call_sym, update_fn);
    let update_fn_sym = heap.intern("update");
    heap.env_def(update_fn_sym, update_obj);
}

fn format_update(heap: &Heap, receiver: Value) -> Result<String, String> {
    let id = receiver.as_any_object().ok_or("not an update")?;
    let delta_sym = heap.find_symbol("__delta").unwrap_or(0);
    let reply_sym = heap.find_symbol("__reply").unwrap_or(0);
    let delta = heap.get(id).slot_get(delta_sym)
        .map(|v| heap.format_value(v)).unwrap_or_else(|| "nil".into());
    let reply = heap.get(id).slot_get(reply_sym)
        .map(|v| heap.format_value(v)).unwrap_or_else(|| "nil".into());
    Ok(format!("<Update delta:{delta} reply:{reply}>"))
}
