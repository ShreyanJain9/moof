// Clock capability — wall-clock now/millis + monotonic nanos.

use moof_core::{Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

pub struct ClockCapability;

impl CapabilityPlugin for ClockCapability {
    fn name(&self) -> &str { "clock" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj_id = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        // wall-clock seconds since UNIX epoch (float)
        native(heap, obj_id, "now", |_heap, _recv, _args| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now().duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_secs_f64();
            Ok(Value::float(secs))
        });

        // wall-clock milliseconds since UNIX epoch (integer)
        native(heap, obj_id, "millis", |_heap, _recv, _args| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now().duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_millis() as i64;
            Ok(Value::integer(ms))
        });

        // monotonic nanos since process start — for measuring durations
        native(heap, obj_id, "monotonic", |_heap, _recv, _args| {
            use std::time::Instant;
            use std::sync::OnceLock;
            static START: OnceLock<Instant> = OnceLock::new();
            let start = START.get_or_init(Instant::now);
            let ns = start.elapsed().as_nanos() as i64;
            Ok(Value::integer(ns))
        });

        native(heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Clock>"))
        });

        obj_id
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(ClockCapability)
}
