# Moofpaint and the Spatial Interface: Deep Implementation Plan

> **Goal: A 60Hz 3D ZUI with tangible Morphic components and fast native pixmaps.**

This document outlines the strict technical phases for rendering, spatial management, and Morphic interactions.

## 1. The Spatial Primitive: Poses and Matrices

All spatial coordinates are offloaded to an MCO to avoid Rust substrate bloat.

### Phase 1.A: `math3d.mco` ABI
**Files:** `crates/mco-math3d/src/lib.rs`, `examples/wasm-mcos/lib/moof.zig`
**The ABI:** The MCO exposes matrix multiplication and quaternion math.
```rust
#[no_mangle]
pub extern "C" fn math3d_matrix_mul(a_handle: u32, b_handle: u32) -> u32 {
    // 1. Read 16 f32s from A and B Forms
    // 2. Multiply
    // 3. Allocate new Form containing 16 f32s, return handle
}
```

### Phase 1.B: The `Placement` Graph
**Files:** `lib/world/placement.moof`
```moof
(defproto Placement
  (slots form pose children parent cachedMatrix dirty)
  (handlers
    [markDirty]
      (set! self.dirty #true)
      [self.children forEach: |c| [c markDirty]]
    [worldMatrix]
      (if self.dirty
          (do
            (let local [$math3d poseToMatrix: self.pose])
            (set! self.cachedMatrix (if [self.parent is nil] local [$math3d matrixMul: [self.parent worldMatrix] local]))
            (set! self.dirty #false)))
      self.cachedMatrix))
```
**Tests:** `test_placement_matrix_caching`, `test_placement_dirty_propagation`.

## 2. Rendering Protocol (`:render-with:`)

We must support both terminal (braille) and GPU (wgpu) rendering cleanly.

### Phase 2.A: The RenderContext MCOs
**Files:** `crates/mco-render-wgpu/`, `crates/mco-render-term/`
Both MCOs must expose the exact same Moof-side capability API.
```moof
(defprotocol RenderContext
  (requires [drawMesh:color:] [drawTexture:] [pushTransform:] [popTransform]))
```
The Wrapper Vat initializes the specific MCO and passes it down the tree:
```moof
[rootPlacement renderWith: $wgpuContext]
```

## 3. Moofpaint: Pixmaps and Pixel Buffers

The `Pixmap` inhabitant must be fast enough for 60Hz drawing.

### Phase 3.A: Memory Layout of Pixel Buffers
**Files:** `crates/mco-pixel-bits/src/lib.rs`
The pixel buffer is a flat `Vec<u8>` in the MCO's memory (Wasm linear memory or native heap). It is *not* represented as a Moof `Table` or `List` (too slow).
1. **Handle Allocation:** `[$pixelBits allocWidth: 1024 height: 1024]` returns an opaque `Form` containing the pointer/index to the buffer.
2. **Fast Write:** `[$pixelBits set: handle x: 10 y: 20 r: 255 g: 0 b: 0 a: 255]` directly mutates the byte array.

### Phase 3.B: GPU Texture Sync
When `[ctx drawTexture: handle]` is called on the `$wgpuContext`:
1. The `wgpu` MCO reads the `Vec<u8>` from the `pixel-bits` MCO memory space.
2. It uploads it to the GPU via `queue.write_texture()`.

## 4. Morphic Halos (Live AST Injection)

The core moldability feature: inspecting and rewriting a Form visually in 3D.

### Phase 4.A: Raycasting and DNU Interception
1. The Wrapper Vat receives a Right-Click. It calls `[$math3d raycast: ray against: rootPlacement]`.
2. It hits `FormA`.
3. The Wrapper Vat sends `[FormA spawnHalo]`. If `FormA` doesn't implement it, it falls through to `Object:spawnHalo`.

### Phase 4.B: The Halo UI and AST Replacement
**Files:** `lib/morphic/halo.moof`, `lib/morphic/text-editor.moof`
1. **Spawn:** `Object:spawnHalo` creates a Ring menu `Placement` around the Form's pose.
2. **Inspect:** Clicking the "Code" handle spawns a `TextEditor` Inhabitant, passing `[[self proto] source]` to it.
3. **Save (The Injection):**
   ```moof
   ;; Inside TextEditor
   [onSave: newSourceText]
     (let newAst [$compiler parse: newSourceText])
     ;; The injection: compile and hot-swap the proto's method dictionary
     (let newMethods [$compiler evaluateProtoBody: newAst])
     [targetProto setHandlers: newMethods]
     ;; Instantly, all 3D objects using this proto exhibit the new behavior.
   ```
**Tests:** `test_raycast_hits_closest_placement`, `test_halo_spawns_at_target_pose`, `test_text_editor_save_updates_proto_handlers`.
