/// Native handler registration for all MOOF type prototypes.
///
/// Every native operation is a NativeFunction closure in the NativeRegistry.
/// The handler for `+` on Integer is the same kind of thing as `ffi-sin`.
/// One path. One mechanism.

use crate::runtime::value::{Value, HeapObject};
use super::exec::VM;

/// Register all native type prototype handlers.
/// Called after bootstrap defines the prototype objects.
pub fn register_all_natives(vm: &mut VM, root_env: u32) {
    register_integer_natives(vm, root_env);
    register_float_natives(vm, root_env);
    register_boolean_natives(vm, root_env);
    register_string_natives(vm, root_env);
    register_cons_natives(vm, root_env);
    register_nil_natives(vm, root_env);
    register_symbol_natives(vm, root_env);
    register_lambda_natives(vm, root_env);
    register_operative_natives(vm, root_env);
    register_environment_natives(vm, root_env);
    register_io_natives(vm, root_env);
}

/// Helper: look up a prototype by name in the env, set the VM field, return the proto id.
fn lookup_proto(vm: &mut VM, env_id: u32, name: &str, setter: fn(&mut VM) -> &mut Option<u32>) -> Option<u32> {
    let sym = vm.heap.intern(name);
    if let Ok(Value::Object(id)) = vm.env_lookup_helper(env_id, sym) {
        *setter(vm) = Some(id);
        Some(id)
    } else {
        None
    }
}

/// Helper: register a native closure and add it as a handler on a prototype.
fn add_native(vm: &mut VM, proto_id: u32, selector: &str, name: &str, func: Box<dyn Fn(&mut crate::runtime::heap::Heap, &[Value]) -> Result<Value, String>>) {
    let val = vm.register_native(name, func);
    let sel_sym = vm.heap.intern(selector);
    vm.heap.add_handler(proto_id, sel_sym, val);
}

// ── Integer ──

