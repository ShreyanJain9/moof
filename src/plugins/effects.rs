// Effects plugin: Act, FarRef, Update, Vat, Error prototypes.
//
// These are the effectful types — everything that touches the outside
// world or crosses vat boundaries lives here.

use crate::plugins::{Plugin, native};
use crate::heap::*;
use crate::object::HeapObject;
use crate::value::Value;

/// Format an Act for display.
fn format_act(heap: &Heap, receiver: Value) -> Result<String, String> {
    let id = receiver.as_any_object().ok_or("not an act")?;
    let state_sym = heap.find_symbol("__state").unwrap_or(0);
    let result_sym = heap.find_symbol("__result").unwrap_or(0);
    let resolved_sym = heap.find_symbol("resolved").unwrap_or(u32::MAX);
    let failed_sym = heap.find_symbol("failed").unwrap_or(u32::MAX);

    let state = heap.get(id).slot_get(state_sym);
    let is_resolved = state.map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);
    let is_failed = state.map(|v| v == Value::symbol(failed_sym)).unwrap_or(false);

    if is_resolved {
        let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
        let result_str = heap.format_value(result);
        Ok(format!("<Act resolved: {result_str}>"))
    } else if is_failed {
        let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
        let result_str = heap.format_value(result);
        Ok(format!("<Act failed: {result_str}>"))
    } else {
        Ok("<Act pending>".into())
    }
}

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn name(&self) -> &str { "effects" }

    fn register(&self, heap: &mut Heap) {
        let object_proto = heap.type_protos[PROTO_OBJ];

        // -- Error prototype (PROTO_ERROR) --
        let error_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_ERROR] = error_proto;
        let error_id = error_proto.as_any_object().unwrap();

        native(heap, error_id, "message", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("message: not an object")?;
            Ok(heap.get(id).slot_get(heap.sym_message).unwrap_or(Value::NIL))
        });
        native(heap, error_id, "describe", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("describe: not an object")?;
            let msg_val = heap.get(id).slot_get(heap.sym_message).unwrap_or(Value::NIL);
            let msg = heap.format_value(msg_val);
            let s = format!("Error: {}", msg);
            Ok(heap.alloc_string(&s))
        });

        let error_sym = heap.intern("Error");
        heap.env_def(error_sym, error_proto);

        // -- FarRef prototype (PROTO_FARREF) --
        let farref_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_FARREF] = farref_proto;
        let farref_id = farref_proto.as_any_object().unwrap();

        let target_vat_sym = heap.intern("__target_vat");
        let target_obj_sym = heap.intern("__target_obj");

        // FarRef: doesNotUnderstand: — intercept ALL sends, queue to outbox
        {
            let tvs = target_vat_sym;
            let tos = target_obj_sym;
            native(heap, farref_id, "doesNotUnderstand:", move |heap, receiver, args| {
                let id = receiver.as_any_object().ok_or("farref DNU: not an object")?;
                let target_vat = heap.get(id).slot_get(tvs)
                    .and_then(|v| v.as_integer()).ok_or("farref: missing __target_vat")? as u32;
                let target_obj = heap.get(id).slot_get(tos)
                    .and_then(|v| v.as_integer()).ok_or("farref: missing __target_obj")? as u32;

                let selector = args.first().and_then(|v| v.as_symbol()).unwrap_or(0);
                let msg_args = if args.len() > 1 {
                    heap.list_to_vec(args[1])
                } else {
                    Vec::new()
                };

                // create an Act for the result
                let act = heap.make_act(target_vat, target_obj, selector);

                // push to outbox
                heap.outbox.push(OutgoingMessage {
                    target_vat_id: target_vat,
                    target_obj_id: target_obj,
                    selector,
                    args: msg_args,
                    act_id: act.as_any_object().unwrap(),
                });

                Ok(act)
            });
        }

        // FarRef: describe
        {
            let tvs = target_vat_sym;
            let tos = target_obj_sym;
            native(heap, farref_id, "describe", move |heap, receiver, _args| {
                let id = receiver.as_any_object().ok_or("describe: not a farref")?;
                let vat = heap.get(id).slot_get(tvs)
                    .and_then(|v| v.as_integer()).unwrap_or(-1);
                let obj = heap.get(id).slot_get(tos)
                    .and_then(|v| v.as_integer()).unwrap_or(-1);
                let s = format!("<far-ref vat:{vat} obj:{obj}>");
                Ok(heap.alloc_string(&s))
            });
        }

        let farref_sym = heap.intern("FarRef");
        heap.env_def(farref_sym, farref_proto);

        // -- Act prototype (PROTO_ACT) --
        let act_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_ACT] = act_proto;
        let act_id = act_proto.as_any_object().unwrap();

        // Act: describe / show
        {
            native(heap, act_id, "describe", |heap, receiver, _args| {
                let s = format_act(heap, receiver)?;
                Ok(heap.alloc_string(&s))
            });

            native(heap, act_id, "show", |heap, receiver, _args| {
                let s = format_act(heap, receiver)?;
                Ok(heap.alloc_string(&format!("{s}  : Act")))
            });
        }

        // Act: then: — the one chaining operation.
        // appends f to the continuation chain. when the Act resolves,
        // f is called with the resolved value. if f returns an Act,
        // auto-flatten. if f returns a plain value, resolve with it.
        // flatMap: and map: are aliases — no type-level distinction needed.
        {
            let chain_sym = heap.intern("__chain");
            native(heap, act_id, "then:", move |heap, receiver, args| {
                let f = args.first().copied().ok_or("then: needs a function")?;
                let id = receiver.as_any_object().ok_or("then: not an act")?;

                let state_sym_local = heap.intern("__state");
                let resolved_sym = heap.intern("resolved");
                let result_sym = heap.intern("__result");
                let is_resolved = heap.get(id).slot_get(state_sym_local)
                    .map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);

                if is_resolved {
                    // already resolved — create a ready Act for the scheduler to run
                    let result = heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL);
                    let new_act = heap.make_pending_act();
                    let new_act_id = new_act.as_any_object().unwrap();
                    let cont_fn_sym = heap.intern("__cont_fn");
                    let cont_val_sym = heap.intern("__cont_val");
                    heap.get_mut(new_act_id).slot_set(cont_fn_sym, f);
                    heap.get_mut(new_act_id).slot_set(cont_val_sym, result);
                    heap.ready_acts.push(new_act_id);
                    Ok(new_act)
                } else {
                    // pending — append to continuation chain
                    let current_chain = heap.get(id).slot_get(chain_sym).unwrap_or(Value::NIL);
                    let new_link = heap.cons(f, current_chain);
                    heap.get_mut(id).slot_set(chain_sym, new_link);
                    Ok(receiver)
                }
            });
            // flatMap: and map: are aliases for then:
            let then_sym = heap.intern("then:");
            let flatmap_sym = heap.intern("flatMap:");
            let map_sym = heap.intern("map:");
            let then_handler = heap.get(act_id).handler_get(then_sym).unwrap();
            heap.get_mut(act_id).handler_set(flatmap_sym, then_handler);
            heap.get_mut(act_id).handler_set(map_sym, then_handler);
        }

        // Act: recover: — no-op on success, handler on failure
        {
            native(heap, act_id, "recover:", |_heap, receiver, _args| {
                Ok(receiver)
            });
        }

        // Act: ok? — true if resolved successfully
        {
            native(heap, act_id, "ok?", |heap, receiver, _args| {
                let id = receiver.as_any_object().ok_or("ok?: not an act")?;
                let state_sym_local = heap.intern("__state");
                let resolved_sym = heap.intern("resolved");
                let is_resolved = heap.get(id).slot_get(state_sym_local)
                    .map(|v| v == Value::symbol(resolved_sym)).unwrap_or(false);
                Ok(if is_resolved { Value::TRUE } else { Value::FALSE })
            });
        }

        // Act: result — get the resolved value (or nil if pending)
        {
            native(heap, act_id, "result", |heap, receiver, _args| {
                let id = receiver.as_any_object().ok_or("result: not an act")?;
                let result_sym = heap.intern("__result");
                Ok(heap.get(id).slot_get(result_sym).unwrap_or(Value::NIL))
            });
        }

        // Act: inspect — return the Act's description as data
        {
            native(heap, act_id, "inspect", |heap, receiver, _args| {
                let id = receiver.as_any_object().ok_or("inspect: not an act")?;
                let tgt_sym = heap.intern("__target_vat");
                let obj_sym = heap.intern("__target_obj");
                let sel_sym = heap.intern("__selector");
                let state_sym_local = heap.intern("__state");

                let target = heap.get(id).slot_get(tgt_sym).unwrap_or(Value::NIL);
                let obj = heap.get(id).slot_get(obj_sym).unwrap_or(Value::NIL);
                let sel = heap.get(id).slot_get(sel_sym).unwrap_or(Value::NIL);
                let state = heap.get(id).slot_get(state_sym_local).unwrap_or(Value::NIL);

                let t_sym = heap.intern("target");
                let o_sym = heap.intern("object");
                let s_sym = heap.intern("selector");
                let st_sym = heap.intern("state");
                Ok(heap.make_object_with_slots(
                    Value::NIL,
                    vec![t_sym, o_sym, s_sym, st_sym],
                    vec![target, obj, sel, state],
                ))
            });
        }

        let act_sym = heap.intern("Act");
        heap.env_def(act_sym, act_proto);

        // -- Vat prototype --
        let vat_proto = heap.make_object(object_proto);
        let vat_id_obj = vat_proto.as_any_object().unwrap();

        // [Vat spawn: block-or-source] — queue a spawn request, return an Act
        native(heap, vat_id_obj, "spawn:", |heap, _receiver, args| {
            let arg = args.first().copied().ok_or("spawn: needs a block or source string")?;

            let payload = if let Some(obj_id) = arg.as_any_object() {
                match heap.get(obj_id) {
                    HeapObject::Text(s) => {
                        SpawnPayload::Source(s.clone())
                    }
                    HeapObject::Closure { .. } => {
                        SpawnPayload::Closure(arg)
                    }
                    _ => return Err("spawn: argument must be a block or source string".into()),
                }
            } else {
                return Err("spawn: argument must be a block or source string".into());
            };

            let act = heap.make_pending_act();
            let act_obj_id = act.as_any_object().unwrap();

            heap.spawn_queue.push(SpawnRequest {
                payload,
                act_id: act_obj_id,
                serve: false,
            });

            Ok(act)
        });

        // [Vat spawn:with: block args] — pass args to the closure in the new vat
        native(heap, vat_id_obj, "spawn:with:", |heap, _receiver, args| {
            let block = args.first().copied().ok_or("spawn:with: needs a block")?;
            let args_val = args.get(1).copied().ok_or("spawn:with: needs args")?;

            if heap.as_closure(block).is_none() {
                return Err("spawn:with: first arg must be a closure".into());
            }
            let spawn_args = heap.list_to_vec(args_val);

            let act = heap.make_pending_act();
            let act_obj_id = act.as_any_object().unwrap();

            heap.spawn_queue.push(SpawnRequest {
                payload: SpawnPayload::ClosureWithArgs(block, spawn_args),
                act_id: act_obj_id,
                serve: false,
            });

            Ok(act)
        });

        // [Vat serve: block] — spawn a server vat, return FarRef (object stays in vat)
        native(heap, vat_id_obj, "serve:", |heap, _receiver, args| {
            let arg = args.first().copied().ok_or("serve: needs a block or source string")?;
            let payload = if let Some(obj_id) = arg.as_any_object() {
                match heap.get(obj_id) {
                    HeapObject::Text(s) => SpawnPayload::Source(s.clone()),
                    HeapObject::Closure { .. } => SpawnPayload::Closure(arg),
                    _ => return Err("serve: argument must be a block or source string".into()),
                }
            } else {
                return Err("serve: argument must be a block or source string".into());
            };

            let act = heap.make_pending_act();
            let act_obj_id = act.as_any_object().unwrap();

            heap.spawn_queue.push(SpawnRequest {
                payload,
                act_id: act_obj_id,
                serve: true,  // return FarRef, keep vat alive
            });

            Ok(act)
        });

        // [Vat serve:with: block args] — serve with args
        native(heap, vat_id_obj, "serve:with:", |heap, _receiver, args| {
            let block = args.first().copied().ok_or("serve:with: needs a block")?;
            let args_val = args.get(1).copied().ok_or("serve:with: needs args")?;

            if heap.as_closure(block).is_none() {
                return Err("serve:with: first arg must be a closure".into());
            }
            let spawn_args = heap.list_to_vec(args_val);

            let act = heap.make_pending_act();
            let act_obj_id = act.as_any_object().unwrap();

            heap.spawn_queue.push(SpawnRequest {
                payload: SpawnPayload::ClosureWithArgs(block, spawn_args),
                act_id: act_obj_id,
                serve: true,
            });

            Ok(act)
        });

        // [Vat describe]
        native(heap, vat_id_obj, "describe", |heap, _receiver, _args| {
            Ok(heap.alloc_string("<Vat>"))
        });

        let vat_sym = heap.intern("Vat");
        heap.env_def(vat_sym, vat_proto);

        // -- Update type (PROTO_UPDATE) --
        let update_proto = heap.make_object(object_proto);
        heap.type_protos[PROTO_UPDATE] = update_proto;
        let update_id = update_proto.as_any_object().unwrap();

        // Update: describe
        native(heap, update_id, "describe", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("describe: not an update")?;
            let delta_sym = heap.intern("__delta");
            let reply_sym = heap.intern("__reply");
            let delta = heap.get(id).slot_get(delta_sym)
                .map(|v| heap.format_value(v)).unwrap_or("nil".into());
            let reply = heap.get(id).slot_get(reply_sym)
                .map(|v| heap.format_value(v)).unwrap_or("nil".into());
            let s = format!("<Update delta:{delta} reply:{reply}>");
            Ok(heap.alloc_string(&s))
        });

        // Update: show
        native(heap, update_id, "show", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("show: not an update")?;
            let delta_sym = heap.intern("__delta");
            let reply_sym = heap.intern("__reply");
            let delta = heap.get(id).slot_get(delta_sym)
                .map(|v| heap.format_value(v)).unwrap_or("nil".into());
            let reply = heap.get(id).slot_get(reply_sym)
                .map(|v| heap.format_value(v)).unwrap_or("nil".into());
            let s = format!("<Update delta:{delta} reply:{reply}>  : Update");
            Ok(heap.alloc_string(&s))
        });

        // Update: delta
        native(heap, update_id, "delta", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("delta: not an update")?;
            let delta_sym = heap.intern("__delta");
            Ok(heap.get(id).slot_get(delta_sym).unwrap_or(Value::NIL))
        });

        // Update: reply
        native(heap, update_id, "reply", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("reply: not an update")?;
            let reply_sym = heap.intern("__reply");
            Ok(heap.get(id).slot_get(reply_sym).unwrap_or(Value::NIL))
        });

        let update_sym = heap.intern("Update");
        heap.env_def(update_sym, update_proto);

        // -- `update` function --
        // (update delta) — state transition, reply is nil
        // (update delta reply) — state transition with reply
        let update_fn_sym = heap.intern("update");
        let update_fn = heap.register_native("update", |heap, _receiver, args| {
            // args comes in as call: dispatch — args[0] is a cons list of actual args
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
        // register as a global — wrap the native in an object with call: handler
        let update_obj = heap.make_object(object_proto);
        let update_obj_id = update_obj.as_any_object().unwrap();
        let call_sym = heap.sym_call;
        heap.get_mut(update_obj_id).handler_set(call_sym, update_fn);
        heap.env_def(update_fn_sym, update_obj);
    }
}
