// FarRef — a proxy for an object in another vat. Every send to a
// FarRef is intercepted by doesNotUnderstand: and turned into an
// outgoing message + a pending Act for the result.

use crate::plugins::native;
use crate::heap::*;

pub fn register(heap: &mut Heap) {
    let object_proto = heap.type_protos[PROTO_OBJ];

    let farref_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_FARREF] = farref_proto;
    let farref_id = farref_proto.as_any_object().unwrap();

    let target_vat_sym = heap.intern("__target_vat");
    let target_obj_sym = heap.intern("__target_obj");

    // doesNotUnderstand: intercepts ALL sends to a FarRef.
    // queues an OutgoingMessage to the target vat + returns a pending Act.
    native(heap, farref_id, "doesNotUnderstand:", move |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("farref DNU: not an object")?;
        let target_vat = heap.get(id).slot_get(target_vat_sym)
            .and_then(|v| v.as_integer()).ok_or("farref: missing __target_vat")? as u32;
        let target_obj = heap.get(id).slot_get(target_obj_sym)
            .and_then(|v| v.as_integer()).ok_or("farref: missing __target_obj")? as u32;

        let selector = args.first().and_then(|v| v.as_symbol()).unwrap_or(0);
        let msg_args = if args.len() > 1 {
            heap.list_to_vec(args[1])
        } else {
            Vec::new()
        };

        let act = heap.make_act(target_vat, target_obj, selector);
        heap.outbox.push(OutgoingMessage {
            target_vat_id: target_vat,
            target_obj_id: target_obj,
            selector,
            args: msg_args,
            act_id: act.as_any_object().unwrap(),
        });
        Ok(act)
    });

    native(heap, farref_id, "describe", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not a farref")?;
        let vat = heap.get(id).slot_get(target_vat_sym)
            .and_then(|v| v.as_integer()).unwrap_or(-1);
        let obj = heap.get(id).slot_get(target_obj_sym)
            .and_then(|v| v.as_integer()).unwrap_or(-1);
        Ok(heap.alloc_string(&format!("<far-ref vat:{vat} obj:{obj}>")))
    });

    let farref_sym = heap.intern("FarRef");
    heap.env_def(farref_sym, farref_proto);
}
