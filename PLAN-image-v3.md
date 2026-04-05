# Plan: Real Image Persistence (v3)

## The Problem

We don't have an image. We have a build system. Every startup parses, compiles, and evaluates all source from scratch. Live object mutations are lost. The design doc says "things just survive" — we have the opposite.

### Specific failures

1. **No orthogonal persistence** — `[point slotAt: 'x put: 42]` vanishes on restart. Only text-appended `(def ...)` forms survive.
2. **Startup is O(parse+compile+eval)** — rebuilds the world from source every time. A real image deserializes the heap in milliseconds.
3. **Workspace autosave is string hackery** — appending/deduplicating source text is fragile. A real image just has the objects.
4. **The heap/source split** — the heap is truth at runtime, source files are truth at rest. Mutations live in one world, definitions in another.
5. **No GC** — heap grows unbounded. Compaction was removed with the binary image.
6. **Modules conflate organization with persistence** — modules handle deps/sandboxing but shouldn't be the only path to persistence. Runtime-created objects need to persist too.

## The Fix: Two Layers

The binary image for the live heap. Source-level modules for code organization. Both, together.

### Layer 1: The Image (binary, canonical for runtime state)

```
.moof/
  image.bin              — serialized heap + symbol table + module registry
  image.sha256           — integrity hash
```

The image contains:
- **The full heap** — every object, every binding, every environment, exactly as it was
- **The symbol table** — all interned symbols
- **Module registry** — per-module metadata: name, requires, provides, source_hash, env_id
- **Module source texts** — the original authored source for each module, stored as MoofString objects in the heap (not as separate fields — the source IS a heap object)
- **Root env ID** — for recovery

On startup:
1. Deserialize `image.bin` → heap is immediately ready
2. Register native functions (always re-registered from Rust)
3. Reconstruct `ModuleLoader` from the registry metadata
4. REPL — instant

On checkpoint:
1. Compact GC (mark from root_env + all module envs)
2. Serialize heap + symbols + module registry
3. SHA-256 hash
4. Write `image.bin`

**Orthogonal persistence**: every heap mutation survives. `[point slotAt: 'x put: 42]` is in the heap, the heap is in the image, the image persists. No special "save" logic for individual operations.

### Layer 2: Source Modules (text, canonical for code organization)

```
.moof/
  modules/
    bootstrap.moof       — full source with comments
    collections.moof
    ...
```

The source files serve three purposes:
1. **Git diffing** — `git diff .moof/modules/` shows what code changed
2. **Human readability** — you can read the source in any editor
3. **Seeding** — first boot (no image.bin) evaluates from source

Source is **projected** from the image on checkpoint, not the other way around. The image is truth. Source is a view.

Each Module object in the heap has a `source` slot containing the original authored text. On checkpoint, these are written to `.moof/modules/` as a side effect. On fresh boot (no image), source files are read and compiled to build the initial image.

### The Startup Flow

```
if image.bin exists:
    deserialize → heap ready → register natives → REPL      [fast path, ~10ms]
else if .moof/modules/ exists:
    parse + compile + eval → register natives → checkpoint → REPL  [rebuild]
else if lib/ exists:
    seed → parse + compile + eval → register natives → checkpoint → REPL [first boot]
else:
    error: no image and no source
```

### What Changes

#### Bring back: `src/persistence/snapshot.rs`

New `Image` struct:

```rust
#[derive(Serialize, Deserialize)]
pub struct Image {
    pub version: u32,  // 3
    pub objects: Vec<HeapObject>,
    pub symbol_names: Vec<String>,
    pub root_env_id: u32,
    pub module_registry: ModuleRegistry,
}

#[derive(Serialize, Deserialize)]
pub struct ModuleRegistry {
    /// Modules in topo load order
    pub modules: Vec<ModuleEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct ModuleEntry {
    pub name: String,
    pub requires: Vec<String>,
    pub provides: Vec<String>,
    pub source_obj_id: u32,  // heap ID of MoofString holding source text
    pub env_id: u32,         // heap ID of module's sandbox environment
    pub unrestricted: bool,
}
```

