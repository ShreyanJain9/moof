// Act — a value that represents an effect in flight. Acts resolve
// (or fail) asynchronously via the scheduler. then: chains a
// continuation; flatMap: and map: are aliases.

use crate::plugins::native;
use crate::heap::*;
use crate::value::Value;

/// Format an Act for display (resolved / failed / pending).
fn format_act(heap: &Heap, receiver: Value) -> Result<String, String> {
    let id = receiver.as_any_object().ok_or("not an act")?;
    let state_sym    = heap.find_symbol("__state").unwrap_or(0);
    let result_sym   = heap.find_symbol("__result").unwrap_or(0);
    let resolved_sym = heap.find_symbol("resolved").unwrap_or(u32::MAX);
    let failed_sym   = heap.find_symbol("failed").unwrap_or(u32::MAX);

    let state = heap.get(id).slot_get(state_sym);
    let is_resolved = state.map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);
    let is_failed   = state.map(|v| v == Value::symbol(failed_sym)).unwrap_or(false);

    if is_resolved {
        let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
        Ok(format!("<Act resolved: {}>", heap.format_value(result)))
    } else if is_failed {
        let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
        Ok(format!("<Act failed: {}>", heap.format_value(result)))
    } else {
        Ok("<Act pending>".into())
    }
}

pub fn register(heap: &mut Heap) {
    let object_proto = heap.type_protos[PROTO_OBJ];

    let act_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_ACT] = act_proto;
    let act_id = act_proto.as_any_object().unwrap();

    // describe / show
    native(heap, act_id, "describe", |heap, receiver, _args| {
        let s = format_act(heap, receiver)?;
        Ok(heap.alloc_string(&s))
    });
    native(heap, act_id, "show", |heap, receiver, _args| {
        let s = format_act(heap, receiver)?;
        Ok(heap.alloc_string(&format!("{s}  : Act")))
    });

    // then: — the one chaining operation. appends f to the
    // continuation chain. when the Act resolves, f is called with
    // the resolved value. auto-flattens if f returns another Act.
    let chain_sym = heap.intern("__chain");
    native(heap, act_id, "then:", move |heap, receiver, args| {
        let f  = args.first().copied().ok_or("then: needs a function")?;
        let id = receiver.as_any_object().ok_or("then: not an act")?;

        let state_sym    = heap.intern("__state");
        let resolved_sym = heap.intern("resolved");
        let result_sym   = heap.intern("__result");
        let is_resolved  = heap.get(id).slot_get(state_sym)
            .map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);

        if is_resolved {
            // resolved: create a ready Act for the scheduler to run
            let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
            let new_act = heap.make_pending_act();
            let new_act_id = new_act.as_any_object().unwrap();
            let cont_fn_sym  = heap.intern("__cont_fn");
            let cont_val_sym = heap.intern("__cont_val");
            heap.get_mut(new_act_id).slot_set(cont_fn_sym, f);
            heap.get_mut(new_act_id).slot_set(cont_val_sym, result);
            heap.ready_acts.push(new_act_id);
            Ok(new_act)
        } else {
            // pending: append to continuation chain
            let current_chain = heap.get(id).slot_get(chain_sym).unwrap_or(Value::NIL);
            let new_link = heap.cons(f, current_chain);
            heap.get_mut(id).slot_set(chain_sym, new_link);
            Ok(receiver)
        }
    });

    // flatMap: and map: alias then:
    let then_sym    = heap.intern("then:");
    let flatmap_sym = heap.intern("flatMap:");
    let map_sym     = heap.intern("map:");
    let then_handler = heap.get(act_id).handler_get(then_sym).unwrap();
    heap.get_mut(act_id).handler_set(flatmap_sym, then_handler);
    heap.get_mut(act_id).handler_set(map_sym, then_handler);

    native(heap, act_id, "recover:", |_heap, receiver, _args| Ok(receiver));

    native(heap, act_id, "ok?", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("ok?: not an act")?;
        let state_sym    = heap.intern("__state");
        let resolved_sym = heap.intern("resolved");
        let is_resolved  = heap.get(id).slot_get(state_sym)
            .map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);
        Ok(Value::boolean(is_resolved))
    });

    native(heap, act_id, "result", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("result: not an act")?;
        let result_sym = heap.intern("__result");
        Ok(heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL))
    });

    // inspect — return the Act's state as a data record
    native(heap, act_id, "inspect", |heap, receiver, _args| {
        let id  = receiver.as_any_object().ok_or("inspect: not an act")?;
        let tv  = heap.intern("__target_vat");
        let to  = heap.intern("__target_obj");
        let sl  = heap.intern("__selector");
        let st  = heap.intern("__state");
        let tgt   = heap.get(id).slot_get(tv).unwrap_or(Value::NIL);
        let obj   = heap.get(id).slot_get(to).unwrap_or(Value::NIL);
        let sel   = heap.get(id).slot_get(sl).unwrap_or(Value::NIL);
        let state = heap.get(id).slot_get(st).unwrap_or(Value::NIL);

        let t_sym  = heap.intern("target");
        let o_sym  = heap.intern("object");
        let s_sym  = heap.intern("selector");
        let st_sym = heap.intern("state");
        Ok(heap.make_object_with_slots(
            Value::NIL,
            vec![t_sym, o_sym, s_sym, st_sym],
            vec![tgt, obj, sel, state],
        ))
    });

    let act_sym = heap.intern("Act");
    heap.env_def(act_sym, act_proto);
}
