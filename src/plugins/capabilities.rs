// Capability plugins: native vats that mediate IO.
//
// Each capability lives in its own vat. All access crosses the
// FarRef → outbox → scheduler boundary, so the capability's heap
// is isolated from the caller's heap. The `native()` helper from
// plugins/mod.rs operates on whatever heap you hand it, so each
// capability passes its own vat.heap. isolation is preserved.

use crate::vat::Vat;
use crate::value::Value;
use super::{CapabilityPlugin, native};

/// Pull a String out of the first arg, or return an error explaining why.
fn string_arg(heap: &crate::heap::Heap, args: &[Value], label: &str) -> Result<String, String> {
    let v = args.first().copied().unwrap_or(Value::NIL);
    if let Some(id) = v.as_any_object() {
        if let Some(s) = heap.get_string(id) {
            return Ok(s.to_string());
        }
    }
    Err(format!("{label} must be a String"))
}

// ═══════════════════════════════════════════════════════════
// Console — println:, print:
// ═══════════════════════════════════════════════════════════

pub struct ConsoleCapability;

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

        native(heap, obj_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Console>"))
        });

        obj_id
    }
}

// ═══════════════════════════════════════════════════════════
// File — read, write, exists, delete, list, mkdir
// ═══════════════════════════════════════════════════════════
//
// fallible operations return the raw value on success and an Err
// on failure. Err short-circuits through then:, so chains handle
// errors naturally:   [[file read: path] then: |s| (process s)]

pub struct FileCapability;

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

// ═══════════════════════════════════════════════════════════
// Random — PRNG (xorshift64, seeded from time)
// ═══════════════════════════════════════════════════════════
//
// not cryptographic. state is a single u64 in an i64 slot on the
// capability object (bit-reinterpreted). advance_seed reads, steps,
// and writes the seed, returning the new value.

fn xorshift64_step(x: u64) -> u64 {
    let mut x = x;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// Advance the capability's seed slot, returning the new state.
fn advance_seed(heap: &mut crate::heap::Heap, obj_id: u32) -> u64 {
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

        // seed from spawn-time nanos (nonzero required by xorshift)
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64).unwrap_or(1);
        let seed = if nanos == 0 { 1 } else { nanos };
        let seed_sym = heap.intern("seed");
        heap.get_mut(obj_id).slot_set(seed_sym, Value::integer(seed as i64));

        native(heap, obj_id, "next", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("next: not an object")?;
            let next = advance_seed(heap, id);
            // top 53 bits → [0, 1) with full mantissa precision
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

// ═══════════════════════════════════════════════════════════
// Clock — now, millis, monotonic
// ═══════════════════════════════════════════════════════════

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
