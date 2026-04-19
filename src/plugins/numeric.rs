use crate::plugins::{Plugin, native, int_binop, float_binop, float_unary};
use crate::heap::*;
use crate::value::Value;

pub struct NumericPlugin;

impl Plugin for NumericPlugin {
    fn name(&self) -> &str { "numeric" }

    fn register(&self, heap: &mut Heap) {
        // Number proto is already created by CorePlugin and stored in type_protos.
        let number_proto = heap.type_protos[PROTO_NUMBER];

        // -- Integer prototype (parent: Number) --
        let int_proto = heap.make_object(number_proto);
        heap.type_protos[PROTO_INT] = int_proto;
        let int_id = int_proto.as_any_object().unwrap();

        // arithmetic
        int_binop(heap, int_id, "+",  |a, b| Value::integer(a + b));
        int_binop(heap, int_id, "-",  |a, b| Value::integer(a - b));
        int_binop(heap, int_id, "*",  |a, b| Value::integer(a * b));
        int_binop(heap, int_id, "=",  |a, b| Value::boolean(a == b));
        int_binop(heap, int_id, "<",  |a, b| Value::boolean(a < b));
        int_binop(heap, int_id, ">",  |a, b| Value::boolean(a > b));
        int_binop(heap, int_id, "<=", |a, b| Value::boolean(a <= b));
        int_binop(heap, int_id, ">=", |a, b| Value::boolean(a >= b));

        // division and modulo (return Err on zero)
        native(heap, int_id, "/", |heap, receiver, args| {
            let a = receiver.as_integer().ok_or("/: not int")?;
            let b = args.first().and_then(|v| v.as_integer()).ok_or("/: arg not int")?;
            if b == 0 { return Ok(heap.make_error("division by zero")); }
            Ok(Value::integer(a / b))
        });
        native(heap, int_id, "%", |heap, receiver, args| {
            let a = receiver.as_integer().ok_or("%: not int")?;
            let b = args.first().and_then(|v| v.as_integer()).ok_or("%: arg not int")?;
            if b == 0 { return Ok(heap.make_error("modulo by zero")); }
            Ok(Value::integer(a % b))
        });

        // unary
        native(heap, int_id, "negate", |_heap, receiver, _args| {
            Ok(Value::integer(-receiver.as_integer().ok_or("negate: not int")?))
        });
        // describe returns the decimal representation as a String.
        // Object's describe would work via format_value, but installing
        // it directly avoids the heap.format_value round-trip.
        native(heap, int_id, "describe", |heap, receiver, _args| {
            let n = receiver.as_integer().ok_or("describe: not an integer")?;
            Ok(heap.alloc_string(&n.to_string()))
        });

        // hash — integer is its own hash (identity works; Hashable's
        // invariant [a equal: b] ⇒ [a hash] = [b hash] holds trivially).
        native(heap, int_id, "hash", |_heap, receiver, _args| {
            Ok(Value::integer(receiver.as_integer().ok_or("hash: not int")?))
        });

        // bitwise
        int_binop(heap, int_id, "bitAnd:",    |a, b| Value::integer(a & b));
        int_binop(heap, int_id, "bitOr:",     |a, b| Value::integer(a | b));
        int_binop(heap, int_id, "bitXor:",    |a, b| Value::integer(a ^ b));
        native(heap, int_id, "bitNot", |_heap, receiver, _args| {
            let a = receiver.as_integer().ok_or("bitNot: not int")?;
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

        // toFloat
        native(heap, int_id, "toFloat", |_heap, receiver, _args| {
            let a = receiver.as_integer().ok_or("toFloat: not an integer")?;
            Ok(Value::float(a as f64))
        });

        // -- Float prototype (parent: Number) --
        let float_proto = heap.make_object(number_proto);
        heap.type_protos[PROTO_FLOAT] = float_proto;
        let float_id = float_proto.as_any_object().unwrap();

        // arithmetic binops
        float_binop(heap, float_id, "+",  |a, b| Value::float(a + b));
        float_binop(heap, float_id, "-",  |a, b| Value::float(a - b));
        float_binop(heap, float_id, "*",  |a, b| Value::float(a * b));
        float_binop(heap, float_id, "=",  |a, b| Value::boolean(a == b));
        float_binop(heap, float_id, "<",  |a, b| Value::boolean(a < b));
        float_binop(heap, float_id, ">",  |a, b| Value::boolean(a > b));
        float_binop(heap, float_id, "<=", |a, b| Value::boolean(a <= b));
        float_binop(heap, float_id, ">=", |a, b| Value::boolean(a >= b));
        float_binop(heap, float_id, "pow:",  |a, b| Value::float(a.powf(b)));
        float_binop(heap, float_id, "atan2:", |a, b| Value::float(a.atan2(b)));

        // division (manual — zero check)
        native(heap, float_id, "/", |heap, receiver, args| {
            let a = receiver.as_float().ok_or("/ : not float")?;
            let b = args.first().and_then(|v| v.as_float()).ok_or("/ : arg not float")?;
            if b == 0.0 { return Ok(heap.make_error("division by zero")); }
            Ok(Value::float(a / b))
        });

        // unary math
        float_unary(heap, float_id, "negate", |a| Value::float(-a));
        float_unary(heap, float_id, "sqrt",   |a| Value::float(a.sqrt()));
        float_unary(heap, float_id, "floor",  |a| Value::float(a.floor()));
        float_unary(heap, float_id, "ceil",   |a| Value::float(a.ceil()));
        float_unary(heap, float_id, "round",  |a| Value::float(a.round()));
        float_unary(heap, float_id, "sin",    |a| Value::float(a.sin()));
        float_unary(heap, float_id, "cos",    |a| Value::float(a.cos()));
        float_unary(heap, float_id, "tan",    |a| Value::float(a.tan()));
        float_unary(heap, float_id, "asin",   |a| Value::float(a.asin()));
        float_unary(heap, float_id, "acos",   |a| Value::float(a.acos()));
        float_unary(heap, float_id, "atan",   |a| Value::float(a.atan()));
        float_unary(heap, float_id, "log",    |a| Value::float(a.ln()));
        float_unary(heap, float_id, "log10",  |a| Value::float(a.log10()));
        float_unary(heap, float_id, "log2",   |a| Value::float(a.log2()));
        float_unary(heap, float_id, "exp",    |a| Value::float(a.exp()));

        // predicates
        float_unary(heap, float_id, "nan?",      |a| Value::boolean(a.is_nan()));
        float_unary(heap, float_id, "infinite?",  |a| Value::boolean(a.is_infinite()));
        float_unary(heap, float_id, "finite?",    |a| Value::boolean(a.is_finite()));

        // toInteger, describe (manual — need type conversion / heap access)
        native(heap, float_id, "toInteger", |_heap, receiver, _args| {
            Ok(Value::integer(receiver.as_float().ok_or("toInteger: not a float")? as i64))
        });
        native(heap, float_id, "describe", |heap, receiver, _args| {
            Ok(heap.alloc_string(&format!("{}", receiver.as_float().ok_or("describe: not a float")?)))
        });

        // hash — use the bit pattern. NaN values may hash unequally
        // to each other (there are many NaN bit patterns), matching
        // IEEE 754 where NaN != NaN — our values_equal agrees.
        native(heap, float_id, "hash", |_heap, receiver, _args| {
            let f = receiver.as_float().ok_or("hash: not float")?;
            Ok(Value::integer(f.to_bits() as i64))
        });

        // constants
        native(heap, float_id, "pi",       |_heap, _r, _a| Ok(Value::float(std::f64::consts::PI)));
        native(heap, float_id, "e",        |_heap, _r, _a| Ok(Value::float(std::f64::consts::E)));
        native(heap, float_id, "infinity", |_heap, _r, _a| Ok(Value::float(f64::INFINITY)));
        native(heap, float_id, "nan",      |_heap, _r, _a| Ok(Value::float(f64::NAN)));

        // -- register globals --
        let int_sym = heap.intern("Integer");
        heap.env_def(int_sym, int_proto);
        let float_sym = heap.intern("Float");
        heap.env_def(float_sym, float_proto);
    }
}
