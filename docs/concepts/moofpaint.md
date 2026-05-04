# Moofpaint and the Spatial Interface

> **A 3D, zoomable, collaborative Morphic environment.**

Moof is designed to be a "spatial finder" and a tangible, moldable world. The forcing function for this paradigm is **Moofpaint** — a multi-user, collaborative, MacPaint-inspired spatial application.

## The EagleMode / Croquet Vision

Moof abandons the traditional 2D overlapping-window desktop metaphor in favor of a continuous 3D Zoomable User Interface (ZUI), drawing inspiration from EagleMode and Croquet.

- **Zoom = Fly:** Navigation is not about minimizing and maximizing windows; it's about spatial translation and scaling. You zoom *into* a directory to see its contents; you zoom *into* a Form to see its slots and internal structure.
- **Continuous Space:** Every entity in the Moof environment is an "inhabitant" with a `Pose` (position, orientation, scale in 3D space).

## The Morphic Tangibility

Everything in the world is a Form, and everything is tangible.

- **Direct Manipulation:** There are no hidden configuration files determining how something looks. The visual representation of a Form *is* the Form, constructed via the `:render-with: ctx` protocol.
- **Live Editing (Halos):** In the spirit of Morphic, any object can be inspected at runtime. Clicking a Form brings up its "Halo" — a ring of handles allowing you to resize, rotate, inspect its code, or duplicate it instantly.

## Moofpaint: The Canonical Inhabitant

Moofpaint isn't a separate app; it's a set of capabilities and tools within the world.

- **The Pixmap:** The core primitive is the `Pixmap` proto — a textured plane floating in space. It is backed by a highly optimized native bit-vector MCO for performance.
- **Tools as Forms:** Tools like the Pencil, Eraser, or Color Picker are themselves tangible inhabitants. You "pick up" a tool (assigning it to your spatial pointer) and interact with the Pixmap.
- **Live Collaboration:** Because Moof uses Croquet-style deterministic replicated vats, multiple users can inhabit the same space.
    - **Presence:** You see the 3D cursors (or avatars) of other users.
    - **Immediate Convergence:** When Alice draws a stroke, the input intent is broadcasted, and both Alice and Bob compute the exact same resulting Pixmap deterministically within 50ms.

## Escaping the File System

In this spatial paradigm, "files" do not exist as opaque byte arrays in folders.

- **Spatial Organization:** Information is organized by spatial arrangement. A "folder" is merely a designated spatial region or a bounded container Form that holds other Forms.
- **Persistence is Implicit:** Because the entire heap is persisted via the input journal (ACID message turns), you never "save" a Moofpaint canvas. You simply walk away. When you return, the universe is exactly as you left it.
