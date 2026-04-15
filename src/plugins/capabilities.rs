// Capability plugins: native vats that mediate IO.
//
// Each capability is a vat with native handlers. All access
// goes through FarRef → Act. The capability's Rust code is
// the only thing that touches the outside world.

use crate::scheduler::Vat;
use crate::object::HeapObject;
use crate::value::Value;
use super::CapabilityPlugin;

// ═══════════════════════════════════════════════════════════
// Console — println:, print:
// ═══════════════════════════════════════════════════════════

pub struct ConsoleCapability;

impl CapabilityPlugin for ConsoleCapability {
    fn name(&self) -> &str { "console" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL);
        let obj_id = obj.as_any_object().unwrap();

        // println: — print value + newline
        let sym = vat.heap.intern("println:");
        let h = vat.heap.register_native("println:", |heap, _recv, args| {
            let val = args.first().copied().unwrap_or(Value::NIL);
            if let Some(id) = val.as_any_object() {
                if let HeapObject::Text(s) = heap.get(id) {
                    println!("{s}");
                } else {
                    println!("{}", heap.format_value(val));
                }
            } else {
                println!("{}", heap.format_value(val));
            }
            Ok(Value::NIL)
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // print: — print without newline
        let sym = vat.heap.intern("print:");
        let h = vat.heap.register_native("print:", |heap, _recv, args| {
            let val = args.first().copied().unwrap_or(Value::NIL);
            if let Some(id) = val.as_any_object() {
                if let HeapObject::Text(s) = heap.get(id) {
                    print!("{s}");
                } else {
                    print!("{}", heap.format_value(val));
                }
            } else {
                print!("{}", heap.format_value(val));
            }
            use std::io::Write;
            let _ = std::io::stdout().flush();
            Ok(Value::NIL)
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // describe
        let sym = vat.heap.intern("describe");
        let h = vat.heap.register_native("describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Console>"))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        obj_id
    }
}

// ═══════════════════════════════════════════════════════════
// Clock — now, measure:
// ═══════════════════════════════════════════════════════════

pub struct ClockCapability;

impl CapabilityPlugin for ClockCapability {
    fn name(&self) -> &str { "clock" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL);
        let obj_id = obj.as_any_object().unwrap();

        // now — current time as float (seconds since epoch)
        let sym = vat.heap.intern("now");
        let h = vat.heap.register_native("now", |_heap, _recv, _args| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Ok(Value::float(secs))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // millis — current time as integer milliseconds
        let sym = vat.heap.intern("millis");
        let h = vat.heap.register_native("millis", |_heap, _recv, _args| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            Ok(Value::integer(ms))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // describe
        let sym = vat.heap.intern("describe");
        let h = vat.heap.register_native("describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Clock>"))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        obj_id
    }
}
