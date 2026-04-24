use moof_core::{Plugin, native, float_binop, float_unary};
use moof_core::heap::*;
use moof_core::value::Value;

pub struct NumericPlugin;

// -- integer ops that participate in Integer = i48 ∪ BigInt --
//
// Every arithmetic handler goes through to_bigint so primitive and
// foreign operands mix freely, then through alloc_integer so a
// small result demotes back to i48. Users see one Integer type;
// the i48 fast path is just a representation optimization.

fn int_arith<F>(heap: &mut Heap, sel: &str, op: F)
where F: Fn(&num_bigint::BigInt, &num_bigint::BigInt) -> num_bigint::BigInt + 'static,
{
    let name = sel.to_string();
    let int_id = heap.type_protos[PROTO_INT].as_any_object().unwrap();
    native(heap, int_id, sel, move |heap, receiver, args| {
        let a = heap.to_bigint(receiver).ok_or_else(|| format!("{name}: receiver not an integer"))?;
        let b_val = args.first().copied().unwrap_or(Value::NIL);
        let b = heap.to_bigint(b_val).ok_or_else(|| format!("{name}: arg not an integer"))?;
        Ok(heap.alloc_integer(op(&a, &b)))
    });
}

fn int_cmp<F>(heap: &mut Heap, sel: &str, op: F)
where F: Fn(&num_bigint::BigInt, &num_bigint::BigInt) -> bool + 'static,
{
    let name = sel.to_string();
    let int_id = heap.type_protos[PROTO_INT].as_any_object().unwrap();
    native(heap, int_id, sel, move |heap, receiver, args| {
        let a = heap.to_bigint(receiver).ok_or_else(|| format!("{name}: receiver not an integer"))?;
        let b_val = args.first().copied().unwrap_or(Value::NIL);
        let b = heap.to_bigint(b_val).ok_or_else(|| format!("{name}: arg not an integer"))?;
        Ok(Value::boolean(op(&a, &b)))
    });
}

// -- bitwise / shift helpers: i64 semantics for now. BigInt values
//    get passed through num-bigint which natively supports them; a
//    too-big shift count errors rather than allocating a gigabyte. --

fn shift_count(v: Value) -> Option<u32> {
    v.as_integer().and_then(|n| u32::try_from(n).ok())
}

impl Plugin for NumericPlugin {
    fn name(&self) -> &str { "numeric" }

