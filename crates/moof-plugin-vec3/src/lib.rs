// Vec3 — smoke-test foreign type for wave 5.0.
//
// Purely immutable 3-component float vector. Demonstrates the
// whole foreign-type pipeline end to end:
//   - rust-side type registration via ForeignType impl
//   - virtual slots (.x, .y, .z — not stored as slots, computed
//     on demand from the rust payload)
//   - handler methods (add:, scale:, dot:, magnitude) that return
//     new Vec3 values — no mutation
//   - serialization (24 bytes, fixed-layout)
//   - cross-vat copy
//   - image round-trip
//
// The handlers do `foreign_clone::<Vec3>(receiver)` then alloc a
// new Vec3 — this is the canonical pattern for borrowing + re-
// allocating in the same &mut Heap call.

use moof_core::foreign::ForeignType;
use moof_core::heap::Heap;
use moof_core::value::Value;
use moof_core::{Plugin, native};

#[derive(Clone, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl ForeignType for Vec3 {
    fn type_name() -> &'static str { "moof.Vec3" }
    fn prototype_name() -> &'static str { "Vec3" }
    fn schema_version() -> u32 { 1 }

    fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
        out
    }

    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 24 { return Err(format!("Vec3: expected 24 bytes, got {}", bytes.len())); }
        let read = |slice: &[u8]| -> f64 {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(slice);
            f64::from_le_bytes(arr)
        };
        Ok(Vec3 {
            x: read(&bytes[0..8]),
            y: read(&bytes[8..16]),
            z: read(&bytes[16..24]),
        })
    }

    fn equal(&self, other: &Self) -> bool { self == other }

    fn describe(&self) -> String {
        format!("Vec3({}, {}, {})", self.x, self.y, self.z)
    }

    fn virtual_slot(&self, sym: u32) -> Option<Value> {
        // Slot names are symbols; we look them up on the fly via
        // a thread-local? No — we can't access the heap from here.
        // So virtual_slot_names returns the symbol IDs at the time
        // they're queried, and we use a static mapping keyed by
        // the names we registered in the plugin setup. But that's
        // brittle. Instead the plugin bootstrap populates a per-
        // type symbol cache (passed through ForeignType some other
        // way) — for Vec3 we just match against the name string
        // via the intern-reverse path held in the heap.
        //
        // Simpler route: virtual_slot takes &self and we compare
        // against symbol IDs burned in at plugin-registration time,
        // stashed in thread_local/static. For wave 5.0 smoke test
        // we go pragmatic and do the name-match through globals
        // initialized when the Vec3 plugin registers.
        let syms = VEC3_SLOT_SYMS.load();
        if sym == syms.x { Some(Value::float(self.x)) }
        else if sym == syms.y { Some(Value::float(self.y)) }
        else if sym == syms.z { Some(Value::float(self.z)) }
        else { None }
    }

    fn virtual_slot_names(&self) -> Vec<u32> {
        let syms = VEC3_SLOT_SYMS.load();
        vec![syms.x, syms.y, syms.z]
    }
}

impl Vec3 {
    pub fn add(&self, other: &Vec3) -> Vec3 {
        Vec3 { x: self.x + other.x, y: self.y + other.y, z: self.z + other.z }
    }
    pub fn scale(&self, k: f64) -> Vec3 {
        Vec3 { x: self.x * k, y: self.y * k, z: self.z * k }
    }
    pub fn dot(&self, other: &Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
    pub fn magnitude(&self) -> f64 {
        self.dot(self).sqrt()
    }
}

// A tiny global slot-symbol cache. The symbol IDs for x/y/z are
// interned once at plugin registration and read lock-free here.
// Value 0 means "not yet registered" — virtual_slot returns None.
#[derive(Clone, Copy)]
struct Vec3SlotSyms { x: u32, y: u32, z: u32 }

struct AtomicVec3Syms {
    x: std::sync::atomic::AtomicU32,
    y: std::sync::atomic::AtomicU32,
    z: std::sync::atomic::AtomicU32,
}
impl AtomicVec3Syms {
    const fn new() -> Self {
        AtomicVec3Syms {
            x: std::sync::atomic::AtomicU32::new(0),
            y: std::sync::atomic::AtomicU32::new(0),
            z: std::sync::atomic::AtomicU32::new(0),
        }
    }
    fn load(&self) -> Vec3SlotSyms {
        use std::sync::atomic::Ordering::Relaxed;
        Vec3SlotSyms {
            x: self.x.load(Relaxed),
            y: self.y.load(Relaxed),
            z: self.z.load(Relaxed),
        }
    }
    fn store(&self, s: Vec3SlotSyms) {
        use std::sync::atomic::Ordering::Relaxed;
        self.x.store(s.x, Relaxed);
        self.y.store(s.y, Relaxed);
        self.z.store(s.z, Relaxed);
    }
}
static VEC3_SLOT_SYMS: AtomicVec3Syms = AtomicVec3Syms::new();

pub struct Vec3Plugin;

impl Plugin for Vec3Plugin {
    fn name(&self) -> &str { "vec3" }