fn register_integer_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Integer", |vm| &mut vm.proto_integer) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "+", "Integer.+", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("+ expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("+ expects integer argument")?;
        Ok(Value::Integer(a + b))
    }));

    add_native(vm, proto_id, "-", "Integer.-", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("- expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("- expects integer argument")?;
        Ok(Value::Integer(a - b))
    }));

    add_native(vm, proto_id, "*", "Integer.*", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("* expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("* expects integer argument")?;
        Ok(Value::Integer(a * b))
    }));

    add_native(vm, proto_id, "/", "Integer./", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("/ expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("/ expects integer argument")?;
        if b == 0 { return Err("Division by zero".into()); }
        Ok(Value::Integer(a / b))
    }));

    add_native(vm, proto_id, "%", "Integer.%", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("% expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("% expects integer argument")?;
        if b == 0 { return Err("Modulo by zero".into()); }
        Ok(Value::Integer(a % b))
    }));

    add_native(vm, proto_id, "<", "Integer.<", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("< expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("< expects integer argument")?;
        Ok(if a < b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, ">", "Integer.>", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("> expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("> expects integer argument")?;
        Ok(if a > b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "=", "Integer.=", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("= expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("= expects integer argument")?;
        Ok(if a == b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "<=", "Integer.<=", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("<= expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or("<= expects integer argument")?;
        Ok(if a <= b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, ">=", "Integer.>=", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or(">= expects integer receiver")?;
        let b = args.get(1).and_then(|v| v.as_integer()).ok_or(">= expects integer argument")?;
        Ok(if a >= b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "negate", "Integer.negate", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("negate expects integer")?;
        Ok(Value::Integer(-a))
    }));

    add_native(vm, proto_id, "abs", "Integer.abs", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("abs expects integer")?;
        Ok(Value::Integer(a.abs()))
    }));

    add_native(vm, proto_id, "toString", "Integer.toString", Box::new(|heap, args| {
        let a = args[0].as_integer().ok_or("toString expects integer")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));

    add_native(vm, proto_id, "describe", "Integer.describe", Box::new(|heap, args| {
        let a = args[0].as_integer().ok_or("describe expects integer")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));

    add_native(vm, proto_id, "asString", "Integer.asString", Box::new(|heap, args| {
        let a = args[0].as_integer().ok_or("asString expects integer")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));

    add_native(vm, proto_id, "toFloat", "Integer.toFloat", Box::new(|_heap, args| {
        let a = args[0].as_integer().ok_or("toFloat expects integer")?;
        Ok(Value::Float(a as f64))
    }));
}

// ── Float ──

fn register_float_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Float", |vm| &mut vm.proto_float) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "+", "Float.+", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("+ expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("+ expects number argument")?;
        Ok(Value::Float(a + b))
    }));

    add_native(vm, proto_id, "-", "Float.-", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("- expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("- expects number argument")?;
        Ok(Value::Float(a - b))
    }));

    add_native(vm, proto_id, "*", "Float.*", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("* expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("* expects number argument")?;
        Ok(Value::Float(a * b))
    }));

    add_native(vm, proto_id, "/", "Float./", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("/ expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("/ expects number argument")?;
        Ok(Value::Float(a / b))
    }));

    add_native(vm, proto_id, "%", "Float.%", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("% expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("% expects number argument")?;
        Ok(Value::Float(a % b))
    }));

    add_native(vm, proto_id, "<", "Float.<", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("< expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("< expects number argument")?;
        Ok(if a < b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, ">", "Float.>", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("> expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("> expects number argument")?;
        Ok(if a > b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "=", "Float.=", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("= expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("= expects number argument")?;
        Ok(if a == b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "<=", "Float.<=", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("<= expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or("<= expects number argument")?;
        Ok(if a <= b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, ">=", "Float.>=", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or(">= expects number receiver")?;
        let b = args.get(1).and_then(|v| v.as_float()).ok_or(">= expects number argument")?;
        Ok(if a >= b { Value::True } else { Value::False })
    }));

    add_native(vm, proto_id, "negate", "Float.negate", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("negate expects number")?;
        Ok(Value::Float(-a))
    }));

    add_native(vm, proto_id, "abs", "Float.abs", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("abs expects number")?;
        Ok(Value::Float(a.abs()))
    }));

    add_native(vm, proto_id, "floor", "Float.floor", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("floor expects number")?;
        Ok(Value::Integer(a.floor() as i64))
    }));

    add_native(vm, proto_id, "ceil", "Float.ceil", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("ceil expects number")?;
        Ok(Value::Integer(a.ceil() as i64))
    }));

    add_native(vm, proto_id, "round", "Float.round", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("round expects number")?;
        Ok(Value::Integer(a.round() as i64))
    }));

    add_native(vm, proto_id, "sqrt", "Float.sqrt", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("sqrt expects number")?;
        Ok(Value::Float(a.sqrt()))
    }));

    add_native(vm, proto_id, "sin", "Float.sin", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("sin expects number")?;
        Ok(Value::Float(a.sin()))
    }));

    add_native(vm, proto_id, "cos", "Float.cos", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("cos expects number")?;
        Ok(Value::Float(a.cos()))
    }));

    add_native(vm, proto_id, "toInteger", "Float.toInteger", Box::new(|_heap, args| {
        let a = args[0].as_float().ok_or("toInteger expects number")?;
        Ok(Value::Integer(a as i64))
    }));

    add_native(vm, proto_id, "toString", "Float.toString", Box::new(|heap, args| {
        let a = args[0].as_float().ok_or("toString expects number")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));

    add_native(vm, proto_id, "describe", "Float.describe", Box::new(|heap, args| {
        let a = args[0].as_float().ok_or("describe expects number")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));

    add_native(vm, proto_id, "asString", "Float.asString", Box::new(|heap, args| {
        let a = args[0].as_float().ok_or("asString expects number")?;
        Ok(heap.alloc_string(&format!("{}", a)))
    }));
}

// ── Boolean ──

fn register_boolean_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Boolean", |vm| &mut vm.proto_boolean) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "not", "Boolean.not", Box::new(|_heap, args| {
        match args[0] {
            Value::True => Ok(Value::False),
            Value::False => Ok(Value::True),
            _ => Err("not expects boolean".into()),
        }
    }));

    add_native(vm, proto_id, "toString", "Boolean.toString", Box::new(|heap, args| {
        match args[0] {
            Value::True => Ok(heap.alloc_string("true")),
            Value::False => Ok(heap.alloc_string("false")),
            _ => Err("toString expects boolean".into()),
        }
    }));

    add_native(vm, proto_id, "describe", "Boolean.describe", Box::new(|heap, args| {
        match args[0] {
            Value::True => Ok(heap.alloc_string("true")),
            Value::False => Ok(heap.alloc_string("false")),
            _ => Err("describe expects boolean".into()),
        }
    }));

    // NOTE: ifTrue:, ifTrue:ifFalse:, ifFalse:, and:, or: need call_value,
    // which requires the VM — not just the heap. These stay in primitive_send.
}

