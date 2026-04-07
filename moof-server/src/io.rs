/// IO capability handlers.
///
/// System vats (console, filesystem, clock) are created by the server.
/// This module registers handlers on their interface objects.
/// User vats get references to these objects as capabilities.

use moof_fabric::*;
use crate::Server;
use std::io::{self, BufRead, Write as IoWrite};

/// Register handlers on the server's system vat objects.
pub fn register_system_handlers(server: &mut Server) {
    let mut native = NativeInvoker::new();

    // Read system object ids before borrowing fabric
    let console_obj = server.system.Console;
    let fs_obj = server.system.Filesystem;
    let clock_obj = server.system.Clock;

    register_console(server.fabric(), console_obj, &mut native);
    register_filesystem(server.fabric(), fs_obj, &mut native);
    register_clock(server.fabric(), clock_obj, &mut native);

    // One invoker for all system IO natives
    server.register_invoker(Box::new(native));
}

fn reg(fabric: &mut Fabric, obj: u32, native: &mut NativeInvoker,
       selector: &str, native_name: &str,
       f: impl Fn(&mut Heap, &[Value]) -> Result<Value, String> + Send + 'static) {
    native.register(native_name, Box::new(f));
    let h = NativeInvoker::make_handler(&mut fabric.heap, native_name);
    let s = fabric.intern(selector);
    fabric.heap.add_handler(obj, s, Value::Object(h));
}

fn register_console(fabric: &mut Fabric, obj: u32, native: &mut NativeInvoker) {
    reg(fabric, obj, native, "writeLine:", "Console.writeLine:", |heap, args| {
        let text = value_to_string(heap, args.get(1).copied().unwrap_or(Value::Nil));
        println!("{}", text);
        Ok(Value::Nil)
    });

    reg(fabric, obj, native, "write:", "Console.write:", |heap, args| {
        let text = value_to_string(heap, args.get(1).copied().unwrap_or(Value::Nil));
        print!("{}", text);
        let _ = io::stdout().flush();
        Ok(Value::Nil)
    });

    reg(fabric, obj, native, "readLine", "Console.readLine", |heap, _args| {
        let stdin = io::stdin();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => Ok(Value::Nil),
            Ok(_) => {
                if line.ends_with('\n') { line.pop(); }
                if line.ends_with('\r') { line.pop(); }
                Ok(heap.alloc_string(&line))
            }
            Err(e) => Err(format!("readLine: {}", e)),
        }
    });

    reg(fabric, obj, native, "describe", "Console.describe", |heap, _| {
        Ok(heap.alloc_string("<Console>"))
    });
}

fn register_filesystem(fabric: &mut Fabric, obj: u32, native: &mut NativeInvoker) {
    reg(fabric, obj, native, "read:", "Filesystem.read:", |heap, args| {
        let path = extract_string(heap, args.get(1).copied().unwrap_or(Value::Nil))?;
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("read: {}: {}", path, e))?;
        Ok(heap.alloc_string(&content))
    });

    reg(fabric, obj, native, "write:to:", "Filesystem.write:to:", |heap, args| {
        let content = extract_string(heap, args.get(1).copied().unwrap_or(Value::Nil))?;
        let path = extract_string(heap, args.get(2).copied().unwrap_or(Value::Nil))?;
        std::fs::write(&path, &content).map_err(|e| format!("write: {}: {}", path, e))?;
        Ok(Value::True)
    });

    reg(fabric, obj, native, "describe", "Filesystem.describe", |heap, _| {
        Ok(heap.alloc_string("<Filesystem>"))
    });
}

fn register_clock(fabric: &mut Fabric, obj: u32, native: &mut NativeInvoker) {
    reg(fabric, obj, native, "now", "Clock.now", |_heap, _args| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        Ok(Value::Integer(secs as i64))
    });

    reg(fabric, obj, native, "describe", "Clock.describe", |heap, _| {
        Ok(heap.alloc_string("<Clock>"))
    });
}

fn value_to_string(heap: &Heap, val: Value) -> String {
    match val {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::False => "false".into(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => heap.symbol_name(id).to_string(),
        Value::Object(id) => match heap.get(id) {
            HeapObject::String(s) => s.clone(),
            _ => format!("<object #{}>", id),
        },
    }
}

fn extract_string(heap: &Heap, val: Value) -> Result<String, String> {
    match val {
        Value::Object(id) => match heap.get(id) {
            HeapObject::String(s) => Ok(s.clone()),
            _ => Err("expected string".into()),
        },
        _ => Err("expected string".into()),
    }
}
