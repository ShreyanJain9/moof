# vm

**type:** reference

> the rust-side moof VM. bytecode, dispatch, heap layout. this
> doc is for people modifying `crates/moof-core` and
> `crates/moof-lang`.

---

## crate layout

```
crates/
├── moof-core/          object model, heap, protocols, primitives
├── moof-lang/          lexer, parser, compiler, VM
├── moof-runtime/       scheduler, vat management, cross-vat I/O
├── moof-cli/           entry point, REPL, shell
├── moof-plugin-*/      type plugins (Vec3, JSON, etc)
└── moof-cap-*/         capability plugins (Console, Clock, etc)
```

```
crates/moof-core/
  src/
    value.rs          NaN-boxed Value type
    object.rs         HeapObject struct
    heap/
      mod.rs          Heap — the god file (see "owed cleanup")
      format.rs       display/formatting
      gc.rs           mark-and-sweep
      image.rs        load_image / save_image
      pair.rs         Cons
      text.rs         String
      bytes.rs        Bytes
      table.rs        Table
      bigint.rs       BigInt
    dispatch.rs       message send / handler lookup
    foreign.rs        ForeignType trait + registry
    canonical.rs      content-addressing / hashing
    plugin.rs         Plugin trait + helpers
```

---

## Value — the NaN-boxed 64-bit word

every moof value is a single 64-bit `Value`. the encoding:

- **IEEE 754 doubles** reserve a "quiet NaN" bit pattern; we
  abuse this space to pack tagged payloads.
- tag occupies bits 50–48; payload occupies bits 47–0.
- tag 0 = nil, 1 = true, 2 = false, 3 = integer, 4 = symbol,
  5 = object (old-gen), 6 = nursery (new-gen object), 7 = reserved.
- tag 3 payload is a signed 48-bit integer (i48). BigInts spill to
  heap objects.
- tag 4 payload is a 32-bit symbol id.
- tag 5/6 payload is a 32-bit object id.
- ordinary floats (non-NaN) are just themselves — `f.to_bits()`.

this gives us **one-word values for 90% of typical data**: small
ints, floats, booleans, nil, symbols, and object references.

see `crates/moof-core/src/value.rs`.

---

## HeapObject — the universal shape

everything not fitting in a tagged Value goes on the heap:

```rust
struct HeapObject {
  proto: Value,
  slot_names: Vec<u32>,   // symbol ids
  slot_values: Vec<Value>,
  handlers: Vec<(u32, Value)>,  // selector, handler (closure or native)
  foreign: Option<ForeignData>,  // optional native payload
}
```

- `proto` — delegation parent for message dispatch.
- `slot_names` + `slot_values` — parallel vectors, fixed-shape per
  instance.
- `handlers` — selector-keyed handler table. handlers added by
  `handler_set`; looked up by dispatch.
- `foreign` — optional. holds rust-owned payload for types like
  Cons, Text, Bytes, Table, BigInt, Vec3 — things the VM wants to
  access efficiently but still present as objects.

handler values are themselves moof values — usually closure
objects (Block-proto) or wrapped native handlers. native handlers
are Block-proto objects with a `native_idx` slot pointing into
`heap.natives[]`.

---

## dispatch

`[receiver sel: arg]` compiles to a SEND opcode. at runtime:

```
lookup_handler(heap, receiver, selector):
  if receiver has handler sel directly:  return it
  else walk prototype chain (max 256 steps):
    if found sel:  cache and return
  if still not found:
    raise doesNotUnderstand:
```

the lookup uses a `(proto_id, selector) → handler` cache on the
heap (`send_cache`). most dispatches hit the cache. cache is
flushed when the proto chain changes.

call semantics depend on handler type:
- **closure** (Block-proto with `code_idx`): push a frame, execute
  bytecode.
- **native** (Block-proto with `native_idx`): call
  `natives[native_idx](heap, receiver, args)`.

see `crates/moof-core/src/dispatch.rs`.

---

## bytecode

the compiler emits a simple stack-ish bytecode. opcodes include:

- `CONST idx` — push constant from chunk.constants
- `LOAD sym` / `STORE sym` — lookup/assign in the current env
- `SEND sel, nargs` — the message-send opcode (the most-used one)
- `CALL nargs` — applicative call (sugar; emits SEND of `call:`)
- `POP`, `DUP`, `JUMP`, `JUMP_IF_FALSE`
- `MAKE_CLOSURE desc_idx` — build a closure capturing the current
  env
- `MAKE_OBJECT slot_count handler_count` — object literal
- `RETURN`

