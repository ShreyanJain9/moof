// Example external TYPE plugin for moof. Unlike the counter
// capability in examples/rust-plugin (which runs in its own vat
// and holds mutable state), this is a pure value type — every
// vat that loads it can allocate `Color(r, g, b)` objects whose
// payload is immutable.
//
// Build:  cargo build --release
// Load:   add to moof.toml [types]:
//           color = "examples/type-plugin/target/release/libmoof_color_plugin.dylib"
// Use:    [Color r: 255 g: 0 b: 128]   → Color(255, 0, 128)
//         c.r / c.g / c.b               → 255 / 0 / 128
//         [c mix: other]                → Color(avg, avg, avg)
//         [c hex]                       → "#ff0080"
//
// The whole point of wave 5: this type wasn't compiled into moof
// and lives in its own crate, but it's indistinguishable at the
// moof level from a built-in like Vec3.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use moof::{Heap, Plugin, Value};
use moof::foreign::ForeignType;
use moof::plugins::native;

#[derive(Clone, Debug, PartialEq)]
pub struct Color { pub r: u8, pub g: u8, pub b: u8 }

impl ForeignType for Color {
    fn type_name() -> &'static str { "color.Color" }
    fn prototype_name() -> &'static str { "Color" }

    fn serialize(&self) -> Vec<u8> { vec![self.r, self.g, self.b] }
    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 3 { return Err(format!("Color: expected 3 bytes, got {}", bytes.len())); }
        Ok(Color { r: bytes[0], g: bytes[1], b: bytes[2] })
    }

    fn equal(&self, other: &Self) -> bool { self == other }
    fn describe(&self) -> String { format!("Color({}, {}, {})", self.r, self.g, self.b) }

    fn virtual_slot(&self, sym: u32) -> Option<Value> {
        let s = COLOR_SYMS.load();
        if sym == s.r { Some(Value::integer(self.r as i64)) }
        else if sym == s.g { Some(Value::integer(self.g as i64)) }
        else if sym == s.b { Some(Value::integer(self.b as i64)) }
        else { None }
    }

    fn virtual_slot_names(&self) -> Vec<u32> {
        let s = COLOR_SYMS.load();
        vec![s.r, s.g, s.b]
    }
}

#[derive(Clone, Copy)]
struct ColorSyms { r: u32, g: u32, b: u32 }

struct AtomicColorSyms { r: AtomicU32, g: AtomicU32, b: AtomicU32 }
impl AtomicColorSyms {
    const fn new() -> Self {
        AtomicColorSyms { r: AtomicU32::new(0), g: AtomicU32::new(0), b: AtomicU32::new(0) }
    }
    fn load(&self) -> ColorSyms {
        ColorSyms { r: self.r.load(Relaxed), g: self.g.load(Relaxed), b: self.b.load(Relaxed) }
    }
    fn store(&self, s: ColorSyms) {
        self.r.store(s.r, Relaxed); self.g.store(s.g, Relaxed); self.b.store(s.b, Relaxed);
    }
}
static COLOR_SYMS: AtomicColorSyms = AtomicColorSyms::new();

struct ColorPlugin;

impl Plugin for ColorPlugin {
    fn name(&self) -> &str { "color" }

    fn register(&self, heap: &mut Heap) {
        heap.register_foreign_type::<Color>().expect("register Color");
        COLOR_SYMS.store(ColorSyms {
            r: heap.intern("r"),
            g: heap.intern("g"),
            b: heap.intern("b"),
        });

        // Object prototype hoisted in core plugin — we inherit from it.
        let object_proto = heap.type_protos[moof::heap::PROTO_OBJ];
        let proto = heap.make_object(object_proto);
        let proto_id = proto.as_any_object().unwrap();

        native(heap, proto_id, "r:g:b:", |heap, _recv, args| {
            let r = args.first().and_then(|v| v.as_integer()).ok_or("r: must be Integer")?;
            let g = args.get(1).and_then(|v| v.as_integer()).ok_or("g: must be Integer")?;
            let b = args.get(2).and_then(|v| v.as_integer()).ok_or("b: must be Integer")?;
            let proto = heap.lookup_type("Color");
            heap.alloc_foreign(proto, Color {
                r: r.clamp(0, 255) as u8,
                g: g.clamp(0, 255) as u8,
                b: b.clamp(0, 255) as u8,
            })
        });

        native(heap, proto_id, "mix:", |heap, receiver, args| {
            let a = heap.foreign_clone::<Color>(receiver).ok_or("mix: receiver not a Color")?;
            let other = args.first().copied().ok_or("mix: missing argument")?;
            let b = heap.foreign_clone::<Color>(other).ok_or("mix: argument not a Color")?;
            let proto = heap.lookup_type("Color");
            heap.alloc_foreign(proto, Color {
                r: ((a.r as u16 + b.r as u16) / 2) as u8,
                g: ((a.g as u16 + b.g as u16) / 2) as u8,
                b: ((a.b as u16 + b.b as u16) / 2) as u8,
            })
        });

        native(heap, proto_id, "hex", |heap, receiver, _args| {
            let c = heap.foreign_clone::<Color>(receiver).ok_or("hex: receiver not a Color")?;
            Ok(heap.alloc_string(&format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)))
        });

        let sym = heap.intern("Color");
        heap.env_def(sym, proto);
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn Plugin> {
    Box::new(ColorPlugin)
}
