# plugins

**type:** reference

> plugins are rust crates that extend moof: new types (ForeignType)
> or new capabilities (CapabilityPlugin). compiled as cdylibs,
> loaded at runtime.

---

## two kinds

| kind | purpose | examples |
|------|---------|----------|
| **type plugin** | new heap value types with native payload | Vec3, Color, JsonValue, GUI widgets |
| **capability plugin** | new capability vats (effects) | Console, Clock, File, Random |

the moof runtime loads both via `libloading`. type plugins register
types on each vat's heap; capability plugins spawn a vat and
install native handlers on its root object.

---

## writing a type plugin

**cargo setup:**

```toml
[package]
name = "moof-plugin-vec3"
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
moof-core = { path = "../moof-core" }
```

**implementation:**

```rust
use moof_core::{Plugin, native, ForeignType, Heap, Value};

#[derive(Clone, Debug)]
pub struct Vec3 { pub x: f64, pub y: f64, pub z: f64 }

impl ForeignType for Vec3 {
    fn type_name() -> &'static str { "moof.plugin.Vec3" }
    fn prototype_name() -> &'static str { "Vec3" }
    fn serialize(&self) -> Vec<u8> {
        // bincode or similar
        bincode::serialize(&(self.x, self.y, self.z)).unwrap()
    }
    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        let (x, y, z) = bincode::deserialize(bytes).map_err(|e| e.to_string())?;
        Ok(Vec3 { x, y, z })
    }
    fn equal(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z
    }
    fn describe(&self) -> String {
        format!("(vec3 {} {} {})", self.x, self.y, self.z)
    }
}

pub struct Vec3Plugin;

impl Plugin for Vec3Plugin {
    fn name(&self) -> &str { "vec3" }

    fn register(&self, heap: &mut Heap) {
        let proto = moof_core::register_foreign_proto::<Vec3>(heap);
        let proto_id = proto.as_any_object().unwrap();

        native(heap, proto_id, "new:y:z:", |heap, _recv, args| {
            let x = args[0].as_float().ok_or("x not float")?;
            let y = args[1].as_float().ok_or("y not float")?;
            let z = args[2].as_float().ok_or("z not float")?;
            let proto = heap.lookup_type("Vec3");
            heap.alloc_foreign(proto, Vec3 { x, y, z })
        });

        native(heap, proto_id, "magnitude", |heap, receiver, _args| {
            let v = heap.foreign_clone::<Vec3>(receiver).ok_or("not Vec3")?;
            Ok(Value::float((v.x*v.x + v.y*v.y + v.z*v.z).sqrt()))
        });

        // ... etc ...
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn Plugin> {
    Box::new(Vec3Plugin)
}
```

register at the manifest (moof.toml):

```toml
[types]
vec3 = "builtin:vec3"
```

or:

```toml
[types]
myplugin = "path/to/libmyplugin.dylib"
```

---

## writing a capability plugin

capability plugins spawn their own vat at runtime, populated with
native handlers that wrap rust-side state.

```rust
use moof_runtime::capability::CapabilityPlugin;
use moof_runtime::vat::Vat;
use moof_core::{Value, native};

pub struct ClockCap { /* internal state */ }

impl CapabilityPlugin for ClockCap {
    fn name(&self) -> &str { "clock" }

    // setup is called with the newly-spawned vat. install handlers
    // on its root object; return the root object's id.
    fn setup(&self, vat: &mut Vat) -> u32 {
        let root = vat.heap.make_object(vat.heap.type_protos[PROTO_OBJ]);
        let root_id = root.as_any_object().unwrap();

        native(&mut vat.heap, root_id, "now", |_h, _r, _a| {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            Ok(Value::integer(ms))
        });

        native(&mut vat.heap, root_id, "monotonic", |_h, _r, _a| {
            // ...
        });

        root_id
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_plugin() -> Box<dyn CapabilityPlugin> {
    Box::new(ClockCap { /* ... */ })
}
```

register in manifest:

```toml
[capabilities]
clock = "builtin:clock"
```

the system will spawn a vat, call setup, and grant the resulting
FarRef to any interface (repl, script) whose manifest grant list
mentions `clock`.

---

## the `native` helper

`moof_core::native` is the easiest way to add a native handler:

```rust
native(heap, proto_id, "selector:args:", |heap, receiver, args| {
    // do rust work
    Ok(return_value)
});
```

