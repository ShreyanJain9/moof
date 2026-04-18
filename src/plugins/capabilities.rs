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
// File — read, write, exists, delete, list, mkdir
// ═══════════════════════════════════════════════════════════
//
// fallible operations return the raw value on success and an Err
// on failure. Err short-circuits through then:, so chains handle
// errors naturally without explicit match:
//   [[file read: path] then: |s| (process s)]
// and `do` unwraps successful Results the same way.
// errors are moof values, not panics. the vat survives.

pub struct FileCapability;

/// Pull a path String from the first arg, or return a descriptive error.
fn path_arg(heap: &crate::heap::Heap, args: &[Value]) -> Result<String, String> {
    let v = args.first().copied().unwrap_or(Value::NIL);
    if let Some(id) = v.as_any_object() {
        if let HeapObject::Text(s) = heap.get(id) {
            return Ok(s.clone());
        }
    }
    Err("path must be a String".into())
}

impl CapabilityPlugin for FileCapability {
    fn name(&self) -> &str { "file" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL);
        let obj_id = obj.as_any_object().unwrap();

        // read: path → Ok(String) | Err
        let sym = vat.heap.intern("read:");
        let h = vat.heap.register_native("read:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            match std::fs::read_to_string(&path) {
                Ok(contents) => Ok(heap.alloc_string(&contents)),
                Err(e) => Ok(heap.make_error(&format!("read {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // write:contents: path str → Ok(nil) | Err
        let sym = vat.heap.intern("write:contents:");
        let h = vat.heap.register_native("write:contents:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            let contents_v = args.get(1).copied().unwrap_or(Value::NIL);
            let contents = if let Some(id) = contents_v.as_any_object() {
                if let HeapObject::Text(s) = heap.get(id) { s.clone() }
                else { return Err("contents must be a String".into()); }
            } else { return Err("contents must be a String".into()); };
            match std::fs::write(&path, contents) {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("write {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // append:contents: path str → Ok(nil) | Err
        let sym = vat.heap.intern("append:contents:");
        let h = vat.heap.register_native("append:contents:", |heap, _recv, args| {
            use std::io::Write;
            let path = path_arg(heap, args)?;
            let contents_v = args.get(1).copied().unwrap_or(Value::NIL);
            let contents = if let Some(id) = contents_v.as_any_object() {
                if let HeapObject::Text(s) = heap.get(id) { s.clone() }
                else { return Err("contents must be a String".into()); }
            } else { return Err("contents must be a String".into()); };
            let result = std::fs::OpenOptions::new()
                .create(true).append(true).open(&path)
                .and_then(|mut f| f.write_all(contents.as_bytes()));
            match result {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("append {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // exists: path → Boolean
        let sym = vat.heap.intern("exists:");
        let h = vat.heap.register_native("exists:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            Ok(Value::boolean(std::path::Path::new(&path).exists()))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // isFile: path → Boolean
        let sym = vat.heap.intern("isFile:");
        let h = vat.heap.register_native("isFile:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            Ok(Value::boolean(std::path::Path::new(&path).is_file()))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // isDir: path → Boolean
        let sym = vat.heap.intern("isDir:");
        let h = vat.heap.register_native("isDir:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            Ok(Value::boolean(std::path::Path::new(&path).is_dir()))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // delete: path → Ok(nil) | Err
        let sym = vat.heap.intern("delete:");
        let h = vat.heap.register_native("delete:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            let p = std::path::Path::new(&path);
            let result = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            match result {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("delete {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // list: path → Ok(list of entry names) | Err
        let sym = vat.heap.intern("list:");
        let h = vat.heap.register_native("list:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            match std::fs::read_dir(&path) {
                Ok(iter) => {
                    let mut names: Vec<Value> = Vec::new();
                    for entry in iter.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            names.push(heap.alloc_string(name));
                        }
                    }
                    names.sort_by(|a, b| {
                        let aid = a.as_any_object().unwrap();
                        let bid = b.as_any_object().unwrap();
                        let astr = heap.get_string(aid).unwrap_or("");
                        let bstr = heap.get_string(bid).unwrap_or("");
                        astr.cmp(bstr)
                    });
                    Ok(heap.list(&names))
                }
                Err(e) => Ok(heap.make_error(&format!("list {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // mkdir: path → Ok(nil) | Err (creates parents too)
        let sym = vat.heap.intern("mkdir:");
        let h = vat.heap.register_native("mkdir:", |heap, _recv, args| {
            let path = path_arg(heap, args)?;
            match std::fs::create_dir_all(&path) {
                Ok(()) => Ok(Value::NIL),
                Err(e) => Ok(heap.make_error(&format!("mkdir {}: {}", path, e))),
            }
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        let sym = vat.heap.intern("describe");
        let h = vat.heap.register_native("describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<File>"))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        obj_id
    }
}

// ═══════════════════════════════════════════════════════════
// Random — PRNG (xorshift64, seeded from time)
// ═══════════════════════════════════════════════════════════
//
// not cryptographic. fine for shuffling, sampling, procedural
// generation. seed: sets state explicitly; default seed from
// clock on capability spawn. state is a single u64 stored in
// an i64 slot on the capability object (bit-reinterpreted).

fn xorshift64_step(x: u64) -> u64 {
    let mut x = x;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

pub struct RandomCapability;

impl CapabilityPlugin for RandomCapability {
    fn name(&self) -> &str { "random" }

    fn setup(&self, vat: &mut Vat) -> u32 {
        let obj = vat.heap.make_object(Value::NIL);
        let obj_id = obj.as_any_object().unwrap();

        // seed from current time nanoseconds (nonzero required by xorshift)
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64).unwrap_or(1);
        let seed = if nanos == 0 { 1 } else { nanos };
        let seed_sym = vat.heap.intern("seed");
        vat.heap.get_mut(obj_id).slot_set(seed_sym, Value::integer(seed as i64));

        // next → Float in [0, 1)
        let sym = vat.heap.intern("next");
        let h = vat.heap.register_native("next", |heap, receiver, _args| {
            let id = receiver.as_any_object().ok_or("next: not an object")?;
            let seed_sym = heap.intern("seed");
            let cur = heap.get(id).slot_get(seed_sym)
                .and_then(|v| v.as_integer()).unwrap_or(1) as u64;
            let next = xorshift64_step(if cur == 0 { 1 } else { cur });
            heap.get_mut(id).slot_set(seed_sym, Value::integer(next as i64));
            // map u64 to [0, 1) by taking top 53 bits (mantissa precision)
            let f = (next >> 11) as f64 / ((1u64 << 53) as f64);
            Ok(Value::float(f))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // integer: max → Integer in [0, max)
        let sym = vat.heap.intern("integer:");
        let h = vat.heap.register_native("integer:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("integer:: not an object")?;
            let max = args.first().and_then(|v| v.as_integer())
                .ok_or("integer:: arg must be Integer")?;
            if max <= 0 { return Ok(heap.make_error("integer:: max must be positive")); }
            let seed_sym = heap.intern("seed");
            let cur = heap.get(id).slot_get(seed_sym)
                .and_then(|v| v.as_integer()).unwrap_or(1) as u64;
            let next = xorshift64_step(if cur == 0 { 1 } else { cur });
            heap.get_mut(id).slot_set(seed_sym, Value::integer(next as i64));
            Ok(Value::integer((next % max as u64) as i64))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // between:and: lo hi → Integer in [lo, hi] inclusive
        let sym = vat.heap.intern("between:and:");
        let h = vat.heap.register_native("between:and:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("between:and:: not an object")?;
            let lo = args.first().and_then(|v| v.as_integer())
                .ok_or("between:and:: lo not Integer")?;
            let hi = args.get(1).and_then(|v| v.as_integer())
                .ok_or("between:and:: hi not Integer")?;
            if hi < lo { return Ok(heap.make_error("between:and:: hi < lo")); }
            let seed_sym = heap.intern("seed");
            let cur = heap.get(id).slot_get(seed_sym)
                .and_then(|v| v.as_integer()).unwrap_or(1) as u64;
            let next = xorshift64_step(if cur == 0 { 1 } else { cur });
            heap.get_mut(id).slot_set(seed_sym, Value::integer(next as i64));
            let range = (hi - lo + 1) as u64;
            Ok(Value::integer(lo + (next % range) as i64))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        // seed: n — set the PRNG state explicitly (non-zero)
        let sym = vat.heap.intern("seed:");
        let h = vat.heap.register_native("seed:", |heap, receiver, args| {
            let id = receiver.as_any_object().ok_or("seed:: not an object")?;
            let s = args.first().and_then(|v| v.as_integer())
                .ok_or("seed:: arg must be Integer")?;
            let s = if s == 0 { 1 } else { s };
            let seed_sym = heap.intern("seed");
            heap.get_mut(id).slot_set(seed_sym, Value::integer(s));
            Ok(receiver)
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        let sym = vat.heap.intern("describe");
        let h = vat.heap.register_native("describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Random>"))
        });
        vat.heap.get_mut(obj_id).handler_set(sym, h);

        obj_id
    }
}

// ═══════════════════════════════════════════════════════════
// Clock — now, millis, monotonic, measure:
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

        // monotonic — nanoseconds from a monotonic clock source.
        // good for measuring durations; wall-clock jumps don't affect it.
        let sym = vat.heap.intern("monotonic");
        let h = vat.heap.register_native("monotonic", |_heap, _recv, _args| {
            use std::time::Instant;
            // Instant::now() is monotonic; we convert to nanos since
            // some arbitrary fixed start point via duration_since(UNIX_EPOCH)
            // — but Instant doesn't have that. use a static reference instead.
            use std::sync::OnceLock;
            static START: OnceLock<Instant> = OnceLock::new();
            let start = START.get_or_init(Instant::now);
            let ns = start.elapsed().as_nanos() as i64;
            Ok(Value::integer(ns))
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
