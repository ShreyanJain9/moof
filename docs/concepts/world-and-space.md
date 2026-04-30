# world and space

> **the world is a 3D zoomable space. every spatially-placed Form
> has a Placement; every Placement lives in a Frame; Frames nest.
> rendering is `[form render-with: ctx]`. navigation is moving the
> camera. inspect is a ray-cast. presence is a Form among forms.
> nothing is a special "ui element" — there is one substrate, with
> uniform protocols, all the way through.**

a moof world is a single, continuous 3D environment that you wake,
inhabit, and let sleep. you don't "open windows" or "launch apps."
you fly to where the thing is, edit it in place, save the world.
this is the croquet/teaTime ambition (kay et al. ~2003) made the
default rather than an experiment, with the moldable inspector
culture of glamorous toolkit (gîrba) carried into 3D.

## the model

```
World
└── Frame (root)              ; coordinate origin, scale = 1
    ├── Placement
    │     pose: { pos: (0, 0, 0), orient: identity, scale: 1 }
    │     form: <Pixmap …>          ; a macpaint-style 1-bit canvas
    ├── Placement
    │     pose: { pos: (3, 0, -2), orient: …, scale: 0.5 }
    │     form: <Counter count: 42>
    ├── Placement
    │     pose: …
    │     form: <Frame>             ; a sub-world; frames nest
    │       └── …
    ├── Placement
    │     form: <Cursor name: 'alice>   ; presence; another inhabitant
    └── …
```

a world is composed of:

- a root `Frame` Form.
- a tree of nested `Frame`s (a frame is a coordinate space; frames
  inside frames are children with their own coordinate origin).
- `Placement` Forms inside frames — each placement is `{form, pose,
  frame}`. the form is what you see; the pose says where; the frame
  says in whose coordinate space.

**every spatially-placed Form has a Placement. unplaced Forms are
ordinary heap citizens but not world-citizens.** a method, a type,
an unmounted scratchpad — all live in the heap, none are visible
in space until placed.

## coordinates

3D, continuous, f64. units are arbitrary but conventional: 1 unit
≈ 1 meter at world origin. the root frame's scale is 1. nested
frames may scale (a frame at scale=0.001 nested in the root is a
"detail world" you fly into).

```moof
{Pose
  position: #[0.0 1.5 -3.0]              ; vec3, world-units
  orientation: #[0.0 0.0 0.0 1.0]        ; unit quaternion (xyz, w)
  scale: 1.0}                            ; isotropic by default
```

scale is a scalar by default but `Pose` accepts `scale: #[sx sy sz]`
for non-uniform if a Form needs it. orientations are quaternions to
avoid gimbal lock and to interpolate cleanly.

## the universal view protocol

every Form-with-a-view answers `:render-with: ctx`. the context
carries everything the renderer needs:

```moof
{RenderContext
  camera: <Camera>                 ; pose, projection, fov
  viewport: <Viewport>             ; width, height, pixel-density
  frame: <Frame>                   ; the frame this form lives in
  pose: <Pose>                     ; the form's placement pose
  detail: <Detail-budget>          ; LOD hint: pixels-this-form-occupies
  surface: $canvas                 ; the cap to draw to
  …}
```

a render handler does whatever it likes with this — typically:

```moof
(defproto Pixmap
  (proto Form)
  (slots width height bits)
  (handlers
    [render-with: ctx]
      ;; project the bits onto a textured plane in 3D.
      [(ctx :surface)
        textured-quad-at: (ctx :pose)
        size: #[(.width / 100.0) (.height / 100.0)]
        texture: .bits
        camera: (ctx :camera)]))
```

```moof
(defproto Counter
  (proto Form)
  (slots count)
  (handlers
    [render-with: ctx]
      ;; render the integer as a 3D extruded numeral.
      [(ctx :surface)
        text-extrude: [.count to-string]
        at: (ctx :pose)
        depth: 0.05
        font: 'chicago-bold])
    [incr]
      [self count: [.count + 1]]))
```

```moof
(defproto Cube
  (proto Form)
  (slots size color)
  (handlers
    [render-with: ctx]
      [(ctx :surface)
        cube-at: (ctx :pose)
        size: .size
        color: .color]))
```

a Frame's render iterates its placements:

```moof
(defproto Frame
  (proto Form)
  (slots placements name)
  (handlers
    [render-with: ctx]
      [.placements for-each: |p|
        [(p :form) render-with:
          [ctx with-pose: [ctx pose] composed-with: (p :pose)]]]))
```

rendering is recursive. composition of poses follows standard 3D
transform rules.

## level of detail

`ctx.detail` is the renderer's budget. a Form occupying ten pixels
on screen renders nothing fancy; a Form occupying half the screen
renders its full inspector view. forms decide their own LOD strategy:

