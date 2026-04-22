// File-system capability.
//
// fallible operations return the raw value on success and an Err
// on failure. Err short-circuits through then:, so chains handle
// errors naturally:   [[file read: path] then: |s| (process s)]

use moof_core::{Heap, Value, native};
use moof_runtime::{CapabilityPlugin, Vat};

pub struct FileCapability;

fn string_arg(heap: &Heap, args: &[Value], label: &str) -> Result<String, String> {
    let v = args.first().copied().unwrap_or(Value::NIL);
    if let Some(id) = v.as_any_object() {
        if let Some(s) = heap.get_string(id) {
            return Ok(s.to_string());
        }
    }
    Err(format!("{label} must be a String"))
}

impl CapabilityPlugin for FileCapability {
    fn name(&self) -> &str { "file" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj_id = vat.heap.make_object(Value::NIL).as_any_object().unwrap();
        let heap = &mut vat.heap;

        native(heap, obj_id, "read:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            match std::fs::read_to_string(&path) {
                Ok(contents) => Ok(heap.alloc_string(&contents)),
                Err(e) => Ok(heap.make_error(&format!("read {path}: {e}"))),
            }
        });

        native(heap, obj_id, "write:contents:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            let contents = string_arg(heap, &args[1..], "contents")?;
            match std::fs::write(&path, contents) {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("write {path}: {e}"))),
            }
        });

        native(heap, obj_id, "append:contents:", |heap, _recv, args| {
            use std::io::Write;
            let path = string_arg(heap, args, "path")?;
            let contents = string_arg(heap, &args[1..], "contents")?;
            let result = std::fs::OpenOptions::new()
                .create(true).append(true).open(&path)
                .and_then(|mut f| f.write_all(contents.as_bytes()));
            match result {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("append {path}: {e}"))),
            }
        });

        native(heap, obj_id, "exists:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            Ok(Value::boolean(std::path::Path::new(&path).exists()))
        });

        native(heap, obj_id, "isFile:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            Ok(Value::boolean(std::path::Path::new(&path).is_file()))
        });

        native(heap, obj_id, "isDir:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            Ok(Value::boolean(std::path::Path::new(&path).is_dir()))
        });

        native(heap, obj_id, "delete:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            let p = std::path::Path::new(&path);
            let result = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            match result {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("delete {path}: {e}"))),
            }
        });

        native(heap, obj_id, "list:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            match std::fs::read_dir(&path) {
                Ok(iter) => {
                    let mut entries: Vec<String> = iter.flatten()
                        .filter_map(|e| e.file_name().to_str().map(String::from))
                        .collect();
                    entries.sort();
                    let names: Vec<Value> = entries.iter().map(|s| heap.alloc_string(s)).collect();
                    Ok(heap.list(&names))
                }
                Err(e) => Ok(heap.make_error(&format!("list {path}: {e}"))),
            }
        });

        native(heap, obj_id, "mkdir:", |heap, _recv, args| {
            let path = string_arg(heap, args, "path")?;
            match std::fs::create_dir_all(&path) {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("mkdir {path}: {e}"))),
            }
        });

        native(heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<File>"))
        });

        obj_id
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(FileCapability)
}