    fn register(&self, heap: &mut Heap) {
        // Number proto is already created by CorePlugin and stored in type_protos.
        let number_proto = heap.type_protos[PROTO_NUMBER];

        // -- Integer prototype (parent: Number) --
        let int_proto = heap.make_object(number_proto);
        heap.type_protos[PROTO_INT] = int_proto;

        // Wire BigInt's foreign proto so `alloc_integer` hands back
        // objects whose dispatch target is THIS Integer proto. The
        // foreign type was registered during Heap::new but didn't
        // have an Integer proto to point at yet.
        //
        // (alloc_integer reads type_protos[PROTO_INT] at call time,
        // so no extra wiring is needed — this comment documents the
        // invariant for anyone reading the plugin.)

        // arithmetic — overflow-safe, BigInt-aware, always demoted
        int_arith(heap, "+",  |a, b| a + b);
        int_arith(heap, "-",  |a, b| a - b);
        int_arith(heap, "*",  |a, b| a * b);

        // comparison — works across i48/BigInt boundaries
        int_cmp(heap, "=",  |a, b| a == b);
        int_cmp(heap, "<",  |a, b| a < b);
        int_cmp(heap, ">",  |a, b| a > b);
        int_cmp(heap, "<=", |a, b| a <= b);
        int_cmp(heap, ">=", |a, b| a >= b);

        let int_id = heap.type_protos[PROTO_INT].as_any_object().unwrap();

        // division and modulo — error on zero. num-bigint uses truncated
        // division by default; match that for i48 too (Rust's i64 / %).
        native(heap, int_id, "/", |heap, receiver, args| {
            let a = heap.to_bigint(receiver).ok_or("/ : receiver not an integer")?;
            let b_val = args.first().copied().unwrap_or(Value::NIL);
            let b = heap.to_bigint(b_val).ok_or("/ : arg not an integer")?;
            if b.sign() == num_bigint::Sign::NoSign {
                return Ok(heap.make_error("division by zero"));
            }
            Ok(heap.alloc_integer(a / b))
        });
        native(heap, int_id, "%", |heap, receiver, args| {
            let a = heap.to_bigint(receiver).ok_or("% : receiver not an integer")?;
            let b_val = args.first().copied().unwrap_or(Value::NIL);
            let b = heap.to_bigint(b_val).ok_or("% : arg not an integer")?;
            if b.sign() == num_bigint::Sign::NoSign {
                return Ok(heap.make_error("modulo by zero"));
            }
            Ok(heap.alloc_integer(a % b))
        });

        // unary
        native(heap, int_id, "negate", |heap, receiver, _args| {
            let a = heap.to_bigint(receiver).ok_or("negate: not an integer")?;
            Ok(heap.alloc_integer(-a))
        });

        // describe — decimal form regardless of backing.
        native(heap, int_id, "describe", |heap, receiver, _args| {
            let a = heap.to_bigint(receiver).ok_or("describe: not an integer")?;
            Ok(heap.alloc_string(&a.to_string()))
        });

        // hash — FNV over the decimal bytes; this gives i48 and BigInt
        // representations of the same numeric value the SAME hash
        // (Integer 5 hashes like BigInt 5), so equal ints are equal
        // keys regardless of storage.
        native(heap, int_id, "hash", |heap, receiver, _args| {
            let a = heap.to_bigint(receiver).ok_or("hash: not an integer")?;
            let bytes = a.to_signed_bytes_be();
            Ok(Value::integer(moof_core::fnv1a_64(&bytes) as i64))
        });

        // bitwise — num-bigint's & | ^ handle arbitrary widths.
        int_arith(heap, "bitAnd:", |a, b| a & b);
        int_arith(heap, "bitOr:",  |a, b| a | b);
        int_arith(heap, "bitXor:", |a, b| a ^ b);
        native(heap, int_id, "bitNot", |heap, receiver, _args| {
            let a = heap.to_bigint(receiver).ok_or("bitNot: not an integer")?;
            Ok(heap.alloc_integer(!a))
        });
        native(heap, int_id, "shiftLeft:", |heap, receiver, args| {
            let a = heap.to_bigint(receiver).ok_or("shiftLeft: receiver not an integer")?;
            let n = shift_count(args.first().copied().unwrap_or(Value::NIL))
                .ok_or("shiftLeft: count must be a non-negative i48")?;
            Ok(heap.alloc_integer(a << n))
        });
        native(heap, int_id, "shiftRight:", |heap, receiver, args| {
            let a = heap.to_bigint(receiver).ok_or("shiftRight: receiver not an integer")?;
            let n = shift_count(args.first().copied().unwrap_or(Value::NIL))
                .ok_or("shiftRight: count must be a non-negative i48")?;
            Ok(heap.alloc_integer(a >> n))
        });

        // toFloat — BigInt -> f64 is lossy for large magnitudes; we
        // just convert, letting the result become infinity if it
        // exceeds f64's range, matching IEEE behavior.
        native(heap, int_id, "toFloat", |heap, receiver, _args| {
            use num_traits::ToPrimitive;
            let a = heap.to_bigint(receiver).ok_or("toFloat: not an integer")?;
            Ok(Value::float(a.to_f64().unwrap_or(f64::INFINITY)))
        });

        // typeName — same `'Integer` for both i48 and BigInt backings,
        // since they share this proto.
        native(heap, int_id, "typeName", |heap, _r, _a| {
            let s = heap.intern("Integer");
            Ok(Value::symbol(s))
        });

        // -- Float prototype (parent: Number) --
        let float_proto = heap.make_object(number_proto);
        heap.type_protos[PROTO_FLOAT] = float_proto;
        let float_id = float_proto.as_any_object().unwrap();

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

        native(heap, float_id, "/", |heap, receiver, args| {
            let a = receiver.as_float().ok_or("/ : not float")?;
            let b = args.first().and_then(|v| v.as_float()).ok_or("/ : arg not float")?;
            if b == 0.0 { return Ok(heap.make_error("division by zero")); }
            Ok(Value::float(a / b))
        });

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

        float_unary(heap, float_id, "nan?",       |a| Value::boolean(a.is_nan()));
        float_unary(heap, float_id, "infinite?",  |a| Value::boolean(a.is_infinite()));
        float_unary(heap, float_id, "finite?",    |a| Value::boolean(a.is_finite()));

        // float.toInteger — any result fits a BigInt, so no overflow
        // worries even for huge floats. i48 fast path where possible.
        native(heap, float_id, "toInteger", |heap, receiver, _args| {
            use num_traits::FromPrimitive;
            let f = receiver.as_float().ok_or("toInteger: not a float")?;
            if !f.is_finite() { return Err("toInteger: not finite".into()); }
            let b = num_bigint::BigInt::from_f64(f.trunc())
                .ok_or("toInteger: conversion failed")?;
            Ok(heap.alloc_integer(b))
        });
        native(heap, float_id, "describe", |heap, receiver, _args| {
            Ok(heap.alloc_string(&format!("{}", receiver.as_float().ok_or("describe: not a float")?)))
        });

        native(heap, float_id, "hash", |_heap, receiver, _args| {
            let bits = receiver.to_bits().to_le_bytes();
            Ok(Value::integer(moof_core::fnv1a_64(&bits) as i64))
        });

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

/// Entry point for dylib loading. moof-cli's manifest loader
/// calls this via `libloading` when a `[types]` entry points
/// at this crate's cdylib.
#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn moof_core::Plugin> {
    Box::new(NumericPlugin)
}