- `heap` — the Heap (needed for alloc, lookups).
- `receiver` — the object the message was sent to.
- `args` — slice of Value arguments.
- return `Ok(Value)` on success, `Err(String)` on failure.

the helper:
1. registers the closure in `heap.natives[]`.
2. creates a Block-proto heap object with a `native_idx` slot
   pointing at the registered closure.
3. installs it as a handler on `proto_id`.

the resulting handler is indistinguishable from a moof-defined
closure at the moof level. `[receiver selector:args:]` works.

---

## ABI stability and drift

**load-bearing invariant**: plugins are compiled against a
specific moof-core version. if moof-core changes in a way that
alters Heap layout, ForeignType trait, or the native-registration
ABI, stale plugins WILL segfault.

the safest workflow:

1. plugins live in the workspace (`crates/moof-plugin-*`). they
   rebuild automatically on every `cargo build --release`.
2. plugins outside the workspace (`examples/type-plugin/`) are
   NOT rebuilt automatically. if you change moof-core, you must
   also `cd examples/type-plugin && cargo build --release` to
   refresh the dylib.

a stale plugin segfaults at SHUTDOWN because its natives are
dropped after its dylib's memory layout no longer matches the
host's expectations. the segfault is loud but unhelpful — the
clue is "was this plugin rebuilt against current moof-core?"

there is no runtime ABI version check today. there should be.
added to the roadmap.

---

## cross-vat copy for foreign types

when a foreign value crosses a vat boundary (sent as a message
arg), the scheduler:

1. calls `foreign.serialize()` in the sender.
2. calls `ForeignType::deserialize(bytes)` in the receiver.
3. attaches the result to a fresh HeapObject in the receiver's
   heap.

identity-by-name (via `ForeignTypeName`) is the stable key. each
dylib has its own TypeId; we can't use Rust's `TypeId` across
dylibs. the name-based identity works even when sender and
receiver loaded the type from the same dylib but registered it
independently in their heaps.

if your foreign type needs to track other Values (children,
refs), implement `trace` so GC visits them:

```rust
fn trace(&self, visit: &mut dyn FnMut(Value)) {
    visit(self.child_ref);
    for v in &self.values { visit(*v); }
}
```

---

## virtual slots

a foreign type can expose virtual slots — slots that aren't in the
`slot_values` vec but appear to moof as if they were:

```rust
fn virtual_slot(&self, sym: u32) -> Option<Value> {
    if sym == CAR_SYM.load() { Some(self.car) }
    else if sym == CDR_SYM.load() { Some(self.cdr) }
    else { None }
}

fn virtual_slot_names(&self) -> Vec<u32> {
    vec![CAR_SYM.load(), CDR_SYM.load()]
}
```

Cons uses this to expose `car` and `cdr` as slots even though it
stores them as optimized Value pairs. `[pair.car]` and `[pair
slotNames]` both work as if they were real slots.

---

## publishing a plugin

right now there's no registry or package manager. to publish a
plugin:

1. host the crate on github (or wherever).
2. users clone + build: `cargo build --release`.
3. users register in their `moof.toml`:
   `myplugin = "path/to/libmyplugin.dylib"`.

a future moof-registry might automate this. not a priority.

---

## anti-patterns

- **don't mutate heap state from a foreign payload's drop impl.**
  drops happen during GC; the heap is in the middle of a sweep.
- **don't hold Heap references inside your foreign struct.** when
  the object moves across vats, the reference becomes stale.
- **don't capture `&mut Heap` in a native handler's closure.** the
  handler can be invoked when Heap is in various states.
- **don't use Rust's TypeId for identity across dylibs.** use the
  name-based `ForeignTypeName` via the registry.
- **don't assume your plugin's types are registered before
  yours.** use `heap.lookup_type("Name")` at call time, not at
  register time, for types you don't own.

---

## what you need to know

- plugins are cdylibs compiled against moof-core.
- type plugins add ForeignType implementations and register them.
- capability plugins spawn a vat and install native handlers on
  its root.
- the `native` helper is the standard way to add a native handler.
- ABI stability is fragile — rebuild plugins whenever moof-core
  changes.
- cross-vat transfer uses serialize/deserialize; identity is
  name-based.

---

## next

- [vm.md](vm.md) — the runtime plugins plug into.
- [syntax.md](syntax.md) — what user code looks like.
- [../concepts/capabilities.md](../concepts/capabilities.md) —
  the security model capability plugins participate in.