// ── String ──

fn register_string_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "String", |vm| &mut vm.proto_string) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "toString", "String.toString", Box::new(|_heap, args| {
        Ok(args[0])
    }));

    // Content equality for strings (eq is identity, = is content)
    add_native(vm, proto_id, "=", "String.=", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("= expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("= expects string".into()),
        };
        if let Some(Value::Object(other_id)) = args.get(1) {
            if let HeapObject::MoofString(other) = heap.get(*other_id) {
                return Ok(if s == *other { Value::True } else { Value::False });
            }
        }
        Ok(Value::False)
    }));

    add_native(vm, proto_id, "describe", "String.describe", Box::new(|_heap, args| {
        Ok(args[0])
    }));

    add_native(vm, proto_id, "length", "String.length", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("length expects string object")?;
        match heap.get(id) {
            HeapObject::MoofString(s) => Ok(Value::Integer(s.chars().count() as i64)),
            _ => Err("length expects string".into()),
        }
    }));

    add_native(vm, proto_id, "++", "String.++", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("++ expects string receiver")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("++ expects string receiver".into()),
        };
        if let Some(Value::Object(other_id)) = args.get(1) {
            if let HeapObject::MoofString(other) = heap.get(*other_id) {
                let new_s = format!("{}{}", s, other);
                return Ok(heap.alloc_string(&new_s));
            }
        }
        // Fallback: format the arg
        if let Some(&arg) = args.get(1) {
            let other_s = match arg {
                Value::Integer(n) => format!("{}", n),
                Value::Float(f) => format!("{}", f),
                Value::Nil => "nil".to_string(),
                Value::True => "true".to_string(),
                Value::False => "false".to_string(),
                Value::Symbol(sid) => format!("'{}", heap.symbol_name(sid)),
                Value::Object(oid) => match heap.get(oid) {
                    HeapObject::MoofString(s) => s.clone(),
                    _ => format!("<object #{}>", oid),
                },
            };
            let new_s = format!("{}{}", s, other_s);
            return Ok(heap.alloc_string(&new_s));
        }
        Err("++ expects an argument".into())
    }));

    add_native(vm, proto_id, "substring:to:", "String.substring:to:", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("substring:to: expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("substring:to: expects string".into()),
        };
        let start = args.get(1).and_then(|v| v.as_integer())
            .ok_or("substring:to: expects integer start")? as usize;
        let end = args.get(2).and_then(|v| v.as_integer())
            .ok_or("substring:to: expects integer end")? as usize;
        let chars: Vec<char> = s.chars().collect();
        let end = end.min(chars.len());
        let start = start.min(end);
        let sub: String = chars[start..end].iter().collect();
        Ok(heap.alloc_string(&sub))
    }));

    add_native(vm, proto_id, "at:", "String.at:", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("at: expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("at: expects string".into()),
        };
        let idx = args.get(1).and_then(|v| v.as_integer())
            .ok_or("at: expects integer index")? as usize;
        let chars: Vec<char> = s.chars().collect();
        if idx < chars.len() {
            let ch: String = chars[idx..idx+1].iter().collect();
            Ok(heap.alloc_string(&ch))
        } else {
            Ok(Value::Nil)
        }
    }));

    add_native(vm, proto_id, "indexOf:", "String.indexOf:", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("indexOf: expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("indexOf: expects string".into()),
        };
        if let Some(Value::Object(other_id)) = args.get(1) {
            if let HeapObject::MoofString(needle) = heap.get(*other_id) {
                if let Some(pos) = s.find(needle.as_str()) {
                    let char_pos = s[..pos].chars().count();
                    return Ok(Value::Integer(char_pos as i64));
                }
            }
        }
        Ok(Value::Nil)
    }));

    add_native(vm, proto_id, "split:", "String.split:", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("split: expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("split: expects string".into()),
        };
        if let Some(Value::Object(other_id)) = args.get(1) {
            if let HeapObject::MoofString(delim) = heap.get(*other_id) {
                let delim = delim.clone();
                let parts: Vec<Value> = s.split(&delim)
                    .map(|part| heap.alloc_string(part))
                    .collect();
                return Ok(heap.list(&parts));
            }
        }
        Err("split: expects string delimiter".into())
    }));

    add_native(vm, proto_id, "trim", "String.trim", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("trim expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("trim expects string".into()),
        };
        Ok(heap.alloc_string(s.trim()))
    }));

    // startsWith:, endsWith:, contains: — migrated to system.moof

    add_native(vm, proto_id, "toUpper", "String.toUpper", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("toUpper expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("toUpper expects string".into()),
        };
        Ok(heap.alloc_string(&s.to_uppercase()))
    }));

    add_native(vm, proto_id, "toLower", "String.toLower", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("toLower expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("toLower expects string".into()),
        };
        Ok(heap.alloc_string(&s.to_lowercase()))
    }));

    add_native(vm, proto_id, "toSymbol", "String.toSymbol", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("toSymbol expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("toSymbol expects string".into()),
        };
        let sym = heap.intern(&s);
        Ok(Value::Symbol(sym))
    }));

    add_native(vm, proto_id, "toInteger", "String.toInteger", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("toInteger expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("toInteger expects string".into()),
        };
        match s.trim().parse::<i64>() {
            Ok(n) => Ok(Value::Integer(n)),
            Err(_) => Ok(Value::Nil),
        }
    }));

    add_native(vm, proto_id, "chars", "String.chars", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("chars expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("chars expects string".into()),
        };
        let chars: Vec<Value> = s.chars()
            .map(|c| heap.alloc_string(&c.to_string()))
            .collect();
        Ok(heap.list(&chars))
    }));

    add_native(vm, proto_id, "replace:with:", "String.replace:with:", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("replace:with: expects string")?;
        let s = match heap.get(id) {
            HeapObject::MoofString(s) => s.clone(),
            _ => return Err("replace:with: expects string".into()),
        };
        if let (Some(Value::Object(from_id)), Some(Value::Object(to_id))) = (args.get(1), args.get(2)) {
            if let (HeapObject::MoofString(from), HeapObject::MoofString(to)) = (heap.get(*from_id), heap.get(*to_id)) {
                let result = s.replace(from.as_str(), to.as_str());
                return Ok(heap.alloc_string(&result));
            }
        }
        Err("replace:with: expects string arguments".into())
    }));
}

