// Act — a value that represents an effect in flight. Acts resolve
// (or fail) asynchronously via the scheduler. then: chains a
// continuation; flatMap: and map: are aliases.

use moof_core::native;
use moof_core::heap::*;
use moof_core::value::Value;

pub fn register(heap: &mut Heap) {
    // pre-intern every symbol — captured by u32 in closures.
    let state_sym        = heap.intern("__state");
    let result_sym       = heap.intern("__result");
    let resolved_sym     = heap.intern("resolved");
    let failed_sym       = heap.intern("failed");
    let chain_sym        = heap.intern("__chain");
    let cont_fn_sym      = heap.intern("__cont_fn");
    let cont_val_sym     = heap.intern("__cont_val");
    let then_sym         = heap.intern("then:");
    let flatmap_sym      = heap.intern("flatMap:");
    let map_sym          = heap.intern("map:");
    let act_sym          = heap.intern("Act");

    let object_proto = heap.type_protos[PROTO_OBJ];
    let act_proto    = heap.make_object(object_proto);
    heap.type_protos[PROTO_ACT] = act_proto;
    let act_id = act_proto.as_any_object().unwrap();

    // describe / show — both delegate to format_act
    native(heap, act_id, "describe", move |heap, receiver, _args| {
        let s = format_act(heap, receiver, state_sym, result_sym, resolved_sym, failed_sym)?;
        Ok(heap.alloc_string(&s))
    });
    native(heap, act_id, "show", move |heap, receiver, _args| {
        let s = format_act(heap, receiver, state_sym, result_sym, resolved_sym, failed_sym)?;
        Ok(heap.alloc_string(&format!("{s}  : Act")))
    });

    // then: — the one chaining operation. appends f to the
    // continuation chain. when the Act resolves, f is called with
    // the resolved value. auto-flattens if f returns another Act.
    native(heap, act_id, "then:", move |heap, receiver, args| {
        let f  = args.first().copied().ok_or("then: needs a function")?;
        let id = receiver.as_any_object().ok_or("then: not an act")?;
        let is_resolved = heap.get(id).slot_get(state_sym)
            .map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);

        if is_resolved {
            let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
            let new_act = heap.make_pending_act();
            let new_act_id = new_act.as_any_object().unwrap();
            heap.get_mut(new_act_id).slot_set(cont_fn_sym, f);
            heap.get_mut(new_act_id).slot_set(cont_val_sym, result);
            heap.ready_acts.push(new_act_id);
            Ok(new_act)
        } else {
            let current_chain = heap.get(id).slot_get(chain_sym).unwrap_or(Value::NIL);
            let new_link = heap.cons(f, current_chain);
            heap.get_mut(id).slot_set(chain_sym, new_link);
            Ok(receiver)
        }
    });

    // flatMap: and map: alias then:
    let then_handler = heap.get(act_id).handler_get(then_sym).unwrap();
    heap.get_mut(act_id).handler_set(flatmap_sym, then_handler);
    heap.get_mut(act_id).handler_set(map_sym, then_handler);

    native(heap, act_id, "recover:", |_heap, receiver, _args| Ok(receiver));

    // Acts are deliberately OPAQUE. no `ok?`, no `result`, no
    // `inspect` probes from userland — those would let code
    // bypass the scheduler's resolution machinery. the ONLY way
    // to interact with an Act is to compose: [act then: f] or
    // [act recover: f]. see docs/concepts/effects.md.
    //
    // debugging tools that need to see act state use rust-side
    // heap introspection, not moof-side handlers.

    heap.env_def(act_sym, act_proto);
}

/// Format an Act for display (resolved / failed / pending).
fn format_act(heap: &Heap, receiver: Value, state_sym: u32, result_sym: u32,
              resolved_sym: u32, failed_sym: u32) -> Result<String, String> {
    let id = receiver.as_any_object().ok_or("not an act")?;
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