```moof
(defproto Counter
  …
  (handlers
    [render-with: ctx]
      (cond
        [[(ctx :detail) < 16]      [(ctx :surface) dot-at: (ctx :pose)]]
        [[(ctx :detail) < 100]     [(ctx :surface) text: [.count to-string] at: (ctx :pose)]]
        [#true                     [(ctx :surface) full-counter-view: self at: (ctx :pose)]])))
```

LOD is the moldable hook for "what does this Form look like at
arm's length vs across the room vs zoomed-in to the slot detail?"
the inspector view at extreme zoom is *just another LOD level* of
the form's render. *zooming in IS opening the inspector.* (this is
piccolo zoomable-ui (univ. of maryland, ~2002) made first-class.)

## cameras and viewports

a viewport is one observer's window into the world. each replica
has at least one viewport.

```moof
{Camera
  pose: <Pose>                     ; where the camera is
  projection: 'perspective         ; or 'orthographic
  fov: 60.0                        ; degrees, perspective
  near: 0.01
  far: 1000.0}

{Viewport
  camera: <Camera>
  width: 1920                      ; pixels
  height: 1080
  pixel-density: 2.0               ; HiDPI scale}
```

**viewports are per-replica** (`concepts/replication.md`). alice's
camera might be aimed at a Pixmap; bob's at a Cube three frames
away. they share the world; they do not share where each is
looking. moving the camera is *not* a replicated input — it's a
local viewport mutation.

a viewport lives in the *wrapper vat* (solo, per-replica). the
wrapper holds `$canvas` and `$pointer` caps; the viewport
translates between screen-space and world-space.

## navigation

navigation is a small set of canonical gestures, all routed through
`$pointer` and `$keyboard`:

| gesture | action |
|---|---|
| right-drag | orbit camera around hover-target |
| middle-drag | pan camera in view-plane |
| scroll / pinch | dolly camera along view direction (zoom) |
| double-click on a Form | "focus" — fly camera to frame the Form |
| `wasd` + look | first-person fly |

these are wrapper-vat behaviors; the world doesn't see them. only
state changes (a stroke; a slot-edit; a new placement) become input
envelopes.

## ray-cast and hit-test

clicking a pixel produces a *world-ray* (origin = camera, direction
= through-pixel). the wrapper vat ray-casts against the visible
frames, returning the first-hit Placement plus a hit-point in that
form's local coordinate space.

```
[viewport pick: #[mouse-x mouse-y]]
;; → {Hit
;;     placement: <P>
;;     form: <F>
;;     world-point: vec3
;;     local-point: vec3            ; in F's coords
;;     local-uv: vec2}               ; if F is parameterizable
```

the wrapper translates a click into a world-vat input envelope:

```
{PointerOnForm
  form-id: <F's id>
  uv: (0.42, 0.31)
  buttons: #{'left}
  modifiers: #{}}
```

the world-vat receives this and dispatches `[F clicked-at: uv with:
buttons]`. the world-vat doesn't know about pixels; the wrapper
doesn't know about Form semantics. clean split.

every Form can answer `:hit-test:ray:` to participate in picking;
the default tests against an axis-aligned bounding box.

## presence

a Cursor is a Placement — a small Form with `:render-with:` showing
a labeled marker (an arrow + name), placed at the user's gaze
target. as the user moves the camera, their replica sends
`CursorMove { logical-now, pose }` envelopes. all replicas update
the placement; everyone sees alice's arrow drift.

```moof
(defproto Cursor
  (proto Form)
  (slots author name color gaze-target)
  (handlers
    [render-with: ctx]
      ;; small translucent arrow + text label.
      …
    [moved-to: pose]
      [self gaze-target: pose]))
```

cursors are *first-class inhabitants*. nothing about them is a
special-cased "presence layer." they are placements, they're
rendered like everything else, you can click on alice's cursor, you
can give alice's cursor a sticker. (the latter would be silly, but
the substrate doesn't preclude it.)

## the wrapper vat

per-replica, one or more wrapper vats translate between the
substrate (replicated world) and the OS (terminal, gpu, mouse,
keyboard). responsibilities:

- holds local `$canvas`, `$pointer`, `$keyboard`, `$clock` (wall),
  `$random` (os) caps.
- holds the local viewport, camera, render loop.
- translates pointer events to ray-casts; sends hits as input
  envelopes.
- translates keyboard events into world-input envelopes (when they
  cause world-state change) or local-only viewport changes
  (otherwise).
- subscribes to the world-vat's relevant placements; re-renders
  on tick.

solo, never replicated. one per machine per session.

## interaction with replication

