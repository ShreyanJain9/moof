// Result, Ok, Err prototypes.
//
// Ok and Err are native (not moof-defined) because make_error
// needs to construct Err values before the moof stdlib loads.
// Result is the shared parent for [obj is: Result] checks.

use moof_core::native;
use moof_core::heap::*;
use moof_core::value::Value;

pub fn register(heap: &mut Heap) {
    // pre-intern symbols used by every handler
    let value_sym    = heap.intern("value");
    let cont_fn_sym  = heap.intern("__cont_fn");
    let cont_val_sym = heap.intern("__cont_val");
    let wrap_ok_sym  = heap.intern("__wrap_ok");
    let result_sym   = heap.intern("Result");
    let ok_sym       = heap.intern("Ok");
    let err_sym      = heap.intern("Err");

    let object_proto = heap.type_protos[PROTO_OBJ];

    // Result — shared parent
    let result_proto = heap.make_object(object_proto);
    let result_id    = result_proto.as_any_object().unwrap();
    native(heap, result_id, "describe", |heap, _recv, _args| {
        Ok(heap.alloc_string("<Result>"))
    });
    heap.env_def(result_sym, result_proto);

    // -- Ok --
    let ok_proto = heap.make_object(result_proto);
    heap.type_protos[PROTO_OK] = ok_proto;
    let ok_id = ok_proto.as_any_object().unwrap();

    native(heap, ok_id, "value", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("value: not ok")?;
        Ok(heap.get(id).slot_get(value_sym).unwrap_or(Value::NIL))
    });

    // then: — unwrap, apply f via a ready Act (can't call closures from native).
    native(heap, ok_id, "then:", move |heap, receiver, args| {
        let f = args.first().copied().ok_or("then: needs a function")?;
        let id = receiver.as_any_object().ok_or("then: not ok")?;
        let val = heap.get(id).slot_get(value_sym).unwrap_or(Value::NIL);
        schedule_continuation(heap, f, val, false, cont_fn_sym, cont_val_sym, wrap_ok_sym)
    });

    // map: — like then: but wraps result back in Ok (via __wrap_ok flag).
    native(heap, ok_id, "map:", move |heap, receiver, args| {
        let f = args.first().copied().ok_or("map: needs a function")?;
        let id = receiver.as_any_object().ok_or("map: not ok")?;
        let val = heap.get(id).slot_get(value_sym).unwrap_or(Value::NIL);
        schedule_continuation(heap, f, val, true, cont_fn_sym, cont_val_sym, wrap_ok_sym)
    });

    native(heap, ok_id, "recover:", |_heap, receiver, _args| Ok(receiver));
    native(heap, ok_id, "ok?",      |_heap, _receiver, _args| Ok(Value::TRUE));

    native(heap, ok_id, "describe", move |heap, receiver, _args| {
        let id  = receiver.as_any_object().ok_or("describe: not ok")?;
        let val = heap.get(id).slot_get(value_sym).unwrap_or(Value::NIL);
        Ok(heap.alloc_string(&format!("Ok({})", heap.format_value(val))))
    });
    native(heap, ok_id, "show", move |heap, receiver, _args| {
        let id  = receiver.as_any_object().ok_or("show: not ok")?;
        let val = heap.get(id).slot_get(value_sym).unwrap_or(Value::NIL);
        Ok(heap.alloc_string(&format!("Ok({})  : Result", heap.format_value(val))))
    });

    heap.env_def(ok_sym, ok_proto);

    // -- Err --
    let err_proto = heap.make_object(result_proto);
    heap.type_protos[PROTO_ERR] = err_proto;
    let err_id = err_proto.as_any_object().unwrap();
    let message_sym = heap.sym_message;

    native(heap, err_id, "message", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("message: not err")?;
        Ok(heap.get(id).slot_get(message_sym).unwrap_or(Value::NIL))
    });

    // Err short-circuits: then: and map: return self unchanged.
    native(heap, err_id, "then:", |_heap, receiver, _args| Ok(receiver));
    native(heap, err_id, "map:",  |_heap, receiver, _args| Ok(receiver));

    // recover: is the ONE handler that unwraps an Err — applies f to the message.
    native(heap, err_id, "recover:", move |heap, receiver, args| {
        let f = args.first().copied().ok_or("recover: needs a function")?;
        let id = receiver.as_any_object().ok_or("recover: not err")?;
        let msg = heap.get(id).slot_get(message_sym).unwrap_or(Value::NIL);
        schedule_continuation(heap, f, msg, false, cont_fn_sym, cont_val_sym, wrap_ok_sym)
    });

    native(heap, err_id, "ok?", |_heap, _receiver, _args| Ok(Value::FALSE));

    native(heap, err_id, "describe", move |heap, receiver, _args| {
        let id  = receiver.as_any_object().ok_or("describe: not err")?;
        let msg = heap.get(id).slot_get(message_sym).unwrap_or(Value::NIL);
        Ok(heap.alloc_string(&format!("Err({})", heap.format_value(msg))))
    });
    native(heap, err_id, "show", move |heap, receiver, _args| {
        let id  = receiver.as_any_object().ok_or("show: not err")?;
        let msg = heap.get(id).slot_get(message_sym).unwrap_or(Value::NIL);
        Ok(heap.alloc_string(&format!("Err({})  : Result", heap.format_value(msg))))
    });

    heap.env_def(err_sym, err_proto);
}

/// Create a pending Act that will call f(val) when the scheduler
/// drains. Shared between Ok#then:, Ok#map:, and Err#recover:.
fn schedule_continuation(
    heap: &mut Heap, f: Value, val: Value, wrap_ok: bool,
    cont_fn_sym: u32, cont_val_sym: u32, wrap_ok_sym: u32,
) -> Result<Value, String> {
    let new_act = heap.make_pending_act();
    let new_act_id = new_act.as_any_object().unwrap();
    heap.get_mut(new_act_id).slot_set(cont_fn_sym, f);
    heap.get_mut(new_act_id).slot_set(cont_val_sym, val);
    if wrap_ok {
        heap.get_mut(new_act_id).handler_set(wrap_ok_sym, Value::TRUE);
    }
    heap.ready_acts.push(new_act_id);
    Ok(new_act)
}
