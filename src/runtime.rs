// Runtime initialization: type prototypes and native handlers.
//
// Extracted from compiler.rs. The compiler compiles AST to bytecode.
// This module creates the objectspace: prototypes, handlers, globals.

use crate::heap::*;
use crate::object::HeapObject;
use crate::value::Value;

/// Register a native handler on a prototype.
/// Handles symbol interning and handler installation in one call,
/// avoiding the mutable borrow conflict.
fn native(
    heap: &mut Heap,
    proto_id: u32,
    selector: &str,
    f: impl Fn(&mut Heap, Value, &[Value]) -> Result<Value, String> + 'static,
) {
    let sym = heap.intern(selector);
    let h = heap.register_native(selector, f);
    heap.get_mut(proto_id).handler_set(sym, h);
}

/// Register type prototypes and native handlers on the heap.
pub fn register_type_protos(heap: &mut Heap) {
    // pre-intern symbols used by the compiler's defmethod
    heap.intern("self");
    // create the Object prototype (root of all delegation)
    let object_proto = heap.make_object(Value::NIL);
    heap.type_protos[PROTO_OBJ] = object_proto; // object type
    let obj_id = object_proto.as_any_object().unwrap();

    // Object: slotAt:
    let slot_at_handler = heap.register_native("obj_slotAt", |heap, receiver, args| {
        let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt: arg must be a symbol")?;
        if let Some(id) = receiver.as_any_object() {
            // for Pair, support car/cdr as virtual slots
            match heap.get(id) {
                HeapObject::Pair(car, cdr) => {
                    let car_v = *car; let cdr_v = *cdr;
                    if name == heap.sym_car { return Ok(car_v); }
                    if name == heap.sym_cdr { return Ok(cdr_v); }
                    return Ok(Value::NIL);
                }
                _ => {}
            }
            Ok(heap.get(id).slot_get(name).unwrap_or(Value::NIL))
        } else {
            Ok(Value::NIL) // primitives have no slots
        }
    });
    let slot_at_sym = heap.sym_slot_at;
    heap.get_mut(obj_id).handler_set(slot_at_sym, slot_at_handler);

    // Object: slotAt:put:
    let slot_at_put_handler = heap.register_native("obj_slotAtPut", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("slotAt:put: receiver is not a mutable object")?;
        let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt:put: arg0 must be a symbol")?;
        let val = args.get(1).copied().unwrap_or(Value::NIL);
        heap.get_mut(id).slot_set(name, val);
        Ok(val)
    });
    let slot_at_put_sym = heap.sym_slot_at_put;
    heap.get_mut(obj_id).handler_set(slot_at_put_sym, slot_at_put_handler);

    // Object: parent
    // Object: parent — works for ALL types (primitives, optimized variants, general objects)
    let parent_handler = heap.register_native("obj_parent", |heap, receiver, _args| {
        Ok(heap.prototype_of(receiver))
    });
    let parent_sym = heap.sym_parent;
    heap.get_mut(obj_id).handler_set(parent_sym, parent_handler);

    // Object: slotNames — works for ALL types
    let slot_names_handler = heap.register_native("obj_slotNames", |heap, receiver, _args| {
        if let Some(id) = receiver.as_any_object() {
            let names = heap.get(id).slot_names();
            let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
            Ok(heap.list(&syms))
        } else {
            Ok(Value::NIL) // primitives have no slots
        }
    });
    let slot_names_sym = heap.sym_slot_names;
    heap.get_mut(obj_id).handler_set(slot_names_sym, slot_names_handler);

    // Object: handlerNames — walks the full prototype chain for ALL types
    let handler_names_handler = heap.register_native("obj_handlerNames", |heap, receiver, _args| {
        let names = heap.all_handler_names(receiver);
        let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
        Ok(heap.list(&syms))
    });
    let handler_names_sym = heap.sym_handler_names;
    heap.get_mut(obj_id).handler_set(handler_names_sym, handler_names_handler);

    // Object: handle:with:
    let handle_with_handler = heap.register_native("obj_handleWith", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("handle:with: receiver is not a mutable object")?;
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("handle:with: selector must be a symbol")?;
        let handler = args.get(1).copied().ok_or("handle:with: need handler value")?;
        heap.get_mut(id).handler_set(sel, handler);
        Ok(receiver)
    });
    let handle_with_sym = heap.intern("handle:with:");
    heap.get_mut(obj_id).handler_set(handle_with_sym, handle_with_handler);

    // Object: handlerAt: — read a handler value by selector (for aliasing)
    let h = heap.register_native("obj_handlerAt", |heap, receiver, args| {
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("handlerAt: arg must be a symbol")?;
        // walk the prototype chain looking for the handler
        if let Some(id) = receiver.as_any_object() {
            if let Some(handler) = heap.get(id).handler_get(sel) {
                return Ok(handler);
            }
        }
        // check type proto
        let proto = heap.prototype_of(receiver);
        if let Some(pid) = proto.as_any_object() {
            let mut current = pid;
            for _ in 0..256 {
                if let Some(handler) = heap.get(current).handler_get(sel) {
                    return Ok(handler);
                }
                match heap.get(current).parent().as_any_object() {
                    Some(next) => current = next,
                    None => break,
                }
            }
        }
        Ok(Value::NIL)
    });
    let handler_at_sym = heap.intern("handlerAt:");
    heap.get_mut(obj_id).handler_set(handler_at_sym, h);

    // Object: responds: — check if a handler exists
    let h = heap.register_native("obj_responds", |heap, receiver, args| {
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("responds: arg must be a symbol")?;
        let names = heap.all_handler_names(receiver);
        Ok(Value::boolean(names.contains(&sel)))
    });
    let responds_sym = heap.intern("responds:");
    heap.get_mut(obj_id).handler_set(responds_sym, h);

    // Object: clone — shallow copy
    let h = heap.register_native("obj_clone", |heap, receiver, _args| {
        if let Some(id) = receiver.as_any_object() {
            let cloned = heap.get(id).clone();
            Ok(heap.alloc_val(cloned))
        } else {
            Ok(receiver) // primitives are immutable, return self
        }
    });
    let clone_sym = heap.intern("clone");
    heap.get_mut(obj_id).handler_set(clone_sym, h);

    // Object: describe
    let describe_handler = heap.register_native("obj_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    let describe_sym = heap.sym_describe;
    heap.get_mut(obj_id).handler_set(describe_sym, describe_handler);

    // Number prototype (shared parent for Integer and Float)
    let number_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_NUMBER] = number_proto;

    // Symbol prototype
    let sym_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_SYM] = sym_proto;
    let sym_proto_id = sym_proto.as_any_object().unwrap();

    // Symbol: name — the string name of the symbol
    let h = heap.register_native("sym_name", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("name: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&name))
    });
    let name_sym = heap.intern("name");
    heap.get_mut(sym_proto_id).handler_set(name_sym, h);

    // Symbol: toString — alias for name
    let to_string_sym = heap.intern("toString");
    let name_handler = heap.get(sym_proto_id).handler_get(name_sym).unwrap();
    heap.get_mut(sym_proto_id).handler_set(to_string_sym, name_handler);

    // Symbol: describe
    let h = heap.register_native("sym_describe", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("describe: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&name))
    });
    heap.get_mut(sym_proto_id).handler_set(describe_sym, h);

    // Symbol: show
    let h = heap.register_native("sym_show", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("show: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&format!("'{name}")))
    });
    let show_sym = heap.intern("show");
    heap.get_mut(sym_proto_id).handler_set(show_sym, h);

    // Integer prototype (parent: Number, not Object)
    let int_proto = heap.make_object(number_proto);
    heap.type_protos[PROTO_INT] = int_proto;

    // register native handlers for integer arithmetic
    let add_handler = heap.register_native("__int_add", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("+ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("+ : arg not an integer")?;
        Ok(Value::integer(a + b))
    });
    let int_id = int_proto.as_any_object().unwrap();
    let plus_sym = heap.intern("+");
    heap.get_mut(int_id).handler_set(plus_sym, add_handler);

    let sub_handler = heap.register_native("__int_sub", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("- : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("- : arg not an integer")?;
        Ok(Value::integer(a - b))
    });
    let minus_sym = heap.intern("-");
    heap.get_mut(int_id).handler_set(minus_sym, sub_handler);

    let mul_handler = heap.register_native("__int_mul", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("* : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("* : arg not an integer")?;
        Ok(Value::integer(a * b))
    });
    let mul_sym = heap.intern("*");
    heap.get_mut(int_id).handler_set(mul_sym, mul_handler);

    let div_handler = heap.register_native("__int_div", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("/ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("/ : arg not an integer")?;
        if b == 0 { return Err("division by zero".into()); }
        Ok(Value::integer(a / b))
    });
    let div_sym = heap.intern("/");
    heap.get_mut(int_id).handler_set(div_sym, div_handler);

    let lt_handler = heap.register_native("__int_lt", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("< : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("< : arg not an integer")?;
        Ok(Value::boolean(a < b))
    });
    let lt_sym = heap.intern("<");
    heap.get_mut(int_id).handler_set(lt_sym, lt_handler);

    let gt_handler = heap.register_native("__int_gt", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("> : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("> : arg not an integer")?;
        Ok(Value::boolean(a > b))
    });
    let gt_sym = heap.intern(">");
    heap.get_mut(int_id).handler_set(gt_sym, gt_handler);

    let eq_handler = heap.register_native("__int_eq", |heap, receiver, args| {
        let a = receiver.as_integer().ok_or("= : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("= : arg not an integer")?;
        Ok(Value::boolean(a == b))
    });
    let eq_sym = heap.intern("=");
    heap.get_mut(int_id).handler_set(eq_sym, eq_handler);

    let h = heap.register_native("__int_gte", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or(">= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or(">= : arg not int")?;
        Ok(Value::boolean(a >= b))
    });
    let gte_sym = heap.intern(">=");
    heap.get_mut(int_id).handler_set(gte_sym, h);

    let h = heap.register_native("__int_lte", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("<= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("<= : arg not int")?;
        Ok(Value::boolean(a <= b))
    });
    let lte_sym = heap.intern("<=");
    heap.get_mut(int_id).handler_set(lte_sym, h);

    let h = heap.register_native("__int_mod", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("% : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("% : arg not int")?;
        if b == 0 { return Err("modulo by zero".into()); }
        Ok(Value::integer(a % b))
    });
    let mod_sym = heap.intern("%");
    heap.get_mut(int_id).handler_set(mod_sym, h);

    let h = heap.register_native("__int_negate", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("negate: not int")?;
        Ok(Value::integer(-a))
    });
    let neg_sym = heap.intern("negate");
    heap.get_mut(int_id).handler_set(neg_sym, h);

    let describe_handler = heap.register_native("__int_describe", |_heap, receiver, _args| {
        Ok(receiver)
    });
    let describe_sym = heap.intern("describe");
    heap.get_mut(int_id).handler_set(describe_sym, describe_handler);

    // Integer: bit operations
    let bit_and_sym = heap.intern("bitAnd:");
    let h = heap.register_native("__int_bit_and", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitAnd: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitAnd: arg not an integer")?;
        Ok(Value::integer(a & b))
    });
    heap.get_mut(int_id).handler_set(bit_and_sym, h);

    let bit_or_sym = heap.intern("bitOr:");
    let h = heap.register_native("__int_bit_or", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitOr: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitOr: arg not an integer")?;
        Ok(Value::integer(a | b))
    });
    heap.get_mut(int_id).handler_set(bit_or_sym, h);

    let bit_xor_sym = heap.intern("bitXor:");
    let h = heap.register_native("__int_bit_xor", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitXor: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitXor: arg not an integer")?;
        Ok(Value::integer(a ^ b))
    });
    heap.get_mut(int_id).handler_set(bit_xor_sym, h);

    let bit_not_sym = heap.intern("bitNot");
    let h = heap.register_native("__int_bit_not", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("bitNot: not an integer")?;
        Ok(Value::integer(!a))
    });
    heap.get_mut(int_id).handler_set(bit_not_sym, h);

    let shl_sym = heap.intern("shiftLeft:");
    let h = heap.register_native("__int_shl", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("shiftLeft: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("shiftLeft: arg not an integer")?;
        Ok(Value::integer(a << b))
    });
    heap.get_mut(int_id).handler_set(shl_sym, h);

    let shr_sym = heap.intern("shiftRight:");
    let h = heap.register_native("__int_shr", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("shiftRight: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("shiftRight: arg not an integer")?;
        Ok(Value::integer(a >> b))
    });
    heap.get_mut(int_id).handler_set(shr_sym, h);

    // Integer: toFloat
    let to_float_sym = heap.intern("toFloat");
    let h = heap.register_native("__int_to_float", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("toFloat: not an integer")?;
        Ok(Value::float(a as f64))
    });
    heap.get_mut(int_id).handler_set(to_float_sym, h);

    // -- Range prototype (a General object with start/end/step slots) --
    let range_proto = heap.make_object(object_proto);
    let range_id = range_proto.as_any_object().unwrap();
    // store range_proto as a global so moof code can access it
    let range_sym = heap.intern("Range");
    heap.globals.insert(range_sym, range_proto);

    // Range: each: is implemented in moof (lib/range.moof) using while loop.
    // Iterable conformance gives 40+ methods from that one each: implementation.

    // Range: describe
    let r_start_sym = heap.intern("start");
    let r_end_sym = heap.intern("end");
    let r_step_sym = heap.intern("step");
    let h = heap.register_native("__range_describe", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not a range")?;
        let start = heap.get(id).slot_get(r_start_sym)
            .map(|v| heap.format_value(v)).unwrap_or("?".into());
        let end = heap.get(id).slot_get(r_end_sym)
            .map(|v| heap.format_value(v)).unwrap_or("?".into());
        let step = heap.get(id).slot_get(r_step_sym)
            .and_then(|v| v.as_integer()).unwrap_or(1);
        let s = if step == 1 {
            format!("({start} to: {end})")
        } else {
            format!("({start} to: {end} by: {step})")
        };
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(range_id).handler_set(describe_sym, h);

    // Integer: to: — creates a Range
    let to_sym = heap.intern("to:");
    let h = heap.register_native("__int_to", move |heap, receiver, args| {
        let start = receiver.as_integer().ok_or("to: receiver not an integer")?;
        let end = args.first().and_then(|v| v.as_integer()).ok_or("to: arg not an integer")?;
        let rp = heap.globals.get(&range_sym).copied().unwrap_or(Value::NIL);
        Ok(heap.make_object_with_slots(
            rp,
            vec![r_start_sym, r_end_sym, r_step_sym],
            vec![Value::integer(start), Value::integer(end), Value::integer(1)],
        ))
    });
    heap.get_mut(int_id).handler_set(to_sym, h);

    // Integer: to:by: — creates a Range with step
    let to_by_sym = heap.intern("to:by:");
    let h = heap.register_native("__int_to_by", move |heap, receiver, args| {
        let start = receiver.as_integer().ok_or("to:by: receiver not an integer")?;
        let end = args.get(0).and_then(|v| v.as_integer()).ok_or("to:by: first arg not an integer")?;
        let step = args.get(1).and_then(|v| v.as_integer()).ok_or("to:by: second arg not an integer")?;
        if step == 0 { return Err("to:by: step cannot be zero".into()); }
        let rp = heap.globals.get(&range_sym).copied().unwrap_or(Value::NIL);
        Ok(heap.make_object_with_slots(
            rp,
            vec![r_start_sym, r_end_sym, r_step_sym],
            vec![Value::integer(start), Value::integer(end), Value::integer(step)],
        ))
    });
    heap.get_mut(int_id).handler_set(to_by_sym, h);

    // -- Nil prototype (type_protos[PROTO_NIL]) --
    let nil_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_NIL] = nil_proto;
    let nil_id = nil_proto.as_any_object().unwrap();

    let h = heap.register_native("__nil_describe", |heap, _receiver, _args| {
        Ok(heap.alloc_string("nil"))
    });
    heap.get_mut(nil_id).handler_set(describe_sym, h);

    // -- Boolean prototype (type_protos[PROTO_BOOL]) --
    let bool_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_BOOL] = bool_proto;
    let bool_id = bool_proto.as_any_object().unwrap();

    let h = heap.register_native("__bool_not", |_heap, receiver, _args| {
        Ok(Value::boolean(!receiver.is_truthy()))
    });
    let not_sym = heap.intern("not");
    heap.get_mut(bool_id).handler_set(not_sym, h);

    let h = heap.register_native("__bool_describe", |heap, receiver, _args| {
        let s = if receiver.is_true() { "true" } else { "false" };
        Ok(heap.alloc_string(s))
    });
    heap.get_mut(bool_id).handler_set(describe_sym, h);

    let h = heap.register_native("__bool_if_true_false", |_heap, receiver, args| {
        let true_val = args.first().copied().unwrap_or(Value::NIL);
        let false_val = args.get(1).copied().unwrap_or(Value::NIL);
        Ok(if receiver.is_truthy() { true_val } else { false_val })
    });
    let if_sym = heap.intern("ifTrue:ifFalse:");
    heap.get_mut(bool_id).handler_set(if_sym, h);

    // Nil: ifTrue:ifFalse: — nil is falsy, always returns false branch
    let h = heap.register_native("__nil_if_true_false", |_heap, _receiver, args| {
        let false_val = args.get(1).copied().unwrap_or(Value::NIL);
        Ok(false_val)
    });
    heap.get_mut(nil_id).handler_set(if_sym, h);

    // -- Float prototype (parent: Number) --
    let float_proto = heap.make_object(number_proto);
    heap.type_protos[PROTO_FLOAT] = float_proto;
    let float_id = float_proto.as_any_object().unwrap();

    let h = heap.register_native("__float_add", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("+ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("+ : arg not numeric")?;
        Ok(Value::float(a + b))
    });
    heap.get_mut(float_id).handler_set(plus_sym, h);

    let h = heap.register_native("__float_sub", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("- : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("- : arg not numeric")?;
        Ok(Value::float(a - b))
    });
    heap.get_mut(float_id).handler_set(minus_sym, h);

    let h = heap.register_native("__float_mul", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("* : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("* : arg not numeric")?;
        Ok(Value::float(a * b))
    });
    heap.get_mut(float_id).handler_set(mul_sym, h);

    let h = heap.register_native("__float_div", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("/ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("/ : arg not numeric")?;
        if b == 0.0 { return Err("division by zero".into()); }
        Ok(Value::float(a / b))
    });
    heap.get_mut(float_id).handler_set(div_sym, h);

    let h = heap.register_native("__float_lt", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("< : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("< : arg not numeric")?;
        Ok(Value::boolean(a < b))
    });
    heap.get_mut(float_id).handler_set(lt_sym, h);

    let h = heap.register_native("__float_gt", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("> : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("> : arg not numeric")?;
        Ok(Value::boolean(a > b))
    });
    heap.get_mut(float_id).handler_set(gt_sym, h);

    let h = heap.register_native("__float_eq", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("= : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("= : arg not numeric")?;
        Ok(Value::boolean(a == b))
    });
    heap.get_mut(float_id).handler_set(eq_sym, h);

    let h = heap.register_native("__float_gte", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or(">= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or(">= : arg not numeric")?;
        Ok(Value::boolean(a >= b))
    });
    heap.get_mut(float_id).handler_set(gte_sym, h);

    let h = heap.register_native("__float_lte", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("<= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("<= : arg not numeric")?;
        Ok(Value::boolean(a <= b))
    });
    heap.get_mut(float_id).handler_set(lte_sym, h);

    let h = heap.register_native("__float_sqrt", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("sqrt: not numeric")?;
        Ok(Value::float(a.sqrt()))
    });
    let sqrt_sym = heap.intern("sqrt");
    heap.get_mut(float_id).handler_set(sqrt_sym, h);

    let h = heap.register_native("__float_floor", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("floor: not numeric")?;
        Ok(Value::float(a.floor()))
    });
    let floor_sym = heap.intern("floor");
    heap.get_mut(float_id).handler_set(floor_sym, h);

    let h = heap.register_native("__float_ceil", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("ceil: not numeric")?;
        Ok(Value::float(a.ceil()))
    });
    let ceil_sym = heap.intern("ceil");
    heap.get_mut(float_id).handler_set(ceil_sym, h);

    let h = heap.register_native("__float_round", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("round: not numeric")?;
        Ok(Value::float(a.round()))
    });
    let round_sym = heap.intern("round");
    heap.get_mut(float_id).handler_set(round_sym, h);

    let h = heap.register_native("__float_to_integer", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("toInteger: not numeric")?;
        Ok(Value::integer(a as i64))
    });
    let to_int_sym = heap.intern("toInteger");
    heap.get_mut(float_id).handler_set(to_int_sym, h);

    let h = heap.register_native("__float_describe", |heap, receiver, _args| {
        let a = receiver.as_float().ok_or("describe: not numeric")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    });
    heap.get_mut(float_id).handler_set(describe_sym, h);

    // Float: negate (required for Numeric protocol conformance)
    let h = heap.register_native("__float_negate", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("negate: not numeric")?;
        Ok(Value::float(-a))
    });
    let float_neg_sym = heap.intern("negate");
    heap.get_mut(float_id).handler_set(float_neg_sym, h);

    // Float: trig
    let sin_sym = heap.intern("sin");
    let h = heap.register_native("__float_sin", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("sin: not numeric")?.sin()))
    });
    heap.get_mut(float_id).handler_set(sin_sym, h);

    let cos_sym = heap.intern("cos");
    let h = heap.register_native("__float_cos", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("cos: not numeric")?.cos()))
    });
    heap.get_mut(float_id).handler_set(cos_sym, h);

    let tan_sym = heap.intern("tan");
    let h = heap.register_native("__float_tan", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("tan: not numeric")?.tan()))
    });
    heap.get_mut(float_id).handler_set(tan_sym, h);

    let asin_sym = heap.intern("asin");
    let h = heap.register_native("__float_asin", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("asin: not numeric")?.asin()))
    });
    heap.get_mut(float_id).handler_set(asin_sym, h);

    let acos_sym = heap.intern("acos");
    let h = heap.register_native("__float_acos", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("acos: not numeric")?.acos()))
    });
    heap.get_mut(float_id).handler_set(acos_sym, h);

    let atan_sym = heap.intern("atan");
    let h = heap.register_native("__float_atan", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("atan: not numeric")?.atan()))
    });
    heap.get_mut(float_id).handler_set(atan_sym, h);

    let atan2_sym = heap.intern("atan2:");
    let h = heap.register_native("__float_atan2", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("atan2: not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("atan2: arg not numeric")?;
        Ok(Value::float(a.atan2(b)))
    });
    heap.get_mut(float_id).handler_set(atan2_sym, h);

    // Float: log/exp
    let log_sym = heap.intern("log");
    let h = heap.register_native("__float_log", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log: not numeric")?.ln()))
    });
    heap.get_mut(float_id).handler_set(log_sym, h);

    let log10_sym = heap.intern("log10");
    let h = heap.register_native("__float_log10", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log10: not numeric")?.log10()))
    });
    heap.get_mut(float_id).handler_set(log10_sym, h);

    let log2_sym = heap.intern("log2");
    let h = heap.register_native("__float_log2", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log2: not numeric")?.log2()))
    });
    heap.get_mut(float_id).handler_set(log2_sym, h);

    let exp_sym = heap.intern("exp");
    let h = heap.register_native("__float_exp", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("exp: not numeric")?.exp()))
    });
    heap.get_mut(float_id).handler_set(exp_sym, h);

    let fpow_sym = heap.intern("pow:");
    let h = heap.register_native("__float_pow", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("pow: not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("pow: arg not numeric")?;
        Ok(Value::float(a.powf(b)))
    });
    heap.get_mut(float_id).handler_set(fpow_sym, h);

    // Float: predicates
    let nan_pred_sym = heap.intern("nan?");
    let h = heap.register_native("__float_nan", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("nan?: not numeric")?.is_nan()))
    });
    heap.get_mut(float_id).handler_set(nan_pred_sym, h);

    let inf_pred_sym = heap.intern("infinite?");
    let h = heap.register_native("__float_infinite", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("infinite?: not numeric")?.is_infinite()))
    });
    heap.get_mut(float_id).handler_set(inf_pred_sym, h);

    let finite_sym = heap.intern("finite?");
    let h = heap.register_native("__float_finite", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("finite?: not numeric")?.is_finite()))
    });
    heap.get_mut(float_id).handler_set(finite_sym, h);

    // Float constants: [Float pi], [Float e], [Float infinity], [Float nan]
    let pi_sym = heap.intern("pi");
    let h = heap.register_native("__float_pi", |_heap, _receiver, _args| {
        Ok(Value::float(std::f64::consts::PI))
    });
    heap.get_mut(float_id).handler_set(pi_sym, h);

    let e_sym = heap.intern("e");
    let h = heap.register_native("__float_e", |_heap, _receiver, _args| {
        Ok(Value::float(std::f64::consts::E))
    });
    heap.get_mut(float_id).handler_set(e_sym, h);

    let inf_sym = heap.intern("infinity");
    let h = heap.register_native("__float_infinity", |_heap, _receiver, _args| {
        Ok(Value::float(f64::INFINITY))
    });
    heap.get_mut(float_id).handler_set(inf_sym, h);

    let nan_sym = heap.intern("nan");
    let h = heap.register_native("__float_nan_val", |_heap, _receiver, _args| {
        Ok(Value::float(f64::NAN))
    });
    heap.get_mut(float_id).handler_set(nan_sym, h);

    // -- Cons prototype (type_protos[PROTO_CONS]) --
    let cons_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_CONS] = cons_proto;
    let cons_id = cons_proto.as_any_object().unwrap();

    let h = heap.register_native("__cons_car", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("car: not a cons")?;
        Ok(heap.car(id))
    });
    let car_sym = heap.intern("car");
    heap.get_mut(cons_id).handler_set(car_sym, h);

    let h = heap.register_native("__cons_cdr", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("cdr: not a cons")?;
        Ok(heap.cdr(id))
    });
    let cdr_sym = heap.intern("cdr");
    heap.get_mut(cons_id).handler_set(cdr_sym, h);

    let h = heap.register_native("__cons_length", |heap, receiver, _args| {
        let mut count = 0i64;
        let mut cur = receiver;
        while let Some(id) = cur.as_any_object() {
            match heap.get(id) {
                HeapObject::Pair(_, cdr) => { count += 1; cur = *cdr; }
                _ => break,
            }
        }
        Ok(Value::integer(count))
    });
    let length_sym = heap.intern("length");
    heap.get_mut(cons_id).handler_set(length_sym, h);

    let h = heap.register_native("__cons_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(cons_id).handler_set(describe_sym, h);

    // -- String prototype (type_protos[PROTO_STR]) --
    let str_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_STR] = str_proto;
    let str_id = str_proto.as_any_object().unwrap();

    let h = heap.register_native("__str_length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a string")?;
        let s = heap.get_string(id).ok_or("length: not a Text object")?;
        Ok(Value::integer(s.len() as i64))
    });
    heap.get_mut(str_id).handler_set(length_sym, h);

    let h = heap.register_native("__str_at", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at: not a string")?;
        let s = heap.get_string(id).ok_or("at: not a Text object")?;
        let idx = args.first().and_then(|v| v.as_integer()).ok_or("at: arg not an integer")? as usize;
        let ch = s.chars().nth(idx).map(|c| c.to_string()).unwrap_or_default();
        Ok(heap.alloc_string(&ch))
    });
    let at_sym = heap.intern("at:");
    heap.get_mut(str_id).handler_set(at_sym, h);

    let h = heap.register_native("__str_concat", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("++: not a string")?;
        let a = heap.get_string(id).ok_or("++: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let b = if let Some(bid) = arg.as_any_object() {
            heap.get_string(bid).map(|s| s.to_string()).unwrap_or_else(|| heap.format_value(arg))
        } else {
            heap.format_value(arg)
        };
        Ok(heap.alloc_string(&format!("{}{}", a, b)))
    });
    let concat_sym = heap.intern("++");
    heap.get_mut(str_id).handler_set(concat_sym, h);

    let h = heap.register_native("__str_substring_to", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("substring:to: not a string")?;
        let s = heap.get_string(id).ok_or("substring:to: not a Text object")?;
        let from = args.first().and_then(|v| v.as_integer()).ok_or("substring:to: arg0 not int")? as usize;
        let to = args.get(1).and_then(|v| v.as_integer()).ok_or("substring:to: arg1 not int")? as usize;
        let chars: Vec<char> = s.chars().collect();
        let end = to.min(chars.len());
        let start = from.min(end);
        let sub: String = chars[start..end].iter().collect();
        Ok(heap.alloc_string(&sub))
    });
    let substr_sym = heap.intern("substring:to:");
    heap.get_mut(str_id).handler_set(substr_sym, h);

    let h = heap.register_native("__str_split", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("split: not a string")?;
        let s = heap.get_string(id).ok_or("split: not a Text object")?.to_string();
        let delim_arg = args.first().copied().unwrap_or(Value::NIL);
        let did = delim_arg.as_any_object().ok_or("split: arg not a string")?;
        let delim = heap.get_string(did).ok_or("split: arg not a Text object")?.to_string();
        let parts: Vec<Value> = s.split(&delim).map(|p| heap.alloc_string(p)).collect();
        Ok(heap.list(&parts))
    });
    let split_sym = heap.intern("split:");
    heap.get_mut(str_id).handler_set(split_sym, h);

    let h = heap.register_native("__str_trim", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("trim: not a string")?;
        let s = heap.get_string(id).ok_or("trim: not a Text object")?.trim().to_string();
        Ok(heap.alloc_string(&s))
    });
    let trim_sym = heap.intern("trim");
    heap.get_mut(str_id).handler_set(trim_sym, h);

    let h = heap.register_native("__str_contains", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("contains: not a string")?;
        let s = heap.get_string(id).ok_or("contains: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let nid = arg.as_any_object().ok_or("contains: arg not a string")?;
        let needle = heap.get_string(nid).ok_or("contains: arg not a Text object")?;
        Ok(Value::boolean(s.contains(needle)))
    });
    let contains_sym = heap.intern("contains:");
    heap.get_mut(str_id).handler_set(contains_sym, h);

    let h = heap.register_native("__str_starts_with", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("startsWith: not a string")?;
        let s = heap.get_string(id).ok_or("startsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let pid = arg.as_any_object().ok_or("startsWith: arg not a string")?;
        let prefix = heap.get_string(pid).ok_or("startsWith: arg not a Text object")?;
        Ok(Value::boolean(s.starts_with(prefix)))
    });
    let starts_sym = heap.intern("startsWith:");
    heap.get_mut(str_id).handler_set(starts_sym, h);

    let h = heap.register_native("__str_ends_with", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("endsWith: not a string")?;
        let s = heap.get_string(id).ok_or("endsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let sid = arg.as_any_object().ok_or("endsWith: arg not a string")?;
        let suffix = heap.get_string(sid).ok_or("endsWith: arg not a Text object")?;
        Ok(Value::boolean(s.ends_with(suffix)))
    });
    let ends_sym = heap.intern("endsWith:");
    heap.get_mut(str_id).handler_set(ends_sym, h);

    let h = heap.register_native("__str_to_upper", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toUpper: not a string")?;
        let s = heap.get_string(id).ok_or("toUpper: not a Text object")?;
        Ok(heap.alloc_string(&s.to_uppercase()))
    });
    let upper_sym = heap.intern("toUpper");
    heap.get_mut(str_id).handler_set(upper_sym, h);

    let h = heap.register_native("__str_to_lower", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toLower: not a string")?;
        let s = heap.get_string(id).ok_or("toLower: not a Text object")?;
        Ok(heap.alloc_string(&s.to_lowercase()))
    });
    let lower_sym = heap.intern("toLower");
    heap.get_mut(str_id).handler_set(lower_sym, h);

    let h = heap.register_native("__str_to_integer", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toInteger: not a string")?;
        let s = heap.get_string(id).ok_or("toInteger: not a Text object")?;
        let n: i64 = s.trim().parse().map_err(|_| format!("toInteger: cannot parse '{}'", s))?;
        Ok(Value::integer(n))
    });
    heap.get_mut(str_id).handler_set(to_int_sym, h);

    let h = heap.register_native("__str_describe", |_heap, receiver, _args| {
        Ok(receiver) // strings describe as themselves
    });
    heap.get_mut(str_id).handler_set(describe_sym, h);

    // String: indexOf:
    let indexof_sym = heap.intern("indexOf:");
    let h = heap.register_native("__str_indexof", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("indexOf: not a string")?;
        let sub_id = args.first().and_then(|v| v.as_any_object()).ok_or("indexOf: arg not a string")?;
        match (heap.get(id), heap.get(sub_id)) {
            (HeapObject::Text(s), HeapObject::Text(sub)) => {
                match s.find(sub.as_str()) {
                    Some(pos) => Ok(Value::integer(pos as i64)),
                    None => Ok(Value::NIL),
                }
            }
            _ => Err("indexOf: not strings".into()),
        }
    });
    heap.get_mut(str_id).handler_set(indexof_sym, h);

    // String: replace:with:
    let replace_sym = heap.intern("replace:with:");
    let h = heap.register_native("__str_replace", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("replace:with: not a string")?;
        let old_id = args.get(0).and_then(|v| v.as_any_object()).ok_or("replace:with: first arg not a string")?;
        let new_id = args.get(1).and_then(|v| v.as_any_object()).ok_or("replace:with: second arg not a string")?;
        match (heap.get(id), heap.get(old_id), heap.get(new_id)) {
            (HeapObject::Text(s), HeapObject::Text(old), HeapObject::Text(new)) => {
                let result = s.replacen(old.as_str(), new.as_str(), 1);
                Ok(heap.alloc_string(&result))
            }
            _ => Err("replace:with: not strings".into()),
        }
    });
    heap.get_mut(str_id).handler_set(replace_sym, h);

    // String: replaceAll:with:
    let replace_all_sym = heap.intern("replaceAll:with:");
    let h = heap.register_native("__str_replace_all", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("replaceAll:with: not a string")?;
        let old_id = args.get(0).and_then(|v| v.as_any_object()).ok_or("replaceAll:with: first arg not a string")?;
        let new_id = args.get(1).and_then(|v| v.as_any_object()).ok_or("replaceAll:with: second arg not a string")?;
        match (heap.get(id), heap.get(old_id), heap.get(new_id)) {
            (HeapObject::Text(s), HeapObject::Text(old), HeapObject::Text(new)) => {
                let result = s.replace(old.as_str(), new.as_str());
                Ok(heap.alloc_string(&result))
            }
            _ => Err("replaceAll:with: not strings".into()),
        }
    });
    heap.get_mut(str_id).handler_set(replace_all_sym, h);

    // String: toFloat
    let str_to_float_sym = heap.intern("toFloat");
    let h = heap.register_native("__str_to_float", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toFloat: not a string")?;
        match heap.get(id) {
            HeapObject::Text(s) => match s.parse::<f64>() {
                Ok(n) => Ok(Value::float(n)),
                Err(_) => Err(format!("toFloat: cannot parse '{s}'")),
            },
            _ => Err("toFloat: not a string".into()),
        }
    });
    heap.get_mut(str_id).handler_set(str_to_float_sym, h);

    // -- Table prototype (type_protos[PROTO_TABLE]) --
    let table_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_TABLE] = table_proto;
    let table_id = table_proto.as_any_object().unwrap();

    // Table: at:
    let h = heap.register_native("__table_at", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        match heap.get(id) {
            HeapObject::Table { seq, map } => {
                // try integer index into seq first
                if let Some(idx) = key.as_integer() {
                    if idx >= 0 && (idx as usize) < seq.len() {
                        return Ok(seq[idx as usize]);
                    }
                }
                // then check map (content equality for strings)
                for (k, v) in map {
                    if heap.values_equal(*k, key) { return Ok(*v); }
                }
                Ok(Value::NIL)
            }
            _ => Err("at: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(at_sym, h);

    // Table: at:put:
    let at_put_sym = heap.intern("at:put:");
    let h = heap.register_native("__table_at_put", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at:put: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        let val = args.get(1).copied().unwrap_or(Value::NIL);
        // find existing key index using content equality (before mutable borrow)
        let existing_idx = match heap.get(id) {
            HeapObject::Table { map, .. } => {
                map.iter().position(|(k, _)| heap.values_equal(*k, key))
            }
            _ => return Err("at:put: not a Table".into()),
        };
        match heap.get_mut(id) {
            HeapObject::Table { seq, map } => {
                if let Some(idx) = key.as_integer() {
                    if idx >= 0 && (idx as usize) < seq.len() {
                        seq[idx as usize] = val;
                        return Ok(val);
                    }
                }
                if let Some(pos) = existing_idx {
                    map[pos].1 = val;
                } else {
                    map.push((key, val));
                }
                Ok(val)
            }
            _ => Err("at:put: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(at_put_sym, h);

    // Table: push:
    let push_sym = heap.intern("push:");
    let h = heap.register_native("__table_push", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("push: not a table")?;
        let val = args.first().copied().unwrap_or(Value::NIL);
        match heap.get_mut(id) {
            HeapObject::Table { seq, .. } => {
                seq.push(val);
                Ok(val)
            }
            _ => Err("push: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(push_sym, h);

    // Table: length
    let h = heap.register_native("__table_length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a table")?;
        match heap.get(id) {
            HeapObject::Table { seq, .. } => Ok(Value::integer(seq.len() as i64)),
            _ => Err("length: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(length_sym, h);

    // Table: keys
    let keys_sym = heap.intern("keys");
    let h = heap.register_native("__table_keys", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("keys: not a table")?;
        let keys: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(k, _)| *k).collect(),
            _ => return Err("keys: not a Table".into()),
        };
        Ok(heap.list(&keys))
    });
    heap.get_mut(table_id).handler_set(keys_sym, h);

    // Table: values
    let values_sym = heap.intern("values");
    let h = heap.register_native("__table_values", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("values: not a table")?;
        let vals: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(_, v)| *v).collect(),
            _ => return Err("values: not a Table".into()),
        };
        Ok(heap.list(&vals))
    });
    heap.get_mut(table_id).handler_set(values_sym, h);

    // Table: describe
    let h = heap.register_native("__table_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(table_id).handler_set(describe_sym, h);

    // Table: contains:
    let h = heap.register_native("__table_contains", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("contains: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        match heap.get(id) {
            HeapObject::Table { seq, map } => {
                for v in seq {
                    if heap.values_equal(*v, key) { return Ok(Value::TRUE); }
                }
                for (k, _) in map {
                    if heap.values_equal(*k, key) { return Ok(Value::TRUE); }
                }
                Ok(Value::FALSE)
            }
            _ => Err("contains: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(contains_sym, h);

    // Table: remove:
    let remove_sym = heap.intern("remove:");
    let h = heap.register_native("__table_remove", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("remove: not a table")?;
        let key = args.first().copied().unwrap_or(Value::NIL);
        let pos = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().position(|(k, _)| heap.values_equal(*k, key)),
            _ => return Err("remove: not a Table".into()),
        };
        match heap.get_mut(id) {
            HeapObject::Table { map, .. } => {
                if let Some(pos) = pos {
                    let (_, val) = map.remove(pos);
                    Ok(val)
                } else {
                    Ok(Value::NIL)
                }
            }
            _ => Err("remove: not a Table".into()),
        }
    });
    heap.get_mut(table_id).handler_set(remove_sym, h);

    // -- Error prototype (type_protos[PROTO_ERROR]) --
    let error_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_ERROR] = error_proto;
    let error_id = error_proto.as_any_object().unwrap();

    // Error: message — read the message slot
    let h = heap.register_native("__error_message", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("message: not an object")?;
        Ok(heap.get(id).slot_get(heap.sym_message).unwrap_or(Value::NIL))
    });
    let message_sym = heap.intern("message");
    heap.get_mut(error_id).handler_set(message_sym, h);

    // Error: describe
    let h = heap.register_native("__error_describe", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not an object")?;
        let msg_val = heap.get(id).slot_get(heap.sym_message).unwrap_or(Value::NIL);
        let msg = heap.format_value(msg_val);
        let s = format!("Error: {}", msg);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(error_id).handler_set(describe_sym, h);

    // -- register all prototypes as globals so they're accessible by name --
    let obj_sym = heap.intern("Object");
    heap.globals.insert(obj_sym, object_proto);
    let int_sym = heap.intern("Integer");
    heap.globals.insert(int_sym, int_proto);
    let nil_sym = heap.intern("Nil");
    heap.globals.insert(nil_sym, nil_proto);
    let bool_sym = heap.intern("Boolean");
    heap.globals.insert(bool_sym, bool_proto);
    let float_sym = heap.intern("Float");
    heap.globals.insert(float_sym, float_proto);
    let cons_sym = heap.intern("Cons");
    heap.globals.insert(cons_sym, cons_proto);
    let string_sym = heap.intern("String");
    heap.globals.insert(string_sym, str_proto);
    let table_sym = heap.intern("Table");
    heap.globals.insert(table_sym, table_proto);
    let number_sym = heap.intern("Number");
    heap.globals.insert(number_sym, number_proto);
    let symbol_sym = heap.intern("Symbol");
    heap.globals.insert(symbol_sym, sym_proto);
    let error_sym = heap.intern("Error");
    heap.globals.insert(error_sym, error_proto);

    // -- String: native < for Comparable --
    let h = heap.register_native("str_lt", |heap, receiver, args| {
        let a_id = receiver.as_any_object().ok_or("< : not a string")?;
        let b_id = args.first().and_then(|v| v.as_any_object()).ok_or("< : arg not a string")?;
        match (heap.get(a_id), heap.get(b_id)) {
            (HeapObject::Text(a), HeapObject::Text(b)) => Ok(Value::boolean(a < b)),
            _ => Err("< : not strings".into()),
        }
    });
    let lt_sym = heap.intern("<");
    heap.get_mut(str_id).handler_set(lt_sym, h);

    // -- handlers on Object prototype (inherited by everything) --

    // print — [obj print] outputs and returns self
    let h = heap.register_native("obj_print", |heap, receiver, _args| {
        println!("{}", heap.format_value(receiver));
        Ok(receiver)
    });
    let print_sym = heap.intern("print");
    heap.get_mut(obj_id).handler_set(print_sym, h);

    // println — [obj println] outputs and returns nil
    let h = heap.register_native("obj_println", |heap, receiver, _args| {
        println!("{}", heap.format_value(receiver));
        Ok(Value::NIL)
    });
    let println_sym = heap.intern("println");
    heap.get_mut(obj_id).handler_set(println_sym, h);

    // type — [obj type] returns a symbol for the type
    let h = heap.register_native("obj_type", |heap, receiver, _args| {
        let name = if receiver.is_nil() { "Nil" }
            else if receiver.is_bool() { "Boolean" }
            else if receiver.is_integer() { "Integer" }
            else if receiver.is_float() { "Float" }
            else if receiver.is_symbol() { "Symbol" }
            else if let Some(id) = receiver.as_any_object() {
                match heap.get(id) {
                    HeapObject::Closure { is_operative, .. } => {
                        if *is_operative { "Operative" } else { "Fn" }
                    }
                    HeapObject::General { .. } => "Object",
                    HeapObject::Pair(_, _) => "Cons",
                    HeapObject::Text(_) => "String",
                    HeapObject::Buffer(_) => "Bytes",
                    HeapObject::Table { .. } => "Table",
                }
            } else { "Unknown" };
        Ok(Value::symbol(heap.intern(name)))
    });
    let type_sym = heap.intern("type");
    heap.get_mut(obj_id).handler_set(type_sym, h);

    // equal? — content equality (like Ruby's eql?)
    let h = heap.register_native("obj_equal", |heap, receiver, args| {
        let other = args.first().copied().unwrap_or(Value::NIL);
        Ok(Value::boolean(heap.values_equal(receiver, other)))
    });
    let equal_sym = heap.intern("equal:");
    heap.get_mut(obj_id).handler_set(equal_sym, h);

    // show — default display for REPL (Showable protocol base)
    let h = heap.register_native("obj_show", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    let show_sym = heap.intern("show");
    heap.get_mut(obj_id).handler_set(show_sym, h);

    // identical: — bit-level identity test (semantic foundation for eq)
    let h = heap.register_native("obj_identical", |_heap, receiver, args| {
        let other = args.first().copied().unwrap_or(Value::NIL);
        Ok(Value::boolean(receiver == other))
    });
    let identical_sym = heap.intern("identical:");
    heap.get_mut(obj_id).handler_set(identical_sym, h);

    // -- Block/Closure prototype (type_protos[PROTO_CLOSURE]) --
    let block_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_CLOSURE] = block_proto;
    let block_id = block_proto.as_any_object().unwrap();

    // Block: wrap — convert operative to applicative (Kernel's wrap)
    // [operative wrap] => applicative (same code, args evaluated by caller)
    let h = heap.register_native("closure_wrap", |heap, receiver, _args| {
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
    let wrap_sym = heap.intern("wrap");
    heap.get_mut(block_id).handler_set(wrap_sym, h);

    // Block: describe
    let h = heap.register_native("closure_describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(block_id).handler_set(describe_sym, h);

    // register Block as global
    let block_sym = heap.intern("Block");
    heap.globals.insert(block_sym, block_proto);

    // -- Root environment object --
    let env_proto = heap.make_object(object_proto);
    let env_sym = heap.intern("Environment");
    heap.globals.insert(env_sym, env_proto);

    // -- FarRef prototype (PROTO_FARREF) --
    // a far reference is a proxy for an object in another vat.
    // all sends are intercepted via doesNotUnderstand: and queued.
    let farref_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_FARREF] = farref_proto;
    let farref_id = farref_proto.as_any_object().unwrap();

    // FarRef: doesNotUnderstand: — intercept ALL sends, queue to outbox
    let target_vat_sym = heap.intern("__target_vat");
    let target_obj_sym = heap.intern("__target_obj");
    let h = heap.register_native("farref_dnu", move |heap, receiver, args| {
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

        // create a promise for the result
        let promise_proto = heap.type_protos[PROTO_PROMISE];
        let state_sym = heap.intern("__state");
        let pending_sym = heap.intern("pending");
        let buffer_sym = heap.intern("__buffer");
        let promise = heap.make_object_with_slots(
            promise_proto,
            vec![state_sym, buffer_sym],
            vec![Value::symbol(pending_sym), Value::NIL],
        );
        let promise_obj_id = promise.as_any_object().unwrap();

        // push to outbox
        heap.outbox.push(crate::heap::OutgoingMessage {
            target_vat_id: target_vat,
            target_obj_id: target_obj,
            selector,
            args: msg_args,
            promise_id: promise_obj_id,
        });

        Ok(promise)
    });
    let dnu_sym = heap.sym_dnu;
    heap.get_mut(farref_id).handler_set(dnu_sym, h);

    // FarRef: describe
    let h = heap.register_native("farref_describe", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not a farref")?;
        let vat = heap.get(id).slot_get(target_vat_sym)
            .and_then(|v| v.as_integer()).unwrap_or(-1);
        let obj = heap.get(id).slot_get(target_obj_sym)
            .and_then(|v| v.as_integer()).unwrap_or(-1);
        let s = format!("<far-ref vat:{vat} obj:{obj}>");
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(farref_id).handler_set(describe_sym, h);

    let farref_sym = heap.intern("FarRef");
    heap.globals.insert(farref_sym, farref_proto);

    // -- Promise prototype (PROTO_PROMISE) --
    // a promise represents a future value. sends to unresolved promises
    // are buffered via doesNotUnderstand: and forwarded when resolved.
    let promise_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_PROMISE] = promise_proto;
    let promise_id = promise_proto.as_any_object().unwrap();

    // Promise: describe
    let state_sym = heap.intern("__state");
    let h = heap.register_native("promise_describe", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not a promise")?;
        let state = heap.get(id).slot_get(state_sym)
            .map(|v| heap.format_value(v)).unwrap_or("?".into());
        let s = format!("<promise {state}>");
        Ok(heap.alloc_string(&s))
    });
    heap.get_mut(promise_id).handler_set(describe_sym, h);

    let promise_sym = heap.intern("Promise");
    heap.globals.insert(promise_sym, promise_proto);

    // -- Vat prototype --
    // [Vat spawn: block] creates a new vat. The block runs in the new vat
    // and its return value becomes accessible via a far reference.
    let vat_proto = heap.make_object(object_proto);
    let vat_sym = heap.intern("Vat");
    heap.globals.insert(vat_sym, vat_proto);
}
