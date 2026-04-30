# pixmap

> **a Pixmap is one inhabitant proto among many. 1-bit raster,
> macpaint-aesthetic, edited in place. Pencil/Eraser/FloodFill/Lasso
> are *Tool* protos that operate on the active pixmap. there is no
> "moofpaint app." there is the world; pixmaps are some of what
> lives in it.**

renamed from `concepts/moofpaint.md` after the v4-take-2 reframe:
moof has no apps. drawing happens because pixmaps are spatially-
placed Forms in the world (`concepts/world-and-space.md`); when you
zoom into one, your tool palette becomes contextually-relevant; you
draw; the strokes update the pixmap's slot; the world's input log
records them; replicas converge.

## the proto

```moof
(defproto Pixmap
  (proto Form)
  (slots width height bits      ;; bits: a packed bit-vector
         strokes                ;; the canonical stroke log
         undo-stacks)           ;; per-author undo
  (handlers
    [render-with: ctx]
      ;; project a 1-bit raster as a textured plane in 3D.
      [(ctx :surface)
        bit-quad-at: (ctx :pose)
        size: #[(.width / 100.0) (.height / 100.0)]
        bits: .bits
        camera: (ctx :camera)]
    [hit-test: ray]
      [self plane-hit-test: ray]    ;; uv in [0,1]² if hit
    [add-stroke: s]
      (do
        [self strokes: [.strokes append: s]]
        [self bits: [self redraw]])
    [redraw]
      ;; pure fold: replay all strokes onto a fresh bit-vector.
      [.strokes reduce-from: [Bits zeros: .width by: .height]
                with: |bits s|
                  [(s tool-proto) apply-to: bits at: s]]))
```

a Pixmap's *visible* state is its `bits`, but its *canonical* state
is the `strokes` log. bits is a derived view; replays of the log
produce identical bits on every replica. (this is the same
discipline as the world-vat overall — state is a fold over inputs.)

## tools

tools are Forms with a `:apply-to:at:` handler:

```moof
(defproto Tool
  (proto Form)
  (handlers
    [apply-to: bits at: stroke]
      ;; default: no-op
      bits))

(defproto Pencil
  (proto Tool)
  (handlers
    [apply-to: bits at: stroke]
      [(stroke segments) reduce-from: bits with: |b seg|
        [b line-from: (seg :start) to: (seg :end)
            weight: (stroke :weight)]]))

(defproto Eraser
  (proto Tool)
  (handlers
    [apply-to: bits at: stroke]
      [(stroke segments) reduce-from: bits with: |b seg|
        [b line-from: (seg :start) to: (seg :end)
            weight: (stroke :weight) value: 0]]))

(defproto FloodFill
  (proto Tool)
  (handlers
    [apply-to: bits at: stroke]
      [bits flood-fill-from: (stroke :seed)
                       with: (stroke :value)]))
```

Pencil/Eraser/Line/Rect/Oval/FloodFill/Lasso/Marquee are all just
protos. user code can define new ones at runtime. live-edit Pencil's
`:apply-to:at:` and the next stroke uses the new code (because proto
edits are turn-envelopes, `laws/determinism-laws.md` D8).

## tool palette

a `ToolPalette` is itself a Form-with-a-view, placed in the world
near where the user is drawing:

```moof
{ToolPalette
  active-tool: Pencil
  tools: #[Pencil Eraser Line Rect Oval FloodFill Lasso]
  ;; rendered as a small floating array of buttons in 3D
  …}
```

clicking a tool button changes the palette's `active-tool` slot.
which palette is "active" for a given user is per-replica state in
the wrapper vat (multiple users can have different active tools at
once).

## the stroke

```moof
{Stroke
  author: 'alice
  tool-proto: Pencil
  weight: 2
  segments: '(... list of {start: vec2 end: vec2}...)
  logical-now: 4789}
```

strokes carry enough information to be replayed deterministically.
position is in pixmap-local 2D coordinates (uv in [0, width] ×
[0, height]).

## input flow

1. user clicks on a Pixmap (in 3D space, with camera somewhere).
2. wrapper vat ray-casts; hit on the pixmap's plane; returns
   `local-uv = (0.42, 0.31)`.
3. wrapper sends `[world-vat input: {PointerOnForm form-id: <pix>
   uv: …}]`.