Source text is stored as a regular MoofString on the heap, referenced by `source_obj_id`. It participates in GC normally — if a module is removed, its source string becomes garbage and gets collected.

#### Bring back: compacting GC

`compact_image()` returns, but module-aware:
- Mark from root_env + every module env_id + every module source_obj_id
- Forward table for all Value::Object references
- Rewrite env_ids and source_obj_ids in the registry through forwarding table

#### Modify: `src/modules/loader.rs`

Add `from_image_registry()`:
- Reconstruct `ModuleGraph` from `ModuleRegistry` entries
- Populate `loaded_envs` from stored env_ids
- Populate `exports` by reading provides symbols from stored envs
- Populate `source_texts` by reading MoofStrings from heap
- **No re-parsing, no re-compiling, no re-executing**

Add `to_image_registry()`:
- Pack current loader state into `ModuleRegistry`
- Source text stored as heap objects

#### Modify: `src/main.rs`

Unified startup:
1. Try `image.bin` → fast path (deserialize, register natives, reconstruct loader)
2. Try `.moof/modules/` → rebuild path (discover, compile, eval, checkpoint)
3. Try `lib/` with `--seed` → first boot

Checkpoint saves: image.bin + projects source to .moof/modules/

#### Modify: `src/runtime/heap.rs`

- Bring back `from_image()` (already exists but unused)
- No WAL (not needed — checkpoint is the persistence boundary)

#### Keep: `src/persistence/image.rs`

The directory-based manifest/module system stays for source projection and seeding. It's the rebuild/export path, not the primary persistence.

#### Keep: `modules.moof`, workspace, Modules object

The moof-level module API stays. Module objects get their `source` slot from the heap (where it's a real MoofString) rather than from disk reads.

### What About the WAL?

No WAL in v3. Rationale:
- The WAL existed to provide crash recovery between checkpoints
- With auto-checkpoint on exit and periodic checkpoints, the window of loss is small
- The WAL adds complexity (append-only file, replay logic, WAL-aware mutations)
- If crash safety becomes critical, add it back as a targeted feature

Alternative: auto-checkpoint more aggressively (every N seconds, or every N allocations). The directory-based source projection provides a second safety net — even if image.bin is corrupted, `.moof/modules/` has the source.

### What About Workspace?

Workspace becomes simpler. Currently it's text-append hackery. With a real image:

- REPL definitions go into the workspace environment (a real heap environment)
- The workspace Module object's `source` slot accumulates the text (for readability/export)
- But the REAL persistence is the heap — the bindings are in the environment, the environment is in the image
- On restart from image, workspace bindings are just... there. No re-parsing needed.

Workspace autosave becomes: "checkpoint periodically" rather than "append text to a file and re-save."

### Migration Path

Phase 1: Bring back binary image serialization alongside the directory format. Both coexist. `--no-image` flag to force source-only mode.

Phase 2: Make image.bin the default startup path. Source files projected on checkpoint.

Phase 3: Auto-checkpoint (on exit + periodic). Remove explicit `(checkpoint)` requirement.

Phase 4: Aggressive testing — verify mutations persist, verify GC correctness, verify source projection round-trips.

### Verification

- Delete image.bin, start from source → builds image → restart loads from image instantly
- Modify a slot at REPL → checkpoint → restart → slot value persisted
- Create objects, add handlers interactively → checkpoint → restart → all there
- `(export-modules lib)` → files identical to pre-image source
- GC: create garbage, checkpoint, image is smaller
- Module reload: edit a .moof file, `[module reload]`, changes take effect

### Files to Change

| File | Change |
|------|--------|
| `src/persistence/snapshot.rs` | **NEW** — Image v3 struct, serialize/deserialize, compact_image |
| `src/persistence/image.rs` | Keep — source projection + seeding |
| `src/modules/loader.rs` | Add `from_image_registry`, `to_image_registry` |
| `src/main.rs` | Unified 3-tier startup, checkpoint on exit |
| `src/runtime/heap.rs` | Re-add `from_image()`, `objects()`, `symbol_names_ref()` |
| `.moof/modules/*.moof` | Unchanged — still the source files |
| `modules.moof` | Minor — source slot reads from heap instead of disk |
