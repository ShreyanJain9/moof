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

    // fix up root environment's parent to Object (it was NIL at allocation time)
    if let HeapObject::Environment { parent, .. } = heap.get_mut(heap.env) {
        *parent = object_proto;
    }

    // Object: slotAt:
    native(heap, obj_id, "slotAt:", |heap, receiver, args| {
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

    // Object: slotAt:put:
    native(heap, obj_id, "slotAt:put:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("slotAt:put: receiver is not a mutable object")?;
        let name = args.first().and_then(|v| v.as_symbol()).ok_or("slotAt:put: arg0 must be a symbol")?;
        let val = args.get(1).copied().unwrap_or(Value::NIL);
        heap.get_mut(id).slot_set(name, val);
        Ok(val)
    });

    // Object: parent — works for ALL types (primitives, optimized variants, general objects)
    native(heap, obj_id, "parent", |heap, receiver, _args| {
        Ok(heap.prototype_of(receiver))
    });

    // Object: slotNames — works for ALL types
    native(heap, obj_id, "slotNames", |heap, receiver, _args| {
        if let Some(id) = receiver.as_any_object() {
            let names = heap.get(id).slot_names();
            let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
            Ok(heap.list(&syms))
        } else {
            Ok(Value::NIL) // primitives have no slots
        }
    });

    // Object: handlerNames — walks the full prototype chain for ALL types
    native(heap, obj_id, "handlerNames", |heap, receiver, _args| {
        let names = heap.all_handler_names(receiver);
        let syms: Vec<Value> = names.into_iter().map(Value::symbol).collect();
        Ok(heap.list(&syms))
    });

    // Object: handle:with:
    native(heap, obj_id, "handle:with:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("handle:with: receiver is not a mutable object")?;
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("handle:with: selector must be a symbol")?;
        let handler = args.get(1).copied().ok_or("handle:with: need handler value")?;
        heap.get_mut(id).handler_set(sel, handler);
        Ok(receiver)
    });

    // Object: handlerAt: — read a handler value by selector (for aliasing)
    native(heap, obj_id, "handlerAt:", |heap, receiver, args| {
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

    // Object: responds: — check if a handler exists (walks prototype chain)
    native(heap, obj_id, "responds:", |heap, receiver, args| {
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("responds: arg must be a symbol")?;
        let names = heap.all_handler_names(receiver);
        Ok(Value::boolean(names.contains(&sel)))
    });

    // Object: hasOwnHandler: — check if THIS object has the handler directly (no chain walk)
    native(heap, obj_id, "hasOwnHandler:", |heap, receiver, args| {
        let sel = args.first().and_then(|v| v.as_symbol()).ok_or("hasOwnHandler: arg must be a symbol")?;
        if let Some(id) = receiver.as_any_object() {
            Ok(Value::boolean(heap.get(id).handler_get(sel).is_some()))
        } else {
            Ok(Value::FALSE)
        }
    });

    // Object: clone — shallow copy
    native(heap, obj_id, "clone", |heap, receiver, _args| {
        if let Some(id) = receiver.as_any_object() {
            let cloned = heap.get(id).clone();
            Ok(heap.alloc_val(cloned))
        } else {
            Ok(receiver) // primitives are immutable, return self
        }
    });

    // Object: describe
    native(heap, obj_id, "describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });

    // Number prototype (shared parent for Integer and Float)
    let number_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_NUMBER] = number_proto;

    // Symbol prototype
    let sym_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_SYM] = sym_proto;
    let sym_proto_id = sym_proto.as_any_object().unwrap();

    // Symbol: name — the string name of the symbol
    native(heap, sym_proto_id, "name", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("name: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&name))
    });

    // Symbol: toString — alias for name
    let name_sym = heap.intern("name");
    let to_string_sym = heap.intern("toString");
    let name_handler = heap.get(sym_proto_id).handler_get(name_sym).unwrap();
    heap.get_mut(sym_proto_id).handler_set(to_string_sym, name_handler);

    // Symbol: describe
    native(heap, sym_proto_id, "describe", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("describe: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&name))
    });

    // Symbol: show
    native(heap, sym_proto_id, "show", |heap, receiver, _args| {
        let sym_id = receiver.as_symbol().ok_or("show: not a symbol")?;
        let name = heap.symbol_name(sym_id).to_string();
        Ok(heap.alloc_string(&format!("'{name}")))
    });

    // Integer prototype (parent: Number, not Object)
    let int_proto = heap.make_object(number_proto);
    heap.type_protos[PROTO_INT] = int_proto;

    // register native handlers for integer arithmetic
    let int_id = int_proto.as_any_object().unwrap();
    native(heap, int_id, "+", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("+ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("+ : arg not an integer")?;
        Ok(Value::integer(a + b))
    });
    native(heap, int_id, "-", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("- : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("- : arg not an integer")?;
        Ok(Value::integer(a - b))
    });
    native(heap, int_id, "*", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("* : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("* : arg not an integer")?;
        Ok(Value::integer(a * b))
    });
    native(heap, int_id, "/", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("/ : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("/ : arg not an integer")?;
        if b == 0 { return Err("division by zero".into()); }
        Ok(Value::integer(a / b))
    });
    native(heap, int_id, "<", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("< : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("< : arg not an integer")?;
        Ok(Value::boolean(a < b))
    });
    native(heap, int_id, ">", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("> : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("> : arg not an integer")?;
        Ok(Value::boolean(a > b))
    });
    native(heap, int_id, "=", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("= : receiver not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("= : arg not an integer")?;
        Ok(Value::boolean(a == b))
    });
    native(heap, int_id, ">=", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or(">= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or(">= : arg not int")?;
        Ok(Value::boolean(a >= b))
    });
    native(heap, int_id, "<=", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("<= : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("<= : arg not int")?;
        Ok(Value::boolean(a <= b))
    });
    native(heap, int_id, "%", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("% : not int")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("% : arg not int")?;
        if b == 0 { return Err("modulo by zero".into()); }
        Ok(Value::integer(a % b))
    });
    native(heap, int_id, "negate", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("negate: not int")?;
        Ok(Value::integer(-a))
    });
    native(heap, int_id, "describe", |_heap, receiver, _args| {
        Ok(receiver)
    });

    // Integer: bit operations
    native(heap, int_id, "bitAnd:", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitAnd: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitAnd: arg not an integer")?;
        Ok(Value::integer(a & b))
    });
    native(heap, int_id, "bitOr:", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitOr: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitOr: arg not an integer")?;
        Ok(Value::integer(a | b))
    });
    native(heap, int_id, "bitXor:", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("bitXor: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("bitXor: arg not an integer")?;
        Ok(Value::integer(a ^ b))
    });
    native(heap, int_id, "bitNot", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("bitNot: not an integer")?;
        Ok(Value::integer(!a))
    });
    native(heap, int_id, "shiftLeft:", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("shiftLeft: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("shiftLeft: arg not an integer")?;
        Ok(Value::integer(a << b))
    });
    native(heap, int_id, "shiftRight:", |_heap, receiver, args| {
        let a = receiver.as_integer().ok_or("shiftRight: not an integer")?;
        let b = args.first().and_then(|v| v.as_integer()).ok_or("shiftRight: arg not an integer")?;
        Ok(Value::integer(a >> b))
    });

    // Integer: toFloat
    native(heap, int_id, "toFloat", |_heap, receiver, _args| {
        let a = receiver.as_integer().ok_or("toFloat: not an integer")?;
        Ok(Value::float(a as f64))
    });

    // Range: fully defined in lib/range.moof as a pure moof object literal.
    // Integer#to: and Integer#to:by: are also defined there.

    // -- Nil prototype (type_protos[PROTO_NIL]) --
    let nil_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_NIL] = nil_proto;
    let nil_id = nil_proto.as_any_object().unwrap();

    native(heap, nil_id, "describe", |heap, _receiver, _args| {
        Ok(heap.alloc_string("nil"))
    });

    // -- Boolean prototype (type_protos[PROTO_BOOL]) --
    let bool_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_BOOL] = bool_proto;
    let bool_id = bool_proto.as_any_object().unwrap();

    native(heap, bool_id, "not", |_heap, receiver, _args| {
        Ok(Value::boolean(!receiver.is_truthy()))
    });
    native(heap, bool_id, "describe", |heap, receiver, _args| {
        let s = if receiver.is_true() { "true" } else { "false" };
        Ok(heap.alloc_string(s))
    });
    native(heap, bool_id, "ifTrue:ifFalse:", |_heap, receiver, args| {
        let true_val = args.first().copied().unwrap_or(Value::NIL);
        let false_val = args.get(1).copied().unwrap_or(Value::NIL);
        Ok(if receiver.is_truthy() { true_val } else { false_val })
    });

    // Nil: ifTrue:ifFalse: — nil is falsy, always returns false branch
    native(heap, nil_id, "ifTrue:ifFalse:", |_heap, _receiver, args| {
        let false_val = args.get(1).copied().unwrap_or(Value::NIL);
        Ok(false_val)
    });

    // -- Float prototype (parent: Number) --
    let float_proto = heap.make_object(number_proto);
    heap.type_protos[PROTO_FLOAT] = float_proto;
    let float_id = float_proto.as_any_object().unwrap();

    native(heap, float_id, "+", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("+ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("+ : arg not numeric")?;
        Ok(Value::float(a + b))
    });
    native(heap, float_id, "-", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("- : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("- : arg not numeric")?;
        Ok(Value::float(a - b))
    });
    native(heap, float_id, "*", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("* : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("* : arg not numeric")?;
        Ok(Value::float(a * b))
    });
    native(heap, float_id, "/", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("/ : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("/ : arg not numeric")?;
        if b == 0.0 { return Err("division by zero".into()); }
        Ok(Value::float(a / b))
    });
    native(heap, float_id, "<", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("< : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("< : arg not numeric")?;
        Ok(Value::boolean(a < b))
    });
    native(heap, float_id, ">", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("> : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("> : arg not numeric")?;
        Ok(Value::boolean(a > b))
    });
    native(heap, float_id, "=", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("= : receiver not a float")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("= : arg not numeric")?;
        Ok(Value::boolean(a == b))
    });
    native(heap, float_id, ">=", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or(">= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or(">= : arg not numeric")?;
        Ok(Value::boolean(a >= b))
    });
    native(heap, float_id, "<=", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("<= : not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("<= : arg not numeric")?;
        Ok(Value::boolean(a <= b))
    });
    native(heap, float_id, "sqrt", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("sqrt: not numeric")?.sqrt()))
    });
    native(heap, float_id, "floor", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("floor: not numeric")?.floor()))
    });
    native(heap, float_id, "ceil", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("ceil: not numeric")?.ceil()))
    });
    native(heap, float_id, "round", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("round: not numeric")?.round()))
    });
    native(heap, float_id, "toInteger", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("toInteger: not numeric")?;
        Ok(Value::integer(a as i64))
    });
    native(heap, float_id, "describe", |heap, receiver, _args| {
        let a = receiver.as_float().ok_or("describe: not numeric")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    });
    native(heap, float_id, "negate", |_heap, receiver, _args| {
        let a = receiver.as_float().ok_or("negate: not numeric")?;
        Ok(Value::float(-a))
    });

    // Float: trig
    native(heap, float_id, "sin", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("sin: not numeric")?.sin()))
    });
    native(heap, float_id, "cos", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("cos: not numeric")?.cos()))
    });
    native(heap, float_id, "tan", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("tan: not numeric")?.tan()))
    });
    native(heap, float_id, "asin", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("asin: not numeric")?.asin()))
    });
    native(heap, float_id, "acos", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("acos: not numeric")?.acos()))
    });
    native(heap, float_id, "atan", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("atan: not numeric")?.atan()))
    });
    native(heap, float_id, "atan2:", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("atan2: not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("atan2: arg not numeric")?;
        Ok(Value::float(a.atan2(b)))
    });

    // Float: log/exp
    native(heap, float_id, "log", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log: not numeric")?.ln()))
    });
    native(heap, float_id, "log10", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log10: not numeric")?.log10()))
    });
    native(heap, float_id, "log2", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("log2: not numeric")?.log2()))
    });
    native(heap, float_id, "exp", |_heap, receiver, _args| {
        Ok(Value::float(receiver.as_float().ok_or("exp: not numeric")?.exp()))
    });
    native(heap, float_id, "pow:", |_heap, receiver, args| {
        let a = receiver.as_float().ok_or("pow: not numeric")?;
        let b = args.first().and_then(|v| v.as_float()).ok_or("pow: arg not numeric")?;
        Ok(Value::float(a.powf(b)))
    });

    // Float: predicates
    native(heap, float_id, "nan?", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("nan?: not numeric")?.is_nan()))
    });
    native(heap, float_id, "infinite?", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("infinite?: not numeric")?.is_infinite()))
    });
    native(heap, float_id, "finite?", |_heap, receiver, _args| {
        Ok(Value::boolean(receiver.as_float().ok_or("finite?: not numeric")?.is_finite()))
    });

    // Float constants: [Float pi], [Float e], [Float infinity], [Float nan]
    native(heap, float_id, "pi", |_heap, _receiver, _args| {
        Ok(Value::float(std::f64::consts::PI))
    });
    native(heap, float_id, "e", |_heap, _receiver, _args| {
        Ok(Value::float(std::f64::consts::E))
    });
    native(heap, float_id, "infinity", |_heap, _receiver, _args| {
        Ok(Value::float(f64::INFINITY))
    });
    native(heap, float_id, "nan", |_heap, _receiver, _args| {
        Ok(Value::float(f64::NAN))
    });

    // -- Cons prototype (type_protos[PROTO_CONS]) --
    let cons_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_CONS] = cons_proto;
    let cons_id = cons_proto.as_any_object().unwrap();

    native(heap, cons_id, "car", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("car: not a cons")?;
        Ok(heap.car(id))
    });
    native(heap, cons_id, "cdr", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("cdr: not a cons")?;
        Ok(heap.cdr(id))
    });
    native(heap, cons_id, "length", |heap, receiver, _args| {
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
    native(heap, cons_id, "describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });

    // -- String prototype (type_protos[PROTO_STR]) --
    let str_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_STR] = str_proto;
    let str_id = str_proto.as_any_object().unwrap();

    native(heap, str_id, "length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a string")?;
        let s = heap.get_string(id).ok_or("length: not a Text object")?;
        Ok(Value::integer(s.len() as i64))
    });
    native(heap, str_id, "at:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("at: not a string")?;
        let s = heap.get_string(id).ok_or("at: not a Text object")?;
        let idx = args.first().and_then(|v| v.as_integer()).ok_or("at: arg not an integer")? as usize;
        let ch = s.chars().nth(idx).map(|c| c.to_string()).unwrap_or_default();
        Ok(heap.alloc_string(&ch))
    });
    native(heap, str_id, "++", |heap, receiver, args| {
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
    native(heap, str_id, "substring:to:", |heap, receiver, args| {
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
    native(heap, str_id, "split:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("split: not a string")?;
        let s = heap.get_string(id).ok_or("split: not a Text object")?.to_string();
        let delim_arg = args.first().copied().unwrap_or(Value::NIL);
        let did = delim_arg.as_any_object().ok_or("split: arg not a string")?;
        let delim = heap.get_string(did).ok_or("split: arg not a Text object")?.to_string();
        let parts: Vec<Value> = s.split(&delim).map(|p| heap.alloc_string(p)).collect();
        Ok(heap.list(&parts))
    });
    native(heap, str_id, "trim", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("trim: not a string")?;
        let s = heap.get_string(id).ok_or("trim: not a Text object")?.trim().to_string();
        Ok(heap.alloc_string(&s))
    });
    native(heap, str_id, "contains:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("contains: not a string")?;
        let s = heap.get_string(id).ok_or("contains: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let nid = arg.as_any_object().ok_or("contains: arg not a string")?;
        let needle = heap.get_string(nid).ok_or("contains: arg not a Text object")?;
        Ok(Value::boolean(s.contains(needle)))
    });
    native(heap, str_id, "startsWith:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("startsWith: not a string")?;
        let s = heap.get_string(id).ok_or("startsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let pid = arg.as_any_object().ok_or("startsWith: arg not a string")?;
        let prefix = heap.get_string(pid).ok_or("startsWith: arg not a Text object")?;
        Ok(Value::boolean(s.starts_with(prefix)))
    });
    native(heap, str_id, "endsWith:", |heap, receiver, args| {
        let id = receiver.as_any_object().ok_or("endsWith: not a string")?;
        let s = heap.get_string(id).ok_or("endsWith: not a Text object")?.to_string();
        let arg = args.first().copied().unwrap_or(Value::NIL);
        let sid = arg.as_any_object().ok_or("endsWith: arg not a string")?;
        let suffix = heap.get_string(sid).ok_or("endsWith: arg not a Text object")?;
        Ok(Value::boolean(s.ends_with(suffix)))
    });
    native(heap, str_id, "toUpper", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toUpper: not a string")?;
        let s = heap.get_string(id).ok_or("toUpper: not a Text object")?;
        Ok(heap.alloc_string(&s.to_uppercase()))
    });
    native(heap, str_id, "toLower", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toLower: not a string")?;
        let s = heap.get_string(id).ok_or("toLower: not a Text object")?;
        Ok(heap.alloc_string(&s.to_lowercase()))
    });
    native(heap, str_id, "toInteger", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toInteger: not a string")?;
        let s = heap.get_string(id).ok_or("toInteger: not a Text object")?;
        let n: i64 = s.trim().parse().map_err(|_| format!("toInteger: cannot parse '{}'", s))?;
        Ok(Value::integer(n))
    });
    native(heap, str_id, "describe", |_heap, receiver, _args| {
        Ok(receiver) // strings describe as themselves
    });
    native(heap, str_id, "indexOf:", |heap, receiver, args| {
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
    native(heap, str_id, "replace:with:", |heap, receiver, args| {
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
    native(heap, str_id, "replaceAll:with:", |heap, receiver, args| {
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
    native(heap, str_id, "toFloat", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("toFloat: not a string")?;
        match heap.get(id) {
            HeapObject::Text(s) => match s.parse::<f64>() {
                Ok(n) => Ok(Value::float(n)),
                Err(_) => Err(format!("toFloat: cannot parse '{s}'")),
            },
            _ => Err("toFloat: not a string".into()),
        }
    });

    // -- Table prototype (type_protos[PROTO_TABLE]) --
    let table_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_TABLE] = table_proto;
    let table_id = table_proto.as_any_object().unwrap();

    native(heap, table_id, "at:", |heap, receiver, args| {
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
    native(heap, table_id, "at:put:", |heap, receiver, args| {
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
    native(heap, table_id, "push:", |heap, receiver, args| {
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
    native(heap, table_id, "length", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("length: not a table")?;
        match heap.get(id) {
            HeapObject::Table { seq, .. } => Ok(Value::integer(seq.len() as i64)),
            _ => Err("length: not a Table".into()),
        }
    });
    native(heap, table_id, "keys", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("keys: not a table")?;
        let keys: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(k, _)| *k).collect(),
            _ => return Err("keys: not a Table".into()),
        };
        Ok(heap.list(&keys))
    });
    native(heap, table_id, "values", |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("values: not a table")?;
        let vals: Vec<Value> = match heap.get(id) {
            HeapObject::Table { map, .. } => map.iter().map(|(_, v)| *v).collect(),
            _ => return Err("values: not a Table".into()),
        };
        Ok(heap.list(&vals))
    });
    native(heap, table_id, "describe", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    native(heap, table_id, "contains:", |heap, receiver, args| {
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
    native(heap, table_id, "remove:", |heap, receiver, args| {
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

    // -- Error prototype (type_protos[PROTO_ERROR]) --
    let error_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_ERROR] = error_proto;
    let error_id = error_proto.as_any_object().unwrap();

    // Error: message — read the message slot
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

    // -- register all prototypes as globals so they're accessible by name --
    let obj_sym = heap.intern("Object");
    heap.env_def(obj_sym, object_proto);
    let int_sym = heap.intern("Integer");
    heap.env_def(int_sym, int_proto);
    let nil_sym = heap.intern("Nil");
    heap.env_def(nil_sym, nil_proto);
    let bool_sym = heap.intern("Boolean");
    heap.env_def(bool_sym, bool_proto);
    let float_sym = heap.intern("Float");
    heap.env_def(float_sym, float_proto);
    let cons_sym = heap.intern("Cons");
    heap.env_def(cons_sym, cons_proto);
    let string_sym = heap.intern("String");
    heap.env_def(string_sym, str_proto);
    let table_sym = heap.intern("Table");
    heap.env_def(table_sym, table_proto);
    let number_sym = heap.intern("Number");
    heap.env_def(number_sym, number_proto);
    let symbol_sym = heap.intern("Symbol");
    heap.env_def(symbol_sym, sym_proto);
    let error_sym = heap.intern("Error");
    heap.env_def(error_sym, error_proto);

    // -- String: native < for Comparable --
    native(heap, str_id, "<", |heap, receiver, args| {
        let a_id = receiver.as_any_object().ok_or("< : not a string")?;
        let b_id = args.first().and_then(|v| v.as_any_object()).ok_or("< : arg not a string")?;
        match (heap.get(a_id), heap.get(b_id)) {
            (HeapObject::Text(a), HeapObject::Text(b)) => Ok(Value::boolean(a < b)),
            _ => Err("< : not strings".into()),
        }
    });

    // -- handlers on Object prototype (inherited by everything) --

    // print — [obj print] outputs and returns self
    native(heap, obj_id, "print", |heap, receiver, _args| {
        println!("{}", heap.format_value(receiver));
        Ok(receiver)
    });
    // println — [obj println] outputs and returns nil
    native(heap, obj_id, "println", |heap, receiver, _args| {
        println!("{}", heap.format_value(receiver));
        Ok(Value::NIL)
    });
    // type — [obj type] returns a symbol for the type
    native(heap, obj_id, "type", |heap, receiver, _args| {
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
                    HeapObject::Environment { .. } => "Environment",
                }
            } else { "Unknown" };
        Ok(Value::symbol(heap.intern(name)))
    });
    // equal: — content equality (like Ruby's eql?)
    native(heap, obj_id, "equal:", |heap, receiver, args| {
        let other = args.first().copied().unwrap_or(Value::NIL);
        Ok(Value::boolean(heap.values_equal(receiver, other)))
    });
    // show — default display for REPL (Showable protocol base)
    native(heap, obj_id, "show", |heap, receiver, _args| {
        let s = heap.format_value(receiver);
        Ok(heap.alloc_string(&s))
    });
    // identical: — bit-level identity test (semantic foundation for eq)
    native(heap, obj_id, "identical:", |_heap, receiver, args| {
        let other = args.first().copied().unwrap_or(Value::NIL);
        Ok(Value::boolean(receiver == other))
    });

    // -- Block/Closure prototype (type_protos[PROTO_CLOSURE]) --
    let block_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_CLOSURE] = block_proto;
    let block_id = block_proto.as_any_object().unwrap();

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

    // -- FarRef prototype (PROTO_FARREF) --
    // a far reference is a proxy for an object in another vat.
    // all sends are intercepted via doesNotUnderstand: and queued.
    let farref_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_FARREF] = farref_proto;
    let farref_id = farref_proto.as_any_object().unwrap();

    // FarRef: doesNotUnderstand: — intercept ALL sends, queue to outbox
    let target_vat_sym = heap.intern("__target_vat");
    let target_obj_sym = heap.intern("__target_obj");
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

    // -- Promise prototype (PROTO_PROMISE) --
    // a promise represents a future value. sends to unresolved promises
    // are buffered via doesNotUnderstand: and forwarded when resolved.
    let promise_proto = heap.make_object(object_proto);
    heap.type_protos[PROTO_PROMISE] = promise_proto;
    let promise_id = promise_proto.as_any_object().unwrap();

    // Promise: describe
    let state_sym = heap.intern("__state");
    native(heap, promise_id, "describe", move |heap, receiver, _args| {
        let id = receiver.as_any_object().ok_or("describe: not a promise")?;
        let state = heap.get(id).slot_get(state_sym)
            .map(|v| heap.format_value(v)).unwrap_or("?".into());
        let s = format!("<promise {state}>");
        Ok(heap.alloc_string(&s))
    });

    let promise_sym = heap.intern("Promise");
    heap.env_def(promise_sym, promise_proto);

    // -- Vat prototype --
    // [Vat spawn: block] creates a new vat. The block runs in the new vat
    // and its return value becomes accessible via a far reference.
    let vat_proto = heap.make_object(object_proto);
    let vat_sym = heap.intern("Vat");
    heap.env_def(vat_sym, vat_proto);
}
