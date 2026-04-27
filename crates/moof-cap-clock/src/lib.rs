// Clock capability — wall-clock now/millis + monotonic nanos +
// timer-driven sleep:.
//
// `sleep:` is the substrate for vat-level "wake me later" semantics:
// the cap returns a pending Act and registers a timer with this
// vat's heap. The scheduler's drain loop polls timers across all
// vats; when one's due, the act is resolved (with NIL), which fires
// any continuations chained via `then:`. While the timer is
// pending, the scheduler can run other vats' work — so a moof-side
// `(do (_ <- (sleep 100)) body)` doesn't block; `body` runs as a
// fresh dispatch when the wake fires, and intervening messages
// (e.g. console prints from other vats) interleave naturally.

use moof_core::{Value, native};
use moof_core::heap::TimerEntry;
use moof_runtime::{CapabilityPlugin, Vat};

// Use the scheduler's monotonic clock so the timer wheel and
// `clock monotonic` agree on the same epoch.
use moof_runtime::scheduler::now_monotonic_ns;

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
            Ok(Value::integer(now_monotonic_ns() as i64))
        });

        // sleep: ms — return a pending Act that resolves after `ms`
        // milliseconds. The scheduler's timer wheel handles the
        // wake; the moof-side caller's `then:` continuation fires
        // as a fresh dispatch when ready. Other work runs in
        // between; nothing blocks.
        native(heap, obj_id, "sleep:", |heap, _recv, args| {
            let ms = args.first()
                .and_then(|v| v.as_integer())
                .ok_or("sleep:: ms must be an integer")?;
            let due = now_monotonic_ns()
                .saturating_add((ms.max(0) as u128) * 1_000_000);
            let act = heap.make_pending_act();
            if let Some(act_id) = act.as_any_object() {
                heap.timers.push(TimerEntry { act_id, due_ns: due });
            }
            Ok(act)
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