// ── Cons ──

fn register_cons_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Cons", |vm| &mut vm.proto_cons) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "car", "Cons.car", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("car expects cons")?;
        match heap.get(id) {
            HeapObject::Cons { car, .. } => Ok(*car),
            _ => Err("car expects cons".into()),
        }
    }));

    add_native(vm, proto_id, "cdr", "Cons.cdr", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("cdr expects cons")?;
        match heap.get(id) {
            HeapObject::Cons { cdr, .. } => Ok(*cdr),
            _ => Err("cdr expects cons".into()),
        }
    }));

    add_native(vm, proto_id, "toString", "Cons.toString", Box::new(|heap, args| {
        // Simple cons toString — format as list
        let s = format_cons_value(heap, args[0]);
        Ok(heap.alloc_string(&s))
    }));

    add_native(vm, proto_id, "describe", "Cons.describe", Box::new(|heap, args| {
        let s = format_cons_value(heap, args[0]);
        Ok(heap.alloc_string(&s))
    }));
}

/// Format a cons list for display (used by Cons.toString).
fn format_cons_value(heap: &crate::runtime::heap::Heap, val: Value) -> String {
    let mut parts = Vec::new();
    let mut current = val;
    loop {
        match current {
            Value::Nil => break,
            Value::Object(id) => {
                match heap.get(id) {
                    HeapObject::Cons { car, cdr } => {
                        parts.push(format_simple_value(heap, *car));
                        current = *cdr;
                    }
                    _ => {
                        parts.push(format!(". {}", format_simple_value(heap, current)));
                        break;
                    }
                }
            }
            other => {
                parts.push(format!(". {}", format_simple_value(heap, other)));
                break;
            }
        }
    }
    format!("({})", parts.join(" "))
}

