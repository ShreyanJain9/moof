# V4 Track C.3 — stdlib bootstrap (rust as build-time oracle) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `moof-zig system.vat` boots the moof stdlib end-to-end. The full vat-image is produced at BUILD time by reusing the existing rust substrate as a compiler+serializer (option D from the dual-brainstorm). No new self-hosting required this session. Rust stays as the build-tool until parser.moof + compiler.moof self-host (phase A-self-host).

**Strategy (D):** rust's `new_world()` already runs the entire stdlib bootstrap at runtime — reading lib/main.moof, recursively loading compiler/*.moof, flipping `$compiler useMoof`, loading early/* + stdlib/*. By the time `new_world()` returns, the rust World is a fully-populated moof environment. We just **walk that World and serialize it as a V4 vat-image**. moof-zig loads it.

**Why D over A/B/C/E (from the brainstorm):**
- (A) requires runtime reader — we don't have one in zig
- (B) requires implementing macro expansion in OCaml — large
- (C) requires parser.moof — doesn't exist yet
- (E) requires moof VM in OCaml — large
- **(D) reuses 100% of the rust runtime work, no duplication, ships THIS session**

After parser.moof + compiler.moof self-host (separate phase), rust drops out and OCaml seed (with Gemini's "Oracle Bootstrap" hybrid) takes over. For now, D.

**Tech stack:** rust 2021 (build-time only), zig 0.16.0 (the runtime), V4 byte format per `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md` §3 + §4 + §10.

**Project state (HEAD: `67b1687`):**
- zig substrate boots, intrinsics installed, byte-encoded chunks run.
- OCaml seed produces V4-spec bytes for simple programs; cross-stack verified.
- Rust substrate is the safety net; produces V3 bytecode at runtime.
- moof-zig has `moof-zig decode <file>` working.

---

## File Structure

| file | role |
|---|---|
| `crates/substrate/src/v4_export.rs` (new) | walks the rust World, converts Vec<Op> to V4 byte-tagged bytecode, emits per-vat image per V4 §10.3 |
| `crates/substrate/src/main.rs` | add `moof export-v4 [path]` subcommand |
| `crates/zig-substrate/src/intrinsics.zig` | add `pub const REGISTRY: std.StaticStringMap(NativeFn)` populated at comptime; expose to image.zig |
| `crates/zig-substrate/src/world.zig` | wire `lookupNativeByName` to use intrinsics.REGISTRY |
| `crates/zig-substrate/src/image.zig` | fix sym-table hydration (replace, not append); test load path end-to-end |
| `crates/zig-substrate/src/main.zig` | add `moof-zig load <file.vat>` subcommand that loads + prints world state |

---

## Track 1: rust V4 exporter

**Goal:** `cargo run -p moof --bin moof -- export-v4 --output /tmp/system.vat` produces a V4 vat-image of the fully-bootstrapped rust World.

### Task 1.1: V3 Op → V4 byte-encoded conversion table

**Files:** `crates/substrate/src/v4_export.rs` (new)

For each rust `Op` variant, emit the V4 byte equivalent per spec §3. V3 has fewer ops than V4; missing ops (LoadHere, JumpIfTrue, SendSelf/Here, TailSendSelf/Here, SendDynamic, Suspend, Resume) won't be emitted. moof-zig still handles them.

- [ ] **Step 1: create v4_export.rs with the converter:**

```rust
//! V4 byte-tagged bytecode export. Walks the rust World produced
//! by new_world() and serializes it as a V4 vat-image per the spec
//! at docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md.
//!
//! Strategy D from the 2026-05-10 C.3 brainstorm: rust is the
//! build-time oracle. This is throwaway code that will be deleted
//! once parser.moof + compiler.moof self-host (phase A-self-host).

use crate::opcodes::Op;
use crate::value::Value;
use crate::form::FormId;
use crate::world::World;
use std::io::Write;

/// Compute the byte size of an Op in V4 encoding.
pub fn op_byte_size(op: &Op) -> usize {
    match op {
        Op::PushNil | Op::PushTrue | Op::PushFalse | Op::Pop | Op::Dup |
        Op::LoadSelf | Op::Return => 1,
        Op::LoadConst(_) => 3,           // tag + u16
        Op::LoadName(_) => 5,             // tag + u32
        Op::Send { .. } => 8,             // tag + u32 + u8 + u16
        Op::TailSend { .. } => 6,         // tag + u32 + u8
        Op::SuperSend { .. } => 8,
        Op::Jump(_) | Op::JumpIfFalse(_) => 3,  // tag + i16
        Op::PushClosure { .. } => 5,      // tag + u32
    }
}

/// Encode a single Op to V4 byte-tagged bytecode, appending to `buf`.
/// `byte_positions` maps op-index → byte-offset for the chunk's
/// `Vec<Op>` (computed in advance) — needed to convert Jump
/// op-index-based offsets to byte-based offsets.
pub fn encode_op(op: &Op, buf: &mut Vec<u8>, op_idx: usize, byte_positions: &[usize]) {
    match op {
        Op::PushNil => buf.push(0x01),
        Op::PushTrue => buf.push(0x02),
        Op::PushFalse => buf.push(0x03),
        Op::LoadConst(idx) => {
            buf.push(0x04);
            buf.extend_from_slice(&idx.to_be_bytes());
        }
        Op::LoadSelf => buf.push(0x05),
        Op::LoadName(sym) => {
            buf.push(0x07);
            buf.extend_from_slice(&(sym.0 as u32).to_be_bytes());
        }
        Op::Pop => buf.push(0x10),
        Op::Dup => buf.push(0x11),
        Op::Send { selector, argc, ic_idx } => {
            buf.push(0x20);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
            buf.extend_from_slice(&ic_idx.to_be_bytes());
        }
        Op::TailSend { selector, argc } => {
            buf.push(0x21);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
        }
        Op::SuperSend { selector, argc, ic_idx } => {
            buf.push(0x22);
            buf.extend_from_slice(&(selector.0 as u32).to_be_bytes());
            buf.push(*argc);
            buf.extend_from_slice(&ic_idx.to_be_bytes());
        }
        Op::Jump(op_offset) => {
            buf.push(0x30);
            // Convert op-index offset → byte offset
            let target_op = (op_idx as isize + *op_offset as isize) as usize;
            let target_byte = byte_positions[target_op] as isize;
            let current_byte = byte_positions[op_idx] as isize;
            let byte_offset = (target_byte - current_byte) as i16;
            buf.extend_from_slice(&byte_offset.to_be_bytes());
        }
        Op::JumpIfFalse(op_offset) => {
            buf.push(0x31);
            // ... same conversion as Jump
            let target_op = (op_idx as isize + *op_offset as isize) as usize;
            let target_byte = byte_positions[target_op] as isize;
            let current_byte = byte_positions[op_idx] as isize;
            let byte_offset = (target_byte - current_byte) as i16;
            buf.extend_from_slice(&byte_offset.to_be_bytes());
        }
        Op::Return => buf.push(0x33),
        Op::PushClosure { chunk } => {
            buf.push(0x40);
            buf.extend_from_slice(&(chunk.0 as u32).to_be_bytes());
        }
    }
}

/// Encode a whole chunk's Vec<Op> to V4 byte-tagged bytecode.
pub fn encode_chunk_ops(ops: &[Op]) -> Vec<u8> {
    // pass 1: compute byte position of each op
    let mut byte_positions = Vec::with_capacity(ops.len() + 1);
    let mut cursor = 0;
    for op in ops {
        byte_positions.push(cursor);
        cursor += op_byte_size(op);
    }
    byte_positions.push(cursor); // sentinel for "past end"

    // pass 2: emit bytes
    let mut buf = Vec::with_capacity(cursor);
    for (i, op) in ops.iter().enumerate() {
        encode_op(op, &mut buf, i, &byte_positions);
    }
    buf
}
```

VERIFY:
- Op variant field names + types (Op::Send fields are `selector: SymId, argc: u8, ic_idx: u16`).
- `SymId(u32)` — verify by reading `crates/substrate/src/sym.rs`.
- `FormId(u32)` — verify; the .0 field accesses the wrapped u32.
- `crate::opcodes::Op` — verify path.

- [ ] **Step 2: V4 Value encoding (inline byte-tagged per spec §10.3):**

```rust
/// Encode a Value as V4 byte-tagged inline (within FormSection).
/// Per spec §10.3 Value byte tags 0xC0-0xC7.
pub fn encode_value(v: Value, buf: &mut Vec<u8>) {
    match v {
        Value::Nil => buf.push(0xC0),
        Value::Bool(false) => buf.push(0xC1),
        Value::Bool(true) => buf.push(0xC2),
        Value::Int(n) => {
            buf.push(0xC3);
            buf.extend_from_slice(&(n as i64).to_be_bytes());
        }
        Value::Sym(s) => {
            buf.push(0xC4);
            buf.extend_from_slice(&(s.0 as u32).to_be_bytes());
        }
        Value::Char(cp) => {
            buf.push(0xC5);
            buf.extend_from_slice(&(cp as u32).to_be_bytes());
        }
        Value::Float(f) => {
            buf.push(0xC6);
            buf.extend_from_slice(&f.to_bits().to_be_bytes());
        }
        Value::Form(id) => {
            buf.push(0xC7);
            buf.extend_from_slice(&(id.0 as u32).to_be_bytes());
        }
    }
}
```

VERIFY: rust's `Value` enum exact variants in `crates/substrate/src/value.rs`. If rust has `Float(f64)` or includes Bigint/Bytes/etc. inline, handle each.

- [ ] **Step 3: full image serialization:**

```rust
pub fn serialize_world(world: &World) -> Vec<u8> {
    let mut buf = Vec::new();
    
    // Magic + Version
    buf.extend_from_slice(b"MVAT");
    buf.extend_from_slice(&0x0004_u16.to_be_bytes());
    
    // Header (16-byte vat_id, counts, here_form_id, macros_form_id, protos table, external_vat_refs)
    let vat_id = [0u8; 16];  // TODO: real ULID
    buf.extend_from_slice(&vat_id);
    
    let num_forms = world.heap.len() as u32 - 1; // exclude sentinel
    let num_syms = world.syms.len() as u32;
    let num_chunks = world.chunk_ops.len() as u32;
    buf.extend_from_slice(&num_forms.to_be_bytes());
    buf.extend_from_slice(&num_syms.to_be_bytes());
    buf.extend_from_slice(&num_chunks.to_be_bytes());
    
    buf.extend_from_slice(&world.here_form.0.to_be_bytes());
    buf.extend_from_slice(&world.macros_form.0.to_be_bytes());
    
    // Protos table (18 × u32 BE)
    let p = &world.protos;
    buf.extend_from_slice(&p.object.0.to_be_bytes());
    buf.extend_from_slice(&p.nil.0.to_be_bytes());
    // ... all 18 protos in spec §10.3 order
    
    // external_vat_refs_count = 0 for now
    buf.extend_from_slice(&0_u16.to_be_bytes());
    
    // SymTableSection
    buf.extend_from_slice(&num_syms.to_be_bytes());
    for sym_name in world.syms.iter() {
        let bytes = sym_name.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(bytes);
    }
    
    // FormSection — each Form's proto + slots + handlers + meta + frozen
    buf.extend_from_slice(&num_forms.to_be_bytes());
    for i in 1..world.heap.len() {
        let form_id = FormId::vat_local(i as u32);
        let form = world.heap.get(form_id);
        encode_value(form.proto, &mut buf);
        // slots
        buf.extend_from_slice(&(form.slots.len() as u16).to_be_bytes());
        for (sym, val) in form.slots.iter() {
            buf.extend_from_slice(&(sym.0 as u32).to_be_bytes());
            encode_value(*val, &mut buf);
        }
        // handlers
        buf.extend_from_slice(&(form.handlers.len() as u16).to_be_bytes());
        for (sel, method) in form.handlers.iter() {
            buf.extend_from_slice(&(sel.0 as u32).to_be_bytes());
            encode_value(*method, &mut buf);
        }
        // meta
        buf.extend_from_slice(&(form.meta.len() as u16).to_be_bytes());
        for (key, val) in form.meta.iter() {
            buf.extend_from_slice(&(key.0 as u32).to_be_bytes());
            encode_value(*val, &mut buf);
        }
        buf.push(if form.frozen { 1 } else { 0 });
    }
    
    // ChunkSection
    buf.extend_from_slice(&num_chunks.to_be_bytes());
    for (chunk_id, ops) in world.chunk_ops.iter() {
        buf.extend_from_slice(&chunk_id.0.to_be_bytes()); // source_form_id (placeholder: use chunk_id itself)
        let body = encode_chunk_ops(ops);
        buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
        buf.extend_from_slice(&body);
        // consts
        let consts = world.chunk_consts.get(chunk_id).cloned().unwrap_or_default();
        buf.extend_from_slice(&(consts.len() as u16).to_be_bytes());
        for c in consts {
            encode_value(c, &mut buf);
        }
        // ic_count
        let ic_count = world.chunk_ics.get(chunk_id).map(|v| v.len()).unwrap_or(0);
        buf.extend_from_slice(&(ic_count as u16).to_be_bytes());
        // params — empty for top-level chunks; populated for fn closures
        // TODO: pull from the chunk's :params slot in heap if it's there
        buf.extend_from_slice(&0_u16.to_be_bytes());
    }
    
    // NativeRefsSection — for each method-Form that has a native implementation,
    // emit (method_form_id, name). The name format is "Proto:selector" matching
    // what zig's intrinsics.zig REGISTRY uses.
    let natives = collect_native_methods(world);  // helper that walks proto handlers
    buf.extend_from_slice(&(natives.len() as u32).to_be_bytes());
    for (method_id, name) in natives {
        buf.extend_from_slice(&method_id.0.to_be_bytes());
        let bytes = name.as_bytes();
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    
    // McoBindingsSection — empty for now (skip wasm mcos at load)
    buf.extend_from_slice(&0_u32.to_be_bytes());
    
    // FarRefsSection — empty
    buf.extend_from_slice(&0_u32.to_be_bytes());
    
    // Footer hash — blake3 of everything above (or skip; zig stubs it)
    let hash = [0u8; 32];  // TODO: real blake3
    buf.extend_from_slice(&hash);
    
    buf
}
```

Helper `collect_native_methods` is the trickiest part: walk each proto's handler table, identify which methods are native (have an entry in `world.native_fns`), and produce `"ProtoName:selectorName"` strings. The proto name comes from the proto-Form's `:name` meta. Native methods are method-Forms whose FormId is a key in `world.native_fns`.

- [ ] **Step 4: integrate into main.rs:**

```rust
// in main()
if args.len() >= 2 && args[1] == "export-v4" {
    let output = if args.len() >= 4 && args[2] == "--output" { &args[3] } else { "system.vat" };
    let world = moof::new_world();
    let bytes = moof::v4_export::serialize_world(&world);
    std::fs::write(output, &bytes).unwrap();
    println!("wrote {} ({} bytes)", output, bytes.len());
    return;
}
```

- [ ] **Step 5: smoke:**

```bash
cargo run -p moof --bin moof -- export-v4 --output /tmp/system.vat
xxd /tmp/system.vat | head -3  # should start with "MVAT" 00 04
ls -la /tmp/system.vat         # probably 1-5 MB
```

- [ ] **Step 6: commit:**

```
substrate: v4_export — serialize World as V4 vat-image (build-time oracle)

Adds crates/substrate/src/v4_export.rs and a `moof export-v4` CLI
subcommand. Walks the fully-bootstrapped rust World produced by
new_world() and emits a V4 vat-image per the spec.

This is throwaway code (Strategy D from the 2026-05-10 C.3 brainstorm)
— rust serves as a build-time oracle until parser.moof + compiler.moof
self-host. Once that lands, this module + the rust runtime are deleted.

The big bypass: V3 has fewer opcodes than V4; LoadHere/SendSelf/SendHere
fusions don't fire (rust doesn't emit them). moof-zig sees the unfused
LoadSelf+Send shapes. Functionally identical, slightly suboptimal.
Phase A-self-host's OCaml seed will emit the fused shapes.
```

---

## Track 2: zig intrinsic registry + image-load wiring

**Goal:** moof-zig loads system.vat without panicking, populates World from the image bytes, native methods bound correctly.

### Task 2.1: comptime intrinsic registry

**Files:** `crates/zig-substrate/src/intrinsics.zig`, `crates/zig-substrate/src/world.zig`

Per Gemini's brainstorm finding: use `std.StaticStringMap` with comptime population so no manual registration boilerplate.

- [ ] **Step 1:** in intrinsics.zig, add at the bottom:

```zig
/// Comptime-built registry of native methods by canonical name.
/// At image-load, World.lookupNativeByName queries this to bind
/// native methods back to function pointers.
///
/// Names match the rust v4_export's NativeRefsSection format:
/// "ProtoName:selector" e.g. "Object:+:" or "Env:bind:to:".
pub const REGISTRY = std.StaticStringMap(NativeFn).initComptime(.{
    .{ "Integer:+", intPlus },
    .{ "Integer:-", intMinus },
    .{ "Integer:*", intMultiply },
    .{ "Integer:/", intDivide },
    .{ "Integer:=", intEq },
    .{ "Integer:<", intLt },
    .{ "Integer:>", intGt },
    .{ "Integer:toString", intToString },
    .{ "Object:!!", objBangBang },
    .{ "Nil:!!", nilBangBang },
    .{ "Bool:!!", boolBangBang },
    .{ "Object:is", objIs },
    .{ "Object:proto", objProto },
    .{ "Object:identity", objIdentity },
    .{ "Object:slot:", objSlot },
    .{ "Object:slotSet!:", objSlotSet },
    .{ "Cons:car", consCar },
    .{ "Cons:cdr", consCdr },
    .{ "Env:bind:to:", envBindTo },
    .{ "Env:set:to:", envSetTo },
    .{ "Env:lookup:", envLookupTo },
    .{ "Env:parent", envParent },
    .{ "Env:current", envCurrent },
    .{ "Closure:callIn:withSelf:", closureCallInWithSelf },
    .{ "Object:become:", objBecome },
    .{ "Object:doesNotUnderstand:with:", objDoesNotUnderstand },
    .{ "Object:perform:withArgs:", objPerformWithArgs },
    .{ "Bool:ifTrue:ifFalse:", boolIfTrueIfFalse },
    .{ "Object:toString", objToString },
});
```

- [ ] **Step 2:** in world.zig, replace the stub `lookupNativeByName`:

```zig
pub fn lookupNativeByName(self: *World, name: []const u8) ?intrinsics.NativeFn {
    _ = self;
    return intrinsics.REGISTRY.get(name);
}
```

### Task 2.2: image.zig sym-table hydration fix

**Files:** `crates/zig-substrate/src/image.zig`

Per Gemini's brainstorm finding: image-load MUST REPLACE the World's sym table with the image's sym table (not append). Otherwise SymIds in the image's chunks reference indexes that don't exist in moof-zig's freshly-initialized sym table.

- [ ] **Step 1:** in `readSymTable`, before interning:

```zig
// V4 §10 hydration semantics: REPLACE the World's sym table
// with the image's. Don't append. The image's chunks reference
// SymIds by index into this table.
world.syms.clearRetainingCapacity();
// Then intern in order, populating index N with the Nth string
```

Verify the SymTable type supports `clearRetainingCapacity` (or equivalent reset). If not, add it.

### Task 2.3: image-load end-to-end smoke

**Files:** `crates/zig-substrate/src/main.zig`

- [ ] **Step 1:** add `moof-zig load <file.vat>` subcommand:

```zig
if (args.len >= 3 and std.mem.eql(u8, args[1], "load")) {
    return runLoad(allocator, args[2]);
}

fn runLoad(allocator: std.mem.Allocator, path: []const u8) !void {
    const p = std.debug.print;
    
    // Initialize a bare World (no protos yet — image populates them)
    var world = try World.initBare(allocator);
    defer world.deinit();
    
    // Install intrinsics REGISTRY (populates name → fn pointer)
    // Note: actually populate the intrinsics REGISTRY at comptime;
    // World.lookupNativeByName queries it lazily.
    
    // Read the file
    const bytes = try std.fs.cwd().readFileAlloc(allocator, path, 64 * 1024 * 1024);
    defer allocator.free(bytes);
    
    // Load the image
    try image.loadVatImage(&world, bytes, allocator);
    
    p("loaded {s}\n", .{path});
    p("  heap.len = {}\n", .{world.heap.len()});
    p("  syms.len = {}\n", .{world.syms.len()});
    p("  chunks   = {}\n", .{world.chunk_bytecode.count()});
    p("  natives  = {}\n", .{world.native_fns.count()});
    p("  here_form = FormId.{{ scope={s}, payload={} }}\n",
        .{ @tagName(world.here_form.scope), world.here_form.payload });
    p("V4 vat-image alive ٩(◕‿◕｡)۶\n", .{});
}
```

- [ ] **Step 2:** add `World.initBare` if not present — like `init` but doesn't allocate protos / here_form / macros_form. Those come from the image.

### Task 2.4: zig commit

- [ ] **Step:**

```
zig-substrate: comptime intrinsic registry + image-load wiring

- intrinsics.zig: std.StaticStringMap REGISTRY of "Proto:selector" → NativeFn
  (29 mappings, populated at compile time via comptime).
- world.zig: lookupNativeByName queries the registry; image-load uses
  this to re-bind native methods after deserialization.
- image.zig: sym-table hydration now CLEARS world.syms before
  populating (replace semantics per V4 §10 + Gemini brainstorm).
- main.zig: new `moof-zig load <file>` subcommand. Loads a V4
  vat-image, prints world state, exits.

Pairs with Track 1 (rust v4_export) for cross-stack smoke.
```

---

## Track 3: cross-stack smoke (the proof)

**After Tracks 1 + 2 ship:** run this:

```bash
# Build moof (rust)
cargo build -p moof --release

# Build moof-zig
cd crates/zig-substrate && zig build && cd ../..

# Export the stdlib as a V4 image
cargo run -p moof --bin moof -- export-v4 --output /tmp/system.vat
xxd /tmp/system.vat | head -3
# Expected: 4d56 4154 0004 ...  ("MVAT" + version 4)

# Load it in moof-zig
./crates/zig-substrate/zig-out/bin/moof-zig load /tmp/system.vat
# Expected: 
#   loaded /tmp/system.vat
#     heap.len = 1000+
#     syms.len = 500+
#     chunks   = 200+
#     natives  = 29
#     here_form = FormId { scope=vat_local, payload=1 }
#   V4 vat-image alive
```

If this works, **the polyglot bootstrap is real and rust deletion is one phase away**.

---

## Risks

1. **Op conversion edge cases.** V3 has fewer ops; some V3 patterns may be inefficient as V4 (LoadSelf+Send instead of SendSelf). Acceptable for V4 minimum viable.

2. **Tagged-immediate Value differences.** Rust may store Strings/Bytes/Cons inline in Value; V4 image format requires them as FormIds. If rust has inline String values, we need to allocate them as Forms during serialization. **Verify** by reading `crates/substrate/src/value.rs`.

3. **Form ID stability.** Rust's `Heap::alloc` returns monotonic FormIds. Serializing the heap in alloc-order makes the IDs stable across both sides. Verify ordering.

4. **chunk.consts ownership.** Rust may store consts as `Vec<Value>` per chunk; verify shape, ensure we serialize them in order.

5. **chunk.source.** Each chunk has a `:source` Form (per L5). For now, set `source_form_id = 0` (placeholder); future work links to the actual source-Form.

6. **Wasm mcos.** Skipping for V4 minimum viable. `$hash` is the only critical one and we're skipping hash verification on load. Document this.

7. **Sym table size.** May be very large (thousands of symbols). Verify the V4 format's u16 size for sym name lengths is sufficient.

---

## Exit criteria

- [ ] `cargo run -- export-v4 --output /tmp/system.vat` exits 0, file > 100 KB.
- [ ] `moof-zig load /tmp/system.vat` exits 0, prints world state.
- [ ] world state shows: heap.len > 100, syms.len > 200, chunks > 50, natives = 29.
- [ ] no panics anywhere in the pipeline.
- [ ] **stretch:** execute a hand-constructed chunk against the loaded world that uses a native (e.g. `[1 + 2]`) and verify result.

---

## What's NOT in scope this session

- Running moof source through the loaded world (needs a runtime parser; phase A-self-host).
- Full OCaml seed feature parity with rust seed (when parser.moof self-hosts, OCaml takes over).
- Rust runtime deletion (will happen LATER, after polyglot proves itself).
- Wasm mco re-instantiation (only $hash matters; defer).
- Blake3 hash verification at image-load (zig stubs; rust emits zeros).
- Tail-call-threaded dispatch optimization (separate perf pass).

## see also

- spec: `docs/superpowers/specs/2026-05-10-vm-V4-opcodes-design.md`
- predecessor plan: `docs/superpowers/plans/2026-05-10-vm-V4-polyglot-substrate.md`
- brainstorm: `/tmp/v4-c3-brainstorm.md`