    fn register(&self, heap: &mut Heap) {
        // Cache virtual slot symbols for zero-heap-lookup reads.
        let sx = heap.intern("x");
        let sy = heap.intern("y");
        let sz = heap.intern("z");
        VEC3_SLOT_SYMS.store(Vec3SlotSyms { x: sx, y: sy, z: sz });

        // Register the type, create its proto, install typeName,
        // and bind `Vec3` in the root env — all in one call.
        let proto = moof_core::register_foreign_proto::<Vec3>(heap);
        let proto_id = proto.as_any_object().unwrap();

        // Class-side constructor: (Vec3 new: x y: y z: z)
        // We bind this as a handler on the prototype itself.
        native(heap, proto_id, "new:y:z:", |heap, _receiver, args| {
            let x = args.first().and_then(|v| v.as_float()).ok_or("Vec3 new:y:z: x must be float")?;
            let y = args.get(1).and_then(|v| v.as_float()).ok_or("Vec3 new:y:z: y must be float")?;
            let z = args.get(2).and_then(|v| v.as_float()).ok_or("Vec3 new:y:z: z must be float")?;
            let proto = heap.lookup_type("Vec3");
            heap.alloc_foreign(proto, Vec3 { x, y, z })
        });

        // Instance handlers.
        native(heap, proto_id, "add:", |heap, receiver, args| {
            let me = heap.foreign_clone::<Vec3>(receiver).ok_or("add: receiver not a Vec3")?;
            let other_val = args.first().copied().ok_or("add: missing argument")?;
            let other = heap.foreign_clone::<Vec3>(other_val).ok_or("add: argument not a Vec3")?;
            let proto = heap.lookup_type("Vec3");
            heap.alloc_foreign(proto, me.add(&other))
        });

        native(heap, proto_id, "scale:", |heap, receiver, args| {
            let me = heap.foreign_clone::<Vec3>(receiver).ok_or("scale: receiver not a Vec3")?;
            let k = args.first().and_then(|v| v.as_float()).ok_or("scale: arg must be float")?;
            let proto = heap.lookup_type("Vec3");
            heap.alloc_foreign(proto, me.scale(k))
        });

        native(heap, proto_id, "dot:", |heap, receiver, args| {
            let me = heap.foreign_clone::<Vec3>(receiver).ok_or("dot: receiver not a Vec3")?;
            let other_val = args.first().copied().ok_or("dot: missing argument")?;
            let other = heap.foreign_clone::<Vec3>(other_val).ok_or("dot: argument not a Vec3")?;
            Ok(Value::float(me.dot(&other)))
        });

        native(heap, proto_id, "magnitude", |heap, receiver, _args| {
            let me = heap.foreign_clone::<Vec3>(receiver).ok_or("magnitude: receiver not a Vec3")?;
            Ok(Value::float(me.magnitude()))
        });
    }
}

/// Entry point for dylib loading. moof-cli's manifest loader
/// calls this via `libloading` when a `[types]` entry points
/// at this crate's cdylib.
#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn moof_core::Plugin> {
    Box::new(Vec3Plugin)
}