/// Simple value formatting for use in closures (no VM access).
fn format_simple_value(heap: &crate::runtime::heap::Heap, val: Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::True => "true".to_string(),
        Value::False => "false".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'{}", heap.symbol_name(id)),
        Value::Object(id) => {
            match heap.get(id) {
                HeapObject::Cons { .. } => format_cons_value(heap, val),
                HeapObject::MoofString(s) => format!("\"{}\"", s),
                HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
                HeapObject::BytecodeChunk(_) => "<bytecode>".to_string(),
                HeapObject::Operative { .. } => "<operative>".to_string(),
                HeapObject::Lambda { .. } => "<lambda>".to_string(),
                HeapObject::Environment(_) => "<environment>".to_string(),
                HeapObject::NativeFunction { name } => format!("<native {}>", name),
            }
        }
    }
}

// ── Nil ──

fn register_nil_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Nil", |vm| &mut vm.proto_nil) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "isNil", "Nil.isNil", Box::new(|_heap, _args| {
        Ok(Value::True)
    }));

    add_native(vm, proto_id, "toString", "Nil.toString", Box::new(|heap, _args| {
        Ok(heap.alloc_string("nil"))
    }));

    add_native(vm, proto_id, "describe", "Nil.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("nil"))
    }));
}

// ── Symbol ──

fn register_symbol_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Symbol", |vm| &mut vm.proto_symbol) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "toString", "Symbol.toString", Box::new(|heap, args| {
        if let Value::Symbol(id) = args[0] {
            let s = format!("'{}", heap.symbol_name(id));
            Ok(heap.alloc_string(&s))
        } else {
            Err("toString expects symbol".into())
        }
    }));

    add_native(vm, proto_id, "describe", "Symbol.describe", Box::new(|heap, args| {
        if let Value::Symbol(id) = args[0] {
            let s = format!("'{}", heap.symbol_name(id));
            Ok(heap.alloc_string(&s))
        } else {
            Err("describe expects symbol".into())
        }
    }));

    add_native(vm, proto_id, "asString", "Symbol.asString", Box::new(|heap, args| {
        if let Value::Symbol(id) = args[0] {
            let name = heap.symbol_name(id).to_string();
            Ok(heap.alloc_string(&name))
        } else {
            Err("asString expects symbol".into())
        }
    }));

    add_native(vm, proto_id, "name", "Symbol.name", Box::new(|heap, args| {
        if let Value::Symbol(id) = args[0] {
            let name = heap.symbol_name(id).to_string();
            Ok(heap.alloc_string(&name))
        } else {
            Err("name expects symbol".into())
        }
    }));
}

