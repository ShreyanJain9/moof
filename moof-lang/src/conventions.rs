/// Standard conventions: type prototypes and their handlers.
/// These make the fabric's data types respond to messages.
///
/// Integer: +, -, *, /, %, <, >, =, <=, >=, negate, abs, toString, describe
/// Float: same as Integer
/// Boolean: not, toString, describe, ifTrue:, ifFalse:, ifTrue:ifFalse:, and:, or:
/// String: length, ++, at:, toString, describe, =
/// Cons: car, cdr, toString, describe
/// Nil: isNil, toString, describe
/// Symbol: name, toString, describe

use moof_fabric::*;

/// Register all standard conventions on a fabric.
pub fn register(fabric: &mut Fabric, native: &mut NativeInvoker) {
    let root_obj = fabric.create_object(Value::Nil);

    // ── Integer ──
    let int_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.integer = Some(int_proto);

    reg(fabric, native, int_proto, "+", "Integer.+", |heap, args| {
        let a = args[0].as_integer().ok_or("+ expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("+ expects integer arg")?;
        Ok(Value::Integer(a + b))
    });
    reg(fabric, native, int_proto, "-", "Integer.-", |heap, args| {
        let a = args[0].as_integer().ok_or("- expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("- expects integer arg")?;
        Ok(Value::Integer(a - b))
    });
    reg(fabric, native, int_proto, "*", "Integer.*", |heap, args| {
        let a = args[0].as_integer().ok_or("* expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("* expects integer arg")?;
        Ok(Value::Integer(a * b))
    });
    reg(fabric, native, int_proto, "/", "Integer./", |heap, args| {
        let a = args[0].as_integer().ok_or("/ expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("/ expects integer arg")?;
        if b == 0 { return Err("division by zero".into()); }
        Ok(Value::Integer(a / b))
    });
    reg(fabric, native, int_proto, "%", "Integer.%", |heap, args| {
        let a = args[0].as_integer().ok_or("% expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("% expects integer arg")?;
        Ok(Value::Integer(a % b))
    });
    reg(fabric, native, int_proto, "<", "Integer.<", |heap, args| {
        let a = args[0].as_integer().ok_or("< expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("< expects integer arg")?;
        Ok(if a < b { Value::True } else { Value::False })
    });
    reg(fabric, native, int_proto, ">", "Integer.>", |heap, args| {
        let a = args[0].as_integer().ok_or("> expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("> expects integer arg")?;
        Ok(if a > b { Value::True } else { Value::False })
    });
    reg(fabric, native, int_proto, "=", "Integer.=", |heap, args| {
        let a = args[0].as_integer().ok_or("= expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("= expects integer arg")?;
        Ok(if a == b { Value::True } else { Value::False })
    });
    reg(fabric, native, int_proto, "<=", "Integer.<=", |heap, args| {
        let a = args[0].as_integer().ok_or("<= expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("<= expects integer arg")?;
        Ok(if a <= b { Value::True } else { Value::False })
    });
    reg(fabric, native, int_proto, ">=", "Integer.>=", |heap, args| {
        let a = args[0].as_integer().ok_or(">= expects integer")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or(">= expects integer arg")?;
        Ok(if a >= b { Value::True } else { Value::False })
    });
    reg(fabric, native, int_proto, "negate", "Integer.negate", |_heap, args| {
        let a = args[0].as_integer().ok_or("negate expects integer")?;
        Ok(Value::Integer(-a))
    });
    reg(fabric, native, int_proto, "abs", "Integer.abs", |_heap, args| {
        let a = args[0].as_integer().ok_or("abs expects integer")?;
        Ok(Value::Integer(a.abs()))
    });
    reg(fabric, native, int_proto, "toString", "Integer.toString", |heap, args| {
        let a = args[0].as_integer().ok_or("toString expects integer")?;
        Ok(heap.alloc_string(&a.to_string()))
    });
    reg(fabric, native, int_proto, "describe", "Integer.describe", |heap, args| {
        let a = args[0].as_integer().ok_or("describe expects integer")?;
        Ok(heap.alloc_string(&a.to_string()))
    });

    // ── Float ──
    let float_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.float = Some(float_proto);

    for (sel, name, op) in [
        ("+", "Float.+", '+' as char), ("-", "Float.-", '-'),
        ("*", "Float.*", '*'), ("/", "Float./", '/'), ("%", "Float.%", '%'),
    ] {
        let op = op;
        reg(fabric, native, float_proto, sel, name, move |_heap, args| {
            let a = args[0].as_float().ok_or("expects float")?;
            let b = args.get(1).and_then(|v| v.as_float()).ok_or("expects float arg")?;
            Ok(Value::Float(match op { '+' => a + b, '-' => a - b, '*' => a * b, '/' => a / b, '%' => a % b, _ => unreachable!() }))
        });
    }
    for (sel, name, cmp) in [
        ("<", "Float.<", '<' as char), (">", "Float.>", '>'),
        ("=", "Float.=", '='), ("<=", "Float.<=", '{'), (">=", "Float.>=", '}'),
    ] {
        reg(fabric, native, float_proto, sel, name, move |_heap, args| {
            let a = args[0].as_float().ok_or("expects float")?;
            let b = args.get(1).and_then(|v| v.as_float()).ok_or("expects float arg")?;
            Ok(if match cmp { '<' => a < b, '>' => a > b, '=' => a == b, '{' => a <= b, '}' => a >= b, _ => unreachable!() } { Value::True } else { Value::False })
        });
    }
    reg(fabric, native, float_proto, "floor", "Float.floor", |_h, a| Ok(Value::Integer(a[0].as_float().ok_or("floor")?.floor() as i64)));
    reg(fabric, native, float_proto, "ceil", "Float.ceil", |_h, a| Ok(Value::Integer(a[0].as_float().ok_or("ceil")?.ceil() as i64)));
    reg(fabric, native, float_proto, "round", "Float.round", |_h, a| Ok(Value::Integer(a[0].as_float().ok_or("round")?.round() as i64)));
    reg(fabric, native, float_proto, "sqrt", "Float.sqrt", |_h, a| Ok(Value::Float(a[0].as_float().ok_or("sqrt")?.sqrt())));
    reg(fabric, native, float_proto, "sin", "Float.sin", |_h, a| Ok(Value::Float(a[0].as_float().ok_or("sin")?.sin())));
    reg(fabric, native, float_proto, "cos", "Float.cos", |_h, a| Ok(Value::Float(a[0].as_float().ok_or("cos")?.cos())));
    reg(fabric, native, float_proto, "toString", "Float.toString", |h, a| Ok(h.alloc_string(&format!("{}", a[0].as_float().ok_or("toString")?))));
    reg(fabric, native, float_proto, "describe", "Float.describe", |h, a| Ok(h.alloc_string(&format!("{}", a[0].as_float().ok_or("describe")?))));

    // ── Boolean ──
    let bool_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.boolean = Some(bool_proto);

    reg(fabric, native, bool_proto, "not", "Boolean.not", |_h, a| Ok(if a[0].is_truthy() { Value::False } else { Value::True }));
    reg(fabric, native, bool_proto, "toString", "Boolean.toString", |h, a| Ok(h.alloc_string(if a[0] == Value::True { "true" } else { "false" })));
    reg(fabric, native, bool_proto, "describe", "Boolean.describe", |h, a| Ok(h.alloc_string(if a[0] == Value::True { "true" } else { "false" })));
    // NOTE: ifTrue:, ifFalse:, ifTrue:ifFalse:, and:, or: need call_value (lazy eval).
    // They're registered as VM-level intercepts in the interpreter, not here.

    // ── String ──
    let str_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.string = Some(str_proto);

    reg(fabric, native, str_proto, "length", "String.length", |heap, args| {
        if let Value::Object(id) = args[0] {
            if let HeapObject::String(s) = heap.get(id) {
                return Ok(Value::Integer(s.len() as i64));
            }
        }
        Err("length expects string".into())
    });
    reg(fabric, native, str_proto, "++", "String.++", |heap, args| {
        let a = match args[0] { Value::Object(id) => match heap.get(id) { HeapObject::String(s) => s.clone(), _ => return Err("++ expects string".into()) }, _ => return Err("++ expects string".into()) };
        let b = match args.get(1).copied().unwrap_or(Value::Nil) { Value::Object(id) => match heap.get(id) { HeapObject::String(s) => s.clone(), _ => "".into() }, _ => "".into() };
        Ok(heap.alloc_string(&format!("{}{}", a, b)))
    });
    reg(fabric, native, str_proto, "=", "String.=", |heap, args| {
        let a = match args[0] { Value::Object(id) => match heap.get(id) { HeapObject::String(s) => s.clone(), _ => return Ok(Value::False) }, _ => return Ok(Value::False) };
        let b = match args.get(1).copied().unwrap_or(Value::Nil) { Value::Object(id) => match heap.get(id) { HeapObject::String(s) => s.clone(), _ => return Ok(Value::False) }, _ => return Ok(Value::False) };
        Ok(if a == b { Value::True } else { Value::False })
    });
    reg(fabric, native, str_proto, "toString", "String.toString", |_h, a| Ok(a[0]));
    reg(fabric, native, str_proto, "describe", "String.describe", |heap, args| {
        if let Value::Object(id) = args[0] {
            if let HeapObject::String(s) = heap.get(id) {
                return Ok(heap.alloc_string(&format!("\"{}\"", s)));
            }
        }
        Ok(args[0])
    });

    // ── Cons ──
    let cons_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.cons = Some(cons_proto);

    reg(fabric, native, cons_proto, "car", "Cons.car", |heap, args| Ok(heap.car(args[0])));
    reg(fabric, native, cons_proto, "cdr", "Cons.cdr", |heap, args| Ok(heap.cdr(args[0])));
    reg(fabric, native, cons_proto, "toString", "Cons.toString", |heap, args| Ok(heap.alloc_string("<cons>")));
    reg(fabric, native, cons_proto, "describe", "Cons.describe", |heap, args| Ok(heap.alloc_string("<cons>")));

    // ── Nil ──
    let nil_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.nil = Some(nil_proto);

    reg(fabric, native, nil_proto, "isNil", "Nil.isNil", |_h, _a| Ok(Value::True));
    reg(fabric, native, nil_proto, "toString", "Nil.toString", |h, _a| Ok(h.alloc_string("nil")));
    reg(fabric, native, nil_proto, "describe", "Nil.describe", |h, _a| Ok(h.alloc_string("nil")));

    // ── Symbol ──
    let sym_proto = fabric.create_object(Value::Object(root_obj));
    fabric.type_protos.symbol = Some(sym_proto);

    reg(fabric, native, sym_proto, "name", "Symbol.name", |heap, args| {
        if let Value::Symbol(id) = args[0] {
            let name = heap.symbol_name(id).to_string();
            Ok(heap.alloc_string(&name))
        } else { Err("name expects symbol".into()) }
    });
    reg(fabric, native, sym_proto, "toString", "Symbol.toString", |heap, args| {
        if let Value::Symbol(id) = args[0] {
            let name = heap.symbol_name(id).to_string();
            Ok(heap.alloc_string(&name))
        } else { Err("toString expects symbol".into()) }
    });
    reg(fabric, native, sym_proto, "describe", "Symbol.describe", |heap, args| {
        if let Value::Symbol(id) = args[0] {
            Ok(heap.alloc_string(&format!("'{}", heap.symbol_name(id))))
        } else { Err("describe expects symbol".into()) }
    });

    // Bind type proto names in root env for bootstrap access
    let env = fabric.create_object(Value::Nil); // we'll use this differently
    // Store the root object id for later
    let root_sym = fabric.intern("Object");
    // These will be bound in the root environment by the shell
}

/// Helper: register a native function on a prototype.
fn reg(
    fabric: &mut Fabric,
    native: &mut NativeInvoker,
    proto: u32,
    selector: &str,
    native_name: &str,
    f: impl Fn(&mut Heap, &[Value]) -> Result<Value, String> + Send + 'static,
) {
    native.register(native_name, Box::new(f));
    let handler_id = NativeInvoker::make_handler(&mut fabric.heap, native_name);
    let sel_sym = fabric.intern(selector);
    fabric.heap.add_handler(proto, sel_sym, Value::Object(handler_id));
}
