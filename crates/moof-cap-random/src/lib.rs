// Random capability — xorshift64 PRNG, seeded from spawn-time nanos.
// Not cryptographic. State is a single u64 in an i64 slot on the
// capability object (bit-reinterpreted via Value).

use moof_core::{Heap, Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

fn xorshift64_step(x: u64) -> u64 {
    let mut x = x;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn advance_seed(heap: &mut Heap, obj_id: u32) -> u64 {
    let seed_sym = heap.intern("seed");
    let cur = heap.get(obj_id).slot_get(seed_sym)
        .and_then(|v| v.as_integer()).unwrap_or(1) as u64;
    let next = xorshift64_step(if cur == 0 { 1 } else { cur });
    heap.get_mut(obj_id).slot_set(seed_sym, Value::integer(next as i64));
    next
}

pub struct RandomCapability;

impl CapabilityPlugin for RandomCapability {
    fn name(&self) -> &str { "random" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj_id = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64).unwrap_or(1);
        let seed = if nanos == 0 { 1 } else { nanos };
        let seed_sym = heap.intern("seed");
        heap.get_mut(obj_id).slot_set(seed_sym, Value::integer(seed as i64));

        native(heap, obj_id, "next", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("next: not an object")?;
            let next = advance_seed(heap, id);
            let f = (next >> 11) as f64 / ((1u64 << 53) as f64);
            Ok(Value::float(f))
        });

        native(heap, obj_id, "integer:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("integer:: not an object")?;
            let max = args.first().and_then(|v| v.as_integer())
                .ok_or("integer:: arg must be Integer")?;
            if max <= 0 { return Ok(heap.make_error("integer:: max must be positive")); }
            let next = advance_seed(heap, id);
            Ok(Value::integer((next % max as u64) as i64))
        });

        native(heap, obj_id, "between:and:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("between:and:: not an object")?;
            let lo = args.first().and_then(|v| v.as_integer())
                .ok_or("between:and:: lo not Integer")?;
            let hi = args.get(1).and_then(|v| v.as_integer())
                .ok_or("between:and:: hi not Integer")?;
            if hi < lo { return Ok(heap.make_error("between:and:: hi < lo")); }
            let next = advance_seed(heap, id);
            let range = (hi - lo + 1) as u64;
            Ok(Value::integer(lo + (next % range) as i64))
        });

        native(heap, obj_id, "seed:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("seed:: not an object")?;
            let s = args.first().and_then(|v| v.as_integer())
                .ok_or("seed:: arg must be Integer")?;
            let s = if s == 0 { 1 } else { s };
            let seed_sym = heap.intern("seed");
            heap.get_mut(id).slot_set(seed_sym, Value::integer(s));
            Ok(receiver)
        });

        native(heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Random>"))
        });

        obj_id
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(RandomCapability)
}
