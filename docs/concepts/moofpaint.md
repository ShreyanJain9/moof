# Moofpaint and the Spatial Interface: Implementation Plan

> **A 3D, zoomable, collaborative Morphic environment.**

This document outlines the concrete steps to implement Moofpaint, the continuous 3D Zoomable User Interface (ZUI), and the Morphic capabilities that act as the primary interface for Moof.

## 1. The Spatial Primitive: Poses and Frames

Every inhabitant in the world requires a spatial representation. We introduce `Pose` and `Placement` as core Moof protocols, backed by a fast math MCO.

### Phase 1.A: The `math3d.mco` and Spatial Protos
1. **MCO:** Create `core/math3d.mco` (Rust/Wasm) exporting fast matrix/quaternion operations.
2. **Moof Protos:**
   ```moof
   (defproto Pose
     (slots position rotation scale) ;; Vec3, Quaternion, Vec3 handles from MCO
     (handlers
       [translateBy: vec] ...
       [rotateBy: quat] ...
       [matrix] [$math3d toMatrix: self.position rot: self.rotation scale: self.scale]))
   ```

### Phase 1.B: The World Graph
The World is a hierarchical graph of Placements.
```moof
(defproto Placement
  (slots form pose children)
  (handlers
    [addChild: placement] ...
    [worldMatrix] [[self parent] worldMatrix] * [self.pose matrix]))
```

## 2. Rendering Protocol

Rendering is not hardcoded; it is a message sent down the spatial graph.

### Phase 2.A: The `:render-with:` Protocol
Every visible Form must implement the `Renderable` protocol.
```moof
(defprotocol Renderable
  (requires [renderWith: ctx]))

(defproto Cube
  (mixins Renderable)
  (handlers
    [renderWith: ctx]
      [ctx drawMesh: $defaultCubeMesh color: 'gray]))
```

### Phase 2.B: The Wrapper Vat and Render Loop
A local, single-user "Wrapper Vat" manages the connection to the user's hardware.
1. It holds the `$canvas` (wgpu or terminal MCO) and `$pointer` capabilities.
2. Every 16ms (60Hz), it traverses the `Placement` graph and calls `[form renderWith: ctx]`.

## 3. Moofpaint: Pixmaps and Tools

Moofpaint requires fast pixel manipulation and tangible tools.

### Phase 3.A: The `Pixmap` Inhabitant
1. **MCO:** Create `core/pixel-bits.mco` for rapid bit-blitting and texture updates.
2. **Moof Proto:**
   ```moof
   (defproto Pixmap
     (mixins Renderable)
     (slots textureHandle width height)
     (handlers
       [renderWith: ctx]
         [ctx drawTexture: self.textureHandle]
       [setPixelAt: pos color: c]
         [$pixelBits set: self.textureHandle x: pos.x y: pos.y color: c]))
   ```

### Phase 3.B: Spatial Input and Raycasting
The Wrapper Vat captures a mouse click and converts it to a 3D ray.
1. `[$math3d raycast: ray against: rootPlacement] -> HitRecord`
2. If the hit is a `Pixmap`, the Wrapper Vat dispatches an Input Envelope to the Replicated World Vat:
   `{Input event: 'pointerDown target: pixmapFormId localUv: {u v}}`
3. The Pixmap receives the event and interacts with the currently held Tool Form.

## 4. Morphic Halos (Live Editing)

To make the environment moldable, we implement Morphic Halos.

1. **The Halo Invocation:** Right-clicking any Form raycasts to the Form and sends `[form spawnHalo]`.
2. **The Halo Form:** The `spawnHalo` default implementation (on `Object`) creates a circular UI `Placement` surrounding the Form's pose.
3. **Inspect Handle:** Clicking the "Inspect" button on the Halo opens a Text Editor Form in the 3D space, populated with `[[form proto] source]`.
4. **Live Update:** When the user hits save on the Text Editor, it sends `[$compiler compileForm: newAst]` and updates the proto's handler, immediately altering the behavior of the Form in 3D space.