4. world-vat dispatches `[pixmap pointer-down: uv with: 'left]`.
5. pixmap (consulting the local user's active tool) produces a
   `Stroke` and appends to its log.
6. the stroke change is journaled; broadcast to all replicas via
   the input log.
7. on every replica, the pixmap's bits re-derive on next render.

## per-user undo

the canonical `strokes` log is shared. *per-user undo state* is also
in the canonical log:

```moof
;; alice's undo: pop the last stroke whose author is alice.
[pixmap undo-for: 'alice]
;; → updates an undo-pointer slot, recomputes bits.
```

undo doesn't *delete* strokes; it adds an `Undo {author, target-
stroke-id}` log entry, and the bit-derivation skips the undone
strokes. redo removes the undo-marker.

both alice and bob have separate undo stacks; they don't undo each
other's work.

## non-commutative ops the substrate must handle

what makes pixmap a real test of the substrate (and not just CRDT-
tractable):

- **flood-fill** depends on the pixmap state at logical-time T. if
  alice flood-fills at T=100 and bob draws a stroke at T=99, alice's
  flood operates on the post-stroke state.
- **layers** (when added) require ordered insertion.
- **lasso + drag** modifies the selection's contents in place; order
  matters.
- **proto-edit on a tool** (alice changes Pencil to draw blue
  squiggles) propagates to every subsequent pencil-stroke on every
  replica.

these are the "if this works, the substrate is honest about
determinism" cases.

## scope (v0.1)

inhabitant protos delivered with the canonical pixmap library:

- **Pixmap** — the bitmap container.
- **Tool** — abstract base.
- **Pencil**, **Eraser** — single-pixel-width line tools.
- **Line**, **Rect**, **Oval** — geometric tools.
- **FloodFill** — bucket fill.
- **Lasso**, **Marquee** — selection tools.
- **ToolPalette** — the tool array.

deferred:

- **fill patterns** (macpaint had them; we add later).
- **layers** (initially: each pixmap is one layer; nested-pixmaps
  for layering).
- **antialiasing** (1-bit, doesn't apply).
- **rgba pixmaps** (a separate proto eventually: `RGBAPixmap`).
- **pressure / tilt** (mouse only at first).

## the v4 forcing function (revised, pixmap-centric)

```
$ moof world ./worlds/test-world/
> world boots; opens a viewport in the terminal.
> alice flies the camera to a Pixmap floating in space.
> alice double-clicks; viewport focuses on the pixmap.
> alice's tool palette materializes nearby; she selects Pencil.
> she clicks-drags to draw a squiggle on the pixmap surface.
> she selects FloodFill, clicks an enclosed region.
> she opens a second pixmap, draws on it.
> she navigates to a Counter inhabiting a nearby frame, double-
>   clicks to inspect, edits its slot.
> closes the world.
```

```
$ moof world join wss://localhost:7878
> bob's terminal opens. bob arrives in the same world. sees alice's
> pixmaps + counter. sees alice's cursor (small avatar) flying
> around. zooms into a pixmap, picks up the eraser, erases a corner
> of alice's squiggle.
> alice sees bob's edit appear in real time.
> alice live-edits Pencil to draw blue strokes.
> bob's next pencil stroke is blue.
```

```
$ # close both. $ # reopen. each user wakes, world is restored,
$ # canvas state intact.
```

three things to notice in this demo:

1. **there's no "moofpaint" anywhere.** there's `moof world …`. the
   pixmap is just one inhabitant.
2. **the user navigates the world spatially.** they fly to where
   the pixmap is. they fly to a different location to inspect the
   counter.
3. **everything is live-editable in place.** Pencil's behavior is
   editable; the counter's slot is editable; the pixmap is paintable
   — all using the same gesture vocabulary (zoom in, click,
   manipulate).

## what passes the substrate gates

- **phase D gate (`docs/process/impl-plan-v4.md`)**: two in-process
  replicas of a world-vat containing several inhabitant Forms
  (including pixmaps). 10k random envelopes; every turn the
  canonical-hash matches.
- **phase E gate**: one user, one terminal, can navigate the world,
  edit a pixmap, change a counter, save, restart, observe restored
  state.
- **phase F gate**: two users, two terminals, joined via
  websocket; alice and bob both edit pixmaps; counters; live-edit
  protos; presence visible; reconnect after disconnect; converge.

## inspirations

- **macpaint** (atkinson 1984) — the visual aesthetic and tool
  vocabulary. 1-bit b&w forever.
- **hyperCard** (atkinson 1987) — the "any artifact is editable"
  spirit. our pixmaps borrow hyperCard's idea that drawings *and*
  scripts coexist on the same surface.
- **smalltalk's BitBlt** — kay et al. the pixel-level primitive.
- **morphic** (maloney, smith 1995) — direct-manipulation drawing
  and editing.
- **piccolo / pad++** — zoomable interfaces, where the LOD reveal
  pattern was first formalized.
- **figma, tldraw, excalidraw** — modern collaborative drawing UX
  patterns.
- **rust's `pixels` and `tiny-skia` crates** — how the rendering
  mco realizes blit/rasterize.

## see also

- `concepts/world-and-space.md` — what pixmaps live in.
- `concepts/replication.md` — how multi-user editing converges.
- `concepts/effect-intents.md` — how `$canvas` paints come back as
  receipts.
- `concepts/canvas-and-input.md` — the underlying caps.
- `concepts/compiled-objects.md` — the `pixel-bits` mco that backs
  the bit-vector.
- `docs/process/impl-plan-v4.md` — phases E and F.
