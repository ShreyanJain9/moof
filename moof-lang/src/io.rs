/// IO capability objects.
///
/// IO is not a global function. It's a capability object your vat holds.
/// [console writeLine: "hello"]. If you don't have console, you can't print.
///
/// Capabilities:
/// - Console: writeLine:, write:, readLine
/// - Filesystem: read:, write:to:
/// - Clock: now

use moof_fabric::*;
use std::io::{self, BufRead, Write as IoWrite};

/// Create IO capability objects and register their handlers.
/// Returns (console_id, fs_id, clock_id) as object ids.
pub fn create_capabilities(fabric: &mut Fabric, native: &mut NativeInvoker) -> IoCapabilities {
    let console = create_console(fabric, native);
    let fs = create_filesystem(fabric, native);
    let clock = create_clock(fabric, native);
    IoCapabilities { console, filesystem: fs, clock }
}

pub struct IoCapabilities {
    pub console: u32,
    pub filesystem: u32,
    pub clock: u32,
}

fn create_console(fabric: &mut Fabric, native: &mut NativeInvoker) -> u32 {
    let obj = fabric.create_object(Value::Nil);

    // writeLine: — print string + newline to stdout
    native.register("Console.writeLine:", Box::new(|heap, args| {
        let text = value_to_string(heap, args.get(1).copied().unwrap_or(Value::Nil));
        println!("{}", text);
        Ok(Value::Nil)
    }));
    fabric.add_native_handler(obj, "writeLine:", "Console.writeLine:");

    // write: — print string without newline
    native.register("Console.write:", Box::new(|heap, args| {
        let text = value_to_string(heap, args.get(1).copied().unwrap_or(Value::Nil));
        print!("{}", text);
        let _ = io::stdout().flush();
        Ok(Value::Nil)
    }));
    fabric.add_native_handler(obj, "write:", "Console.write:");

    // readLine — read a line from stdin
    native.register("Console.readLine", Box::new(|heap, _args| {
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
    }));
    fabric.add_native_handler(obj, "readLine", "Console.readLine");

    // describe
    native.register("Console.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<Console>"))
    }));
    fabric.add_native_handler(obj, "describe", "Console.describe");

    obj
}

fn create_filesystem(fabric: &mut Fabric, native: &mut NativeInvoker) -> u32 {
    let obj = fabric.create_object(Value::Nil);

    // read: — read file contents as string
    native.register("Filesystem.read:", Box::new(|heap, args| {
        let path = match args.get(1).copied().unwrap_or(Value::Nil) {
            Value::Object(id) => match heap.get(id) {
                HeapObject::String(s) => s.clone(),
                _ => return Err("read: expects string path".into()),
            },
            _ => return Err("read: expects string path".into()),
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(heap.alloc_string(&content)),
            Err(e) => Err(format!("read: {}: {}", path, e)),
        }
    }));
    fabric.add_native_handler(obj, "read:", "Filesystem.read:");

    // write:to: — write string to file
    native.register("Filesystem.write:to:", Box::new(|heap, args| {
        let content = match args.get(1).copied().unwrap_or(Value::Nil) {
            Value::Object(id) => match heap.get(id) {
                HeapObject::String(s) => s.clone(),
                _ => return Err("write:to: expects string content".into()),
            },
            _ => return Err("write:to: expects string content".into()),
        };
        let path = match args.get(2).copied().unwrap_or(Value::Nil) {
            Value::Object(id) => match heap.get(id) {
                HeapObject::String(s) => s.clone(),
                _ => return Err("write:to: expects string path".into()),
            },
            _ => return Err("write:to: expects string path".into()),
        };
        std::fs::write(&path, &content)
            .map_err(|e| format!("write:to: {}: {}", path, e))?;
        Ok(Value::True)
    }));
    fabric.add_native_handler(obj, "write:to:", "Filesystem.write:to:");

    // describe
    native.register("Filesystem.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<Filesystem>"))
    }));
    fabric.add_native_handler(obj, "describe", "Filesystem.describe");

    obj
}

fn create_clock(fabric: &mut Fabric, native: &mut NativeInvoker) -> u32 {
    let obj = fabric.create_object(Value::Nil);

    // now — current unix timestamp as integer
    native.register("Clock.now", Box::new(|_heap, _args| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Ok(Value::Integer(secs as i64))
    }));
    fabric.add_native_handler(obj, "now", "Clock.now");

    native.register("Clock.describe", Box::new(|heap, _args| {
        Ok(heap.alloc_string("<Clock>"))
    }));
    fabric.add_native_handler(obj, "describe", "Clock.describe");

    obj
}

/// Convert any value to a printable string.
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