closures have a `ClosureDescriptor` — a reusable template with the
bytecode, constants, arity, capture names. `closure_descs` is a
Vec; each closure instance references it by index.

see `crates/moof-lang/src/vm.rs` for the dispatch loop.

---

## the scheduler

one vat at a time — the scheduler iterates, each vat gets a fuel
budget, runs until it yields, moves on. cross-vat sends populate
the outbox; the scheduler drains outboxes into mailboxes; vats
process their mailboxes during their turn.

pseudocode:

```
loop:
  for vat in vats:
    if vat has ready work:
      vat.run_until_fuel_exhausted()
  deliver outbox messages to target mailboxes
  resolve pending Acts
  if nothing is ready: sleep
```

see `crates/moof-runtime/src/scheduler.rs`.

---

## foreign types

a `ForeignType` is a rust struct exposed as a first-class moof
value. examples: BigInt, Vec3, Color, JsonValue.

```rust
pub trait ForeignType: Any + Clone + Send + Sync + 'static {
    fn type_name() -> &'static str;
    fn prototype_name() -> &'static str { "Object" }
    fn serialize(&self) -> Vec<u8>;
    fn deserialize(bytes: &[u8]) -> Result<Self, String>;
    fn equal(&self, other: &Self) -> bool;
    fn describe(&self) -> String;
    // optional:
    fn trace(&self, visit: &mut dyn FnMut(Value)) {}
    fn virtual_slot(&self, sym: u32) -> Option<Value> { None }
    fn virtual_slot_names(&self) -> Vec<u32> { Vec::new() }
}
```

registration gives the type a `ForeignTypeId` + a `ForeignTypeName`
(used for stable cross-session identity). a vtable of function
pointers is stored in the heap's `foreign_registry`.

at runtime, a HeapObject's `foreign: ForeignData` holds the Arc
payload + the type_id. the dispatch-side type check is by NAME
(not Rust TypeId) so dylib-loaded plugins can interop with the
host.

see `crates/moof-core/src/foreign.rs`.

---

## content-addressing

every immutable value can be canonically serialized and hashed to
a BLAKE3 digest. the heap's `hash_blob` function walks the value,
produces canonical bytes, hashes. cycles use a fixpoint
placeholder to converge.

blob store is LMDB-backed (via `heed`). three tables: blobs, refs,
meta.

see `crates/moof-core/src/canonical.rs` and
`crates/moof-runtime/src/blobstore.rs`.

---

## GC

mark-and-sweep, triggered when nursery objects exceed a budget.
roots: the env, the current VM frame stack, the stack of ready
Acts, server mailboxes. sweep is straightforward. no generational
collection yet (but the nursery/old split exists in the Value tag,
for future use).

see `crates/moof-core/src/heap/gc.rs`.

---

## plugin loading

plugins are cdylibs. loading:

1. `libloading::Library::new(path)` opens the dylib.
2. look up `moof_create_type_plugin` (or `moof_create_plugin`)
   symbol.
3. call it; returns a `Box<dyn Plugin>` (or `Box<dyn CapabilityPlugin>`).
4. keep the dylib handle alive for the lifetime of the scheduler —
   critically, until every vat that references the plugin's
   natives has dropped (otherwise: segfault).

plugins register types (via `ForeignType`) or capabilities (via
`CapabilityPlugin::setup`) into a target vat's heap.

see `crates/moof-runtime/src/dynload.rs` and `plugin.rs`.

---

## owed cleanup

- **Heap is a god file** (~1200 lines, 70 public methods). extract
  `SymbolTable`, `ProtoRegistry`, `Arena` as separate types;
  Heap composes them. wave 10.0.
- **scheduler-as-rust-struct.** wave 10 turns it into a moof-level
  capability.
- **bootstrap source replay** is replaced by seed-image hydration.
  wave 10.1+.
- **plugin ABI drift** — a stale plugin can segfault at shutdown
  (documented incident). add a version check at load time, crash
  early with a clear message.

---

## what you need to know

- Value is NaN-boxed; small ints, floats, bools, nil, symbols, and
  object refs fit in 8 bytes.
- HeapObject is uniform: proto, slots, handlers, optional foreign
  payload.
- dispatch walks proto chain, up to 256 depth, with a send cache.
- bytecode is stack-shaped; SEND is the hot opcode.
- scheduler runs vats fairly with fuel-based preemption.
- foreign types bring rust data into moof as first-class values.
- plugin ABI uses libloading; drop order matters.

---

## next

- [plugins.md](plugins.md) — how to write a plugin.
- [syntax.md](syntax.md) — what compiles into this bytecode.
- [../concepts/objects.md](../concepts/objects.md) — the
  semantic model behind the rust layer.
