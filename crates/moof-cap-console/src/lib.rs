// Console capability — stdin/stdout/stderr.

use moof_core::{Heap, Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

pub struct ConsoleCapability;

/// Pull a String out of the first arg, or return an error explaining why.
fn string_arg(heap: &Heap, args: &[Value], label: &str) -> Result<String, String> {
    let v = args.first().copied().unwrap_or(Value::NIL);
    if let Some(id) = v.as_any_object() {
        if let Some(s) = heap.get_string(id) {
            return Ok(s.to_string());
        }
    }
    Err(format!("{label} must be a String"))
}

impl CapabilityPlugin for ConsoleCapability {
    fn name(&self) -> &str { "console" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj_id = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        native(heap, obj_id, "println:", |heap, _recv, args| {
            let val = args.first().copied().unwrap_or(Value::NIL);
            let s = if let Some(id) = val.as_any_object() {
                if let Some(s) = heap.get_string(id) { s.to_string() }
                else { heap.format_value(val) }
            } else { heap.format_value(val) };
            println!("{s}");
            Ok(Value::NIL)
        });

        native(heap, obj_id, "print:", |heap, _recv, args| {
            let val = args.first().copied().unwrap_or(Value::NIL);
            let s = if let Some(id) = val.as_any_object() {
                if let Some(s) = heap.get_string(id) { s.to_string() }
                else { heap.format_value(val) }
            } else { heap.format_value(val) };
            print!("{s}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            Ok(Value::NIL)
        });

        native(heap, obj_id, "eprintln:", |heap, _recv, args| {
            let s = string_arg(heap, args, "msg")?;
            eprintln!("{s}");
            Ok(Value::NIL)
        });

        native(heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Console>"))
        });

        obj_id
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(ConsoleCapability)
}
