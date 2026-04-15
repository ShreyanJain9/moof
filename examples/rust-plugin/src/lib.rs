// Example moof plugin in Rust: an atomic counter capability.
//
// Build:  cargo build --release
// Load:   (load-plugin "target/release/libmoof_counter_plugin.dylib")
// Use:    [counter next]       → Act<1>, Act<2>, Act<3>, ...
//         [counter peek]       → Act<current-value>
//         [counter reset]      → Act<0>

use std::sync::atomic::{AtomicI64, Ordering};

use moof::{CapabilityPlugin, Vat, Value};
use moof::plugins::native;

static COUNTER: AtomicI64 = AtomicI64::new(0);

struct CounterPlugin;

impl CapabilityPlugin for CounterPlugin {
    fn name(&self) -> &str { "counter" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL);
        let obj_id = obj.as_any_object().unwrap();

        native(&mut vat.heap, obj_id, "next", |_heap, _recv, _args| {
            Ok(Value::integer(COUNTER.fetch_add(1, Ordering::SeqCst) + 1))
        });

        native(&mut vat.heap, obj_id, "peek", |_heap, _recv, _args| {
            Ok(Value::integer(COUNTER.load(Ordering::SeqCst)))
        });

        native(&mut vat.heap, obj_id, "reset", |_heap, _recv, _args| {
            COUNTER.store(0, Ordering::SeqCst);
            Ok(Value::integer(0))
        });

        native(&mut vat.heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Counter>"))
        });

        obj_id
    }
}

/// Entry point for dynamic loading. Moof calls this to get the plugin.
#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(CounterPlugin)
}