// ── Lambda ──

fn register_lambda_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Lambda", |vm| &mut vm.proto_lambda) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "source", "Lambda.source", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("source expects lambda")?;
        match heap.get(id) {
            HeapObject::Lambda { source, .. } => Ok(*source),
            _ => Err("source expects lambda".into()),
        }
    }));

    add_native(vm, proto_id, "params", "Lambda.params", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("params expects lambda")?;
        match heap.get(id) {
            HeapObject::Lambda { params, .. } => Ok(*params),
            _ => Err("params expects lambda".into()),
        }
    }));

    add_native(vm, proto_id, "arity", "Lambda.arity", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("arity expects lambda")?;
        match heap.get(id) {
            HeapObject::Lambda { params, .. } => {
                let n = heap.list_to_vec(*params).len();
                Ok(Value::Integer(n as i64))
            }
            _ => Err("arity expects lambda".into()),
        }
    }));

    add_native(vm, proto_id, "toString", "Lambda.toString", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<lambda>"))
    }));

    add_native(vm, proto_id, "describe", "Lambda.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<lambda>"))
    }));
}

// ── Operative ──

fn register_operative_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Operative", |vm| &mut vm.proto_operative) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "source", "Operative.source", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("source expects operative")?;
        match heap.get(id) {
            HeapObject::Operative { source, .. } => Ok(*source),
            _ => Err("source expects operative".into()),
        }
    }));

    add_native(vm, proto_id, "params", "Operative.params", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("params expects operative")?;
        match heap.get(id) {
            HeapObject::Operative { params, .. } => Ok(*params),
            _ => Err("params expects operative".into()),
        }
    }));

    add_native(vm, proto_id, "envParam", "Operative.envParam", Box::new(|heap, args| {
        let id = args[0].as_object().ok_or("envParam expects operative")?;
        match heap.get(id) {
            HeapObject::Operative { env_param, .. } => Ok(Value::Symbol(*env_param)),
            _ => Err("envParam expects operative".into()),
        }
    }));

    add_native(vm, proto_id, "toString", "Operative.toString", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<operative>"))
    }));

    add_native(vm, proto_id, "describe", "Operative.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<operative>"))
    }));
}

// ── Environment ──
// NOTE: eval:, lookup:, set:to: need VM-level access (call_value, env_lookup, env_set).
// These stay in primitive_send. We only register the pure ones here.

fn register_environment_natives(vm: &mut VM, root_env: u32) {
    let proto_id = match lookup_proto(vm, root_env, "Environment", |vm| &mut vm.proto_environment) {
        Some(id) => id,
        None => return,
    };

    add_native(vm, proto_id, "toString", "Environment.toString", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<environment>"))
    }));

    add_native(vm, proto_id, "describe", "Environment.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<environment>"))
    }));
}

// ── I/O ──
// Minimal stdio natives for MCP and general I/O.