| state | replicated? |
|---|---|
| frames + their nesting | yes |
| placements + their poses | yes |
| forms (counters, pixmaps, etc.) | yes (their slots) |
| cursors (as Forms) | yes |
| viewports + cameras | **no** (per-replica) |
| selection (highlight) | **no** (per-replica) |
| render output (pixels) | **no** (per-replica) |
| keyboard / mouse buffers | **no** (per-replica edge) |

the world-vat's `canonical-hash` includes everything in the world
but excludes per-replica viewport state. two replicas of the same
world are bit-identical; their windows on it are not.

## interaction with capabilities

the *world-vat* (replicated) holds **no OS caps**. it is a pure
deterministic state machine over the input log
(`laws/determinism-laws.md`). all actual rendering, input, sound,
file i/o happens in *wrapper vats* (solo) that translate between
the abstract world and the local hardware.

a Form can request rendering by sending `[$render request: self]`,
which becomes an `EffectIntent`
(`concepts/effect-intents.md`) that the wrapper vat reads. typically
forms do *not* directly drive rendering; the wrapper vat just polls
the world's frame tree on each frame.

## interaction with persistence

the world-vat persists like any other vat
(`concepts/persistence.md`): canonical input log + snapshot. on
reboot, the world replays its log, all placements + poses + form
states converge.

per-replica viewport state (camera, selections, scroll position)
persists *separately* in the wrapper vat's own per-vat directory.
when alice reopens the world, she finds her camera right where she
left it; bob's camera is wherever he last had his.

## what kinds of inhabitants are typical

- **Pixmap** — 1-bit raster, macpaint-shaped (`concepts/pixmap.md`).
- **TextNote** — a flat editable string rendered as a 3D plane.
- **Counter** — the canonical "small object," a 3D extruded numeral.
- **Cube / Sphere / Mesh** — primitive 3D shapes.
- **Source** — a moof source-form rendered as code-on-a-plane;
  click to live-edit.
- **Inspector** — a form-rendering of another form's slots and
  handlers (the inspector is *itself* an inhabitant, placed in the
  world; you can have multiple open).
- **Frame** — a sub-world; fly into it for a magnified detail-world.
- **Cursor** — a presence.
- **Tool palette** — a 3D button-array; clicking activates a tool.

every one is just a Form. the substrate has no special-cased "UI
elements." the visual language is whatever the protos and their
`:render-with:` handlers compose.

## why 3D and not 2D

three reasons:

1. **zoom is more legible in 3D.** dollying a camera through space
   gives natural occlusion and parallax. 2D zoom (piccolo-style)
   approximates this with scale-invariant rendering, which works
   for some content but fails for arrangements where stacking
   *means* something.
2. **multiple users feel each other better in 3D.** seeing alice's
   cursor *behind* a frame is more spatial than seeing it
   stamped-on-top of a 2D plane. presence reads better.
3. **the renderer cost isn't different.** wgpu (via mco) is 3D-
   first; rendering 2D in a 3D scene is a 2D quad. so 3D-by-default
   imposes ~zero extra cost while opening a much wider design space.

what 3D *doesn't* mean: you don't have to build VR or use gamepads
or model gravity. the default look is "drawings floating in a
luminous room"; users navigate with a mouse and keyboard like
google-earth or sketchfab. VR is a future renderer-MCO; gamepads
are a future input-MCO; physics is a far-future inhabitant proto.

## inspirations

- **croquet / teaTime**: kay, reed, smith, lombardi, ducasse,
  miller ~2003. shared 3D worlds with deterministic-replicated
  models. the *direct* ancestor of moof's design.
- **morphic**: maloney, smith 1995. directly-graspable visual
  objects with halos and handles. moof's `:render-with:` is morphic
  in 3D.
- **piccolo / jazz**: bederson, hollan et al., univ. of maryland
  ~2002. zoomable user interface. inspired the LOD-as-zoom
  primitive.
- **lively kernel**: ingalls et al. 2007. morphic in the browser;
  proves these primitives work over a network.
- **second life / open simulator**: rosedale et al. 2003. shared 3D
  user-built worlds. moof differs by being *moldable down to the
  substrate*, not just the asset library.
- **glamorous toolkit**: gîrba et al. moldable per-object views;
  in moof those views are 3D.
- **HyperCard** (atkinson 1987) for the spirit: any artifact you
  encounter is editable in place, no separate dev mode.
- **genera** (symbolics, 1980s): presentations as live objects;
  moof's render-with-context is a 3D presentation framework.

## see also

- `concepts/forms.md` — what's being placed.
- `concepts/replication.md` — what's replicated vs per-replica.
- `concepts/effect-intents.md` — render as effect.
- `concepts/canvas-and-input.md` — `$canvas`, `$pointer` mechanics.
- `concepts/pixmap.md` — one inhabitant proto.
- `concepts/moldability.md` — the *spirit* of inhabit-and-edit.