fn register_io_natives(vm: &mut VM, root_env: u32) {
    use std::io::{self, BufRead, Write as IoWrite};

    // __save-image: serialize heap to .moof/image.bin (VM-level native)
    let val = vm.register_native("__save-image", Box::new(|_heap, _args| {
        // Actual work is done in VM::call_native via intercept
        Ok(Value::True)
    }));
    let sym = vm.heap.intern("__save-image");
    vm.heap.env_define(root_env, sym, val);

    // __save-source removed — no more source projection

    // __try: error containment (VM-level native)
    let val = vm.register_native("__try", Box::new(|_heap, _args| {
        Ok(Value::Nil) // actual work done in VM::call_native intercept
    }));
    let sym = vm.heap.intern("__try");
    vm.heap.env_define(root_env, sym, val);

    // read-line: reads a line from stdin, returns string or nil on EOF
    let val = vm.register_native("io:read-line", Box::new(|heap, _args| {
        let stdin = io::stdin();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => Ok(Value::Nil), // EOF
            Ok(_) => {
                // Trim trailing newline
                if line.ends_with('\n') { line.pop(); }
                if line.ends_with('\r') { line.pop(); }
                Ok(heap.alloc_string(&line))
            }
            Err(e) => Err(format!("read-line: {}", e)),
        }
    }));
    let sym = vm.heap.intern("read-line");
    vm.heap.env_define(root_env, sym, val);

    // write: writes a string to stdout (no newline)
    let val = vm.register_native("io:write", Box::new(|heap, args| {
        let s = match args.first() {
            Some(Value::Object(id)) => match heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("write: expected string".into()),
            },
            _ => return Err("write: expected string".into()),
        };
        print!("{}", s);
        io::stdout().flush().ok();
        Ok(Value::Nil)
    }));
    let sym = vm.heap.intern("write");
    vm.heap.env_define(root_env, sym, val);

    // write-line: writes a string to stdout with newline
    let val = vm.register_native("io:write-line", Box::new(|heap, args| {
        let s = match args.first() {
            Some(Value::Object(id)) => match heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("write-line: expected string".into()),
            },
            _ => return Err("write-line: expected string".into()),
        };
        println!("{}", s);
        Ok(Value::Nil)
    }));
    let sym = vm.heap.intern("write-line");
    vm.heap.env_define(root_env, sym, val);

    // write-err: writes to stderr (for logging without polluting stdout)
    let val = vm.register_native("io:write-err", Box::new(|heap, args| {
        let s = match args.first() {
            Some(Value::Object(id)) => match heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("write-err: expected string".into()),
            },
            _ => return Err("write-err: expected string".into()),
        };
        eprintln!("{}", s);
        Ok(Value::Nil)
    }));
    let sym = vm.heap.intern("write-err");
    vm.heap.env_define(root_env, sym, val);

    // read-file: reads a file to a string
    let val = vm.register_native("io:read-file", Box::new(|heap, args| {
        let path = match args.first() {
            Some(Value::Object(id)) => match heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("read-file: expected string path".into()),
            },
            _ => return Err("read-file: expected string path".into()),
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(heap.alloc_string(&content)),
            Err(e) => Err(format!("read-file: {}", e)),
        }
    }));
    let sym = vm.heap.intern("read-file");
    vm.heap.env_define(root_env, sym, val);

    // write-file: writes a string to a file
    let val = vm.register_native("io:write-file", Box::new(|_heap, args| {
        let path = match args.first() {
            Some(Value::Object(id)) => match _heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("write-file: expected string path".into()),
            },
            _ => return Err("write-file: expected string path".into()),
        };
        let content = match args.get(1) {
            Some(Value::Object(id)) => match _heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("write-file: expected string content".into()),
            },
            _ => return Err("write-file: expected string content".into()),
        };
        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, &content)
            .map_err(|e| format!("write-file: {}", e))?;
        Ok(Value::Nil)
    }));
    let sym = vm.heap.intern("write-file");
    vm.heap.env_define(root_env, sym, val);

    // read: parse a string into an unevaluated AST (no eval, just parse)
    let val = vm.register_native("io:read", Box::new(|heap, args| {
        use crate::reader::lexer::Lexer;
        use crate::reader::parser::Parser;

        let source = match args.first() {
            Some(Value::Object(id)) => match heap.get(*id) {
                HeapObject::MoofString(s) => s.clone(),
                _ => return Err("read: expected string".into()),
            },
            _ => return Err("read: expected string".into()),
        };
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize()
            .map_err(|e| format!("read: lex error: {}", e))?;
        let mut parser = Parser::new(tokens);
        let exprs = parser.parse_all(heap)
            .map_err(|e| format!("read: parse error: {}", e))?;
        // Return single expression or list of expressions
        match exprs.len() {
            0 => Ok(Value::Nil),
            1 => Ok(exprs[0]),
            _ => Ok(heap.list(&exprs)),
        }
    }));
    let sym = vm.heap.intern("read");
    vm.heap.env_define(root_env, sym, val);
}
